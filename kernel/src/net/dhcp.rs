//! DHCP client (RFC 2131, options from RFC 2132): asks the network for our address, mask, gateway and
//! name server, and renews the lease when half of it has passed.

use super::{Ipv4, config, set_config, udp};
use crate::arch::x86_64::apic;
use alloc::vec::Vec;

const CLIENT: u16 = 68;
const SERVER: u16 = 67;
const DISCOVER: u8 = 1;
const OFFER: u8 = 2;
const REQUEST: u8 = 3;
const ACK: u8 = 5;
const NAK: u8 = 6;

/// What the server gave us.
#[derive(Clone, Copy, Debug)]
pub struct Lease {
    pub ip: Ipv4,
    pub mask: Ipv4,
    pub gateway: Ipv4,
    pub dns: Ipv4,
    pub server: Ipv4,
    pub seconds: u32,
}

fn message(kind: u8, xid: u32, ciaddr: Ipv4, requested: Option<(Ipv4, Ipv4)>) -> Vec<u8> {
    let mac = config().map_or([0; 6], |c| c.mac);
    let mut m = alloc::vec![0u8; 240];
    m[0] = 1; // request
    m[1] = 1; // Ethernet
    m[2] = 6;
    m[4..8].copy_from_slice(&xid.to_be_bytes());
    m[10] = 0x80; // broadcast flag: answer to everyone, we have no address yet
    m[12..16].copy_from_slice(&ciaddr);
    m[28..34].copy_from_slice(&mac);
    m[236..240].copy_from_slice(&[99, 130, 83, 99]); // magic cookie
    m.extend_from_slice(&[53, 1, kind]);
    if let Some((ip, server)) = requested {
        m.extend_from_slice(&[50, 4]);
        m.extend_from_slice(&ip);
        m.extend_from_slice(&[54, 4]);
        m.extend_from_slice(&server);
    }
    m.extend_from_slice(&[55, 4, 1, 3, 6, 51]); // ask for: mask, router, DNS, lease time
    m.push(255);
    m
}

/// Message type and lease from an answer to `xid`.
fn parse(d: &[u8], xid: u32) -> Option<(u8, Lease)> {
    if d.len() < 240 || d[0] != 2 || d[4..8] != xid.to_be_bytes() || d[236..240] != [99, 130, 83, 99] {
        return None;
    }
    let mut lease = Lease { ip: d[16..20].try_into().ok()?, mask: [255, 255, 255, 0], gateway: [0; 4], dns: [0; 4], server: [0; 4], seconds: 3600 };
    let mut kind = 0;
    let mut i = 240;
    while i < d.len() && d[i] != 255 {
        if d[i] == 0 {
            i += 1;
            continue;
        }
        let (code, len) = (d[i], *d.get(i + 1)? as usize);
        let v = d.get(i + 2..i + 2 + len)?;
        let ip = || -> Option<Ipv4> { v.get(..4)?.try_into().ok() };
        match code {
            53 => kind = *v.first()?,
            1 => lease.mask = ip()?,
            3 => lease.gateway = ip()?,
            6 => lease.dns = ip()?,
            54 => lease.server = ip()?,
            51 => lease.seconds = u32::from_be_bytes(v.get(..4)?.try_into().ok()?),
            _ => {}
        }
        i += 2 + len;
    }
    Some((kind, lease))
}

/// Sends `msg` and waits for an answer of one of `kinds`, retrying a few times.
fn exchange(msg: &[u8], xid: u32, kinds: &[u8]) -> Option<(u8, Lease)> {
    for _ in 0..4 {
        udp::send(CLIENT, [255; 4], SERVER, msg);
        let deadline = apic::ticks() + 2 * apic::HZ;
        while apic::ticks() < deadline {
            let Some((_, _, d)) = udp::receive(CLIENT, 500) else { continue };
            if let Some(answer) = parse(&d, xid).filter(|(k, _)| kinds.contains(k)) {
                return Some(answer);
            }
        }
    }
    None
}

/// Gets a lease (DISCOVER, OFFER, REQUEST, ACK) and applies it.
pub fn obtain() -> Option<Lease> {
    let xid = 0xC0DE_0000 | (apic::ticks() as u32 & 0xFFFF);
    if !udp::bind(CLIENT) {
        return None;
    }
    let result = (|| {
        let (_, offer) = exchange(&message(DISCOVER, xid, [0; 4], None), xid, &[OFFER])?;
        let (kind, lease) = exchange(&message(REQUEST, xid, [0; 4], Some((offer.ip, offer.server))), xid, &[ACK, NAK])?;
        (kind == ACK).then_some(lease)
    })();
    udp::unbind(CLIENT);
    let lease = result?;
    set_config(lease.ip, lease.mask, lease.gateway, lease.dns);
    Some(lease)
}

/// Renews `lease` (REQUEST from our address); applies and returns the new one.
pub fn renew(lease: &Lease) -> Option<Lease> {
    let xid = 0xC0DE_0000 | (apic::ticks() as u32 & 0xFFFF);
    if !udp::bind(CLIENT) {
        return None;
    }
    let result = exchange(&message(REQUEST, xid, lease.ip, None), xid, &[ACK, NAK]);
    udp::unbind(CLIENT);
    let (kind, new) = result?;
    if kind != ACK {
        return None;
    }
    set_config(new.ip, new.mask, new.gateway, new.dns);
    Some(new)
}
