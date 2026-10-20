//! DNS client (RFC 1034/1035): turns a name like `example.com` into an IPv4 address by asking the DNS
//! server received from DHCP (type A query over UDP). Answers are kept until their TTL runs out.

use super::{Ipv4, config, parse_ip, udp};
use crate::arch::x86_64::apic;
use crate::sched::Mutex;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No network card or no DNS server yet.
    NoServer,
    /// The name is not a valid host name.
    BadName,
    /// The server says the name does not exist (or has no IPv4 address).
    NotFound,
    /// No (valid) answer in time.
    Timeout,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Error::NoServer => "nu am server DNS (retea sau DHCP lipsa)",
            Error::BadName => "nume invalid",
            Error::NotFound => "numele nu exista",
            Error::Timeout => "serverul DNS nu a raspuns",
        }
    }
}

/// (name in lower case, address, tick until which it is valid).
static CACHE: Mutex<Vec<(String, Ipv4, u64)>> = Mutex::new(Vec::new());
const CACHE_MAX: usize = 64;
/// Answers are kept at most a day, whatever the server says.
const TTL_MAX: u32 = 86_400;
static NEXT_PORT: AtomicU16 = AtomicU16::new(0);
static NEXT_ID: AtomicU16 = AtomicU16::new(0);

/// A local port for one query (49152..65535, the range for temporary ports).
fn temp_port() -> u16 {
    if NEXT_PORT.load(Ordering::Relaxed) == 0 {
        NEXT_PORT.store(49152 + (apic::ticks() % 8192) as u16, Ordering::Relaxed);
    }
    49152 + NEXT_PORT.fetch_add(1, Ordering::Relaxed) % 16384
}

fn valid(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|l| {
            !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// A query for the A record of `name`.
pub fn query_message(id: u16, name: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(18 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // recursion desired, one question
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]); // end of name, type A, class IN
    q
}

/// Skips a (possibly compressed) name starting at `i`; returns the index after it.
fn skip_name(m: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *m.get(i)?;
        match len {
            0 => return Some(i + 1),
            l if l & 0xC0 == 0xC0 => return Some(i + 2), // pointer: the name ends here
            l if l & 0xC0 == 0 => i += 1 + l as usize,
            _ => return None,
        }
    }
}

/// Reads an answer to query `id`: the first IPv4 address and its TTL.
pub fn parse_answer(m: &[u8], id: u16) -> Result<(Ipv4, u32), Error> {
    if m.len() < 12 || m[0..2] != id.to_be_bytes() || m[2] & 0x80 == 0 {
        return Err(Error::Timeout); // not our answer: keep waiting
    }
    match m[3] & 0x0F {
        0 => {}
        3 => return Err(Error::NotFound),
        _ => return Err(Error::Timeout), // server failure, refused...: try again
    }
    let questions = u16::from_be_bytes([m[4], m[5]]);
    let answers = u16::from_be_bytes([m[6], m[7]]);
    let mut i = 12;
    for _ in 0..questions {
        i = skip_name(m, i).ok_or(Error::Timeout)? + 4;
    }
    for _ in 0..answers {
        i = skip_name(m, i).ok_or(Error::Timeout)?;
        let h = m.get(i..i + 10).ok_or(Error::Timeout)?;
        let (kind, class) = (u16::from_be_bytes([h[0], h[1]]), u16::from_be_bytes([h[2], h[3]]));
        let ttl = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
        let len = u16::from_be_bytes([h[8], h[9]]) as usize;
        let data = m.get(i + 10..i + 10 + len).ok_or(Error::Timeout)?;
        if kind == 1 && class == 1 && len == 4 {
            return Ok((data.try_into().unwrap(), ttl));
        }
        i += 10 + len; // CNAME and others: skip
    }
    Err(Error::NotFound)
}

/// Asks `server`:`port` for `name` (3 tries, 2 s each). Does not use the cache.
pub fn query(server: Ipv4, port: u16, name: &str) -> Result<(Ipv4, u32), Error> {
    if !valid(name) {
        return Err(Error::BadName);
    }
    let local = temp_port();
    if !udp::bind(local) {
        return Err(Error::Timeout);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) ^ (apic::ticks() as u16).wrapping_mul(40503);
    let msg = query_message(id, name);
    let mut result = Err(Error::Timeout);
    'tries: for _ in 0..3 {
        if !udp::send(local, server, port, &msg) {
            continue;
        }
        let deadline = apic::ticks() + 2 * apic::HZ;
        while apic::ticks() < deadline {
            let left = (deadline - apic::ticks()) * 1000 / apic::HZ;
            let Some((src, _, data)) = udp::receive(local, left.max(1)) else { break };
            if src != server {
                continue;
            }
            match parse_answer(&data, id) {
                Err(Error::Timeout) => continue,
                r => {
                    result = r;
                    break 'tries;
                }
            }
        }
    }
    udp::unbind(local);
    result
}

fn cached(name: &str) -> Option<Ipv4> {
    let now = apic::ticks();
    let mut cache = CACHE.lock();
    cache.retain(|e| e.2 > now);
    cache.iter().find(|e| e.0 == name).map(|e| e.1)
}

fn remember(name: String, ip: Ipv4, ttl: u32) {
    let until = apic::ticks() + ttl.min(TTL_MAX) as u64 * apic::HZ;
    let mut cache = CACHE.lock();
    cache.retain(|e| e.0 != name);
    if cache.len() >= CACHE_MAX {
        cache.remove(0);
    }
    cache.push((name, ip, until));
}

/// The address of `name`, from the cache or from `server`:`port`. `from_cache` tells which.
pub fn resolve_via(server: Ipv4, port: u16, name: &str) -> Result<(Ipv4, bool), Error> {
    if let Some(ip) = parse_ip(name) {
        return Ok((ip, false));
    }
    let key = name.trim_end_matches('.').to_ascii_lowercase();
    if let Some(ip) = cached(&key) {
        return Ok((ip, true));
    }
    let (ip, ttl) = query(server, port, &key)?;
    if ttl > 0 {
        remember(key, ip, ttl);
    }
    Ok((ip, false))
}

/// The address of `name`, asking the DNS server received from DHCP.
pub fn resolve(name: &str) -> Result<(Ipv4, bool), Error> {
    if let Some(ip) = parse_ip(name) {
        return Ok((ip, false));
    }
    if !super::wait_ready(3000) {
        return Err(Error::NoServer);
    }
    let server = config().map(|c| c.dns).filter(|d| *d != [0; 4]).ok_or(Error::NoServer)?;
    resolve_via(server, 53, name)
}
