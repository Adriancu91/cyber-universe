//! Kernel threads and the scheduler. Ready threads wait in a round-robin queue; a thread gives up the CPU
//! with `yield_now`, and the timer takes it away every tick (10 ms) unless preemption is disabled.
//! The boot code becomes thread 0 ("main") in `init`.

mod mutex;
mod switch;
mod wait;

#[allow(unused_imports)] // Mutex, sleep_ms: used by the self-tests now, by drivers and processes later
pub use mutex::Mutex;
pub use wait::WaitQueue;
#[allow(unused_imports)]
pub use wait::sleep_ms;

use alloc::{boxed::Box, collections::VecDeque, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Stack size of every new kernel thread.
const STACK_SIZE: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ready,
    /// Waiting for a `WaitQueue` and/or for the tick `wake_at`.
    Blocked,
    Exited,
}

struct Thread {
    id: u64,
    state: State,
    /// Saved stack pointer while the thread is not running.
    rsp: u64,
    /// While blocked: the queue it waits on (its address, 0 = none) and the tick to wake at (0 = none).
    wait_key: usize,
    wake_at: u64,
    /// PML4 the thread runs with (0 = the kernel's own tables).
    root: u64,
    /// Top of the thread's kernel stack (0 for the boot thread): where ring 3 enters the kernel.
    kstack_top: u64,
    /// A system thread (a driver's helper) that runs as long as the kernel: not counted by `alive`.
    system: bool,
    /// Asked to stop (`kill`): a user program ends at its next return to ring 3.
    killed: bool,
    /// Timer ticks during which this thread was running (processor time).
    cpu_ticks: u64,
    /// The thread's stack; `None` for the boot thread, which runs on the kernel's boot stack.
    _stack: Option<Box<[u8]>>,
    /// Saved vector registers (x87/SSE/AVX) while the thread is not running (K6-01).
    fpu: Option<crate::arch::x86_64::fpu::Area>,
}

struct Scheduler {
    current: Option<Box<Thread>>,
    ready: VecDeque<Box<Thread>>,
    /// Blocked threads, in the order they blocked (so wake-ups are first come, first served).
    blocked: Vec<Box<Thread>>,
    /// Runs `hlt` when every other thread is blocked; never in the ready queue.
    idle: Option<Box<Thread>>,
    /// Exited threads whose stacks are freed by the next thread that runs (not on their own stack).
    zombies: Vec<Box<Thread>>,
}

static mut SCHED: Scheduler =
    Scheduler { current: None, ready: VecDeque::new(), blocked: Vec::new(), idle: None, zombies: Vec::new() };
/// Id of the idle thread.
const IDLE_ID: u64 = u64::MAX;
static LOCK: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Preemption is allowed only while this is 0 (see `preempt_disable`). Starts disabled until `init`.
static PREEMPT_OFF: AtomicU64 = AtomicU64::new(1);
/// Context switches done so far.
static SWITCHES: AtomicU64 = AtomicU64::new(0);

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

pub(crate) fn irq_save() -> u64 {
    let flags: u64;
    // SAFETY: reads RFLAGS and masks interrupts on this CPU; restored by `irq_restore`.
    unsafe { core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem)) }
    flags
}

pub(crate) fn irq_restore(flags: u64) {
    if flags & (1 << 9) != 0 {
        // SAFETY: interrupts were enabled before `irq_save`.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    }
}

/// Makes the running boot code thread 0. Must run once, after the heap is ready.
pub fn init() {
    with_sched(|s| {
        s.ready.reserve(16);
        s.blocked.reserve(16);
        s.zombies.reserve(16);
        s.current = Some(Box::new(Thread { id: 0, state: State::Ready, rsp: 0, wait_key: 0, wake_at: 0, root: 0, kstack_top: 0, system: false, killed: false, cpu_ticks: 0, _stack: None, fpu: crate::arch::x86_64::fpu::Area::new() }));
    });
    let idle = new_thread(IDLE_ID, idle_loop);
    with_sched(|s| s.idle = Some(idle));
    PREEMPT_OFF.store(0, Ordering::Release);
    crate::info!(
        "sched: planificator gata (round-robin, felie {} ms, stiva {} KiB)",
        1000 / crate::arch::x86_64::apic::HZ,
        STACK_SIZE / 1024
    );
}

/// Stops the timer from switching threads until the matching `preempt_enable` (calls nest).
#[allow(dead_code)] // used by the self-tests now
pub fn preempt_disable() {
    PREEMPT_OFF.fetch_add(1, Ordering::AcqRel);
}

/// Undoes one `preempt_disable`.
#[allow(dead_code)] // used by the self-tests now
pub fn preempt_enable() {
    PREEMPT_OFF.fetch_sub(1, Ordering::AcqRel);
}

/// Called by the timer interrupt handler (interrupts disabled, EOI already sent): the running thread's
/// time slice is over, so the next ready thread runs.
pub fn timer_tick() {
    let now = crate::arch::x86_64::apic::ticks();
    with_sched(|s| {
        if let Some(t) = s.current.as_mut() {
            t.cpu_ticks += 1;
        }
        wake_where(s, usize::MAX, |t| t.wake_at != 0 && t.wake_at <= now)
    });
    if PREEMPT_OFF.load(Ordering::Acquire) == 0 {
        yield_now();
    }
}

/// Number of context switches since boot.
#[allow(dead_code)] // used by the self-tests now
pub fn switches() -> u64 {
    SWITCHES.load(Ordering::Relaxed)
}

/// Creates a kernel thread that runs `f` and exits when `f` returns. Returns its id.
#[allow(dead_code)] // used by the self-tests now, by the shell and processes in Phase 2
pub fn spawn(f: fn()) -> u64 {
    spawn_in(0, f)
}

/// Creates a system thread (it runs as long as the kernel and is not counted by `alive`).
pub fn spawn_system(f: fn()) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut t = new_thread(id, f);
    t.system = true;
    with_sched(|s| s.ready.push_back(t));
    id
}

