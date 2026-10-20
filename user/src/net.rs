//! Network for programs (P5-05): name resolution, TCP connections, UDP ports, interface information
//! and ping. No heap: data goes through the caller's buffers. Addresses are `[u8; 4]`.

use crate::{SYS_CLOSE, SYS_READ, SYS_WRITE, syscall3, syscall5};

pub const SYS_RESOLVE: u64 = 25;
pub const SYS_TCP_CONNECT: u64 = 26;
pub const SYS_UDP_OPEN: u64 = 27;
pub const SYS_UDP_SEND: u64 = 28;
pub const SYS_UDP_RECV: u64 = 29;
pub const SYS_NET_INFO: u64 = 30;
pub const SYS_PING: u64 = 31;

pub const E_TIMEOUT: i64 = -17;
pub const E_REFUSED: i64 = -18;
pub const E_NO_NETWORK: i64 = -19;
pub const E_CLOSED: i64 = -20;
pub const E_NO_NAME: i64 = -21;

pub type Ipv4 = [u8; 4];

fn packed(ip: Ipv4) -> u64 {
    u32::from_be_bytes(ip) as u64
}

/// The address of `name` (a DNS name or "a.b.c.d").
pub fn resolve(name: &str) -> Result<Ipv4, i64> {
    let r = syscall3(SYS_RESOLVE, name.as_ptr() as u64, name.len() as u64, 0);
    if r < 0 { Err(r) } else { Ok((r as u32).to_be_bytes()) }
}

/// A TCP connection; closed when dropped.
pub struct TcpStream {
    fd: i64,
}

impl TcpStream {
    /// Connects to `ip`:`port`, waiting up to `timeout_ms`.
    pub fn connect(ip: Ipv4, port: u16, timeout_ms: u64) -> Result<TcpStream, i64> {
        let fd = syscall3(SYS_TCP_CONNECT, packed(ip), port as u64, timeout_ms);
        if fd < 0 { Err(fd) } else { Ok(TcpStream { fd }) }
    }

    /// Sends all of `data`.
    pub fn write_all(&mut self, mut data: &[u8]) -> Result<(), i64> {
        while !data.is_empty() {
            let chunk = &data[..data.len().min(64 * 1024)];
            let n = syscall3(SYS_WRITE, self.fd as u64, chunk.as_ptr() as u64, chunk.len() as u64);
            if n < 0 {
                return Err(n);
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    /// Reads what arrived (waits up to 30 s); `Ok(0)` = the server closed the connection.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, i64> {
        let n = syscall3(SYS_READ, self.fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        if n < 0 { Err(n) } else { Ok(n as usize) }
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        syscall3(SYS_CLOSE, self.fd as u64, 0, 0);
    }
}

/// An open UDP port; closed when dropped.
pub struct UdpSocket {
    fd: i64,
}

impl UdpSocket {
    /// Opens `port` (0 = any free port).
    pub fn bind(port: u16) -> Result<UdpSocket, i64> {
        let fd = syscall3(SYS_UDP_OPEN, port as u64, 0, 0);
        if fd < 0 { Err(fd) } else { Ok(UdpSocket { fd }) }
    }

    /// Sends one datagram (at most 1472 bytes) to `ip`:`port`.
    pub fn send_to(&self, data: &[u8], ip: Ipv4, port: u16) -> Result<(), i64> {
        let r = syscall5(SYS_UDP_SEND, self.fd as u64, data.as_ptr() as u64, data.len() as u64, packed(ip), port as u64);
        if r < 0 { Err(r) } else { Ok(()) }
    }

    /// Waits up to `timeout_ms` for a datagram: (bytes copied into `buf`, sender address, sender port).
    pub fn recv_from(&self, buf: &mut [u8], timeout_ms: u64) -> Result<(usize, Ipv4, u16), i64> {
        let mut from = [0u8; 6];
        let n = syscall5(SYS_UDP_RECV, self.fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, timeout_ms, from.as_mut_ptr() as u64);
        if n < 0 {
            return Err(n);
        }
        Ok((n as usize, [from[0], from[1], from[2], from[3]], u16::from_be_bytes([from[4], from[5]])))
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        syscall3(SYS_CLOSE, self.fd as u64, 0, 0);
    }
}

/// The network interface as the kernel sees it.
pub struct Info {
    pub mac: [u8; 6],
    pub ip: Ipv4,
    pub mask: Ipv4,
    pub gateway: Ipv4,
    pub dns: Ipv4,
    /// Seconds left on the DHCP lease (0 = none).
    pub lease_s: u32,
    pub sent: u64,
    pub received: u64,
}

/// The interface, or `None` if there is no network card.
pub fn info() -> Option<Info> {
    let mut r = [0u8; 48];
    if syscall3(SYS_NET_INFO, r.as_mut_ptr() as u64, r.len() as u64, 0) <= 0 {
        return None;
    }
    let ip = |i: usize| [r[i], r[i + 1], r[i + 2], r[i + 3]];
    let u64_at = |i: usize| u64::from_be_bytes(r[i..i + 8].try_into().unwrap());
    Some(Info {
        mac: [r[0], r[1], r[2], r[3], r[4], r[5]],
        ip: ip(8),
        mask: ip(12),
        gateway: ip(16),
        dns: ip(20),
        lease_s: u32::from_be_bytes([r[24], r[25], r[26], r[27]]),
        sent: u64_at(28),
        received: u64_at(36),
    })
}

/// One ping to `ip`: the round-trip time in milliseconds.
pub fn ping(ip: Ipv4, seq: u16, timeout_ms: u64) -> Result<u64, i64> {
    let r = syscall3(SYS_PING, packed(ip), seq as u64, timeout_ms);
    if r < 0 { Err(r) } else { Ok(r as u64) }
}

/// "a.b.c.d" -> address.
pub fn parse_ip(s: &str) -> Option<Ipv4> {
    let mut ip = [0u8; 4];
    let mut parts = s.split('.');
    for b in ip.iter_mut() {
        *b = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(ip)
}
