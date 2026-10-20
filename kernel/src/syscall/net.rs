//! Network system calls (P5-05). Sockets are descriptors in the same table as files: `read`/`write`
//! on a TCP descriptor receive/send on the connection, `close` ends it (and a program's sockets are
//! closed when it exits). Addresses travel as u32 in network order packed big-endian (a.b.c.d = a<<24|...).

use super::{E_FAULT, E_INVALID, E_TOO_MANY, user_path, user_range_ok};
use crate::net::{self, Ipv4, dns, tcp, udp};
use crate::proc::{OpenFile, Socket};

pub const E_TIMEOUT: i64 = -17; // no answer in time
pub const E_REFUSED: i64 = -18; // connection refused or reset
pub const E_NO_NETWORK: i64 = -19; // no card or no address
pub const E_CLOSED: i64 = -20; // the connection is closed
pub const E_NO_NAME: i64 = -21; // the name does not exist

/// How long a `read` on a TCP descriptor waits for data before giving up.
pub const READ_TIMEOUT_MS: u64 = 30_000;

fn ip_of(v: u64) -> Ipv4 {
    (v as u32).to_be_bytes()
}

fn tcp_error(e: tcp::Error) -> i64 {
    match e {
        tcp::Error::NoNetwork => E_NO_NETWORK,
        tcp::Error::Refused => E_REFUSED,
        tcp::Error::Timeout => E_TIMEOUT,
        tcp::Error::Closed => E_CLOSED,
    }
}

fn add(socket: Socket) -> i64 {
    match crate::proc::add_file(OpenFile { path: alloc::string::String::new(), offset: 0, socket: Some(socket) }) {
        Some(fd) => fd as i64,
        None => E_TOO_MANY,
    }
}

/// The socket behind descriptor `fd` (if it is one).
pub fn socket_of(fd: u64) -> Option<Socket> {
    let index = (fd as usize).checked_sub(crate::proc::FIRST_FD)?;
    crate::proc::with_files(|f| f.get(index).and_then(|o| o.as_ref()).and_then(|o| o.socket)).flatten()
}

/// resolve(name, len): the IPv4 address of a name (or of "a.b.c.d").
pub fn resolve(ptr: u64, len: u64) -> i64 {
    let name = match user_path(ptr, len) {
        Ok(n) => n,
        Err(e) => return e,
    };
    match dns::resolve(&name) {
        Ok((ip, _)) => u32::from_be_bytes(ip) as i64,
        Err(dns::Error::NotFound) | Err(dns::Error::BadName) => E_NO_NAME,
        Err(dns::Error::NoServer) => E_NO_NETWORK,
        Err(dns::Error::Timeout) => E_TIMEOUT,
    }
}

/// tcp_connect(ip, port, timeout_ms): opens a connection; returns its descriptor.
pub fn tcp_connect(ip: u64, port: u64, timeout_ms: u64) -> i64 {
    if port == 0 || port > u16::MAX as u64 {
        return E_INVALID;
    }
    match tcp::connect(ip_of(ip), port as u16, timeout_ms.clamp(100, 60_000)) {
        Ok(id) => {
            let fd = add(Socket::Tcp(id));
            if fd < 0 {
                tcp::close(id, 0);
            }
            fd
        }
        Err(e) => tcp_error(e),
    }
}

/// TCP read: up to `n` bytes into `out` (0 = the server closed its side).
pub fn tcp_read(id: u32, out: &mut [u8]) -> i64 {
    match tcp::recv(id, out.len(), READ_TIMEOUT_MS) {
        Ok(d) => {
            out[..d.len()].copy_from_slice(&d);
            d.len() as i64
        }
        Err(e) => tcp_error(e),
    }
}

/// TCP write: queues all of `data`.
pub fn tcp_write(id: u32, data: &[u8]) -> i64 {
    match tcp::send(id, data, READ_TIMEOUT_MS) {
        Ok(()) => data.len() as i64,
        Err(e) => tcp_error(e),
    }
}

