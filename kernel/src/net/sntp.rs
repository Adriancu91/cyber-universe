//! SNTP client (RFC 4330, P6-61 step 7 / C-84 item 7): the time from the network. One 48-byte request over UDP
//! (version 4, mode 3 = client), the answer's transmit time (seconds since 1900) turned into UTC. The battery clock
//! of the computer is NOT changed (Windows keeps it in local time); the command `ora` shows the difference.
//! Written from the public RFC; the date arithmetic is the usual days-from-civil method, our own code.

use super::{Ipv4, udp};
use crate::arch::x86_64::apic;

/// Seconds between 1900-01-01 (NTP) and 1970-01-01 (Unix).
const NTP_UNIX: u64 = 2_208_988_800;

pub struct Answer {
    /// UTC, seconds since 1970.
    pub unix: u64,
    pub stratum: u8,
    pub rtt_ms: u64,
}

/// Asks `server`:`port` (3 tries, 2 s each).
pub fn query(server: Ipv4, port: u16) -> Result<Answer, &'static str> {
    let local = udp::temp_port();
    if !udp::bind(local) {
        return Err("niciun port UDP liber");
    }
    let mut req = [0u8; 48];
    req[0] = 0x23; // LI 0, version 4, mode 3 (client)
    let mut result = Err("serverul de timp nu raspunde (3 incercari x 2 s)");
    'tries: for _ in 0..3 {
        let sent = apic::ticks();
        if !udp::send(local, server, port, &req) {
            continue;
        }
        let deadline = sent + 2 * apic::HZ;
        while apic::ticks() < deadline {
            let left = (deadline - apic::ticks()) * 1000 / apic::HZ;
            let Some((src, _, data)) = udp::receive(local, left.max(1)) else { break };
            if src != server || data.len() < 48 {
                continue;
            }
            result = parse(&data).map(|(unix, stratum)| Answer { unix, stratum, rtt_ms: (apic::ticks() - sent) * 1000 / apic::HZ });
            break 'tries;
        }
    }
    udp::unbind(local);
    result
}

fn parse(d: &[u8]) -> Result<(u64, u8), &'static str> {
    if d[0] & 0x07 != 4 {
        return Err("raspunsul nu e de la un server (mod gresit)");
    }
    if d[0] >> 6 == 3 || d[1] == 0 {
        return Err("serverul spune ca nu are ora (alarma / strat 0)");
    }
    let secs = u32::from_be_bytes([d[40], d[41], d[42], d[43]]) as u64;
    if secs < NTP_UNIX {
        return Err("ora primita e inainte de 1970");
    }
    Ok((secs - NTP_UNIX, d[1]))
}

/// (year, month, day, hour, minute, second) of a Unix time (UTC).
pub fn civil(unix: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (unix / 86_400) as i64;
    let rest = unix % 86_400;
    // Days since 1970-01-01 -> civil date (proleptic Gregorian), counting 400-year eras from 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day, (rest / 3600) as u32, (rest % 3600 / 60) as u32, (rest % 60) as u32)
}

/// Days since 1970 of a civil date (the inverse of `civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Romania's offset from UTC at `unix`: +3 h (EEST) from the last Sunday of March 01:00 UTC to the last Sunday of
/// October 01:00 UTC (the EU rule), else +2 h (EET).
pub fn romania_offset(unix: u64) -> (i64, &'static str) {
    let (y, ..) = civil(unix);
    let last_sunday = |m: u32| {
        let d31 = days_from_civil(y, m, 31);
        d31 - (d31 + 4).rem_euclid(7) // 1970-01-01 was a Thursday (weekday 4, Sunday = 0)
    };
    let start = last_sunday(3) * 86_400 + 3600;
    let end = last_sunday(10) * 86_400 + 3600;
    let t = unix as i64;
    if t >= start && t < end { (3 * 3600, "EEST") } else { (2 * 3600, "EET") }
}

/// The built-in checks (test builds): known dates and the summer-time edges of 2026.
pub fn self_test() -> Result<(), &'static str> {
    if civil(0) != (1970, 1, 1, 0, 0, 0) || civil(1_793_966_400) != (2026, 11, 6, 12, 0, 0) || civil(951_782_400) != (2000, 2, 29, 0, 0, 0) {
        return Err("civil");
    }
    if days_from_civil(2026, 11, 6) != 20_763 {
        return Err("days_from_civil");
    }
    // 2026: summer time from Sunday 29 March 01:00 UTC to Sunday 25 October 01:00 UTC.
    let at = |m: u32, d: u32, h: u64| (days_from_civil(2026, m, d) as u64) * 86_400 + h * 3600;
    let ok = romania_offset(at(3, 29, 0)).0 == 7200
        && romania_offset(at(3, 29, 1)).0 == 10_800
        && romania_offset(at(10, 25, 0)).0 == 10_800
        && romania_offset(at(10, 25, 1)).0 == 7200
        && romania_offset(at(11, 6, 12)).1 == "EET";
    if ok { Ok(()) } else { Err("ora de vara") }
}
