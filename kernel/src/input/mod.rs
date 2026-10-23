//! Mouse events from every pointing device (PS/2 and USB), in one queue. Key presses have their own
//! queue in `drivers::keyboard` (read by the shell); the window server (P4-04) will read both.

use core::sync::atomic::{AtomicUsize, Ordering};

/// A mouse movement and the state of the buttons after it. `dy` grows downwards (screen direction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    /// Bit 0 = left, bit 1 = right, bit 2 = middle.
    pub buttons: u8,
}

const QUEUE: usize = 64;
static mut EVENTS: [MouseEvent; QUEUE] = [MouseEvent { dx: 0, dy: 0, buttons: 0 }; QUEUE];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);
/// Serialises the producers (interrupt handler, USB driver thread), which may run on different processors.
static PUSH_LOCK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static WAIT: crate::sched::WaitQueue = crate::sched::WaitQueue::new();

/// Adds an event (from an interrupt handler or a driver thread). Dropped if the queue is full.
pub fn push_mouse(e: MouseEvent) {
    let flags = crate::sched::spin::lock(&PUSH_LOCK);
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % QUEUE;
    if next != TAIL.load(Ordering::Acquire) {
        // SAFETY: producers are serialised by PUSH_LOCK (from interrupts and from driver threads on any processor); the reader only reads slots before HEAD.
        unsafe { (*core::ptr::addr_of_mut!(EVENTS))[head] = e };
        HEAD.store(next, Ordering::Release);
        WAIT.wake_all();
    }
    crate::sched::spin::unlock(&PUSH_LOCK, flags);
}

/// The next mouse event, if there is one.
pub fn read_mouse() -> Option<MouseEvent> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: slot `tail` was published before HEAD moved past it; only one reader.
    let e = unsafe { (*core::ptr::addr_of!(EVENTS))[tail] };
    TAIL.store((tail + 1) % QUEUE, Ordering::Release);
    Some(e)
}

/// Waits (sleeping) for the next mouse event.
#[allow(dead_code)] // used by the self-tests now, by the window server (P4-04)
pub fn wait_mouse() -> MouseEvent {
    let mut e = None;
    WAIT.wait_until(|| {
        e = read_mouse();
        e.is_some()
    });
    e.unwrap()
}
