//! IPv4 (RFC 791): sends packets to the right neighbour (directly on our network, otherwise through
//! the gateway) and hands received ones to their protocol. Fragmented packets are not supported yet.

use super::{ETH_IPV4, Ipv4, arp, checksum, config, send_frame};
use core::sync::atomic::{AtomicU16, Ordering};

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;

static NEXT_ID: AtomicU16 = AtomicU16::new(1);

/// Sends `payload` as protocol `proto` to `dst`. False if the neighbour did not answer ARP.
pub fn send(dst: Ipv4, proto: u8, payload: &[u8]) -> bool {
    let Some(cfg) = config() else { return false };
    let same_net = cfg.ip != [0; 4] && (0..4).all(|i| dst[i] & cfg.mask[i] == cfg.ip[i] & cfg.mask[i]);
    let hop = if same_net || dst == [255; 4] { dst } else { cfg.gateway };
    let Some(mac) = arp::resolve(hop, 2000) else { return false };
    let total = 20 + payload.len();
    let mut p = alloc::vec::Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, 0]);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&NEXT_ID.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    p.extend_from_slice(&[0x40, 0, 64, proto, 0, 0]); // don't fragment, TTL 64, checksum later
    p.extend_from_slice(&cfg.ip);
    p.extend_from_slice(&dst);
    let sum = checksum(&p[..20]);
    p[10..12].copy_from_slice(&sum.to_be_bytes());
    p.extend_from_slice(payload);
    send_frame(mac, ETH_IPV4, &p)
}

/// A received IPv4 packet.
pub fn handle(p: &[u8]) {
    let Some(cfg) = config() else { return };
    if p.len() < 20 || p[0] >> 4 != 4 {
        return;
    }
    let header = ((p[0] & 0x0F) as usize) * 4;
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    if header < 20 || total < header || total > p.len() || checksum(&p[..header]) != 0 {
        return;
    }
    let dst: Ipv4 = p[16..20].try_into().unwrap();
    // Without an address yet (DHCP running) everything addressed to this card is accepted.
    if dst != cfg.ip && dst != [255; 4] && cfg.ip != [0; 4] {
        return;
    }
    let fragment = u16::from_be_bytes([p[6], p[7]]);
    if fragment & 0x2000 != 0 || fragment & 0x1FFF != 0 {
        crate::warn!("net: pachet IPv4 fragmentat ignorat");
        return;
    }
    let src: Ipv4 = p[12..16].try_into().unwrap();
    match p[9] {
        PROTO_ICMP => super::icmp::handle(src, &p[header..total]),
        PROTO_UDP => super::udp::handle(src, dst, &p[header..total]),
        _ => {}
    }
}
