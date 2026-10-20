//! ARP (RFC 826): finds the Ethernet address of an IPv4 neighbour and answers for our own address.
//! Answers are kept for 5 minutes.

use super::{ETH_ARP, Ipv4, Mac, config, send_frame};
use crate::arch::x86_64::apic;
use crate::sched::{Mutex, WaitQueue};
use alloc::vec::Vec;

const KEEP_TICKS: u64 = 300 * apic::HZ;

/// (address, Ethernet address, tick until which it is valid).
static CACHE: Mutex<Vec<(Ipv4, Mac, u64)>> = Mutex::new(Vec::new());
static ANSWER: WaitQueue = WaitQueue::new();

fn packet(op: u16, dst_mac: Mac, dst_ip: Ipv4) -> Option<Vec<u8>> {
    let cfg = config()?;
    let mut p = Vec::with_capacity(28);
    p.extend_from_slice(&[0, 1, 0x08, 0, 6, 4]); // Ethernet, IPv4, address lengths
    p.extend_from_slice(&op.to_be_bytes());
    p.extend_from_slice(&cfg.mac);
    p.extend_from_slice(&cfg.ip);
    p.extend_from_slice(&dst_mac);
    p.extend_from_slice(&dst_ip);
    Some(p)
}

fn remember(ip: Ipv4, mac: Mac) {
    let until = apic::ticks() + KEEP_TICKS;
    let mut cache = CACHE.lock();
    match cache.iter_mut().find(|e| e.0 == ip) {
        Some(e) => *e = (ip, mac, until),
        None => cache.push((ip, mac, until)),
    }
    drop(cache);
    ANSWER.wake_all();
}

fn cached(ip: Ipv4) -> Option<Mac> {
    let now = apic::ticks();
    let mut cache = CACHE.lock();
    cache.retain(|e| e.2 > now);
    cache.iter().find(|e| e.0 == ip).map(|e| e.1)
}

/// A received ARP packet.
pub fn handle(p: &[u8]) {
    let Some(cfg) = config() else { return };
    if p.len() < 28 || p[0..6] != [0, 1, 0x08, 0, 6, 4] {
        return;
    }
    let op = u16::from_be_bytes([p[6], p[7]]);
    let sender_mac: Mac = p[8..14].try_into().unwrap();
    let sender_ip: Ipv4 = p[14..18].try_into().unwrap();
    let target_ip: Ipv4 = p[24..28].try_into().unwrap();
    if target_ip != cfg.ip {
        return;
    }
    remember(sender_ip, sender_mac);
    if op == 1 {
        if let Some(reply) = packet(2, sender_mac, sender_ip) {
            send_frame(sender_mac, ETH_ARP, &reply);
        }
    }
}

/// The Ethernet address of `ip` (asking the network if needed), within `timeout_ms`.
pub fn resolve(ip: Ipv4, timeout_ms: u64) -> Option<Mac> {
    if ip == [255; 4] {
        return Some([0xFF; 6]);
    }
    if let Some(mac) = cached(ip) {
        return Some(mac);
    }
    let deadline = apic::ticks() + (timeout_ms * apic::HZ).div_ceil(1000);
    let mut next_ask = 0;
    loop {
        let now = apic::ticks();
        if now >= deadline {
            return None;
        }
        if now >= next_ask {
            send_frame([0xFF; 6], ETH_ARP, &packet(1, [0; 6], ip)?);
            next_ask = now + apic::HZ / 2; // ask again every half second
        }
        let mut found = None;
        ANSWER.wait_until_or(deadline.min(next_ask), || {
            found = CACHE.try_lock().and_then(|c| c.iter().find(|e| e.0 == ip).map(|e| e.1));
            found.is_some()
        });
        if found.is_some() {
            return found;
        }
    }
}
