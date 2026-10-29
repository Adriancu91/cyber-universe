//! P6-27 (command C-25): the KGR1 engine — reads a Kronos model file (F32 or Q8_0) and runs its forward pass on every
//! processor. Written by us from the specification in the hub (`integrare/kronos/format_greutati.md`, Kronos commit
//! 190bd38), without the Kronos code. The elementary functions (exp, sqrt, erf) and GELU come from Cyber-GPU's
//! `gpu-compute` (pinned in Cargo.toml); the matrix-vector products are ours: scalar or AVX2 + FMA. Since P6-29 the
//! programs are built for our own target (`user/x86_64-cyber-user.json`, SSE2 in hardware), so the scalar code runs on
//! the processor's floating point unit instead of being emulated in software (same IEEE results, many times faster).
//!
//! Layout of the computation (spec, "Tensorii și calculul"): x = tok[id] + pos[p]; per layer: pre-LayerNorm, fused
//! QKV, causal attention (head j = columns j*D .. j*D+D-1), output projection, residual; pre-LayerNorm, MLP 4E with
//! exact GELU, residual; final LayerNorm; logits against the (tied) token embedding. Keys and values of past
//! positions are kept (a cache), so generating one more token costs one row, not the whole text again.

use alloc::{string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use gpu_compute::{math, reference};

/// Most threads one forward pass uses (the workstation has 36; the kernel allows 64 per program).
pub const MAX_THREADS: usize = 48;

// ---------------------------------------------------------------------------------------------------------------
// The file.
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    F32,
    Q8,
}

pub struct TensorInfo<'a> {
    pub name: &'a str,
    pub dims: Vec<usize>,
    pub kind: Kind,
    bytes: &'a [u8],
}

pub enum Value<'a> {
    Int(i64),
    Float(f64),
    Text(&'a str),
}

/// A parsed KGR1 file (borrowing its bytes).
pub struct File<'a> {
    pub keys: Vec<(&'a str, Value<'a>)>,
    pub tensors: Vec<TensorInfo<'a>>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], &'static str> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("fisier trunchiat")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, &'static str> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<&'a str, &'static str> {
        let n = self.u64()?;
        let n = usize::try_from(n).map_err(|_| "text prea lung")?;
        core::str::from_utf8(self.take(n)?).map_err(|_| "text care nu e UTF-8")
    }
}

const fn align32(x: usize) -> usize {
    (x + 31) & !31
}

impl<'a> File<'a> {
    /// Parses the header, the keys and the tensor list; checks that every tensor lies inside the file.
    pub fn parse(b: &'a [u8]) -> Result<File<'a>, &'static str> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != b"KGR1" {
            return Err("nu e un fisier KGR1 (lipseste \"KGR1\" la inceput)");
        }
        if r.u32()? != 1 {
            return Err("versiune KGR1 necunoscuta (stiu doar 1)");
        }
        let (nkeys, ntens) = (r.u64()?, r.u64()?);
        if nkeys > 10_000 || ntens > 100_000 {
            return Err("antet nerezonabil (prea multe chei sau tensori)");
        }
        let mut keys = Vec::new();
        for _ in 0..nkeys {
            let k = r.text()?;
            let v = match r.u32()? {
                1 => Value::Int(r.u64()? as i64),
                2 => Value::Float(f64::from_bits(r.u64()?)),
                3 => Value::Text(r.text()?),
                _ => return Err("felul unei chei necunoscut"),
            };
            keys.push((k, v));
        }
        let mut list = Vec::new();
        for _ in 0..ntens {
            let name = r.text()?;
            let nd = r.u32()? as usize;
            if nd == 0 || nd > 4 {
                return Err("tensor cu numar de dimensiuni nerezonabil");
            }
            let mut dims = Vec::new();
            for _ in 0..nd {
                dims.push(usize::try_from(r.u64()?).map_err(|_| "dimensiune prea mare")?);
            }
            let kind = match r.u32()? {
                0 => Kind::F32,
                1 => Kind::Q8,
                _ => return Err("felul unui tensor necunoscut (stiu F32 si Q8_0)"),
            };
            let off = usize::try_from(r.u64()?).map_err(|_| "loc prea mare")?;
            list.push((name, dims, kind, off));
        }
        let data = align32(r.at);
        let mut tensors = Vec::new();
        for (name, dims, kind, off) in list {
            let numel = dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).ok_or("tensor prea mare")?;
            let len = match kind {
                Kind::F32 => numel.checked_mul(4).ok_or("tensor prea mare")?,
                Kind::Q8 => {
                    if dims.len() != 2 || dims[1] % 32 != 0 {
                        return Err("tensor Q8_0 care nu are 2 dimensiuni cu coloanele multiplu de 32");
                    }
                    dims[0] * (dims[1] / 32) * 36
                }
            };
            if off % 32 != 0 {
                return Err("tensor nealiniat la 32");
            }
            let start = data.checked_add(off).ok_or("loc prea mare")?;
            let end = start.checked_add(len).filter(|&e| e <= b.len()).ok_or("tensor in afara fisierului")?;
            tensors.push(TensorInfo { name, dims, kind, bytes: &b[start..end] });
        }
        Ok(File { keys, tensors })
    }

    /// Only the keys at the start of a file (P6-29: `kronos-calcul --cauta` reads the model's sizes from the first
    /// megabytes of the file, without its tensors). `tensors` stays empty.
    pub fn parse_keys(b: &'a [u8]) -> Result<File<'a>, &'static str> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != b"KGR1" {
            return Err("nu e un fisier KGR1 (lipseste \"KGR1\" la inceput)");
        }
        if r.u32()? != 1 {
            return Err("versiune KGR1 necunoscuta (stiu doar 1)");
        }
        let (nkeys, _) = (r.u64()?, r.u64()?);
        if nkeys > 10_000 {
            return Err("antet nerezonabil (prea multe chei)");
        }
        let mut keys = Vec::new();
        for _ in 0..nkeys {
            let k = r.text()?;
            let v = match r.u32()? {
                1 => Value::Int(r.u64()? as i64),
                2 => Value::Float(f64::from_bits(r.u64()?)),
                3 => Value::Text(r.text()?),
                _ => return Err("felul unei chei necunoscut"),
            };
            keys.push((k, v));
        }
        Ok(File { keys, tensors: Vec::new() })
    }

    pub fn key(&self, k: &str) -> Option<&Value<'a>> {
        self.keys.iter().find(|(n, _)| *n == k).map(|(_, v)| v)
    }
    pub fn int(&self, k: &str) -> Result<usize, String> {
        match self.key(k) {
            Some(Value::Int(v)) if *v > 0 && *v < (1 << 40) => Ok(*v as usize),
            _ => Err(alloc::format!("cheia `{}` lipseste sau nu e un numar pozitiv", k)),
        }
    }
    pub fn text(&self, k: &str) -> Option<&'a str> {
        match self.key(k) {
            Some(Value::Text(t)) => Some(t),
            _ => None,
        }
    }
    pub fn tensor(&self, name: &str) -> Option<&TensorInfo<'a>> {
        self.tensors.iter().find(|t| t.name == name)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Weights.
