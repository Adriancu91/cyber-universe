//! K6-02 / K6-02b: every processor started, then threads on all of them. Checks, with QEMU `-smp 4`:
//! - busy threads really run on every processor (the processor index is recorded by each thread);
//! - `sched::Mutex` stays exact when 4 threads on different processors take it 10000 times each;
//! - wait queues lose no wake-up between processors (two threads hand a token back and forth 2000 times);
//! - user programs run at the same time on several processors (system calls use each processor's own kernel stack);
//! - removing a kernel page reaches the other processors' TLBs (shootdown, answered by every processor).

use crate::arch::x86_64::{apic, smp};
use crate::sched;
use core::sync::atomic::{AtomicU64, Ordering};

static SEEN: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);

/// Fails the test if `done()` does not become true within `seconds` (a lost wake-up would hang forever).
fn wait_for(what: &str, seconds: u64, done: impl Fn() -> bool) {
    let limit = apic::ticks() + seconds * apic::HZ;
    while !done() {
        assert!(apic::ticks() < limit, "smp: {} nu s-a terminat in {} s", what, seconds);
        sched::sleep_ms(10);
    }
}

fn busy() {
    let end = apic::ticks() + apic::HZ / 3;
    let mut x = 1u64;
    while apic::ticks() < end {
        SEEN.fetch_or(1 << sched::current_cpu(), Ordering::Relaxed);
        for _ in 0..1000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        core::hint::black_box(x);
    }
    DONE.fetch_add(1, Ordering::AcqRel);
}

static COUNTER: sched::Mutex<u64> = sched::Mutex::new(0);
static MUTEX_CPUS: AtomicU64 = AtomicU64::new(0);

fn mutex_worker() {
    for i in 0..10_000 {
        *COUNTER.lock() += 1;
        MUTEX_CPUS.fetch_or(1 << sched::current_cpu(), Ordering::Relaxed);
        if i % 1000 == 0 {
            sched::yield_now();
        }
    }
    DONE.fetch_add(1, Ordering::AcqRel);
}

const HANDOFFS: u64 = 2000;
static TOKEN: AtomicU64 = AtomicU64::new(0);
static TOKEN_WAIT: sched::WaitQueue = sched::WaitQueue::new();

/// Waits for the token to be `mine`, passes it on as `mine + 1` (two threads: even and odd).
fn ping_pong(parity: u64) {
    let mut turn = parity;
    while turn < HANDOFFS {
        TOKEN_WAIT.wait_until(|| TOKEN.load(Ordering::Acquire) == turn);
        TOKEN.store(turn + 1, Ordering::Release);
        TOKEN_WAIT.wake_all();
        turn += 2;
    }
    DONE.fetch_add(1, Ordering::AcqRel);
}

fn ping() {
    ping_pong(0)
}

fn pong() {
    ping_pong(1)
}

pub fn run() {
    let n = smp::start_all();
    let cpus = crate::acpi::madt::get().cpu_count as u32;
    assert!(n == cpus, "smp: {} nuclee pornite din {}", n, cpus);
    assert!(smp::start_all() == n, "smp: a doua pornire a schimbat ceva");
    assert!(sched::running_cpus() == n as u64, "smp: {} nuclee in planificator din {}", sched::running_cpus(), n);
    let all = (1u64 << n) - 1;

    // 1. Busy threads end up on every processor.
    DONE.store(0, Ordering::Release);
    for _ in 0..2 * n {
        sched::spawn(busy);
    }
    wait_for("firele ocupate", 20, || DONE.load(Ordering::Acquire) == 2 * n as u64);
    let seen = SEEN.load(Ordering::Relaxed);
    assert!(seen == all, "smp: firele au rulat doar pe nucleele {:#b} din {:#b}", seen, all);
    crate::info!("test: smp: {} fire ocupate au rulat pe toate cele {} nuclee", 2 * n, n);

    // 2. The mutex across processors.
    DONE.store(0, Ordering::Release);
    for _ in 0..4 {
        sched::spawn(mutex_worker);
    }
    wait_for("testul mutex", 30, || DONE.load(Ordering::Acquire) == 4);
    let total = *COUNTER.lock();
    assert!(total == 40_000, "smp: mutex: {} in loc de 40000", total);
    crate::info!(
        "test: smp: mutex: 4 fire x 10000 = {} (pe {} nuclee)",
        total,
        MUTEX_CPUS.load(Ordering::Relaxed).count_ones()
    );

    // 3. Wake-ups between processors.
    DONE.store(0, Ordering::Release);
    TOKEN.store(0, Ordering::Release);
    let t0 = apic::ticks();
    sched::spawn(ping);
    sched::spawn(pong);
    wait_for("predarea jetonului", 30, || DONE.load(Ordering::Acquire) == 2);
    crate::info!(
        "test: smp: {} predari intre fire fara trezire pierduta ({} ms)",
        HANDOFFS,
        (apic::ticks() - t0) * 1000 / apic::HZ
    );

    // 4. Programs on several processors at once.
    let ids: alloc::vec::Vec<u64> =
        (0..2 * n).map(|_| crate::proc::spawn_path("/bin/vectori", &["vectori"]).expect("smp: vectori nu porneste")).collect();
    for id in ids {
        let code = crate::proc::wait(id);
        assert!(code == 0, "smp: vectori a iesit cu {}", code);
    }
    crate::info!("test: smp: {} programe vectori in paralel, toate corecte", 2 * n);

    // 5. TLB shootdown.
    const VIRT: u64 = 0xFFFF_C800_0000_0000;
    let before = apic::shootdowns();
    for _ in 0..50 {
        let f = crate::mem::frame::alloc_frame().expect("smp: niciun cadru liber");
        crate::mem::paging::map(VIRT, f, crate::mem::paging::WRITABLE | crate::mem::paging::NO_EXECUTE).expect("smp: maparea");
        // SAFETY: VIRT was just mapped to the frame `f`, owned by this test.
        unsafe { (VIRT as *mut u64).write_volatile(f) };
        assert!(crate::mem::paging::unmap(VIRT) == Some(f), "smp: demapare gresita");
        crate::mem::frame::free_frame(f);
    }
    let done = apic::shootdowns() - before;
    assert!(done == 50, "smp: {} shootdown-uri in loc de 50", done);
    crate::info!("test: smp: 50 pagini demapate, TLB golit pe toate nucleele");

    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    crate::info!("test: smp: {} nuclee ruleaza fire; planificatorul SMP OK", n);
}
