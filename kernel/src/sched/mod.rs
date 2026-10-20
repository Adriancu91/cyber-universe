//! Kernel threads and the scheduler (P2-01: cooperative; a thread gives up the CPU with `yield_now`).
//! Ready threads wait in a round-robin queue. The boot code becomes thread 0 ("main") in `init`.

mod switch;

use alloc::{boxed::Box, collections::VecDeque, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Stack size of every new kernel thread.
const STACK_SIZE: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ready,
    Exited,
}

struct Thread {
    id: u64,
    state: State,
    /// Saved stack pointer while the thread is not running.
    rsp: u64,
    /// The thread's stack; `None` for the boot thread, which runs on the kernel's boot stack.
    _stack: Option<Box<[u8]>>,
}

struct Scheduler {
    current: Option<Box<Thread>>,
    ready: VecDeque<Box<Thread>>,
    /// Exited threads whose stacks are freed by the next thread that runs (not on their own stack).
    zombies: Vec<Box<Thread>>,
}

static mut SCHED: Scheduler = Scheduler { current: None, ready: VecDeque::new(), zombies: Vec::new() };
static LOCK: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Runs `f` with interrupts disabled and exclusive access to the scheduler.
fn with_sched<R>(f: impl FnOnce(&mut Scheduler) -> R) -> R {
    let flags = irq_save();
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: LOCK grants exclusive access until released below.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(SCHED) });
    LOCK.store(false, Ordering::Release);
    irq_restore(flags);
    r
}

fn irq_save() -> u64 {
    let flags: u64;
    // SAFETY: reads RFLAGS and masks interrupts on this CPU; restored by `irq_restore`.
    unsafe { core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem)) }
    flags
}

fn irq_restore(flags: u64) {
    if flags & (1 << 9) != 0 {
        // SAFETY: interrupts were enabled before `irq_save`.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    }
}

/// Makes the running boot code thread 0. Must run once, after the heap is ready.
pub fn init() {
    with_sched(|s| {
        s.ready.reserve(16);
        s.zombies.reserve(16);
        s.current = Some(Box::new(Thread { id: 0, state: State::Ready, rsp: 0, _stack: None }));
    });
    crate::info!("sched: planificator gata (fire cooperative, stiva {} KiB)", STACK_SIZE / 1024);
}

/// Creates a kernel thread that runs `f` and exits when `f` returns. Returns its id.
#[allow(dead_code)] // used by the self-tests now, by the shell and processes in Phase 2
pub fn spawn(f: fn()) -> u64 {
    let stack = vec![0u8; STACK_SIZE].into_boxed_slice();
    let top = (stack.as_ptr() as u64 + STACK_SIZE as u64) & !15;
    // SAFETY: the 56 bytes below `top` belong to `stack`, which the new thread owns.
    let rsp = unsafe { switch::initial_stack(top, f as *const () as usize) };
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let t = Box::new(Thread { id, state: State::Ready, rsp, _stack: Some(stack) });
    with_sched(|s| s.ready.push_back(t));
    id
}

/// Id of the running thread.
#[allow(dead_code)] // used by the self-tests now
pub fn current_id() -> u64 {
    with_sched(|s| s.current.as_ref().map_or(0, |t| t.id))
}

/// Number of threads that have not exited (running + ready).
#[allow(dead_code)] // used by the self-tests now
pub fn alive() -> usize {
    with_sched(|s| s.ready.len() + s.current.is_some() as usize)
}

/// Gives the CPU to the next ready thread, if any. The current thread goes to the back of the queue
/// (or to the zombies, if it has exited).
pub fn yield_now() {
    let flags = irq_save();
    let switch = with_sched(|s| {
        let next = s.ready.pop_front()?;
        let mut prev = s.current.replace(next).expect("sched: niciun fir curent");
        let old_rsp: *mut u64 = &mut prev.rsp; // the Box keeps this address stable when it moves
        if prev.state == State::Exited {
            s.zombies.push(prev);
        } else {
            s.ready.push_back(prev);
        }
        Some((old_rsp, s.current.as_ref().unwrap().rsp))
    });
    if let Some((old_rsp, new_rsp)) = switch {
        // SAFETY: interrupts are off; `old_rsp` points into a Thread kept alive by the scheduler queues;
        // `new_rsp` was saved by `switch_context` or prepared by `initial_stack`.
        unsafe { switch::switch_context(old_rsp, new_rsp) };
        reap();
    }
    irq_restore(flags);
}

/// Ends the current thread. Never returns.
pub fn exit() -> ! {
    with_sched(|s| s.current.as_mut().expect("sched: niciun fir curent").state = State::Exited);
    yield_now();
    panic!("sched: ultimul fir a iesit si nu mai e nimic de rulat");
}

/// Frees the stacks of exited threads. Called right after every switch, never on a zombie's own stack.
fn reap() {
    // Dropped outside the lock, one at a time, so the queue's own buffer is kept.
    while let Some(dead) = with_sched(|s| s.zombies.pop()) {
        drop(dead);
    }
}

/// Start of every new thread (called from `switch::thread_trampoline` with the entry function).
extern "C" fn thread_entry(f: usize) -> ! {
    reap();
    // A new thread starts from a switch made with interrupts disabled.
    // SAFETY: the IDT and the APIC are set up before any thread is spawned.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    // SAFETY: `f` was a `fn()` passed to `spawn`.
    let f: fn() = unsafe { core::mem::transmute(f) };
    f();
    exit()
}
