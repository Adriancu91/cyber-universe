//! Waiting without using the CPU: wait queues and `sleep_ms`.

use super::{block_current, irq_restore, irq_save, wake};
use crate::arch::x86_64::apic;

/// A place where threads wait for something (a key press, a free lock...). Threads are identified with
/// the queue by its address, so a `WaitQueue` must not move while threads wait on it (use it in a
/// `static` or inside a structure that stays in place).
pub struct WaitQueue {
    _not_zero_sized: u8, // gives every queue its own address
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self { _not_zero_sized: 0 }
    }

    fn key(&self) -> usize {
        self as *const Self as usize
    }

    /// Blocks the current thread until `ready()` returns true. `ready` is checked with interrupts
    /// disabled, so a wake-up from an interrupt handler between the check and the sleep cannot be lost.
    pub fn wait_until(&self, mut ready: impl FnMut() -> bool) {
        loop {
            let flags = irq_save();
            if ready() {
                irq_restore(flags);
                return;
            }
            block_current(self.key(), 0);
            irq_restore(flags);
        }
    }

    /// Wakes the thread that has waited longest. Returns whether there was one.
    pub fn wake_one(&self) -> bool {
        wake(self.key(), 1) > 0
    }

    /// Wakes every waiting thread. Returns how many.
    pub fn wake_all(&self) -> usize {
        wake(self.key(), usize::MAX)
    }
}

#[allow(dead_code)] // used by the self-tests now, by drivers and the shell later
/// Puts the current thread to sleep for at least `ms` milliseconds (rounded up to whole timer ticks).
pub fn sleep_ms(ms: u64) {
    let ticks = (ms * apic::HZ).div_ceil(1000).max(1);
    let wake_at = apic::ticks() + ticks;
    while apic::ticks() < wake_at {
        let flags = irq_save();
        block_current(0, wake_at);
        irq_restore(flags);
    }
}
