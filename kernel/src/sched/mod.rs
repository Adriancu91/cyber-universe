//! Kernel threads and the scheduler. Ready threads wait in one round-robin queue shared by every processor
//! (K6-02b); a thread gives up its processor with `yield_now`, and the timer takes it away every tick (10 ms)
//! unless preemption is disabled. Each processor has its own running thread and its own idle thread.
//! The boot code becomes thread 0 ("main") in `init`; an additional processor joins with `ap_join`.
//!
//! On several processors the scheduler lock is handed over across the switch: the switching thread takes it,
//! puts itself in a queue, switches, and the thread that resumes releases it. So no other processor can pick a
//! thread whose registers are not saved yet.

mod mutex;
pub mod spin;
mod switch;
mod wait;

#[allow(unused_imports)] // Mutex, sleep_ms: used by the self-tests now, by drivers and processes later
pub use mutex::Mutex;
pub use wait::WaitQueue;
#[allow(unused_imports)]
pub use wait::sleep_ms;

use crate::arch::x86_64::percpu::{self, MAX_CPUS};
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
    /// Between `prepare_wait` and `block_current`: the thread checks its condition, still running. A wake-up for
    /// `wait_key` in that window clears this flag, and `block_current` then does not sleep (no lost wake-up when the
    /// waker runs on another processor).
    preparing: bool,
}

impl Thread {
    /// Whether a wake-up can reach it: sleeping, about to sleep, or checking its condition before sleeping.
    fn waiting(&self) -> bool {
        self.state == State::Blocked || self.preparing
    }

    fn make_ready(&mut self) {
        self.state = State::Ready;
        self.preparing = false;
        self.wait_key = 0;
        self.wake_at = 0;
    }
}

/// What one processor runs.
struct Cpu {
    current: Option<Box<Thread>>,
    /// Runs `hlt` when every other thread is blocked; never in the ready queue, never on another processor.
    idle: Option<Box<Thread>>,
}

struct Scheduler {
    cpus: [Cpu; MAX_CPUS],
    ready: VecDeque<Box<Thread>>,
    /// Blocked threads, in the order they blocked (so wake-ups are first come, first served).
    blocked: Vec<Box<Thread>>,
    /// Exited threads whose stacks are freed by the next thread that runs (not on their own stack).
    zombies: Vec<Box<Thread>>,
}

static mut SCHED: Scheduler = Scheduler {
    cpus: [const { Cpu { current: None, idle: None } }; MAX_CPUS],
    ready: VecDeque::new(),
    blocked: Vec::new(),
    zombies: Vec::new(),
};
/// Processors that run threads (the boot processor counts once `init` ran).
static RUNNING_CPUS: AtomicU64 = AtomicU64::new(0);
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
    let flags = spin::lock(&LOCK);
    // SAFETY: LOCK grants exclusive access until released below.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(SCHED) });
    spin::unlock(&LOCK, flags);
    r
}

impl Scheduler {
    /// The thread running on this processor (the lock is held, so interrupts are off and it cannot move).
    fn cur(&mut self) -> &mut Thread {
        self.cpus[percpu::index()].current.as_deref_mut().expect("sched: niciun fir curent")
    }

