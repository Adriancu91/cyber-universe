//! A lock for data shared between threads: a thread that finds it taken sleeps instead of spinning.
//! Not for interrupt handlers (they cannot sleep): they keep using spinlocks.

use super::WaitQueue;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct Mutex<T> {
    locked: AtomicBool,
    waiters: WaitQueue,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reachable through a guard, and only one guard exists at a time.
unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

#[allow(dead_code)] // used by the self-tests now, by drivers and processes later
impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), waiters: WaitQueue::new(), value: UnsafeCell::new(value) }
    }

    /// Takes the lock, sleeping while another thread holds it.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.waiters.wait_until(|| {
            self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok()
        });
        MutexGuard { mutex: self }
    }

    /// Takes the lock only if it is free right now (never sleeps).
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        // The guard is built only when the lock was taken: a guard built and dropped on failure (as `then_some`
        // did, it evaluates its argument eagerly) would release the lock of the thread that holds it. That broke
        // `thread_join` (which polls PROCS with `try_lock`) on several processors: two threads in PROCS at once,
        // a program's thread list corrupted ("fir fara proces"), or a hang (kronos-calcul, CI 2026-11-03).
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: this guard is the only one; it holds the lock.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        self.mutex.waiters.wake_one();
    }
}
