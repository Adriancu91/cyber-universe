//! P6-103 (C-152 point 3): the chipset's watchdog (the TCO timer of the Intel ICH / PCH), written from Intel's public
//! datasheets: ICH9 §13.9 "TCO I/O Registers" and the 100-series-and-later PCH datasheets (vol. 2, "SMBus: TCOBASE /
//! TCOCTL" and the TCO registers).
//!
//! Where the registers are (I/O space, 32 bytes):
//! - PCH 100 series and later: the SMBus controller (00:1f.4), config 0x50 TCOBASE (bits 15:5), enabled by
//!   config 0x54 TCOCTL bit 8 (TCO_BASE_EN);
//! - ICH6 … 9 series (and QEMU's q35, an ICH9): the ACPI base in the LPC bridge (00:1f.0), config 0x40 bits 15:7,
//!   enabled by config 0x44 bit 7; the TCO block is at ACPI base + 0x60.
//! Registers used (16 bit): 0x00 TCO_RLD (any write reloads the timer), 0x04 TCO1_STS (bit 3 TIMEOUT), 0x06 TCO2_STS
//! (bit 1 SECOND_TO_STS: the last reset came from the watchdog), 0x08 TCO1_CNT (bit 11 TCO_TMR_HLT: 1 = stopped),
//! 0x12 TCO_TMR (bits 9:0: the start value, in 0.6 s ticks). The timer counts down twice before it resets the machine
//! (the first time it only sets TIMEOUT), so the real delay is about 2 × value × 0.6 s.
//!
//! Safety of this Release: on a real computer the code only READS (`status`). Arming (`arm`) is allowed only in test
//! builds (QEMU), as the plan says: first the reading on the workstation, the arming in a following Release. The
//! NO_REBOOT strap (which some firmware sets so that the watchdog never resets) is not touched.

use alloc::{format, string::String};
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

const RLD: u16 = 0x00;
const TCO1_STS: u16 = 0x04;
const TCO2_STS: u16 = 0x06;
const TCO1_CNT: u16 = 0x08;
const TMR: u16 = 0x12;
const TMR_HLT: u16 = 1 << 11;
const TICK_MS: u64 = 600;
/// Arming on a real computer comes in a following Release (P6-103 plan: reading first).
const ARM_ON_HARDWARE: bool = false;

static BASE: AtomicU16 = AtomicU16::new(0);
static FEEDING: AtomicBool = AtomicBool::new(false);

fn inw(port: u16) -> u16 {
    let v: u16;
    // SAFETY: only called on the TCO block found by `locate` (ports given by the chipset's own configuration); reading
    // these registers has no side effect.
    unsafe { core::arch::asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags)) }
    v
}

fn outw(port: u16, v: u16) {
    // SAFETY: only `arm`, `feed` and `disarm` write, on the TCO block found by `locate`, and only in test builds or
    // with ARM_ON_HARDWARE (see the module comment).
    unsafe { core::arch::asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags)) }
}

