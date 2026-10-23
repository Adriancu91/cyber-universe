//! P6-19 (command C-16): Kronos' arithmetic on the processors of Cyber Universe OS, with the operators of the
//! Cyber-GPU library `gpu-compute` (its CPU backend: the numerical oracle, pinned to commit 41e44ba).
//!
//! `kronos-calcul [n]` (default: every processor that runs threads) checks and times:
//! 1. one Kronos-shaped transformer block (embedding, layernorm, QKV, causal attention, projection, MLP with GELU,
//!    logits, cross-entropy; weights stored as bf16), once with `gpu_compute::reference` on one thread (the oracle)
//!    and once with the faster `gpu_compute::kernels` split by rows over n threads. Declared tolerance: 0 — the
//!    library promises bit-identical results, so every logit and the loss must be equal bit for bit;
//! 2. a 128 x 128 x 128 f32 matmul: reference (1 thread), `kernels::matmul_blocked` on 1 and n threads (tolerance 0),
//!    and our vector kernel (AVX2 + FMA, AVX2, or SSE2: whatever the processor and the kernel offer) on 1 and
//!    n threads. With FMA the rounding differs from the oracle, so the declared tolerance is
//!    |vector - reference| <= 1e-5 * (1 + |reference|); without FMA it is 0 (same operations, same order).
//! Exits with 0 when every comparison is within its tolerance.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{vec, vec::Vec};
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::{AtomicUsize, Ordering};
use cyber_user::{println, thread};
use gpu_compute::{kernels, reference, tensor::f32_to_bf16};

cyber_user::entry!(main);
cyber_user::heap!(8 * 1024 * 1024);

const MAX_THREADS: usize = 16;

// ---------------------------------------------------------------------------------------------------------------
// Work split by rows over threads.
// ---------------------------------------------------------------------------------------------------------------

struct Job<'a> {
    rows: usize,
    parts: usize,
    f: &'a (dyn Fn(usize, usize) + Sync),
}

/// Address of the `Job` the workers run (one job at a time; `par_rows` joins every worker before it returns).
static JOB: AtomicUsize = AtomicUsize::new(0);

fn split(rows: usize, parts: usize, part: usize) -> (usize, usize) {
    (rows * part / parts, rows * (part + 1) / parts)
}

