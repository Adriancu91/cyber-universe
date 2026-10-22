//! Network stack (Phase 5): Ethernet frames from the network card, ARP, IPv4, ICMP (ping), UDP and
//! DHCP. A system thread takes the received frames from the card every few milliseconds and hands them
//! to the protocol that owns them. The address comes from DHCP; without an answer there is no address and DHCP
//! is asked again every 10 s.

pub mod arp;
pub mod dhcp;
pub mod dns;
pub mod icmp;
pub mod ipv4;
pub mod tcp;
pub mod udp;

use crate::drivers::nic::NIC;
use crate::sched::{self, Mutex};
use alloc::vec::Vec;

pub type Ipv4 = [u8; 4];
pub type Mac = [u8; 6];

pub const ETH_ARP: u16 = 0x0806;
pub const ETH_IPV4: u16 = 0x0800;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub mac: Mac,
    pub ip: Ipv4,
    pub mask: Ipv4,
    pub gateway: Ipv4,
    pub dns: Ipv4,
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// The interface settings, if there is a network card.
pub fn config() -> Option<Config> {
    *CONFIG.lock()
}

/// Starts the stack on the network card found at start-up (if any).
pub fn init() {
    let Some(mac) = NIC.lock().as_ref().map(|n| n.mac()) else { return };
    *CONFIG.lock() = Some(Config { mac, ip: [0; 4], mask: [0; 4], gateway: [0; 4], dns: [0; 4] });
    sched::spawn_system(receive_thread);
    sched::spawn_system(dhcp_thread);
}

/// Sets our address, mask, gateway and name server.
pub fn set_config(ip: Ipv4, mask: Ipv4, gateway: Ipv4, dns: Ipv4) {
    if let Some(c) = CONFIG.lock().as_mut() {
        (c.ip, c.mask, c.gateway, c.dns) = (ip, mask, gateway, dns);
    }
}

/// Whether the interface has an address (from DHCP).
pub fn ready() -> bool {
    config().is_some_and(|c| c.ip != [0; 4])
}

fn ip_text(a: Ipv4) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

/// Timer tick at which the DHCP lease runs out (0 = no lease).
static LEASE_UNTIL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn apic_ticks() -> u64 {
    crate::arch::x86_64::apic::ticks()
}

/// Seconds left on the DHCP lease (0 = none).
pub fn lease_left_s() -> u32 {
    let until = LEASE_UNTIL.load(core::sync::atomic::Ordering::Relaxed);
    (until.saturating_sub(apic_ticks()) / crate::arch::x86_64::apic::HZ) as u32
}

/// Gets the address by DHCP; without an answer there is no address (no made-up one) and DHCP is asked again every
/// 10 s (e.g. until a cable is plugged in). Then the lease is renewed at half its time.
fn dhcp_thread() {
    let mut lease = match dhcp::obtain() {
        Some(l) => l,
        None => {
            crate::warn!("net: DHCP nu raspunde; fara adresa, reincerc la 10 s");
            let mut tries = 0u32;
            loop {
                if tries < 6 {
                    // What the card sees, for `log` on a real computer (the first minute only, not forever).
                    if let Some(info) = NIC.lock().as_mut().and_then(|n| n.link_info()) {
                        for line in info.lines() {
                            crate::info!("net: {}", line);
                        }
                    }
                }
                tries += 1;
                sched::sleep_ms(10_000);
                if let Some(l) = dhcp::obtain() {
                    break l;
                }
            }
        }
    };
    loop {
        LEASE_UNTIL.store(apic_ticks() + lease.seconds as u64 * crate::arch::x86_64::apic::HZ, core::sync::atomic::Ordering::Relaxed);
        crate::info!(
            "net: DHCP: adresa {}, masca {}, ruta {}, DNS {}, pentru {} s",
            ip_text(lease.ip),
            ip_text(lease.mask),
            ip_text(lease.gateway),
            ip_text(lease.dns),
            lease.seconds
        );
        sched::sleep_ms(lease.seconds.max(60) as u64 * 1000 / 2);
        match dhcp::renew(&lease) {
            Some(l) => lease = l,
            None => match dhcp::obtain() {
                Some(l) => lease = l,
                None => crate::warn!("net: DHCP: reinnoirea a esuat, pastrez adresa"),
            },
        }
    }
}

/// Waits until the interface has an address (at most `timeout_ms`).
pub fn wait_ready(timeout_ms: u64) -> bool {
    let deadline = crate::arch::x86_64::apic::ticks() + timeout_ms * crate::arch::x86_64::apic::HZ / 1000;
    while !ready() {
        if crate::arch::x86_64::apic::ticks() >= deadline {
            return false;
        }
        sched::sleep_ms(10);
    }
    true
}

/// Sends `payload` in an Ethernet frame of type `ethertype` to `dst`.
pub fn send_frame(dst: Mac, ethertype: u16, payload: &[u8]) -> bool {
    let Some(cfg) = config() else { return false };
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&dst);
    frame.extend_from_slice(&cfg.mac);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    NIC.lock().as_mut().is_some_and(|n| n.send(&frame))
}

/// Takes frames from the card and passes them on; sleeps briefly when there are none.
fn receive_thread() {
    loop {
        let frame = NIC.lock().as_mut().and_then(|n| n.receive());
        match frame {
            Some(f) => handle_frame(&f),
            None => sched::sleep_ms(2),
        }
    }
}

fn handle_frame(f: &[u8]) {
    if f.len() < 14 {
        return;
    }
    let payload = &f[14..];
    match u16::from_be_bytes([f[12], f[13]]) {
        ETH_ARP => arp::handle(payload),
        ETH_IPV4 => ipv4::handle(payload),
        _ => {}
    }
}

/// The Internet checksum (RFC 1071): one's complement of the one's-complement sum of 16-bit words.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in data.chunks(2) {
        sum += u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]) as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
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