    /// Every running thread (one per processor).
    fn running(&mut self) -> impl Iterator<Item = &mut Box<Thread>> {
        self.cpus.iter_mut().filter_map(|c| c.current.as_mut())
    }
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

/// A thread record for code already running on its own stack (the boot code, an additional processor's start).
fn running_thread(id: u64) -> Box<Thread> {
    Box::new(Thread { id, state: State::Ready, rsp: 0, wait_key: 0, wake_at: 0, root: 0, kstack_top: 0, system: false, killed: false, cpu_ticks: 0, _stack: None, fpu: crate::arch::x86_64::fpu::Area::new(), preparing: false })
}

/// Makes the running boot code thread 0. Must run once, after the heap is ready.
pub fn init() {
    let main = running_thread(0);
    with_sched(|s| {
        s.ready.reserve(16);
        s.blocked.reserve(16);
        s.zombies.reserve(16);
        s.cpus[0].current = Some(main);
    });
    let idle = new_thread(IDLE_ID, idle_loop);
    with_sched(|s| s.cpus[0].idle = Some(idle));
    RUNNING_CPUS.store(1, Ordering::Release);
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
        s.cur().cpu_ticks += 1;
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
    Box::new(Thread { id, state: State::Ready, rsp, wait_key: 0, wake_at: 0, root: 0, kstack_top: top, system: false, killed: false, cpu_ticks: 0, _stack: Some(stack), fpu: crate::arch::x86_64::fpu::Area::new(), preparing: false })
}

/// An additional processor (K6-02b) becomes part of the scheduler: the code running now, on the processor's start
/// stack, becomes its idle thread; its timer starts; from here on it runs threads from the shared queue.
/// Interrupts must be off; `index` is the processor's index in percpu.rs. Never returns.
pub fn ap_join(index: usize) -> ! {
    let idle = running_thread(IDLE_ID);
    with_sched(|s| s.cpus[index].current = Some(idle));
    crate::arch::x86_64::apic::mark_active(index);
    crate::arch::x86_64::apic::start_timer_ap();
    RUNNING_CPUS.fetch_add(1, Ordering::AcqRel);
    // SAFETY: the IDT, this processor's GDT/TSS/APIC and the scheduler entry are ready.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    idle_loop();
    unreachable!()
}

/// Processors that run threads.
pub fn running_cpus() -> u64 {
    RUNNING_CPUS.load(Ordering::Acquire)
}

/// Index of the processor running the calling thread (it may move right after; for logs and tests).
pub fn current_cpu() -> usize {
    let flags = irq_save();
    let i = percpu::index();
    irq_restore(flags);
    i
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
    with_sched(|s| s.cpus[percpu::index()].current.as_ref().map_or(0, |t| t.id))
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
        let found = s.running().any(|t| mark(t))
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
        if id == IDLE_ID {
            // the boot processor's idle thread (the time base of the system monitor)
            return s.cpus[0].current.as_deref().and_then(found).or_else(|| s.cpus[0].idle.as_deref().and_then(found));
        }
        s.cpus.iter().find_map(|c| c.current.as_deref().and_then(found))
            .or_else(|| s.ready.iter().find_map(|t| found(t)))
            .or_else(|| s.blocked.iter().find_map(|t| found(t)))
    })
}

/// Id of the idle thread (for `cpu_ticks`: the boot processor's).
pub const IDLE: u64 = IDLE_ID;

/// The running thread switches to the kernel's own page tables for good (before its address space is
/// destroyed): later thread switches must not load the old tables again.
pub fn use_kernel_tables() {
    let flags = irq_save();
    with_sched(|s| s.cur().root = 0);
    // SAFETY: the kernel's tables map the whole kernel half, like every address space does.
    unsafe { crate::mem::paging::load(crate::mem::paging::kernel_root()) };
    irq_restore(flags);
}

/// Whether the running thread was asked to stop.
pub fn killed() -> bool {
    with_sched(|s| s.cur().killed)
}

/// Number of threads that have not exited (running, ready or blocked), without the idle thread and
/// the system threads.
#[allow(dead_code)] // used by the self-tests now
pub fn alive() -> usize {
    with_sched(|s| {
        let counted = |t: &Thread| !t.system && t.id != IDLE_ID && t.state != State::Exited;
        s.ready.iter().filter(|t| counted(t)).count()
            + s.blocked.iter().filter(|t| counted(t)).count()
            + s.cpus.iter().filter(|c| c.current.as_deref().is_some_and(counted)).count()
    })
}

/// Moves up to `max` blocked threads for which `matches` is true to the ready queue, oldest first. Threads still
/// running on a processor while they get ready to sleep (`prepare_wait`, or blocked but not yet switched out) are
/// woken too: they simply do not sleep.
fn wake_where(s: &mut Scheduler, max: usize, matches: impl Fn(&Thread) -> bool) -> usize {
    let mut woken = 0;
    let mut i = 0;
    while i < s.blocked.len() && woken < max {
        if matches(&s.blocked[i]) {
            let mut t = s.blocked.remove(i);
            t.make_ready();
            s.ready.push_back(t);
            woken += 1;
        } else {
            i += 1;
        }
    }
    for t in s.cpus.iter_mut().filter_map(|c| c.current.as_deref_mut()) {
        if woken < max && t.waiting() && t.id != IDLE_ID && matches(t) {
            t.make_ready();
            woken += 1;
        }
    }
    woken
}

