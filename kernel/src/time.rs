//! Time since boot, counted by the APIC timer.

use core::sync::atomic::{AtomicBool, Ordering};

static RUNNING: AtomicBool = AtomicBool::new(false);

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
