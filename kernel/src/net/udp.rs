//! UDP (RFC 768): datagrams to and from ports. A port is opened with `bind`; what arrives for it waits
//! in a queue read with `receive`.

use super::{Ipv4, config, ipv4};
use crate::arch::x86_64::apic;
use crate::sched::{Mutex, WaitQueue};
use alloc::{collections::VecDeque, vec::Vec};

/// A received datagram: (from address, from port, data).
pub type Datagram = (Ipv4, u16, Vec<u8>);

/// Open ports and their queues.
static PORTS: Mutex<Vec<(u16, VecDeque<Datagram>)>> = Mutex::new(Vec::new());
static ARRIVED: WaitQueue = WaitQueue::new();
const QUEUE_MAX: usize = 64;

static NEXT_TEMP: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

/// A candidate temporary port (49152..65535); `bind` tells whether it is free.
pub fn temp_port() -> u16 {
    use core::sync::atomic::Ordering;
    if NEXT_TEMP.load(Ordering::Relaxed) == 0 {
        NEXT_TEMP.store((apic::ticks() % 8192) as u16 + 1, Ordering::Relaxed);
    }
    49152 + NEXT_TEMP.fetch_add(1, Ordering::Relaxed) % 16384
}

/// Opens `port` for receiving. False if it is already open.
pub fn bind(port: u16) -> bool {
    let mut ports = PORTS.lock();
    if ports.iter().any(|p| p.0 == port) {
        return false;
    }
    ports.push((port, VecDeque::new()));
    true
}

pub fn unbind(port: u16) {
    PORTS.lock().retain(|p| p.0 != port);
}

fn udp_checksum(src: Ipv4, dst: Ipv4, datagram: &[u8]) -> u16 {
    ipv4::pseudo_checksum(src, dst, ipv4::PROTO_UDP, datagram)
}

/// Sends `data` from our port `src_port` to `dst`:`dst_port`.
pub fn send(src_port: u16, dst: Ipv4, dst_port: u16, data: &[u8]) -> bool {
    let Some(cfg) = config() else { return false };
    let mut d = Vec::with_capacity(8 + data.len());
    d.extend_from_slice(&src_port.to_be_bytes());
    d.extend_from_slice(&dst_port.to_be_bytes());
    d.extend_from_slice(&((8 + data.len()) as u16).to_be_bytes());
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(data);
    let sum = match udp_checksum(cfg.ip, dst, &d) {
        0 => 0xFFFF, // 0 means "no checksum" in UDP
        s => s,
    };
    d[6..8].copy_from_slice(&sum.to_be_bytes());
    ipv4::send(dst, ipv4::PROTO_UDP, &d)
}

/// A received UDP datagram (inside an IPv4 packet from `src` to `dst`).
pub fn handle(src: Ipv4, dst: Ipv4, d: &[u8]) {
    if d.len() < 8 {
        return;
    }
    let len = u16::from_be_bytes([d[4], d[5]]) as usize;
    if len < 8 || len > d.len() {
        return;
    }
    let d = &d[..len];
    if u16::from_be_bytes([d[6], d[7]]) != 0 && udp_checksum(src, dst, d) != 0 {
        return;
    }
    let (src_port, dst_port) = (u16::from_be_bytes([d[0], d[1]]), u16::from_be_bytes([d[2], d[3]]));
    let mut ports = PORTS.lock();
    if let Some((_, q)) = ports.iter_mut().find(|p| p.0 == dst_port) {
        if q.len() < QUEUE_MAX {
            q.push_back((src, src_port, d[8..].to_vec()));
        }
        drop(ports);
        ARRIVED.wake_all();
    }
}

/// The next datagram for `port`, waiting up to `timeout_ms`.
pub fn receive(port: u16, timeout_ms: u64) -> Option<Datagram> {
    let deadline = apic::ticks() + (timeout_ms * apic::HZ).div_ceil(1000);
    let mut got = None;
    ARRIVED.wait_until_or(deadline, || {
        if let Some(mut ports) = PORTS.try_lock() {
            got = ports.iter_mut().find(|p| p.0 == port).and_then(|p| p.1.pop_front());
        }
        got.is_some()
    });
    got
}
