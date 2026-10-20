//! Real-time clock (CMOS, MC146818-compatible): the date and time kept by the battery-powered clock.
//! Written from the public MC146818 description (OSDev Wiki "CMOS"/"RTC", descriptions only).

use crate::arch::x86_64::{inb, outb};

const INDEX: u16 = 0x70;
const DATA: u16 = 0x71;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn read_reg(reg: u8) -> u8 {
    // SAFETY: selecting a CMOS register (bit 7 = 1 keeps NMIs disabled as they are at boot) and reading it.
    unsafe {
        outb(INDEX, 0x80 | reg);
        inb(DATA)
    }
}

fn raw() -> [u8; 6] {
    // Wait while the clock is updating its registers (register A, bit 7), then read them.
    for _ in 0..100_000 {
        if read_reg(0x0A) & 0x80 == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    [read_reg(0x00), read_reg(0x02), read_reg(0x04), read_reg(0x07), read_reg(0x08), read_reg(0x09)]
}

/// Reads the date and time (twice, until two reads agree, so an update in between cannot mix them).
/// `None` if the values make no sense (no clock).
pub fn now() -> Option<DateTime> {
    let mut a = raw();
    for _ in 0..5 {
        let b = raw();
        if a == b {
            break;
        }
        a = b;
    }
    let status_b = read_reg(0x0B);
    let bcd = |v: u8| if status_b & 0x04 == 0 { (v & 0x0F) + (v >> 4) * 10 } else { v };
    let [s, m, h, d, mo, y] = a;
    let pm = h & 0x80 != 0;
    let mut hour = bcd(h & 0x7F);
    if status_b & 0x02 == 0 {
        // 12-hour clock: 12 AM = 0, 12 PM = 12.
        hour = (hour % 12) + if pm { 12 } else { 0 };
    }
    let t = DateTime { year: 2000 + bcd(y) as u16, month: bcd(mo), day: bcd(d), hour, minute: bcd(m), second: bcd(s) };
    let ok = (1..=12).contains(&t.month) && (1..=31).contains(&t.day) && t.hour < 24 && t.minute < 60 && t.second < 60;
    ok.then_some(t)
}