/// The TCO I/O base and how it was found, or why not.
pub fn locate() -> Result<(u16, &'static str), &'static str> {
    let cached = BASE.load(Ordering::Relaxed);
    if cached != 0 {
        return Ok((cached, "memorat"));
    }
    let devs = crate::pci::devices();
    let at = |dev: u8, func: u8| devs.iter().find(|d| d.bus == 0 && d.dev == dev && d.func == func && d.vendor == 0x8086).copied();
    let found = if let Some(smb) = at(0x1f, 4).filter(|d| d.read32(0x54) & (1 << 8) != 0) {
        let b = (smb.read32(0x50) & 0xFFE0) as u16;
        (b != 0).then_some((b, "SMBus 00:1f.4 TCOBASE"))
    } else {
        None
    };
    let found = found.or_else(|| {
        let lpc = at(0x1f, 0)?;
        if lpc.read32(0x44) & (1 << 7) == 0 {
            return None;
        }
        let acpi = (lpc.read32(0x40) & 0xFF80) as u16;
        (acpi != 0).then_some((acpi + 0x60, "LPC 00:1f.0 ACPI base + 0x60"))
    });
    let (base, how) = found.ok_or("nu gasesc cipul Intel al placii de baza cu TCO (SMBus 00:1f.4 sau LPC 00:1f.0)")?;
    BASE.store(base, Ordering::Relaxed);
    Ok((base, how))
}

/// One line for `hw` and for the cycle's report (reads only).
pub fn line() -> String {
    match locate() {
        Err(why) => format!("ceasul de paza (TCO): necitit ({})", why),
        Ok((b, how)) => {
            let cnt = inw(b + TCO1_CNT);
            let tmr = inw(b + TMR) & 0x3FF;
            let second = inw(b + TCO2_STS) & 0b10 != 0;
            format!(
                "ceasul de paza (TCO): la portul {:#x} ({}); {}; termen {} x 0,6 s (~{} s pana la repornire){}",
                b,
                how,
                if cnt & TMR_HLT != 0 { "oprit" } else { "PORNIT" },
                tmr,
                2 * u64::from(tmr) * TICK_MS / 1000,
                if second { "; ultima repornire a venit de la ceasul de paza" } else { "" }
            )
        }
    }
}

fn armed_allowed() -> bool {
    ARM_ON_HARDWARE || cfg!(feature = "qemu-test")
}

/// Arms the watchdog so that the machine restarts about `secs` seconds after the last `feed`. Test builds only in
/// this Release (see the module comment).
pub fn arm(secs: u64) -> Result<String, String> {
    if !armed_allowed() {
        return Err(String::from("armarea ceasului de paza vine intr-un Release urmator; acum doar citirea starii"));
    }
    let (b, _) = locate().map_err(String::from)?;
    let no_reboot = clear_no_reboot()?;
    let ticks = (secs * 1000 / (2 * TICK_MS)).clamp(4, 0x3FF) as u16;
    outw(b + TMR, (inw(b + TMR) & !0x3FF) | ticks);
    outw(b + TCO1_STS, 1 << 3); // clear TIMEOUT (write 1 to clear)
    outw(b + TCO2_STS, 0b10); // clear SECOND_TO_STS
    outw(b + RLD, 1);
    outw(b + TCO1_CNT, inw(b + TCO1_CNT) & !TMR_HLT);
    Ok(format!(
        "ceasul de paza: armat, {} x 0,6 s (~{} s fara hranire pana la repornire){}",
        ticks,
        2 * u64::from(ticks) * TICK_MS / 1000,
        if no_reboot { "; NO_REBOOT era pus, l-am scos" } else { "" }
    ))
}

/// ICH-style chipsets (ICH6 … 9 series, QEMU q35): the NO_REBOOT bit (GCS bit 5, chipset configuration registers at
/// RCBA + 0x3410; RCBA = LPC config 0xF0 bits 31:14, bit 0 = enabled). When set, the watchdog never resets. Returns
/// whether the bit was set before. Only where arming is allowed (`arm` calls it). PCH 100 series and later keep it
/// elsewhere (not handled: arming on them comes in a following Release, after the reading).
fn clear_no_reboot() -> Result<bool, String> {
    let devs = crate::pci::devices();
    let lpc = devs
        .iter()
        .find(|d| d.bus == 0 && d.dev == 0x1f && d.func == 0 && d.vendor == 0x8086)
        .ok_or_else(|| String::from("fara LPC 00:1f.0"))?;
    let rcba = lpc.read32(0xF0);
    if rcba & 1 == 0 {
        return Err(String::from("RCBA neactivat (cip mai nou de seria 9: NO_REBOOT e in alta parte)"));
    }
    let gcs_phys = u64::from(rcba & 0xFFFF_C000) + 0x3410;
    let page = gcs_phys & !0xFFF;
    let virt = crate::mem::paging::map_mmio(page, 0x1000).map_err(|e| format!("maparea GCS: {:?}", e))? + (gcs_phys - page);
    // SAFETY: GCS is a documented 32-bit chipset register at RCBA + 0x3410, mapped uncached just above; only bit 5
    // (NO_REBOOT) changes, the rest is written back as read.
    unsafe {
        let p = virt as *mut u32;
        let v = p.read_volatile();
        p.write_volatile(v & !(1 << 5));
        Ok(v & (1 << 5) != 0)
    }
}

/// Reloads the timer (the system is alive).
pub fn feed() {
    if let Ok((b, _)) = locate() {
        if armed_allowed() {
            outw(b + RLD, 1);
        }
    }
}

/// Stops the timer.
#[allow(dead_code)] // the cycle stops it at its end in the Release that arms it
pub fn disarm() {
    if let Ok((b, _)) = locate() {
        if armed_allowed() {
            outw(b + TCO1_CNT, inw(b + TCO1_CNT) | TMR_HLT);
        }
    }
}

/// A system thread that feeds the watchdog every `every_ms` while `FEEDING` is set: if the scheduler or the
/// interrupts freeze, the feeding stops and the chipset restarts the machine.
pub fn start_feeding(every_ms: u64) {
    if FEEDING.swap(true, Ordering::AcqRel) {
        return;
    }
    static EVERY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1000);
    EVERY.store(every_ms, Ordering::Relaxed);
    crate::sched::spawn_system(|| {
        while FEEDING.load(Ordering::Acquire) {
            feed();
            crate::sched::sleep_ms(EVERY.load(Ordering::Relaxed));
        }
    });
}

/// Stops the feeding thread (the watchdog keeps running: test of the restart).
pub fn stop_feeding() {
    FEEDING.store(false, Ordering::Release);
}

/// The cycle mode (`ciclu`): the row for its report; arms and feeds only where arming is allowed.
pub fn for_cycle() -> String {
    let status = line();
    match arm(5 * 60) {
        Ok(msg) => {
            start_feeding(10_000);
            format!("{}\n{}", status, msg)
        }
        Err(why) => format!("{}\nceasul de paza: neactivat ({})", status, why),
    }
}

/// Scenario `tco` (test builds, QEMU q35 with `-global ICH9-LPC.noreboot=false`): armed and fed = no restart for
/// longer than the term; then the feeding stops and the machine must restart (the scenario expects a reset).
#[cfg(feature = "test-tco")]
pub fn self_test() {
    crate::info!("{}", line());
    let msg = arm(5).expect("test: tco: armare");
    crate::info!("{}", msg);
    start_feeding(500);
    let start = crate::time::uptime_ms().unwrap_or(0);
    while crate::time::uptime_ms().unwrap_or(0) < start + 12_000 {
        crate::sched::sleep_ms(500);
    }
    assert!(line().contains("PORNIT"), "test: tco: ceasul trebuia sa fie pornit");
    crate::info!("test: tco: hranit 12 s, nicio repornire");
    stop_feeding();
    crate::info!("test: tco: nu mai hranesc; astept repornirea");
    let start = crate::time::uptime_ms().unwrap_or(0);
    while crate::time::uptime_ms().unwrap_or(0) < start + 30_000 {
        crate::sched::sleep_ms(500);
    }
    panic!("test: tco: ceasul de paza nu a repornit calculatorul in 30 s");
}
