//! ICMP (RFC 792): answers ping (echo request) and sends our own pings.

use super::{Ipv4, checksum, ipv4};
use crate::arch::x86_64::apic;
use crate::sched::{Mutex, WaitQueue};
use alloc::vec::Vec;

/// Echo replies received: (from, identifier, sequence).
static REPLIES: Mutex<Vec<(Ipv4, u16, u16)>> = Mutex::new(Vec::new());
static REPLY: WaitQueue = WaitQueue::new();
const ID: u16 = 0xC0DE;

/// An echo message (type 8 = request, 0 = reply) with a correct checksum.
pub fn echo(kind: u8, id: u16, seq: u16, data: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(8 + data.len());
    m.extend_from_slice(&[kind, 0, 0, 0]);
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&seq.to_be_bytes());
    m.extend_from_slice(data);
    let sum = checksum(&m);
    m[2..4].copy_from_slice(&sum.to_be_bytes());
    m
}

/// A received ICMP message from `src`.
pub fn handle(src: Ipv4, m: &[u8]) {
    if m.len() < 8 || checksum(m) != 0 {
        return;
    }
    let (id, seq) = (u16::from_be_bytes([m[4], m[5]]), u16::from_be_bytes([m[6], m[7]]));
    match m[0] {
        8 => {
            ipv4::send(src, ipv4::PROTO_ICMP, &echo(0, id, seq, &m[8..]));
        }
        0 => {
            REPLIES.lock().push((src, id, seq));
            REPLY.wake_all();
        }
        _ => {}
    }
}

/// Sends one ping to `dst` and waits up to `timeout_ms`. Returns the round-trip time in milliseconds.
pub fn ping(dst: Ipv4, seq: u16, timeout_ms: u64) -> Option<u64> {
    let start = apic::ticks();
    if !ipv4::send(dst, ipv4::PROTO_ICMP, &echo(8, ID, seq, b"Cyber Universe ping")) {
        return None;
    }
    let deadline = start + (timeout_ms * apic::HZ).div_ceil(1000);
    let mut got = false;
    REPLY.wait_until_or(deadline, || {
        if let Some(mut r) = REPLIES.try_lock() {
            if let Some(i) = r.iter().position(|&e| e == (dst, ID, seq)) {
                r.swap_remove(i);
                got = true;
            }
        }
        got
    });
    got.then(|| (apic::ticks() - start) * 1000 / apic::HZ)
}