// ---------------------------------------------------------------------------------------------------------------

/// f32 values of an F32 tensor's bytes (the file buffer is aligned and every tensor starts at a multiple of 32).
fn floats(b: &[u8]) -> Result<&[f32], &'static str> {
    if b.as_ptr() as usize % 4 != 0 {
        return Err("fisierul nu e aliniat in memorie");
    }
    // SAFETY: aligned (checked), length a multiple of 4 (F32 tensor), every bit pattern is a valid f32, and x86_64 is
    // little-endian like the file.
    Ok(unsafe { core::slice::from_raw_parts(b.as_ptr() as *const f32, b.len() / 4) })
}

/// A weight matrix [rows, cols] (rows = output features), F32 or Q8_0.
pub struct Mat<'a> {
    pub rows: usize,
    pub cols: usize,
    pub kind: Kind,
    f: &'a [f32],
    q: &'a [u8],
}

impl<'a> Mat<'a> {
    fn new(t: &TensorInfo<'a>, rows: usize, cols: usize) -> Result<Mat<'a>, String> {
        if t.dims.len() != 2 || t.dims[0] != rows || t.dims[1] != cols {
            return Err(alloc::format!("`{}` are forma {:?}, astept [{}, {}]", t.name, t.dims, rows, cols));
        }
        Ok(match t.kind {
            Kind::F32 => Mat { rows, cols, kind: Kind::F32, f: floats(t.bytes)?, q: &[] },
            Kind::Q8 => Mat { rows, cols, kind: Kind::Q8, f: &[], q: t.bytes },
        })
    }

    /// A matrix over f32 values (row-major).
    pub fn from_f32(f: &'a [f32], rows: usize, cols: usize) -> Mat<'a> {
        assert!(f.len() == rows * cols);
        Mat { rows, cols, kind: Kind::F32, f, q: &[] }
    }

    /// A matrix over Q8_0 bytes (each row = cols / 32 blocks of 36 bytes).
    pub fn from_q8(q: &'a [u8], rows: usize, cols: usize) -> Mat<'a> {
        assert!(cols % 32 == 0 && q.len() == rows * (cols / 32) * 36);
        Mat { rows, cols, kind: Kind::Q8, f: &[], q }
    }

    /// Bytes of weights one product reads.
    pub fn bytes(&self) -> usize {
        match self.kind {
            Kind::F32 => self.rows * self.cols * 4,
            Kind::Q8 => self.rows * (self.cols / 32) * 36,
        }
    }

    /// Row `r` as f32 values in `out` (Q8_0: scale * number).
    pub fn row(&self, r: usize, out: &mut [f32]) {
        match self.kind {
            Kind::F32 => out.copy_from_slice(&self.f[r * self.cols..(r + 1) * self.cols]),
            Kind::Q8 => {
                let row = &self.q[r * (self.cols / 32) * 36..(r + 1) * (self.cols / 32) * 36];
                for (blk, o) in row.chunks_exact(36).zip(out.chunks_exact_mut(32)) {
                    let s = f32::from_le_bytes(blk[..4].try_into().unwrap());
                    for (v, &q) in o.iter_mut().zip(&blk[4..]) {
                        *v = s * (q as i8) as f32;
                    }
                }
            }
        }
    }

    /// x[..n] · row r[..n] (n a multiple of 32): only for the deliberately broken variant of the search.
    fn dot_prefix(&self, simd: bool, r: usize, x: &[f32], n: usize) -> f32 {
        let n = n & !31;
        match self.kind {
            Kind::F32 => {
                let w = &self.f[r * self.cols..r * self.cols + n];
                if simd { dot_f32_avx2(w, &x[..n]) } else { w.iter().zip(x).fold(0.0, |a, (w, x)| a + w * x) }
            }
            Kind::Q8 => {
                let row = &self.q[r * (self.cols / 32) * 36..r * (self.cols / 32) * 36 + (n / 32) * 36];
                let m = Mat { rows: 1, cols: n, kind: Kind::Q8, f: &[], q: row };
                m.dot(simd, 0, x)
            }
        }
    }

    /// x · row r.
    fn dot(&self, simd: bool, r: usize, x: &[f32]) -> f32 {
        match self.kind {
            Kind::F32 => {
                let w = &self.f[r * self.cols..(r + 1) * self.cols];
                if simd { dot_f32_avx2(w, x) } else { w.iter().zip(x).fold(0.0, |a, (w, x)| a + w * x) }
            }
            Kind::Q8 => {
                let row = &self.q[r * (self.cols / 32) * 36..(r + 1) * (self.cols / 32) * 36];
                if simd {
                    return dot_q8_avx2(row, x);
                }
                let mut acc = 0.0f32;
                for (blk, xs) in row.chunks_exact(36).zip(x.chunks_exact(32)) {
                    let s = f32::from_le_bytes(blk[..4].try_into().unwrap());
                    let part = blk[4..].iter().zip(xs).fold(0.0f32, |a, (&q, x)| a + (q as i8) as f32 * x);
                    acc += s * part;
                }
                acc
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Vector kernels (AVX2 + FMA), written in assembly (as in kronos-calcul; from before P6-29, when the target was
// soft-float and refused `target_feature`). The kernel saves the AVX registers of every thread (K6-01).
// ---------------------------------------------------------------------------------------------------------------

/// True when the processor has AVX2 and FMA and the kernel enabled the AVX state (OSXSAVE, XCR0).
pub fn has_avx2_fma() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    let l1 = __cpuid(1);
    if l1.ecx & (1 << 27) == 0 || l1.ecx & (1 << 28) == 0 || l1.ecx & (1 << 12) == 0 {
        return false; // no OSXSAVE, no AVX or no FMA
    }
    let xcr0: u32;
    // SAFETY: XGETBV is allowed in ring 3 once CR4.OSXSAVE is set (checked above).
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") xcr0, out("edx") _, options(nomem, nostack)) };
    xcr0 & 0b110 == 0b110 && __cpuid(0).eax >= 7 && __cpuid_count(7, 0).ebx & (1 << 5) != 0
}

/// Sum of w[i] * x[i]; 8 lanes with FMA, then the lanes added; the tail (n % 8) in scalar.
fn dot_f32_avx2(w: &[f32], x: &[f32]) -> f32 {
    let n = w.len().min(x.len());
    let body = n & !7;
    let mut out = 0.0f32;
    if body > 0 {
        // SAFETY: only called when `has_avx2_fma()`; reads w[0..body] and x[0..body] (rcx < body, multiple of 8)
        // and writes the one f32 `out`.
        unsafe {
            core::arch::asm!(
                "vxorps ymm0, ymm0, ymm0",
                "xor ecx, ecx",
                "2:",
                "vmovups ymm1, [{w} + rcx*4]",
                "vfmadd231ps ymm0, ymm1, [{x} + rcx*4]",
                "add rcx, 8",
                "cmp rcx, {n}",
                "jb 2b",
                "vextractf128 xmm1, ymm0, 1",
                "vaddps xmm0, xmm0, xmm1",
                "vmovhlps xmm1, xmm1, xmm0",
                "vaddps xmm0, xmm0, xmm1",
                "vmovshdup xmm1, xmm0",
                "vaddss xmm0, xmm0, xmm1",
                "vmovss [{o}], xmm0",
                "vzeroupper",
                w = in(reg) w.as_ptr(), x = in(reg) x.as_ptr(), n = in(reg) body, o = in(reg) &mut out as *mut f32,
                out("rcx") _, out("ymm0") _, out("ymm1") _,
                options(nostack)
            );
        }
    }
    for i in body..n {
        out += w[i] * x[i];
    }
    out
}

/// Sum over the 32-column blocks of scale * (Σ q[i] * x[i]); `row` = the blocks of one Q8_0 row (36 bytes each).
fn dot_q8_avx2(row: &[u8], x: &[f32]) -> f32 {
    let blocks = row.len() / 36;
    assert!(x.len() >= blocks * 32);
    let mut out = 0.0f32;
    if blocks == 0 {
        return out;
    }
    // SAFETY: only called when `has_avx2_fma()`; per block reads 36 bytes of `row` and 32 f32 of `x` (blocks of both
    // checked above), writes the one f32 `out`.
    unsafe {
        core::arch::asm!(
            "vxorps ymm0, ymm0, ymm0",          // total
            "2:",
            "vpmovsxbd ymm2, [{q} + 4]",
            "vcvtdq2ps ymm2, ymm2",
            "vmulps ymm1, ymm2, [{x}]",          // block sum, lanes 0..7
            "vpmovsxbd ymm2, [{q} + 12]",
            "vcvtdq2ps ymm2, ymm2",
            "vfmadd231ps ymm1, ymm2, [{x} + 32]",
            "vpmovsxbd ymm2, [{q} + 20]",
            "vcvtdq2ps ymm2, ymm2",
            "vfmadd231ps ymm1, ymm2, [{x} + 64]",
            "vpmovsxbd ymm2, [{q} + 28]",
            "vcvtdq2ps ymm2, ymm2",
            "vfmadd231ps ymm1, ymm2, [{x} + 96]",
            "vbroadcastss ymm3, [{q}]",          // scale
            "vfmadd231ps ymm0, ymm1, ymm3",
            "add {q}, 36",
            "add {x}, 128",
            "dec {b}",
            "jnz 2b",
            "vextractf128 xmm1, ymm0, 1",
            "vaddps xmm0, xmm0, xmm1",
            "vmovhlps xmm1, xmm1, xmm0",
            "vaddps xmm0, xmm0, xmm1",
            "vmovshdup xmm1, xmm0",
            "vaddss xmm0, xmm0, xmm1",
            "vmovss [{o}], xmm0",
            "vzeroupper",
            q = inout(reg) row.as_ptr() => _, x = inout(reg) x.as_ptr() => _, b = inout(reg) blocks => _,
            o = in(reg) &mut out as *mut f32,
            out("ymm0") _, out("ymm1") _, out("ymm2") _, out("ymm3") _,
            options(nostack)
        );
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Work split over threads.
// ---------------------------------------------------------------------------------------------------------------

// P6-29 (C-29): a pool of threads started once (the first `par` that needs them) and kept for the whole program.
// Before, every product started and joined fresh threads (stacks mapped and unmapped, TLB flushes on every core):
// 49 products per token x 35 threads on the workstation, so more threads were slower. Now `par` publishes the job
// with a generation counter; the workers spin on it (`pause`, a `yield` now and then, never a 10 ms sleep), run their
// part and count themselves done; `par` waits the same way. Each output element is still computed by exactly one
// thread with the same code, so the results are the same bit for bit whatever the number of threads.

struct Job<'a> {
    n: usize,
    parts: usize,
    f: &'a (dyn Fn(usize, usize, usize) + Sync),
}

/// Address of the `Job` being run (valid until every worker counted itself in DONE).
static JOB: AtomicUsize = AtomicUsize::new(0);
/// Bumped by `par` for every job (and once more to stop the pool).
static GEN: AtomicUsize = AtomicUsize::new(0);
/// Workers done with the current generation.
static DONE: AtomicUsize = AtomicUsize::new(0);
/// Workers started (they are parts 1..=WORKERS); only the main thread starts or stops them.
static WORKERS: AtomicUsize = AtomicUsize::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
/// Thread id and stack of every worker (index = part - 1).
static HANDLES: [(AtomicI64, AtomicUsize); MAX_THREADS] = [const { (AtomicI64::new(0), AtomicUsize::new(0)) }; MAX_THREADS];

/// Spins of a waiting thread between two `yield`s (gives the processor away when threads outnumber processors, as in
/// QEMU; costs ~1 us on a free core).
const SPINS: u32 = 4000;

fn split(n: usize, parts: usize, part: usize) -> (usize, usize) {
    (n * part / parts, n * (part + 1) / parts)
}

fn spin_until(done: impl Fn() -> bool) {
    let mut spins = 0;
    while !done() {
        core::hint::spin_loop();
        spins += 1;
        if spins == SPINS {
            spins = 0;
            cyber_user::yield_now();
        }
    }
}

/// `arg` = part (low 8 bits) | the generation current when the worker was started (so it cannot miss the first job).
extern "C" fn worker(arg: u64) -> i64 {
    let part = (arg & 0xff) as usize;
    let mut seen = (arg >> 8) as usize;
    loop {
        spin_until(|| GEN.load(Ordering::Acquire) != seen);
        seen = GEN.load(Ordering::Acquire);
        if QUIT.load(Ordering::Acquire) {
            DONE.fetch_add(1, Ordering::AcqRel);
            return 0;
        }
        // SAFETY: JOB holds the address of the Job on `par`'s stack, published before GEN was bumped; `par` does not
        // return (so the Job lives) until every worker, this one included, has counted itself in DONE below.
        let job = unsafe { &*(JOB.load(Ordering::Acquire) as *const Job<'static>) };
        if part < job.parts {
            let (a, b) = split(job.n, job.parts, part);
            (job.f)(part, a, b);
        }
        DONE.fetch_add(1, Ordering::AcqRel);
    }
}

/// Starts workers until there are `want` (fewer if the kernel refuses); returns how many there are.
fn grow_pool(want: usize) -> usize {
    let mut have = WORKERS.load(Ordering::Relaxed);
    while have < want.min(MAX_THREADS - 1) {
        let part = have + 1;
        let (id, stack) = cyber_user::thread::spawn_with_stack(worker, part as u64 | (GEN.load(Ordering::Acquire) as u64) << 8);
        if id < 0 {
            break;
        }
        HANDLES[have].0.store(id, Ordering::Relaxed);
        HANDLES[have].1.store(stack as usize, Ordering::Relaxed);
        have += 1;
        WORKERS.store(have, Ordering::Release);
    }
    have
}

/// Stops the pool's threads (the next `par` that needs threads starts them again).
pub fn stop_pool() {
    let w = WORKERS.load(Ordering::Relaxed);
    if w == 0 {
        return;
    }
    DONE.store(0, Ordering::Relaxed);
    QUIT.store(true, Ordering::Release);
    GEN.fetch_add(1, Ordering::AcqRel);
    for h in HANDLES.iter().take(w) {
        cyber_user::thread::join(h.0.load(Ordering::Relaxed));
    }
    // The kernel unmaps only while the program has a single thread (K6-04), so after every join.
    for h in HANDLES.iter().take(w) {
        cyber_user::mem_unmap(h.1.load(Ordering::Relaxed) as *mut u8, cyber_user::thread::STACK);
    }
    WORKERS.store(0, Ordering::Release);
    QUIT.store(false, Ordering::Release);
}

/// Calls `f(start, end)` on `parts` disjoint ranges of `0..n`, one per thread (part 0 on this thread).
pub fn par(n: usize, parts: usize, f: &(dyn Fn(usize, usize) + Sync)) {
    par_parts(n, parts, &|_, a, b| f(a, b));
}

/// As `par`, but `f` also gets the part number (0..parts), e.g. to pick its own scratch buffer.
pub fn par_parts(n: usize, parts: usize, f: &(dyn Fn(usize, usize, usize) + Sync)) {
    let parts = parts.clamp(1, MAX_THREADS).min(n.max(1));
    if parts == 1 {
        f(0, 0, n);
        return;
    }
    let workers = grow_pool(parts - 1);
    let job = Job { n, parts, f };
    JOB.store(&job as *const Job as usize, Ordering::Release);
    DONE.store(0, Ordering::Release);
    GEN.fetch_add(1, Ordering::AcqRel);
    // Parts without a worker (the kernel refused a thread) run here.
    for p in workers + 1..parts {
        let (a, b) = split(n, parts, p);
        f(p, a, b);
    }
    let (a, b) = split(n, parts, 0);
    f(0, a, b);
    let t0 = prof::now();
    spin_until(|| DONE.load(Ordering::Acquire) == workers);
    prof::add(prof::SYNC, prof::now() - t0, 0);
}

// ---------------------------------------------------------------------------------------------------------------
// Profile (P6-29, `kronos --profil`): processor cycles (TSC) per kind of work, summed over the run.
// ---------------------------------------------------------------------------------------------------------------

pub mod prof {
    use core::sync::atomic::{AtomicU64, Ordering};

    pub const QKV: usize = 0;
    pub const PROJ: usize = 1;
    pub const FC: usize = 2;
    pub const OUT: usize = 3;
    pub const HEAD: usize = 4;
    pub const ATTN: usize = 5;
    pub const REST: usize = 6;
    /// Main thread waiting for the other threads at the end of a product (inside the times above).
    pub const SYNC: usize = 7;
    pub const N: usize = 8;
    pub const NAMES: [&str; N] = [
        "produs qkv (+ atentia, la generare)",
        "produs proiectie (straturi)",
        "produs fc 4E (straturi)",
        "produs iesire MLP (straturi)",
        "capul de iesire (vocabular)",
        "atentia",
        "restul (LayerNorm, GELU, adunari)",
        "din care asteptarea firelor",
    ];

    static CYCLES: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
    static BYTES: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

    pub fn now() -> u64 {
        let (lo, hi): (u32, u32);
        // SAFETY: RDTSC only reads the time-stamp counter (allowed in ring 3: the kernel leaves CR4.TSD clear).
        unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) };
        (hi as u64) << 32 | lo as u64
    }

    /// Adds `cycles` (and `bytes` of weights read) to `kind`.
    pub fn add(kind: usize, cycles: u64, bytes: u64) {
        CYCLES[kind].fetch_add(cycles, Ordering::Relaxed);
        BYTES[kind].fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn reset() {
        for i in 0..N {
            CYCLES[i].store(0, Ordering::Relaxed);
            BYTES[i].store(0, Ordering::Relaxed);
        }
    }

    /// TSC cycles per millisecond, measured over ~200 ms of the kernel clock (from one clock edge to another).
    pub fn calibrate() -> u64 {
        let ms = || cyber_user::mem_info().map(|m| m.uptime_ms).unwrap_or(0);
        let edge = |after: u64| loop {
            let t = ms();
            if t > after {
                return t;
            }
            core::hint::spin_loop();
        };
        let a = edge(ms());
        let c0 = now();
        let b = edge(a + 199);
        let c1 = now();
        ((c1 - c0) / (b - a).max(1)).max(1)
    }

    /// (cycles, bytes) of `kind`.
    pub fn get(kind: usize) -> (u64, u64) {
        (CYCLES[kind].load(Ordering::Relaxed), BYTES[kind].load(Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tuning (P6-28, command C-27): how `linear` runs on this computer, chosen by `kronos-calcul --cauta`, which checks
// every variant against a float64 control and keeps the fastest correct one in /usb/kronos/reglaj-<processor>.txt.
// ---------------------------------------------------------------------------------------------------------------

/// Loop order of `linear` for several input rows: `TJ` = for each row, every output feature (the default); `JT` = for
/// each output feature, every row (the weight row is read once for all rows). Both give the same values bit for bit.
/// `Broken` drops the last 8 columns of every dot product: a deliberately wrong variant the search must reject.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Order {
    TJ,
    JT,
    Broken,
}

impl Order {
    pub fn name(self) -> &'static str {
        match self {
            Order::TJ => "t-j",
            Order::JT => "j-t",
            Order::Broken => "stricata",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tuning {
    /// Threads per product (1 ..= MAX_THREADS).
    pub threads: usize,
    /// AVX2 + FMA (only when `has_avx2_fma()`), otherwise scalar.
    pub simd: bool,
    pub order: Order,
}

impl Tuning {
    /// What the engine uses without a tuning file: every processor, vectors when present, order t-j.
    pub fn default_for(cpus: usize) -> Tuning {
        Tuning { threads: cpus.clamp(1, MAX_THREADS), simd: has_avx2_fma(), order: Order::TJ }
    }

    pub fn describe(&self) -> String {
        alloc::format!(
            "{}, {} {}, ordine {}",
            if self.simd { "vectori AVX2+FMA" } else { "fara vectori (scalar)" },
            self.threads,
            if self.threads == 1 { "fir" } else { "fire" },
            self.order.name()
        )
    }

    /// The tuning file: `nume = valoare` lines, `#` comments.
    pub fn to_text(&self, processor: &str, note: &str) -> String {
        alloc::format!(
            "# Reglajul motorului KGR1 pentru acest procesor (kronos-calcul --cauta, P6-28).\n\
             # Il citeste `kronos` la pornire. Se poate sterge: atunci se folosesc valorile implicite.\n\
             procesor = {}\nvectori = {}\nfire = {}\nordine = {}\n{}",
            processor,
            if self.simd { "avx2" } else { "scalar" },
            self.threads,
            self.order.name(),
            note
        )
    }

    /// Reads a tuning file over `base`. Values this computer cannot use are refused with a reason (vectors without
    /// AVX2 + FMA, another processor's file, a broken order); the other keys still apply.
    pub fn parse(text: &str, base: Tuning, processor: &str) -> (Tuning, Vec<String>) {
        let mut t = base;
        let mut notes = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "procesor" if v != processor => notes.push(alloc::format!("fisierul e pentru `{}`, nu pentru `{}`", v, processor)),
                "vectori" => match v {
                    "avx2" if has_avx2_fma() => t.simd = true,
                    "avx2" => notes.push("vectori = avx2, dar procesorul n-are AVX2+FMA: scalar".into()),
                    "scalar" => t.simd = false,
                    _ => notes.push(alloc::format!("vectori = `{}` necunoscut", v)),
                },
                "fire" => match v.parse::<usize>() {
                    Ok(n) if n >= 1 => t.threads = n.min(MAX_THREADS),
                    _ => notes.push(alloc::format!("fire = `{}` nu e un numar", v)),
                },
                "ordine" => match v {
                    "t-j" => t.order = Order::TJ,
                    "j-t" => t.order = Order::JT,
                    _ => notes.push(alloc::format!("ordine = `{}` refuzata", v)),
                },
                _ => {}
            }
        }
        (t, notes)
    }
}

/// The processor's name from CPUID (brand string), with only letters, digits and '-' (for the tuning file name).
pub fn processor_name() -> String {
    use core::arch::x86_64::__cpuid;
    let mut raw = Vec::new();
    if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            let r = __cpuid(leaf);
            for w in [r.eax, r.ebx, r.ecx, r.edx] {
                raw.extend_from_slice(&w.to_le_bytes());
            }
        }
    }
    let mut out = String::new();
    for c in raw.iter().map(|&b| b as char) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "necunoscut".into() } else { out }
}

/// One output of `linear`: x_row · W[j] (+ b[j]), as `tune` says (the broken order drops the last 8 columns).
#[inline]
fn output(w: &Mat, b: Option<&[f32]>, xr: &[f32], j: usize, tune: Tuning) -> f32 {
    let mut v = if tune.order == Order::Broken { w.dot_prefix(tune.simd, j, xr, w.cols - 8) } else { w.dot(tune.simd, j, xr) };
    if let Some(b) = b {
        v += b[j];
    }
    v
}

/// GELU (exact erf form) of one value: the same arithmetic as `reference::gelu_erf` on a slice.
#[inline]
fn gelu1(v: f32) -> f32 {
    let mut a = [v];
    reference::gelu_erf(&mut a);
    a[0]
}

/// y[t, :] = x[t, :] · Wᵀ (+ b), for t < rows; split over the output features as `tune` says.
pub fn linear(w: &Mat, b: Option<&[f32]>, x: &[f32], rows: usize, y: &mut [f32], tune: Tuning) {
    linear_act(w, b, x, rows, y, tune, false);
}

/// `linear`, then (when `gelu`) GELU on every output, by the thread that computed it (P6-36: no serial pass after).
pub fn linear_act(w: &Mat, b: Option<&[f32]>, x: &[f32], rows: usize, y: &mut [f32], tune: Tuning, gelu: bool) {
    let (inp, outp) = (w.cols, w.rows);
    assert!(x.len() >= rows * inp && y.len() >= rows * outp);
    let o = Out(y.as_mut_ptr());
    let one = |t: usize, j: usize| {
        let v = output(w, b, &x[t * inp..(t + 1) * inp], j, tune);
        if gelu { gelu1(v) } else { v }
    };
    par(outp, tune.threads, &|j0, j1| {
        let o = o;
        let put = |t: usize, j: usize, v: f32| {
            // SAFETY: element (t, j) is inside y (asserted above) and only this part writes columns j0..j1.
            unsafe { *o.0.add(t * outp + j) = v };
        };
        if tune.order == Order::JT {
            for j in j0..j1 {
                for t in 0..rows {
                    put(t, j, one(t, j));
                }
            }
        } else {
            for t in 0..rows {
                for j in j0..j1 {
                    put(t, j, one(t, j));
                }
            }
        }
    });
}

/// A raw pointer that may cross threads (each thread writes its own elements).
#[derive(Clone, Copy)]
struct Out(*mut f32);
// SAFETY: used only by `par` jobs, which write disjoint elements and are joined before the buffer is read.
unsafe impl Sync for Out {}
unsafe impl Send for Out {}

// ---------------------------------------------------------------------------------------------------------------
// The model.
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub layers: usize,
    pub heads: usize,
    pub emb: usize,
    pub context: usize,
    pub vocab: usize,
    pub eps: f32,
}

struct Layer<'a> {
    ln1: (&'a [f32], &'a [f32]),
    qkv: Mat<'a>,
    qkv_b: &'a [f32],
    proj: Mat<'a>,
    proj_b: &'a [f32],
    ln2: (&'a [f32], &'a [f32]),
    fc: Mat<'a>,
    fc_b: &'a [f32],
    out: Mat<'a>,
    out_b: &'a [f32],
}

pub struct Model<'a> {
    pub cfg: Config,
    /// `fel` key (f32 / q8_0), `sursa`, the tokenizer JSON text.
    pub kind: &'a str,
    pub source: &'a str,
    pub tokenizer: &'a str,
    tok: Mat<'a>,
    pos: Mat<'a>,
    layers: Vec<Layer<'a>>,
    lnf: (&'a [f32], &'a [f32]),
    /// Keys and values of positions 0..len, per layer: [context, emb] each. One `mem_map` region outside the program
    /// heap (P6-27b: model #459 needs 2 x 12 x 512 x 768 floats = 36 MiB, more than the heap); returned at exit.
    kcache: Vec<&'static mut [f32]>,
    vcache: Vec<&'static mut [f32]>,
    pub len: usize,
    /// Threads, vectors and loop order of the products (P6-28: from the tuning file when there is one).
    pub tune: Tuning,
}

fn vector<'a>(f: &File<'a>, name: &str, n: usize) -> Result<&'a [f32], String> {
    let t = f.tensor(name).ok_or_else(|| alloc::format!("lipseste tensorul `{}`", name))?;
    if t.kind != Kind::F32 || t.dims.len() != 1 || t.dims[0] != n {
        return Err(alloc::format!("`{}` are forma {:?}, astept [{}] F32", name, t.dims, n));
    }
    Ok(floats(t.bytes)?)
}

fn matrix<'a>(f: &File<'a>, name: &str, rows: usize, cols: usize) -> Result<Mat<'a>, String> {
    Mat::new(f.tensor(name).ok_or_else(|| alloc::format!("lipseste tensorul `{}`", name))?, rows, cols)
}

impl<'a> Model<'a> {
    /// Builds the model from a parsed file (checks every name and shape the computation needs).
    pub fn new(f: &File<'a>) -> Result<Model<'a>, String> {
        if f.text("arhitectura") != Some("kronos_gpt") {
            return Err("arhitectura nu e `kronos_gpt`".into());
        }
        let eps = match f.key("ln_eps") {
            Some(Value::Float(e)) => *e as f32,
            None => 1e-5,
            _ => return Err("cheia `ln_eps` nu e un numar".into()),
        };
        let cfg = Config {
            layers: f.int("n_straturi")?,
            heads: f.int("n_capete")?,
            emb: f.int("n_emb")?,
            context: f.int("context")?,
            vocab: f.int("vocabular")?,
            eps,
        };
        if cfg.emb % cfg.heads != 0 {
            return Err("n_emb nu se imparte la n_capete".into());
        }
        if f.int("cap_legat").ok() != Some(1) {
            return Err("stiu doar modele cu `cap_legat` = 1 (capul foloseste emb_token.weight)".into());
        }
        let e = cfg.emb;
        let mut layers = Vec::new();
        for i in 0..cfg.layers {
            let p = |s: &str| alloc::format!("straturi.{}.{}", i, s);
            layers.push(Layer {
                ln1: (vector(f, &p("ln1.weight"), e)?, vector(f, &p("ln1.bias"), e)?),
                qkv: matrix(f, &p("atentie.in_proj_weight"), 3 * e, e)?,
                qkv_b: vector(f, &p("atentie.in_proj_bias"), 3 * e)?,
                proj: matrix(f, &p("atentie.out_proj.weight"), e, e)?,
                proj_b: vector(f, &p("atentie.out_proj.bias"), e)?,
                ln2: (vector(f, &p("ln2.weight"), e)?, vector(f, &p("ln2.bias"), e)?),
                fc: matrix(f, &p("mlp.0.weight"), 4 * e, e)?,
                fc_b: vector(f, &p("mlp.0.bias"), 4 * e)?,
                out: matrix(f, &p("mlp.2.weight"), e, 4 * e)?,
                out_b: vector(f, &p("mlp.2.bias"), e)?,
            });
        }
        let per = cfg.context * e;
        let floats_total = 2 * cfg.layers * per;
        let region = cyber_user::mem_map((floats_total * 4).max(4096))
            .map_err(|err| alloc::format!("memoria cheilor/valorilor ({} MiB): {}", floats_total * 4 >> 20, cyber_user::error_text(err)))?;
        // SAFETY: a fresh region of `floats_total * 4` bytes, page aligned (so aligned for f32), owned by this program
        // and never unmapped while it runs; it is split below into disjoint slices, each used by one layer only.
        let all: &'static mut [f32] = unsafe { core::slice::from_raw_parts_mut(region as *mut f32, floats_total) };
        let mut kcache = Vec::with_capacity(cfg.layers);
        let mut vcache = Vec::with_capacity(cfg.layers);
        for (n, part) in all.chunks_exact_mut(per).enumerate() {
            if n % 2 == 0 { kcache.push(part) } else { vcache.push(part) }
        }
        Ok(Model {
            cfg,
            kind: f.text("fel").unwrap_or("?"),
            source: f.text("sursa").unwrap_or("?"),
            tokenizer: f.text("tokenizer.bpe").unwrap_or(""),
            tok: matrix(f, "emb_token.weight", cfg.vocab, e)?,
            pos: matrix(f, "emb_pozitie.weight", cfg.context, e)?,
            layers,
            lnf: (vector(f, "ln.weight", e)?, vector(f, "ln.bias", e)?),
            kcache,
            vcache,
            len: 0,
            tune: Tuning { threads: 1, simd: false, order: Order::TJ },
        })
    }

    /// Forgets the text so far (the next `forward` starts at position 0).
    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Number of parameters (weights) in the model.
    pub fn parameters(&self) -> usize {
        let m = |m: &Mat| m.rows * m.cols;
        let e = self.cfg.emb;
        m(&self.tok) + m(&self.pos) + 2 * e
            + self.layers.iter().map(|l| m(&l.qkv) + m(&l.proj) + m(&l.fc) + m(&l.out) + 3 * e + 3 * e + 4 * e + e + e).sum::<usize>()
    }

    fn linear(&self, kind: usize, w: &Mat, b: Option<&[f32]>, x: &[f32], rows: usize, y: &mut [f32]) {
        self.linear_act(kind, w, b, x, rows, y, false);
    }

    fn linear_act(&self, kind: usize, w: &Mat, b: Option<&[f32]>, x: &[f32], rows: usize, y: &mut [f32], gelu: bool) {
        let t0 = prof::now();
        linear_act(w, b, x, rows, y, self.tune, gelu);
        prof::add(kind, prof::now() - t0, w.bytes() as u64);
    }

    /// Runs `ids` from position `len` on; returns the logits of every new position ([ids.len(), vocab]) when `all`,
    /// otherwise those of the last one only ([vocab]).
    pub fn forward(&mut self, ids: &[u32], all: bool) -> Result<Vec<f32>, String> {
        let (e, h) = (self.cfg.emb, self.cfg.heads);
        let d = e / h;
        let t = ids.len();
        let p0 = self.len;
        if t == 0 {
            return Err("niciun simbol".into());
        }
        if p0 + t > self.cfg.context {
            return Err(alloc::format!("textul depaseste contextul modelului ({} simboluri)", self.cfg.context));
        }
        if let Some(&bad) = ids.iter().find(|&&i| i as usize >= self.cfg.vocab) {
            return Err(alloc::format!("simbolul {} e in afara vocabularului ({})", bad, self.cfg.vocab));
        }
        let mut x = vec![0.0f32; t * e];
        let mut row = vec![0.0f32; e];
        for (i, &id) in ids.iter().enumerate() {
            let xr = &mut x[i * e..(i + 1) * e];
            self.tok.row(id as usize, xr);
            self.pos.row(p0 + i, &mut row);
            for (a, b) in xr.iter_mut().zip(&row) {
                *a += b;
            }
        }
        let mut hbuf = vec![0.0f32; t * e];
        let mut qkv = vec![0.0f32; t * 3 * e];
        let mut att = vec![0.0f32; t * e];
        let mut tmp = vec![0.0f32; t * e];
        let mut big = vec![0.0f32; t * 4 * e];
        let scale = 1.0 / math::sqrt(d as f32);
        // Attention is split over (position, head) pairs on the thread pool (P6-29); each part has its own scores row.
        let parts = self.tune.threads.clamp(1, MAX_THREADS).min(t * h);
        let mut scores = vec![0.0f32; parts * (p0 + t)];
        let mut rest = 0u64;
        for li in 0..self.layers.len() {
            let l = &self.layers[li];
            let c0 = prof::now();
            layernorm(&x, l.ln1, &mut hbuf, t, e, self.cfg.eps);
            rest += prof::now() - c0;
            if t == 1 {
                // P6-36 (C-40): generating one token, the qkv product and the attention run in ONE split over the heads:
                // each part computes the q, k and v columns of its heads, stores k and v in the cache, then the
                // attention of those heads (it needs no other head's columns). One wait for the threads per layer
                // fewer (49 per token instead of 61). Every value comes from the same code as below: same bits.
                let c0 = prof::now();
                let parts = self.tune.threads.clamp(1, MAX_THREADS).min(h);
                let (kc, vc) = (Out(self.kcache[li].as_mut_ptr()), Out(self.vcache[li].as_mut_ptr()));
                let (qo, ao, so) = (Out(qkv.as_mut_ptr()), Out(att.as_mut_ptr()), Out(scores.as_mut_ptr()));
                let (hb, tune) = (&hbuf[..e], self.tune);
                par_parts(h, parts, &|part, h0, h1| {
                    let (kc, vc, qo, ao, so) = (kc, vc, qo, ao, so);
                    for j in h0..h1 {
                        for c in [j * d, e + j * d, 2 * e + j * d] {
                            for col in c..c + d {
                                // SAFETY: column `col` of qkv row 0 belongs to head j, which only this part has.
                                unsafe { *qo.0.add(col) = output(&l.qkv, Some(l.qkv_b), hb, col, tune) };
                            }
                        }
                        // SAFETY: as above; the cache rows have `e` floats per position, columns j*d..j*d+d are head j's.
                        unsafe {
                            core::ptr::copy_nonoverlapping(qo.0.add(e + j * d), kc.0.add(p0 * e + j * d), d);
                            core::ptr::copy_nonoverlapping(qo.0.add(2 * e + j * d), vc.0.add(p0 * e + j * d), d);
                        }
                    }
                    // SAFETY: `scores` row `part` belongs to this part.
                    let sc = unsafe { core::slice::from_raw_parts_mut(so.0.add(part * (p0 + 1)), p0 + 1) };
                    for j in h0..h1 {
                        // SAFETY: q and the output of head j (row 0, columns j*d..j*d+d) are this part's alone; rows
                        // 0..=p0 of the cache hold head j's keys and values (earlier positions from before this call,
                        // position p0 written just above by this part), and only head j's columns are read.
                        unsafe {
                            let q = core::slice::from_raw_parts(qo.0.add(j * d), d);
                            let out = core::slice::from_raw_parts_mut(ao.0.add(j * d), d);
                            attend(q, kc.0, vc.0, e, j * d, p0, scale, sc, out);
                        }
                    }
                });
                prof::add(prof::QKV, prof::now() - c0, l.qkv.bytes() as u64);
            } else {
            self.linear(prof::QKV, &l.qkv, Some(l.qkv_b), &hbuf, t, &mut qkv);
            let c0 = prof::now();
            let (kc, vc) = (&mut self.kcache[li], &mut self.vcache[li]);
            for i in 0..t {
                let r = &qkv[i * 3 * e..(i + 1) * 3 * e];
                kc[(p0 + i) * e..(p0 + i + 1) * e].copy_from_slice(&r[e..2 * e]);
                vc[(p0 + i) * e..(p0 + i + 1) * e].copy_from_slice(&r[2 * e..]);
            }
            let (kc, vc) = (&*kc, &*vc);
            let (ao, so) = (Out(att.as_mut_ptr()), Out(scores.as_mut_ptr()));
            let qkv = &qkv;
            par_parts(t * h, parts, &|part, a, b| {
                let (ao, so) = (ao, so);
                // SAFETY: part `part` alone uses scores row `part` (parts rows of p0 + t floats were allocated).
                let sc_all = unsafe { core::slice::from_raw_parts_mut(so.0.add(part * (p0 + t)), p0 + t) };
                for item in a..b {
                    let (i, j) = (item / h, item % h);
                    let pos = p0 + i; // sees positions 0..=pos
                    let qh = &qkv[i * 3 * e + j * d..i * 3 * e + (j + 1) * d];
                    // SAFETY: (position i, head j) = att[i*e + j*d .. +d], written only by the part that has this item.
                    let out = unsafe { core::slice::from_raw_parts_mut(ao.0.add(i * e + j * d), d) };
                    // SAFETY: the cache is only read here (filled before the split), rows 0..=pos exist.
                    unsafe { attend(qh, kc.as_ptr(), vc.as_ptr(), e, j * d, pos, scale, sc_all, out) };
                }
            });
            prof::add(prof::ATTN, prof::now() - c0, 0);
            }
            let l = &self.layers[li];
            self.linear(prof::PROJ, &l.proj, Some(l.proj_b), &att, t, &mut tmp);
            let c0 = prof::now();
            for (a, b) in x.iter_mut().zip(&tmp) {
                *a += b;
            }
            layernorm(&x, l.ln2, &mut hbuf, t, e, self.cfg.eps);
            rest += prof::now() - c0;
            self.linear_act(prof::FC, &l.fc, Some(l.fc_b), &hbuf, t, &mut big, true);
            self.linear(prof::OUT, &l.out, Some(l.out_b), &big, t, &mut tmp);
            let c0 = prof::now();
            for (a, b) in x.iter_mut().zip(&tmp) {
                *a += b;
            }
            rest += prof::now() - c0;
        }
        self.len = p0 + t;
        let (first, rows) = if all { (0, t) } else { (t - 1, 1) };
        let c0 = prof::now();
        layernorm(&x[first * e..], self.lnf, &mut hbuf, rows, e, self.cfg.eps);
        prof::add(prof::REST, rest + prof::now() - c0, 0);
        let mut logits = vec![0.0f32; rows * self.cfg.vocab];
        self.linear(prof::HEAD, &self.tok, None, &hbuf, rows, &mut logits);
        Ok(logits)
    }
}

/// Causal attention of one (position, head): `q` (d floats) against the keys of positions 0..=pos (head columns
/// `col..col+d` of the `e`-wide cache rows at `kc` / `vc`), softmax, weighted sum of the values into `out`.
/// `sc` = at least pos + 1 scratch floats.
///
/// # Safety
/// `kc` and `vc` must point at `(pos + 1) * e` floats whose columns `col..col+d` nobody writes during the call (other
/// threads may write other columns: only these columns are ever read, through slices of `d` floats).
#[allow(clippy::too_many_arguments)]
unsafe fn attend(q: &[f32], kc: *const f32, vc: *const f32, e: usize, col: usize, pos: usize, scale: f32, sc: &mut [f32], out: &mut [f32]) {
    let d = q.len();
    // SAFETY: inside the cache rows and columns the caller vouches for.
    let key = |s: usize| unsafe { core::slice::from_raw_parts(kc.add(s * e + col), d) };
    let val = |s: usize| unsafe { core::slice::from_raw_parts(vc.add(s * e + col), d) };
    let sc = &mut sc[..=pos];
    let mut max = f32::NEG_INFINITY;
    for (s, v) in sc.iter_mut().enumerate() {
        *v = q.iter().zip(key(s)).fold(0.0f32, |a, (q, k)| a + q * k) * scale;
        max = max.max(*v);
    }
    let mut sum = 0.0f32;
    for v in sc.iter_mut() {
        *v = math::exp(*v - max);
        sum += *v;
    }
    out.fill(0.0);
    for (s, &p) in sc.iter().enumerate() {
        for (o, v) in out.iter_mut().zip(val(s)) {
            *o += p * v;
        }
    }
    for o in out.iter_mut() {
        *o /= sum;
    }
}

/// LayerNorm of `rows` rows (biased variance, divided by `cols`, as the spec says).
fn layernorm(x: &[f32], (w, b): (&[f32], &[f32]), out: &mut [f32], rows: usize, cols: usize, eps: f32) {
    for r in 0..rows {
        let xr = &x[r * cols..(r + 1) * cols];
        let mean = xr.iter().sum::<f32>() / cols as f32;
        let var = xr.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / cols as f32;
        let inv = 1.0 / math::sqrt(var + eps);
        for c in 0..cols {
            out[r * cols + c] = (xr[c] - mean) * inv * w[c] + b[c];
        }
    }
}

/// Index of the largest value (the first one on a tie).
pub fn argmax(v: &[f32]) -> u32 {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x > v[best] {
            best = i;
        }
    }
    best as u32
}