/// Like `spawn`, but the thread runs with the page tables at `root` (an `AddressSpace`; 0 = kernel's).
/// The address space must outlive the thread.
pub fn spawn_in(root: u64, f: fn()) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut t = new_thread(id, f);
    t.root = root;
    with_sched(|s| s.ready.push_back(t));
    id
}

fn new_thread(id: u64, f: fn()) -> Box<Thread> {
    let stack = vec![0u8; STACK_SIZE].into_boxed_slice();
    let top = (stack.as_ptr() as u64 + STACK_SIZE as u64) & !15;
    // SAFETY: the 56 bytes below `top` belong to `stack`, which the new thread owns.
    let rsp = unsafe { switch::initial_stack(top, f as *const () as usize) };
    Box::new(Thread { id, state: State::Ready, rsp, wait_key: 0, wake_at: 0, root: 0, kstack_top: top, system: false, killed: false, cpu_ticks: 0, _stack: Some(stack), fpu: crate::arch::x86_64::fpu::Area::new() })
}

/// The idle thread: sleeps until an interrupt, then lets whoever was woken run.
fn idle_loop() {
    loop {
        crate::arch::x86_64::hlt();
        yield_now();
    }
}

/// Id of the running thread.
#[allow(dead_code)] // used by the self-tests now
pub fn current_id() -> u64 {
    with_sched(|s| s.current.as_ref().map_or(0, |t| t.id))
}

/// Asks thread `id` to stop and wakes it if it is blocked. The user program on
/// it ends at its next return to ring 3 (`syscall::dispatch`, timer interrupt). Returns whether the
/// thread exists. System threads and the idle thread cannot be stopped.
pub fn kill(id: u64) -> bool {
    with_sched(|s| {
        let mark = |t: &mut Thread| {
            if t.id == id && !t.system && t.id != IDLE_ID && t.state != State::Exited {
                t.killed = true;
                true
            } else {
                false
            }
        };
        let found = s.current.as_deref_mut().is_some_and(mark)
            || s.ready.iter_mut().any(|t| mark(t))
            || s.blocked.iter_mut().any(|t| mark(t));
        if found {
            // Every wait re-checks its condition after waking, so an early wake-up is harmless; the
            // waits that give up for a stopped thread (`wait_until_or`, `sleep_ms`) then return.
            wake_where(s, 1, |t| t.id == id);
        }
        found
    })
}

/// Processor time of thread `id` in timer ticks (`None` if it does not exist), and of the idle thread
/// (time when nothing else had work).
pub fn cpu_ticks(id: u64) -> Option<u64> {
    with_sched(|s| {
        let found = |t: &Thread| (t.id == id).then_some(t.cpu_ticks);
        s.current.as_deref().and_then(found)
            .or_else(|| s.ready.iter().find_map(|t| found(t)))
            .or_else(|| s.blocked.iter().find_map(|t| found(t)))
            .or_else(|| s.idle.as_deref().and_then(found))
    })
}

/// Id of the idle thread (for `cpu_ticks`).
pub const IDLE: u64 = IDLE_ID;