/// udp_open(port): opens a UDP port (0 = any free one); returns its descriptor.
pub fn udp_open(port: u64) -> i64 {
    if port > u16::MAX as u64 {
        return E_INVALID;
    }
    if net::config().is_none() {
        return E_NO_NETWORK;
    }
    let port = if port != 0 {
        if !udp::bind(port as u16) {
            return E_INVALID;
        }
        port as u16
    } else {
        match (0..64).map(|_| udp::temp_port()).find(|p| udp::bind(*p)) {
            Some(p) => p,
            None => return E_TOO_MANY,
        }
    };
    let fd = add(Socket::Udp(port));
    if fd < 0 {
        udp::unbind(port);
    }
    fd
}

/// udp_send(fd, buf, len, ip, port): sends one datagram.
pub fn udp_send(fd: u64, buf: u64, len: u64, ip: u64, port: u64) -> i64 {
    let Some(Socket::Udp(local)) = socket_of(fd) else { return super::E_BAD_FD };
    if len > 1472 || port == 0 || port > u16::MAX as u64 {
        return E_INVALID;
    }
    if !user_range_ok(buf, len, false) {
        return E_FAULT;
    }
    // SAFETY: the range was just checked to be mapped user memory of the current address space.
    let data = alloc::vec::Vec::from(unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) });
    if udp::send(local, ip_of(ip), port as u16, &data) { len as i64 } else { E_NO_NETWORK }
}

/// udp_recv(fd, buf, len, timeout_ms, from): waits for one datagram; writes its sender (ip u32, port u16,
/// 6 bytes big-endian) at `from` if not 0. Returns the bytes copied (the rest of a longer datagram is lost).
pub fn udp_recv(fd: u64, buf: u64, len: u64, timeout_ms: u64, from: u64) -> i64 {
    let Some(Socket::Udp(local)) = socket_of(fd) else { return super::E_BAD_FD };
    let len = len.min(64 * 1024);
    if !user_range_ok(buf, len, true) || (from != 0 && !user_range_ok(from, 6, true)) {
        return E_FAULT;
    }
    let Some((src, sport, data)) = udp::receive(local, timeout_ms.min(60_000)) else { return E_TIMEOUT };
    let n = data.len().min(len as usize);
    // SAFETY: both ranges were checked to be writable user memory of the current address space.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, n);
        if from != 0 {
            let mut f = [0u8; 6];
            f[..4].copy_from_slice(&src);
            f[4..].copy_from_slice(&sport.to_be_bytes());
            core::ptr::copy_nonoverlapping(f.as_ptr(), from as *mut u8, 6);
        }
    }
    n as i64
}

/// Size of the record `net_info` writes.
pub const INFO_LEN: usize = 48;

/// net_info(buf, len): the interface, 48 bytes: MAC (6), 2 zero, IP (4), mask (4), gateway (4), DNS (4),
/// lease seconds left (4, 0 = none), frames sent (8), frames received (8), 4 zero. 0 if there is no card.
pub fn net_info(buf: u64, len: u64) -> i64 {
    if len < INFO_LEN as u64 || !user_range_ok(buf, INFO_LEN as u64, true) {
        return E_FAULT;
    }
    let Some(cfg) = net::config() else { return 0 };
    let (sent, received) = crate::drivers::e1000::NIC.lock().as_ref().map_or((0, 0), |n| (n.sent, n.received));
    let mut r = [0u8; INFO_LEN];
    r[0..6].copy_from_slice(&cfg.mac);
    r[8..12].copy_from_slice(&cfg.ip);
    r[12..16].copy_from_slice(&cfg.mask);
    r[16..20].copy_from_slice(&cfg.gateway);
    r[20..24].copy_from_slice(&cfg.dns);
    r[24..28].copy_from_slice(&net::lease_left_s().to_be_bytes());
    r[28..36].copy_from_slice(&(sent as u64).to_be_bytes());
    r[36..44].copy_from_slice(&(received as u64).to_be_bytes());
    // SAFETY: [buf, buf + INFO_LEN) was checked to be writable user memory.
    unsafe { core::ptr::copy_nonoverlapping(r.as_ptr(), buf as *mut u8, INFO_LEN) };
    INFO_LEN as i64
}

/// ping(ip, seq, timeout_ms): one echo request; the round-trip time in ms.
pub fn ping(ip: u64, seq: u64, timeout_ms: u64) -> i64 {
    if !net::ready() {
        return E_NO_NETWORK;
    }
    match net::icmp::ping(ip_of(ip), seq as u16, timeout_ms.clamp(10, 10_000)) {
        Some(ms) => ms as i64,
        None => E_TIMEOUT,
    }
}