/// First half of a wait: from now on a `wake(key)` (or a stop request) is remembered until `block_current`.
/// Interrupts must be off from here until `block_current` or `cancel_wait`.
fn prepare_wait(key: usize) {
    with_sched(|s| {
        let t = s.cur();
        t.wait_key = key;
        t.preparing = true;
    });
}

/// The condition became true after `prepare_wait`: no sleep.
fn cancel_wait() {
    with_sched(|s| {
        let t = s.cur();
        t.preparing = false;
        t.wait_key = 0;
    });
}

/// Blocks the current thread until `wake(key)` or the tick `wake_at` (0 = no time limit). The caller must have
/// called `prepare_wait(key)` with interrupts disabled, then checked the condition it waits for: a wake-up that came
/// in between (from any processor) is not lost: the thread then does not sleep at all.
fn block_current(key: usize, wake_at: u64) {
    let sleep = with_sched(|s| {
        let t = s.cur();
        if !t.preparing {
            t.make_ready(); // already woken
            return false;
        }
        t.preparing = false;
        t.state = State::Blocked;
        t.wait_key = key;
        t.wake_at = wake_at;
        true
    });
    if sleep {
        yield_now();
    }
}

/// Wakes up to `max` threads blocked on `key`. Safe to call from interrupt handlers.
fn wake(key: usize, max: usize) -> usize {
    with_sched(|s| wake_where(s, max, |t| t.wait_key == key))
}

/// Gives the CPU to the next ready thread, if any. The current thread goes to the back of the queue
/// (or to the zombies, if it has exited).
pub fn yield_now() {
    let flags = spin::lock(&LOCK);
    // SAFETY: LOCK is held (and handed over across the switch below, see the module comment).
    let s = unsafe { &mut *core::ptr::addr_of_mut!(SCHED) };
    let cpu = percpu::index();
    let runnable = s.cpus[cpu].current.as_ref().is_some_and(|t| t.state == State::Ready);
    let next = match s.ready.pop_front() {
        Some(t) => t,
        None if !runnable => s.cpus[cpu].idle.take().expect("sched: firul idle lipseste"),
        None => {
            spin::unlock(&LOCK, flags); // nothing else to run: keep going
            return;
        }
    };
    let mut prev = s.cpus[cpu].current.replace(next).expect("sched: niciun fir curent");
    let old_rsp: *mut u64 = &mut prev.rsp; // the Box keeps this address stable when it moves
    let old_fpu = prev.fpu.as_ref().map_or(core::ptr::null_mut(), |a| a.as_ptr());
    match prev.state {
        State::Exited => s.zombies.push(prev),
        State::Blocked => s.blocked.push(prev),
        State::Ready if prev.id == IDLE_ID => s.cpus[cpu].idle = Some(prev),
        State::Ready => s.ready.push_back(prev),
    }
    let next = s.cpus[cpu].current.as_ref().unwrap();
    let (new_rsp, root, kstack_top) = (next.rsp, next.root, next.kstack_top);
    let new_fpu = next.fpu.as_ref().map_or(core::ptr::null_mut(), |a| a.as_ptr());
    {
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
        // SAFETY: interrupts are off; `old_rsp` points into a Thread kept alive by the scheduler queues (no other
        // processor can take it before LOCK is released, after its registers are saved); `new_rsp` was saved by
        // `switch_context` or prepared by `initial_stack`.
        unsafe { switch::switch_context(old_rsp, new_rsp) };
        // Running again (maybe on another processor): release the lock the switching thread handed over.
        LOCK.store(false, Ordering::Release);
        reap();
    }
    irq_restore(flags);
}

/// Ends the current thread. Never returns.
pub fn exit() -> ! {
    with_sched(|s| s.cur().state = State::Exited);
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
    // The switch that started this thread handed over the scheduler lock (see `yield_now`).
    LOCK.store(false, Ordering::Release);
    reap();
    // A new thread starts from a switch made with interrupts disabled.
    // SAFETY: the IDT and the APIC are set up before any thread is spawned.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    // SAFETY: `f` was a `fn()` passed to `spawn`.
    let f: fn() = unsafe { core::mem::transmute(f) };
    f();
    exit()
}