extern "C" fn worker(part: u64) -> i64 {
    // SAFETY: JOB holds the address of the Job on `par_rows`' stack, which waits (join) for this thread.
    let job = unsafe { &*(JOB.load(Ordering::Acquire) as *const Job<'static>) };
    let (a, b) = split(job.rows, job.parts, part as usize);
    (job.f)(a, b);
    0
}

/// Calls `f(start, end)` on `parts` disjoint row ranges of `0..rows`, one per thread (part 0 on this thread).
fn par_rows(rows: usize, parts: usize, f: &(dyn Fn(usize, usize) + Sync)) {
    let parts = parts.clamp(1, MAX_THREADS).min(rows.max(1));
    if parts == 1 {
        f(0, rows);
        return;
    }
    let job = Job { rows, parts, f };
    JOB.store(&job as *const Job as usize, Ordering::Release);
    let mut threads = [(0i64, core::ptr::null_mut::<u8>()); MAX_THREADS];
    for (p, t) in threads.iter_mut().enumerate().take(parts).skip(1) {
        *t = thread::spawn_with_stack(worker, p as u64);
        if t.0 < 0 {
            println!("kronos-calcul: thread_spawn a esuat: {}", cyber_user::error_text(t.0));
            cyber_user::exit(20);
        }
    }
    let (a, b) = split(rows, parts, 0);
    f(a, b);
    for t in threads.iter().take(parts).skip(1) {
        thread::join(t.0);
        cyber_user::mem_unmap(t.1, thread::STACK);
    }
}

/// Rows `a..b` of a row-major matrix with `cols` columns that lives at `base` (shared between threads).
///
/// # Safety
/// `base` must point to a live f32 buffer of at least `b * cols` elements, and no other thread may touch these rows
/// while the slice is alive (`par_rows` gives each thread its own range).
unsafe fn rows_mut<'a>(base: usize, cols: usize, a: usize, b: usize) -> &'a mut [f32] {
    // SAFETY: guaranteed by the caller.
    unsafe { core::slice::from_raw_parts_mut((base as *mut f32).add(a * cols), (b - a) * cols) }
}

/// "1 fir" / "4 fire".
fn fire(n: usize) -> &'static str {
    if n == 1 { "fir" } else { "fire" }
}

fn now_ms() -> u64 {
    cyber_user::mem_info().map(|m| m.uptime_ms).unwrap_or(0)
}

fn check<T, E: core::fmt::Display>(what: &str, r: Result<T, E>) {
    if let Err(e) = r {
        println!("kronos-calcul: {}: {}", what, e);
        cyber_user::exit(30);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Vector matmul (our own; checked against gpu_compute::reference::matmul).
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Simd {
    Avx2Fma,
    Avx2,
    Sse2,
}

impl Simd {
    fn name(self) -> &'static str {
        match self {
            Simd::Avx2Fma => "AVX2+FMA (8 benzi)",
            Simd::Avx2 => "AVX2 (8 benzi)",
            Simd::Sse2 => "SSE2 (4 benzi)",
        }
    }
}

/// What the processor offers and the kernel saves on a thread switch (XCR0 has SSE and AVX state).
fn detect() -> Simd {
    let l1 = __cpuid(1);
    if l1.ecx & (1 << 27) == 0 || l1.ecx & (1 << 28) == 0 {
        return Simd::Sse2; // no OSXSAVE (kernel) or no AVX
    }
    let xcr0: u32;
    // SAFETY: XGETBV is allowed in ring 3 once CR4.OSXSAVE is set (checked above).
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") xcr0, out("edx") _, options(nomem, nostack)) };
    let avx2 = xcr0 & 0b110 == 0b110 && __cpuid(0).eax >= 7 && __cpuid_count(7, 0).ebx & (1 << 5) != 0;
    match (avx2, l1.ecx & (1 << 12) != 0) {
        (true, true) => Simd::Avx2Fma,
        (true, false) => Simd::Avx2,
        _ => Simd::Sse2,
    }
}

/// One step of a vector matmul: `orow[0..n] += a * brow[0..n]` (n a multiple of 8, or of 4 for SSE2). Written in
/// assembly because this target (`x86_64-unknown-none`, soft-float) refuses `#[target_feature(enable = "sse")]`.
/// With FMA each lane rounds once (a * b + o); otherwise it multiplies, rounds, adds, rounds — exactly what
/// `reference::matmul` does for that element, so the result is bit-identical.
fn axpy(simd: Simd, a: &f32, brow: &[f32], orow: &mut [f32]) {
    let n = orow.len();
    assert!(brow.len() == n && n % 8 == 0 && n > 0);
    let (ap, bp, op) = (a as *const f32, brow.as_ptr(), orow.as_mut_ptr());
    // SAFETY: `simd` comes from `detect` (the processor has the instructions and the kernel saves their registers);
    // the loops read a, brow[0..n] and read/write orow[0..n] only (rcx < n, n a multiple of the lane count).
    unsafe {
        match simd {
            Simd::Avx2Fma => core::arch::asm!(
                "vbroadcastss ymm0, [{a}]",
                "xor ecx, ecx",
                "2:",
                "vmovups ymm1, [{b} + rcx*4]",
                "vmovups ymm2, [{o} + rcx*4]",
                "vfmadd231ps ymm2, ymm0, ymm1",
                "vmovups [{o} + rcx*4], ymm2",
                "add rcx, 8",
                "cmp rcx, {n}",
                "jb 2b",
                "vzeroupper",
                a = in(reg) ap, b = in(reg) bp, o = in(reg) op, n = in(reg) n,
                out("rcx") _, out("ymm0") _, out("ymm1") _, out("ymm2") _,
                options(nostack)
            ),
            Simd::Avx2 => core::arch::asm!(
                "vbroadcastss ymm0, [{a}]",
                "xor ecx, ecx",
                "2:",
                "vmulps ymm1, ymm0, [{b} + rcx*4]",
                "vaddps ymm1, ymm1, [{o} + rcx*4]",
                "vmovups [{o} + rcx*4], ymm1",
                "add rcx, 8",
                "cmp rcx, {n}",
                "jb 2b",
                "vzeroupper",
                a = in(reg) ap, b = in(reg) bp, o = in(reg) op, n = in(reg) n,
                out("rcx") _, out("ymm0") _, out("ymm1") _,
                options(nostack)
            ),
            Simd::Sse2 => core::arch::asm!(
                "movss xmm0, [{a}]",
                "shufps xmm0, xmm0, 0",
                "xor ecx, ecx",
                "2:",
                "movups xmm1, [{b} + rcx*4]",
                "mulps xmm1, xmm0",
                "movups xmm2, [{o} + rcx*4]",
                "addps xmm1, xmm2",
                "movups [{o} + rcx*4], xmm1",
                "add rcx, 4",
                "cmp rcx, {n}",
                "jb 2b",
                a = in(reg) ap, b = in(reg) bp, o = in(reg) op, n = in(reg) n,
                out("rcx") _, out("xmm0") _, out("xmm1") _, out("xmm2") _,
                options(nostack)
            ),
        }
    }
}

/// out[rows, n] = a[rows, k] * b[k, n] with vectors; each output element adds its k products in order p = 0..k,
/// like the reference.
fn matmul_simd(simd: Simd, a: &[f32], b: &[f32], out: &mut [f32], k: usize, n: usize) {
    let rows = out.len() / n;
    for i in 0..rows {
        let orow = &mut out[i * n..(i + 1) * n];
        orow.fill(0.0);
        for p in 0..k {
            axpy(simd, &a[i * k + p], &b[p * n..(p + 1) * n], orow);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Data and comparisons.
// ---------------------------------------------------------------------------------------------------------------

/// Deterministic numbers in [-scale, scale) (64-bit LCG, top 24 bits).
fn fill(v: &mut [f32], seed: u64, scale: f32) {
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    for x in v.iter_mut() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let u = (s >> 40) as u32 as f32 / (1u32 << 24) as f32; // [0, 1)
        *x = (u * 2.0 - 1.0) * scale;
    }
}

fn bf16(v: &[f32]) -> Vec<u16> {
    v.iter().map(|&x| f32_to_bf16(x)).collect()
}

fn decode(w: &[u16]) -> Vec<f32> {
    let mut out = vec![0.0f32; w.len()];
    check("bf16", kernels::bf16_to_f32_slice(w, &mut out));
    out
}

fn bits_equal(a: &[f32], b: &[f32]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x.to_bits() != y.to_bits()).count()
}

/// (max |a - b|, elements outside |a - b| <= tol * (1 + |b|)).
fn within(a: &[f32], b: &[f32], tol: f32) -> (f32, usize) {
    let mut worst = 0.0f32;
    let mut bad = 0;
    for (&x, &y) in a.iter().zip(b) {
        let d = if x > y { x - y } else { y - x };
        let m = if y < 0.0 { -y } else { y };
        if d > worst {
            worst = d;
        }
        if !(d <= tol * (1.0 + m)) {
            bad += 1;
        }
    }
    (worst, bad)
}

/// A speed-up with one decimal (integer math: no float formatting needed).
fn ratio(slow: u64, fast: u64) -> (u64, u64) {
    let r = slow * 10 / fast.max(1);
    (r / 10, r % 10)
}

// ---------------------------------------------------------------------------------------------------------------
// 1. A Kronos-shaped transformer block.
// ---------------------------------------------------------------------------------------------------------------

const T: usize = 32; // tokens
const C: usize = 64; // channels
const H: usize = 4; // heads
const D: usize = C / H;
const F: usize = 4 * C; // MLP width
const V: usize = 128; // vocabulary
const EPS: f32 = 1e-5;

struct Weights {
    emb: Vec<f32>,
    g1: Vec<f32>,
    b1: Vec<f32>,
    wqkv: Vec<u16>,
    wo: Vec<u16>,
    g2: Vec<f32>,
    b2: Vec<f32>,
    w1: Vec<u16>,
    bias1: Vec<f32>,
    w2: Vec<u16>,
    wout: Vec<u16>,
    ids: Vec<u32>,
    targets: Vec<u32>,
}

fn weights() -> Weights {
    let v = |n: usize, seed: u64, scale: f32| {
        let mut x = vec![0.0f32; n];
        fill(&mut x, seed, scale);
        x
    };
    let mut g1 = v(C, 2, 0.1);
    let mut g2 = v(C, 6, 0.1);
    g1.iter_mut().chain(g2.iter_mut()).for_each(|g| *g += 1.0);
    Weights {
        emb: v(V * C, 1, 1.0),
        b1: v(C, 3, 0.1),
        wqkv: bf16(&v(C * 3 * C, 4, 0.125)),
        wo: bf16(&v(C * C, 5, 0.125)),
        b2: v(C, 7, 0.1),
        w1: bf16(&v(C * F, 8, 0.125)),
        bias1: v(F, 9, 0.1),
        w2: bf16(&v(F * C, 10, 0.06)),
        wout: bf16(&v(C * V, 11, 0.125)),
        g1,
        g2,
        ids: (0..T as u32).map(|t| (t * 37 + 11) % V as u32).collect(),
        targets: (0..T as u32).map(|t| (t * 53 + 7) % V as u32).collect(),
    }
}

/// Splits fused QKV rows [T, 3C] into q, k, v [T, C] each.
fn split_qkv(qkv: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let (mut q, mut k, mut v) = (vec![0.0; T * C], vec![0.0; T * C], vec![0.0; T * C]);
    for t in 0..T {
        q[t * C..(t + 1) * C].copy_from_slice(&qkv[t * 3 * C..t * 3 * C + C]);
        k[t * C..(t + 1) * C].copy_from_slice(&qkv[t * 3 * C + C..t * 3 * C + 2 * C]);
        v[t * C..(t + 1) * C].copy_from_slice(&qkv[t * 3 * C + 2 * C..(t + 1) * 3 * C]);
    }
    (q, k, v)
}

/// The oracle: every operator from `gpu_compute::reference`, one thread, weights decoded from bf16 first.
fn block_reference(w: &Weights) -> (Vec<f32>, f32) {
    let mut x = vec![0.0; T * C];
    check("embedding", reference::embedding(&w.ids, &w.emb, V, C, &mut x));
    let mut h = vec![0.0; T * C];
    check("layernorm 1", reference::layernorm(&x, &w.g1, &w.b1, &mut h, T, C, EPS));
    let mut qkv = vec![0.0; T * 3 * C];
    check("qkv", reference::matmul(&h, &decode(&w.wqkv), &mut qkv, T, C, 3 * C));
    let (q, k, v) = split_qkv(&qkv);
    let mut att = vec![0.0; T * C];
    check("atentie", reference::causal_attention(&q, &k, &v, &mut att, T, H, D));
    let mut o = vec![0.0; T * C];
    check("proiectie", reference::matmul(&att, &decode(&w.wo), &mut o, T, C, C));
    let mut x2 = vec![0.0; T * C];
    check("rezidual 1", reference::add(&x, &o, &mut x2));
    check("layernorm 2", reference::layernorm(&x2, &w.g2, &w.b2, &mut h, T, C, EPS));
    let mut f = vec![0.0; T * F];
    check("mlp 1", reference::matmul(&h, &decode(&w.w1), &mut f, T, C, F));
    check("bias", reference::bias_add(&mut f, &w.bias1, T, F));
    reference::gelu_tanh(&mut f);
    let mut f2 = vec![0.0; T * C];
    check("mlp 2", reference::matmul(&f, &decode(&w.w2), &mut f2, T, F, C));
    let mut x3 = vec![0.0; T * C];
    check("rezidual 2", reference::add(&x2, &f2, &mut x3));
    let mut logits = vec![0.0; T * V];
    check("logits", reference::matmul(&x3, &decode(&w.wout), &mut logits, T, C, V));
    let loss = reference::cross_entropy(&logits, &w.targets, V).unwrap_or(f32::NAN);
    (logits, loss)
}

/// The fast path: `kernels::matmul_bf16w` (bf16 weights decoded panel by panel) and the row-wise operators, every
/// one split by rows over `n` threads. Attention runs on one thread (its heads are interleaved in the rows).
fn block_parallel(w: &Weights, n: usize) -> (Vec<f32>, f32) {
    let mut x = vec![0.0; T * C];
    check("embedding", reference::embedding(&w.ids, &w.emb, V, C, &mut x));

    let layernorm = |src: &[f32], g: &[f32], b: &[f32], dst: &mut [f32]| {
        let (sp, dp) = (src.as_ptr() as usize, dst.as_mut_ptr() as usize);
        par_rows(T, n, &|a, e| {
            // SAFETY: `src` and `dst` live for the whole call; each thread gets its own rows a..e.
            let (s, d) = unsafe { (rows_mut(sp, C, a, e), rows_mut(dp, C, a, e)) };
            check("layernorm", reference::layernorm(s, g, b, d, e - a, C, EPS));
        });
    };
    let matmul = |src: &[f32], wt: &[u16], dst: &mut [f32], k: usize, m: usize| {
        let (sp, dp) = (src.as_ptr() as usize, dst.as_mut_ptr() as usize);
        par_rows(T, n, &|a, e| {
            // SAFETY: as above.
            let (s, d) = unsafe { (rows_mut(sp, k, a, e), rows_mut(dp, m, a, e)) };
            check("matmul bf16", kernels::matmul_bf16w(s, wt, d, e - a, k, m));
        });
    };
    let add = |p: &[f32], q: &[f32], dst: &mut [f32]| {
        let (pp, qp, dp) = (p.as_ptr() as usize, q.as_ptr() as usize, dst.as_mut_ptr() as usize);
        par_rows(T, n, &|a, e| {
            // SAFETY: as above.
            let (ps, qs, d) = unsafe { (rows_mut(pp, C, a, e), rows_mut(qp, C, a, e), rows_mut(dp, C, a, e)) };
            check("add", reference::add(ps, qs, d));
        });
    };

    let mut h = vec![0.0; T * C];
    layernorm(&x, &w.g1, &w.b1, &mut h);
    let mut qkv = vec![0.0; T * 3 * C];
    matmul(&h, &w.wqkv, &mut qkv, C, 3 * C);
    let (q, k, v) = split_qkv(&qkv);
    let mut att = vec![0.0; T * C];
    check("atentie", reference::causal_attention(&q, &k, &v, &mut att, T, H, D));
    let mut o = vec![0.0; T * C];
    matmul(&att, &w.wo, &mut o, C, C);
    let mut x2 = vec![0.0; T * C];
    add(&x, &o, &mut x2);
    layernorm(&x2, &w.g2, &w.b2, &mut h);
    let mut f = vec![0.0; T * F];
    matmul(&h, &w.w1, &mut f, C, F);
    let fp = f.as_mut_ptr() as usize;
    par_rows(T, n, &|a, e| {
        // SAFETY: `f` lives for the whole call; each thread gets its own rows.
        let r = unsafe { rows_mut(fp, F, a, e) };
        check("bias", reference::bias_add(r, &w.bias1, e - a, F));
        reference::gelu_tanh(r);
    });
    let mut f2 = vec![0.0; T * C];
    matmul(&f, &w.w2, &mut f2, F, C);
    let mut x3 = vec![0.0; T * C];
    add(&x2, &f2, &mut x3);
    let mut logits = vec![0.0; T * V];
    matmul(&x3, &w.wout, &mut logits, C, V);
    let loss = reference::cross_entropy(&logits, &w.targets, V).unwrap_or(f32::NAN);
    (logits, loss)
}

// ---------------------------------------------------------------------------------------------------------------

const M: usize = 128;

fn main() -> i64 {
    let n = cyber_user::args()
        .nth(1)
        .and_then(|a| a.parse::<usize>().ok())
        .unwrap_or(cyber_user::cpus() as usize)
        .clamp(1, MAX_THREADS);
    let simd = detect();
    println!("kronos-calcul: gpu-compute (cyber-gpu 41e44ba, backend CPU), {} {}, vectori {}", n, fire(n), simd.name());
    let mut ok = true;

    // 1. The block.
    let w = weights();
    let t0 = now_ms();
    let (ref_logits, ref_loss) = block_reference(&w);
    let t1 = now_ms();
    let (par_logits, par_loss) = block_parallel(&w, n);
    let t2 = now_ms();
    let diff = bits_equal(&par_logits, &ref_logits);
    let loss_same = par_loss.to_bits() == ref_loss.to_bits() && ref_loss == ref_loss;
    let loss_milli = (ref_loss * 1000.0) as i64;
    println!(
        "kronos-calcul: bloc Kronos T={} C={} H={} F={} V={} (bf16): pierdere {}.{:03}, toleranta 0: {} logit-uri diferite din {}, pierderea {}",
        T, C, H, F, V, loss_milli / 1000, loss_milli % 1000, diff, T * V,
        if loss_same { "identica" } else { "DIFERITA" }
    );
    println!("kronos-calcul: bloc: referinta 1 fir {} ms, kernels pe {} {} {} ms", t1 - t0, n, fire(n), t2 - t1);
    ok &= diff == 0 && loss_same;

    // 2. Matmul timings.
    let (mut a, mut b) = (vec![0.0f32; M * M], vec![0.0f32; M * M]);
    fill(&mut a, 21, 1.0);
    fill(&mut b, 22, 1.0);
    let mut oracle = vec![0.0f32; M * M];
    let t0 = now_ms();
    check("matmul referinta", reference::matmul(&a, &b, &mut oracle, M, M, M));
    let t_ref = now_ms() - t0;

    let run = |threads: usize, vector: bool| -> (Vec<f32>, u64) {
        let mut out = vec![0.0f32; M * M];
        let (ap, op) = (a.as_ptr() as usize, out.as_mut_ptr() as usize);
        let bs = &b[..];
        let t0 = now_ms();
        par_rows(M, threads, &|r0, r1| {
            // SAFETY: `a` and `out` live for the whole call; each thread gets its own rows.
            let (ar, or) = unsafe { (rows_mut(ap, M, r0, r1), rows_mut(op, M, r0, r1)) };
            if vector {
                matmul_simd(simd, ar, bs, or, M, M);
            } else {
                check("matmul blocked", kernels::matmul_blocked(ar, bs, or, r1 - r0, M, M));
            }
        });
        (out, now_ms() - t0)
    };
    let (s1, t_s1) = run(1, false);
    let (sn, t_sn) = run(n, false);
    let (v1, t_v1) = run(1, true);
    let (vn, t_vn) = run(n, true);
    let scalar_bad = bits_equal(&s1, &oracle) + bits_equal(&sn, &oracle);
    let tol: f32 = if simd == Simd::Avx2Fma { 1e-5 } else { 0.0 };
    let (w1, b1) = within(&v1, &oracle, tol);
    let (wn, bn) = within(&vn, &oracle, tol);
    let worst_e9 = ((if w1 > wn { w1 } else { wn }) * 1e9) as u64;
    println!(
        "kronos-calcul: matmul {}x{}x{}: referinta 1 fir {} ms; kernels 1 fir {} ms, {} {} {} ms (toleranta 0: {} diferente)",
        M, M, M, t_ref, t_s1, n, fire(n), t_sn, scalar_bad
    );
    println!(
        "kronos-calcul: matmul vectori {}: 1 fir {} ms, {} {} {} ms; toleranta |dif| <= {}*(1+|ref|): {} in afara, eroarea maxima {}e-9",
        simd.name(), t_v1, n, fire(n), t_vn, if tol > 0.0 { "1e-5" } else { "0" }, b1 + bn, worst_e9
    );
    let (sx, sy) = ratio(t_s1, t_sn);
    let (vx, vy) = ratio(t_v1, t_vn);
    let (rx, ry) = ratio(t_ref, t_vn);
    println!(
        "kronos-calcul: accelerare pe {} {}: kernels {}.{}x, vectori {}.{}x; vectori pe {} {} fata de referinta {}.{}x",
        n, fire(n), sx, sy, vx, vy, n, fire(n), rx, ry
    );
    ok &= scalar_bad == 0 && b1 + bn == 0;

    if ok {
        println!("kronos-calcul: toate rezultatele in toleranta declarata (OK)");
        0
    } else {
        println!("kronos-calcul: REZULTATE IN AFARA TOLERANTEI");
        1
    }
}
