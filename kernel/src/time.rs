//! Time since boot, counted by the APIC timer, plus a TSC-based boot-duration measurement.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static RUNNING: AtomicBool = AtomicBool::new(false);
/// TSC value at `kmain` entry.
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);
/// TSC ticks per millisecond, measured against the PIT during APIC calibration (0 = unknown).
static TSC_PER_MS: AtomicU64 = AtomicU64::new(0);

/// Records the moment the kernel started. Called first thing in `kmain`.
pub fn mark_boot() {
    BOOT_TSC.store(crate::arch::x86_64::rdtsc(), Ordering::Relaxed);
}

/// Stores the TSC rate measured by the APIC calibration.
pub fn set_tsc_per_ms(rate: u64) {
    TSC_PER_MS.store(rate, Ordering::Relaxed);
}

/// Microseconds since `mark_boot`, if the TSC rate is known.
pub fn since_boot_us() -> Option<u64> {
    let rate = TSC_PER_MS.load(Ordering::Relaxed);
    if rate == 0 {
        return None;
    }
    let delta = crate::arch::x86_64::rdtsc() - BOOT_TSC.load(Ordering::Relaxed);
    Some(delta * 1000 / rate)
}

/// Called once the timer is running.
pub fn start() {
    RUNNING.store(true, Ordering::Release);
}

/// Milliseconds since the timer started, if it is running.
pub fn uptime_ms() -> Option<u64> {
    if RUNNING.load(Ordering::Acquire) {
        Some(crate::arch::x86_64::apic::ticks() * 1000 / crate::arch::x86_64::apic::HZ)
    } else {
        None
    }
}
