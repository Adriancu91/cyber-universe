//! K6-01: the vector registers (x87/SSE/AVX) belong to each thread. Two threads fill XMM0–XMM13 (and, with AVX, the
//! upper halves of YMM0–YMM13) with their own pattern and check it after thousands of switches (yields and timer
//! preemption). Then a ring 3 program computes with AVX2 (or SSE2 without it).
//! The kernel is soft-float, so the compiler never touches these registers: the asm blocks own them.

use crate::arch::x86_64::fpu;
use core::sync::atomic::{AtomicU64, Ordering};

const ROUNDS: u64 = 3000;
static ERRORS: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);

/// Puts `base`, `base+1`, … in XMM0–XMM13 (low 64 bits).
fn fill(base: u64) {
    // SAFETY: only vector registers are written; the soft-float kernel never uses them.
    unsafe {
        core::arch::asm!(
"movq xmm0, rcx", "inc rcx",
"movq xmm1, rcx", "inc rcx",
"movq xmm2, rcx", "inc rcx",
"movq xmm3, rcx", "inc rcx",
"movq xmm4, rcx", "inc rcx",
"movq xmm5, rcx", "inc rcx",
"movq xmm6, rcx", "inc rcx",
"movq xmm7, rcx", "inc rcx",
"movq xmm8, rcx", "inc rcx",
"movq xmm9, rcx", "inc rcx",
"movq xmm10, rcx", "inc rcx",
"movq xmm11, rcx", "inc rcx",
"movq xmm12, rcx", "inc rcx",
"movq xmm13, rcx", "inc rcx",
            inout("rcx") base => _,
            options(nomem, nostack)
        )
    }
    if fpu::has_avx() {
        // SAFETY: AVX is enabled in XCR0 (checked); copies each XMM into the upper half of its YMM.
        unsafe {
            core::arch::asm!(
"vinsertf128 ymm0, ymm0, xmm0, 1",
"vinsertf128 ymm1, ymm1, xmm1, 1",
"vinsertf128 ymm2, ymm2, xmm2, 1",
"vinsertf128 ymm3, ymm3, xmm3, 1",
"vinsertf128 ymm4, ymm4, xmm4, 1",
"vinsertf128 ymm5, ymm5, xmm5, 1",
"vinsertf128 ymm6, ymm6, xmm6, 1",
"vinsertf128 ymm7, ymm7, xmm7, 1",
"vinsertf128 ymm8, ymm8, xmm8, 1",
"vinsertf128 ymm9, ymm9, xmm9, 1",
"vinsertf128 ymm10, ymm10, xmm10, 1",
"vinsertf128 ymm11, ymm11, xmm11, 1",
"vinsertf128 ymm12, ymm12, xmm12, 1",
"vinsertf128 ymm13, ymm13, xmm13, 1",
                options(nomem, nostack)
            )
        }
    }
}

/// 0 if XMM0–XMM13 (and the YMM upper halves) still hold the pattern from `fill(base)`.
fn check(base: u64) -> u64 {
    let mut bad: u64;
    // SAFETY: reads vector registers into general ones.
    unsafe {
        core::arch::asm!(
            "xor eax, eax",
"movq rdx, xmm0", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm1", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm2", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm3", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm4", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm5", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm6", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm7", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm8", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm9", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm10", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm11", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm12", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"movq rdx, xmm13", "xor rdx, rcx", "or rax, rdx", "inc rcx",
            inout("rcx") base => _, out("rax") bad, out("rdx") _,
            options(nomem, nostack)
        )
    }
    if fpu::has_avx() {
        let upper: u64;
        // SAFETY: AVX is enabled; XMM14 is scratch (not part of the pattern).
        unsafe {
            core::arch::asm!(
                "xor eax, eax",
"vextractf128 xmm14, ymm0, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm1, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm2, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm3, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm4, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm5, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm6, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm7, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm8, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm9, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm10, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm11, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm12, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
"vextractf128 xmm14, ymm13, 1", "movq rdx, xmm14", "xor rdx, rcx", "or rax, rdx", "inc rcx",
                inout("rcx") base => _, out("rax") upper, out("rdx") _,
                options(nomem, nostack)
            )
        }
        bad |= upper;
    }
    bad
}

fn worker(base: u64) {
    fill(base);
    for round in 0..ROUNDS {
        if round % 3 == 0 {
            crate::sched::yield_now();
        } else {
            // let the timer interrupt preempt us now and then
            for _ in 0..2000 {
                core::hint::spin_loop();
            }
        }
        if check(base) != 0 {
            ERRORS.fetch_add(1, Ordering::Relaxed);
            fill(base);
        }
    }
    DONE.fetch_add(1, Ordering::Release);
}

fn worker_a() {
    worker(0xA000_0000_0000_0000);
}
fn worker_b() {
    worker(0xB000_0000_0000_0000);
}
/// With several processors (K6-02b) more threads than processors, each with its own pattern, so threads keep
/// switching and moving between processors.
static NEXT: AtomicU64 = AtomicU64::new(0);
fn worker_n() {
    worker(0xD000_0000_0000_0000 + (NEXT.fetch_add(1, Ordering::Relaxed) << 40));
}

pub fn run() {
    let before = crate::sched::switches();
    let cpus = crate::sched::running_cpus();
    let threads = if cpus > 1 { 3 * cpus } else { 3 };
    crate::sched::spawn(worker_a);
    crate::sched::spawn(worker_b);
    for _ in 3..threads {
        crate::sched::spawn(worker_n);
    }
    // the boot thread joins in with its own pattern
    worker(0xC000_0000_0000_0000);
    while DONE.load(Ordering::Acquire) < threads {
        crate::sched::yield_now();
    }
    let switches = crate::sched::switches() - before;
    let errors = ERRORS.load(Ordering::Relaxed);
    assert!(errors == 0, "vectori: {} verificari gresite (registre amestecate intre fire)", errors);
    assert!(switches >= ROUNDS, "vectori: prea putine comutari ({})", switches);
    crate::info!(
        "test: vectori: {} fire x {} runde, {} comutari, registrele XMM{} pastrate",
        threads,
        ROUNDS,
        switches,
        if fpu::has_avx() { " si YMM" } else { "" }
    );

    let file = crate::fs::read_all("/bin/vectori").expect("test: /bin/vectori lipseste");
    let id = crate::proc::spawn_elf(&file, &["vectori"]).expect("test: vectori nu porneste");
    let code = crate::proc::wait(id);
    assert!(code == 0, "vectori (ring 3) a iesit cu {}", code);
    crate::info!("test: vectori in ring 3 OK");
}