/// The running thread switches to the kernel's own page tables for good (before its address space is
/// destroyed): later thread switches must not load the old tables again.
pub fn use_kernel_tables() {
    let flags = irq_save();
    with_sched(|s| {
        if let Some(t) = s.current.as_mut() {
            t.root = 0;
        }
    });
    // SAFETY: the kernel's tables map the whole kernel half, like every address space does.
    unsafe { crate::mem::paging::load(crate::mem::paging::kernel_root()) };
    irq_restore(flags);
}

/// Whether the running thread was asked to stop.
pub fn killed() -> bool {
    with_sched(|s| s.current.as_ref().is_some_and(|t| t.killed))
}

/// Number of threads that have not exited (running, ready or blocked), without the idle thread and
/// the system threads.
#[allow(dead_code)] // used by the self-tests now
pub fn alive() -> usize {
    with_sched(|s| {
        let counted = |t: &Thread| !t.system && t.id != IDLE_ID && t.state != State::Exited;
        s.ready.iter().filter(|t| counted(t)).count()
            + s.blocked.iter().filter(|t| counted(t)).count()
            + s.current.as_deref().is_some_and(counted) as usize
    })
}

/// Moves up to `max` blocked threads for which `matches` is true to the ready queue, oldest first.
fn wake_where(s: &mut Scheduler, max: usize, matches: impl Fn(&Thread) -> bool) -> usize {
    let mut woken = 0;
    let mut i = 0;
    while i < s.blocked.len() && woken < max {
        if matches(&s.blocked[i]) {
            let mut t = s.blocked.remove(i);
            t.state = State::Ready;
            t.wait_key = 0;
            t.wake_at = 0;
            s.ready.push_back(t);
            woken += 1;
        } else {
            i += 1;
        }
    }
    woken
}

/// Blocks the current thread until `wake(key)` or the tick `wake_at` (0 = no time limit).
/// The caller must have disabled interrupts, after checking the condition it waits for (no lost wake-up).
fn block_current(key: usize, wake_at: u64) {
    with_sched(|s| {
        let t = s.current.as_mut().expect("sched: niciun fir curent");
        t.state = State::Blocked;
        t.wait_key = key;
        t.wake_at = wake_at;
    });
    yield_now();
}

/// Wakes up to `max` threads blocked on `key`. Safe to call from interrupt handlers.
fn wake(key: usize, max: usize) -> usize {
    with_sched(|s| wake_where(s, max, |t| t.wait_key == key))
}

/// Gives the CPU to the next ready thread, if any. The current thread goes to the back of the queue
/// (or to the zombies, if it has exited).
pub fn yield_now() {
    let flags = irq_save();
    let switch = with_sched(|s| {
        let runnable = s.current.as_ref().is_some_and(|t| t.state == State::Ready);
        let next = match s.ready.pop_front() {
            Some(t) => t,
            None if !runnable => s.idle.take().expect("sched: firul idle lipseste"),
            None => return None, // nothing else to run: keep going
        };
        let mut prev = s.current.replace(next).expect("sched: niciun fir curent");
        let old_rsp: *mut u64 = &mut prev.rsp; // the Box keeps this address stable when it moves
        let old_fpu = prev.fpu.as_ref().map_or(core::ptr::null_mut(), |a| a.as_ptr());
        match prev.state {
            State::Exited => s.zombies.push(prev),
            State::Blocked => s.blocked.push(prev),
            State::Ready if prev.id == IDLE_ID => s.idle = Some(prev),
            State::Ready => s.ready.push_back(prev),
        }
        let next = s.current.as_ref().unwrap();
        Some((old_rsp, next.rsp, next.root, next.kstack_top, old_fpu, next.fpu.as_ref().map_or(core::ptr::null_mut(), |a| a.as_ptr())))
    });
    if let Some((old_rsp, new_rsp, root, kstack_top, old_fpu, new_fpu)) = switch {
        // The vector registers belong to the thread: saved here, loaded for the next one (the kernel never uses them).
        // SAFETY: both areas come from `fpu::Area` (aligned, `area_size()` bytes) and live in Threads kept by the
        // scheduler queues until after this switch (zombies are freed by `reap` afterwards).
        unsafe {
            if !old_fpu.is_null() {
                crate::arch::x86_64::fpu::save(old_fpu);
            }
            if !new_fpu.is_null() {
                crate::arch::x86_64::fpu::restore(new_fpu);
            }
        }
        if kstack_top != 0 {
            crate::arch::x86_64::syscall::set_kernel_stack(kstack_top);
        }
        SWITCHES.fetch_add(1, Ordering::Relaxed);
        // SAFETY: every root shares the kernel's upper half, where this code and every kernel stack live.
        unsafe { crate::mem::paging::load(if root == 0 { crate::mem::paging::kernel_root() } else { root }) };
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
    unreachable!("sched: un fir terminat a fost reluat");
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
