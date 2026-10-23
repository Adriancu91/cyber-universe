//! Spinlocks for data touched by more than one processor and by interrupt handlers (K6-02b): interrupts are
//! disabled on this processor while the lock is held, so an interrupt handler on the same processor can never
//! wait for a lock its own thread holds, and the holder is never switched out in the middle.
//! While waiting, the processor still answers TLB shootdown requests (otherwise two processors could wait for
//! each other: one for the lock, the other for the shootdown answer).

use core::sync::atomic::{AtomicBool, Ordering};

/// Takes `lock` with interrupts disabled. Returns the saved interrupt flag for `unlock`.
#[inline]
pub fn lock(lock: &AtomicBool) -> u64 {
    let flags = super::irq_save();
    while lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
        crate::arch::x86_64::apic::serve_shootdown();
    }
    flags
}

/// Takes `lock` like `lock`, but gives up after about `tries` attempts (for the log: a line is dropped rather than
/// waiting forever after a fault in the middle of a line). Returns the saved flag if taken.
pub fn try_lock_for(lock: &AtomicBool, tries: u32) -> Option<u64> {
    let flags = super::irq_save();
    for _ in 0..tries {
        if lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return Some(flags);
        }
        core::hint::spin_loop();
        crate::arch::x86_64::apic::serve_shootdown();
    }
    super::irq_restore(flags);
    None
}

/// Releases `lock` and restores the interrupt flag saved by `lock`.
#[inline]
pub fn unlock(lock: &AtomicBool, flags: u64) {
    lock.store(false, Ordering::Release);
    super::irq_restore(flags);
}
