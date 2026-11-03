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
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering};
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

// P6-43 (C-59): on the workstation (36 threads) the threads waited 62.9 % of the time, ~105 us per wait, while a wait on
// 36 cores should cost a few us. Two causes, both fixed here:
// - `par` waited for EVERY worker, also those without a part (qkv + attention and the projection use 12 parts of 36);
//   those idled through the long qkv + attention split and, after 4000 pauses, gave their processor away (`yield`,
//   through the scheduler's single lock), so the next product waited for them to come back. Now the job word holds
//   the generation AND the number of parts: only workers with a part read the job and are waited for.
// - the shared DONE counter (bumped by 35 threads) shared a cache line with GEN (read by all of them in their spin):
//   every bump disturbed every spinner. Now each worker has its own "done" word on its own cache line.
// And the waiting threads stay awake: with no more threads than processors they no longer give their processor away
// every few thousand pauses (only after ~65 000, a few ms: longer than any product).

struct Job<'a> {
    n: usize,
    parts: usize,
    /// P6-45 (C-59, „bucățele”): more pieces than parts = the range is cut in `chunks` pieces and every thread of the job
    /// takes the next free piece from NEXT until none is left (whoever is done helps with the rest); = parts: one
    /// fixed range per part, as before.
    chunks: usize,
    /// Threads per team in this job (<= 1 = none) and the last part served by a worker (teams end there).
    team: usize,
    helped: usize,
    f: &'a (dyn Fn(usize, usize, usize) + Sync),
}

/// The parts a leader waits for: its teammates that run on workers (`leader` excluded).
fn teammates(leader: usize, team: usize, helped: usize) -> core::ops::Range<usize> {
    leader + 1..(leader + team).min(helped + 1)
}

/// P6-45: the next free piece of the current job (pieces mode), on its own cache line.
static NEXT: Line = Line(AtomicUsize::new(0));
/// P6-45: pieces per part (0 or 1 = fixed ranges, the default); set by `kronos --bucati` / the tuning file (`bucati`).
static PIECES: AtomicUsize = AtomicUsize::new(0);

/// P6-45 step 3 (C-59, Adrian's „echipe”): threads per team (0 or 1 = off). With teams, part p belongs to team p / size
/// (neighbouring threads: on the workstation the same physical cores); a team's first part (its leader) waits for its
/// teammates, then signals for the whole team, and `par` waits only for the leaders: a wait in two steps, 1 + parts/size
/// words read by the main thread instead of every part's. The ranges stay the same (same slice per team every token).
static TEAM: AtomicUsize = AtomicUsize::new(0);

/// Sets the threads per team of every later split (0 or 1 = no teams). See `TEAM`.
pub fn set_team(n: usize) {
    TEAM.store(n.min(MAX_THREADS), Ordering::Relaxed);
}

/// Sets the pieces per part of every later split (0 or 1 = one fixed range per part).
pub fn set_pieces(n: usize) {
    PIECES.store(n.min(64), Ordering::Relaxed);
}

/// Runs the pieces of `job` until none is left; `part` only names this thread's scratch (as before).
fn run_pieces(job: &Job<'_>, part: usize) {
    loop {
        let c = NEXT.0.fetch_add(1, Ordering::Relaxed);
        if c >= job.chunks {
            return;
        }
        let (a, b) = split(job.n, job.chunks, c);
        (job.f)(part, a, b);
    }
}

/// One word on its own cache line (no false sharing between the threads that spin on different words).
#[repr(align(128))]
struct Line(AtomicUsize);

/// Address of the `Job` being run (valid until every worker with a part has signalled its generation as done).
static JOB: Line = Line(AtomicUsize::new(0));
/// `generation << 8 | parts` of the current job (written by `par` after `JOB`); generation bumped once per job and once
/// more to stop the pool (then parts = 0xff: every worker sees it). Read in ONE load, so a worker never pairs one
/// job's generation with another job's number of parts.
static STATE: Line = Line(AtomicUsize::new(0));
/// Per worker (index = part - 1): the last generation it finished.
static DONE: [Line; MAX_THREADS] = [const { Line(AtomicUsize::new(0)) }; MAX_THREADS];
/// P6-45 (C-59, step 1): per worker, TSC when it started its part and when it finished it (written before DONE, read
/// by `par` after it saw DONE), so the profile can tell waking/gathering apart from waiting for the slowest part.
#[repr(align(128))]
struct Stamps {
    start: core::sync::atomic::AtomicU64,
    end: core::sync::atomic::AtomicU64,
}
static STAMPS: [Stamps; MAX_THREADS] =
    [const { Stamps { start: core::sync::atomic::AtomicU64::new(0), end: core::sync::atomic::AtomicU64::new(0) } }; MAX_THREADS];
/// Workers started (they are parts 1..=WORKERS); only the main thread starts or stops them.
static WORKERS: AtomicUsize = AtomicUsize::new(0);
static QUIT: AtomicBool = AtomicBool::new(false);
/// Thread id and stack of every worker (index = part - 1).
static HANDLES: [(AtomicI64, AtomicUsize); MAX_THREADS] = [const { (AtomicI64::new(0), AtomicUsize::new(0)) }; MAX_THREADS];
/// Spins of a waiting thread between two `yield`s: SPINS when the program has more threads than processors (as in
/// QEMU: the thread it waits for may need this processor), SPINS_AWAKE otherwise (P6-43).
static SPIN_LIMIT: AtomicU32 = AtomicU32::new(SPINS);

/// Spins between two `yield`s when threads outnumber processors (~1 us of pauses on a free core).
const SPINS: u32 = 4000;
/// Spins between two `yield`s when every thread has its processor (a few ms: far longer than any product, short enough
/// that an idle worker, not a busy one taken by the 10 ms timer, gives its processor to a system thread that needs one).
const SPINS_AWAKE: u32 = 1 << 16;
const PARTS_MASK: usize = 0xff;
const _: () = assert!(MAX_THREADS < PARTS_MASK);

fn split(n: usize, parts: usize, part: usize) -> (usize, usize) {
    (n * part / parts, n * (part + 1) / parts)
}

fn spin_until(mut done: impl FnMut() -> bool) {
    let limit = SPIN_LIMIT.load(Ordering::Relaxed);
    let mut spins = 0;
    while !done() {
        core::hint::spin_loop();
        spins += 1;
        if spins >= limit {
            spins = 0;
            prof::count(prof::YIELDS);
            cyber_user::yield_now();
        }
    }
}

/// `arg` = part (low 8 bits) | the generation current when the worker was started (so it cannot miss the first job).
extern "C" fn worker(arg: u64) -> i64 {
    let part = (arg & 0xff) as usize;
    let mut seen = (arg >> 8) as usize;
    loop {
        let mut state = 0;
        spin_until(|| {
            state = STATE.0.load(Ordering::Acquire);
            state >> 8 != seen
        });
        seen = state >> 8;
        if QUIT.load(Ordering::Acquire) {
            return 0;
        }
        if part >= state & PARTS_MASK {
            continue; // no part in this job: nobody waits for this thread
        }
        // SAFETY: JOB holds the address of the Job on `par`'s stack, written before STATE; this worker has a part in
        // generation `seen`, so `par` does not return (the Job lives, JOB is not overwritten) until this worker has
        // stored `seen` in its DONE word below.
        let job = unsafe { &*(JOB.0.load(Ordering::Acquire) as *const Job<'static>) };
        let stamps = &STAMPS[part - 1];
        stamps.start.store(prof::now(), Ordering::Relaxed);
        if job.chunks > job.parts {
            run_pieces(job, part);
        } else {
            let (a, b) = split(job.n, job.parts, part);
            (job.f)(part, a, b);
        }
        if job.team > 1 && part % job.team == 0 {
            // Team leader: first its teammates (Acquire), then one signal for the whole team (Release: `par` sees
            // every teammate's writes through it).
            for m in teammates(part, job.team, job.helped) {
                spin_until(|| DONE[m - 1].0.load(Ordering::Acquire) == seen);
            }
        }
        stamps.end.store(prof::now(), Ordering::Relaxed);
        DONE[part - 1].0.store(seen, Ordering::Release);
    }
}

/// Starts workers until there are `want` (fewer if the kernel refuses); returns how many there are.
fn grow_pool(want: usize) -> usize {
    let mut have = WORKERS.load(Ordering::Relaxed);
    if have >= want.min(MAX_THREADS - 1) {
        return have;
    }
    while have < want.min(MAX_THREADS - 1) {
        let part = have + 1;
        let generation = STATE.0.load(Ordering::Acquire) >> 8;
        DONE[have].0.store(generation, Ordering::Relaxed);
        let (id, stack) = cyber_user::thread::spawn_with_stack(worker, part as u64 | (generation as u64) << 8);
        if id < 0 {
            break;
        }
        HANDLES[have].0.store(id, Ordering::Relaxed);
        HANDLES[have].1.store(stack as usize, Ordering::Relaxed);
        have += 1;
        WORKERS.store(have, Ordering::Release);
    }
    // The program's threads = the workers + this one: stay awake only if each has its own processor.
    let cpus = cyber_user::cpus().max(1) as usize;
    SPIN_LIMIT.store(if have < cpus { SPINS_AWAKE } else { SPINS }, Ordering::Relaxed);
    have
}

/// Stops the pool's threads (the next `par` that needs threads starts them again).
pub fn stop_pool() {
    let w = WORKERS.load(Ordering::Relaxed);
    if w == 0 {
        return;
    }
    QUIT.store(true, Ordering::Release);
    let generation = (STATE.0.load(Ordering::Acquire) >> 8) + 1;
    STATE.0.store(generation << 8 | PARTS_MASK, Ordering::Release);
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
    // Parts 1..=helped run on workers; parts without a worker (the kernel refused a thread) run here.
    let helped = workers.min(parts - 1);
    let pieces = PIECES.load(Ordering::Relaxed);
    let chunks = if pieces > 1 { (parts * pieces).min(n) } else { parts };
    let team = TEAM.load(Ordering::Relaxed);
    let job = Job { n, parts, chunks, team, helped, f };
    JOB.0.store(&job as *const Job as usize, Ordering::Release);
    // The workers of the previous job are all done (waited for below), so nobody reads NEXT while it is reset; the
    // Release store of STATE publishes it with the job.
    NEXT.0.store(0, Ordering::Relaxed);
    let generation = (STATE.0.load(Ordering::Relaxed) >> 8) + 1;
    let published = prof::now();
    STATE.0.store(generation << 8 | (helped + 1), Ordering::Release);
    if chunks > parts {
        // Pieces: this thread takes pieces too (and so covers the parts the kernel refused a worker for).
        run_pieces(&job, 0);
    } else {
        for p in helped + 1..parts {
            let (a, b) = split(n, parts, p);
            f(p, a, b);
        }
        let (a, b) = split(n, parts, 0);
        f(0, a, b);
    }
    let t0 = prof::now();
    if team > 1 {
        // Two steps: this thread leads team 0 (its teammates), then waits only for the other teams' leaders.
        for m in teammates(0, team, helped) {
            spin_until(|| DONE[m - 1].0.load(Ordering::Acquire) == generation);
        }
        for leader in (team..=helped).step_by(team) {
            spin_until(|| DONE[leader - 1].0.load(Ordering::Acquire) == generation);
        }
    } else {
        for d in DONE.iter().take(helped) {
            spin_until(|| d.0.load(Ordering::Acquire) == generation);
        }
    }
    let t1 = prof::now();
    prof::add(prof::SYNC, t1 - t0, 0);
    prof::count(prof::BARRIERS);
    // P6-45 (C-59, step 1): the wait split in three, from the stamps of the workers (TSC is shared by the cores).
    // waking = publish -> the last worker starts; slowest = the first part done (this thread's included) -> the last;
    // gathering = the last part done -> this thread sees it.
    let (mut first_end, mut last_end, mut last_start) = (t0, t0, published);
    for st in STAMPS.iter().take(helped) {
        let (s0, s1) = (st.start.load(Ordering::Relaxed), st.end.load(Ordering::Relaxed));
        last_start = last_start.max(s0);
        first_end = first_end.min(s1);
        last_end = last_end.max(s1);
    }
    prof::split_wait(last_start.saturating_sub(published), last_end.saturating_sub(first_end), t1.saturating_sub(last_end));
}

// ---------------------------------------------------------------------------------------------------------------
// P6-45 step 4 (C-59, Adrian's idea 3): ONE block per token, no bell between the computations.
// ---------------------------------------------------------------------------------------------------------------
// `par` per product means: the main thread publishes, every worker wakes, works, signals; the main thread gathers and
// publishes the next product (~49-61 times per token). In a block, the whole list of a token's computations is given
// to the threads at once: every thread walks the list, takes pieces of the current stage from its shared counter and,
// when a stage has no piece left, goes on to wait only until the stage before the next one is complete (its counter of
// finished pieces). The thread that finishes the LAST piece of a stage runs the stage's short serial tail (residual
// add + LayerNorm) and opens the next stage. Nobody gathers, nobody republishes: one `par` per token.
// Every element is still computed by the same code, once, from the same inputs: the same bits as without the block.

/// One stage of a block: `run(part, a, b)` computes elements a..b of 0..n (cut in `chunks` pieces taken from a shared
/// counter); `tail` (optional) runs once, on the thread that finished the stage's last piece, before the next stage
/// opens. `kind` = the profile line the stage's time goes to.
pub struct Stage<'a> {
    pub n: usize,
    pub chunks: usize,
    pub kind: usize,
    pub run: &'a (dyn Fn(usize, usize, usize) + Sync),
    pub tail: Option<&'a (dyn Fn() + Sync)>,
}

/// Counters of one stage, 128 bytes apart (so never on one cache line; the program heap does not hand out 128-aligned
/// blocks, hence padding instead of `Line`): the next free piece, the pieces finished, open (1 = the stage before is
/// complete, tail included).
struct StageCtl {
    next: Pad,
    done: Pad,
    open: Pad,
}

/// One counter followed by 120 bytes of padding.
struct Pad([AtomicUsize; 16]);

impl Pad {
    fn new(v: usize) -> Pad {
        let p = Pad([const { AtomicUsize::new(0) }; 16]);
        p.0[0].store(v, Ordering::Relaxed);
        p
    }
}

/// Runs `stages` in order on `parts` threads as ONE split (see above). Returns when every stage is complete.
pub fn run_block(stages: &[Stage<'_>], parts: usize) {
    if stages.is_empty() {
        return;
    }
    let ctl: Vec<StageCtl> = (0..stages.len())
        .map(|s| StageCtl { next: Pad::new(0), done: Pad::new(0), open: Pad::new((s == 0) as usize) })
        .collect();
    let parts = parts.clamp(1, MAX_THREADS);
    let walk = |part: usize| {
        for (s, st) in stages.iter().enumerate() {
            let c = &ctl[s];
            // The wait for the stage before counts in this stage's time too ("din care asteptarea firelor", as in `par`).
            let t0 = prof::now();
            if c.open.0[0].load(Ordering::Acquire) == 0 {
                spin_until(|| c.open.0[0].load(Ordering::Acquire) != 0);
                if part == 0 {
                    prof::add(prof::SYNC, prof::now() - t0, 0);
                    prof::count(prof::BARRIERS);
                }
            }
            let chunks = st.chunks.clamp(1, st.n.max(1));
            loop {
                let k = c.next.0[0].fetch_add(1, Ordering::Relaxed);
                if k >= chunks {
                    break;
                }
                let (a, b) = split(st.n, chunks, k);
                (st.run)(part, a, b);
                // AcqRel: the last finisher sees every other piece's writes (release sequence on `done`).
                if c.done.0[0].fetch_add(1, Ordering::AcqRel) + 1 == chunks {
                    if let Some(tail) = st.tail {
                        tail();
                    }
                    if let Some(n) = ctl.get(s + 1) {
                        n.open.0[0].store(1, Ordering::Release);
                    }
                }
            }
            if part == 0 {
                prof::add(st.kind, prof::now() - t0, 0);
            }
        }
    };
    if parts == 1 {
        walk(0);
        return;
    }
    // One fixed range per part (n = parts), so each thread walks the list exactly once; parts the kernel refused a
    // worker for run on this thread one after another, which is still correct (any thread can take any piece).
    let saved = PIECES.swap(0, Ordering::Relaxed);
    par_parts(parts, parts, &|part, _, _| walk(part));
    PIECES.store(saved, Ordering::Relaxed);
    // Every piece of the last stage was finished inside some thread's walk, and `par_parts` waited for all of them.
}

/// Pieces of a stage of `n` elements on `parts` threads: `pieces` per thread (at least 1), at most n.
pub fn block_chunks(n: usize, parts: usize) -> usize {
    (parts * PIECES.load(Ordering::Relaxed).max(1)).clamp(1, n.max(1))
}

/// P6-45: self-test of `run_block`: 12 stages, each reading ALL of the stage before (a permutation, +1) and a tail
/// that adds a value every element of the next stage needs: every element must be exact, so no thread ever started a
/// stage before the one before was complete.
pub fn self_test_block(max_parts: usize) -> Result<usize, String> {
    const N: usize = 97;
    const S: usize = 12;
    let vals: Vec<AtomicUsize> = (0..N * (S + 1)).map(|_| AtomicUsize::new(0)).collect();
    let bonus: Vec<AtomicUsize> = (0..S + 1).map(|_| AtomicUsize::new(0)).collect();
    let mut runs = 0;
    for parts in 1..=max_parts.clamp(1, MAX_THREADS) {
        for v in &vals {
            v.store(0, Ordering::Relaxed);
        }
        for (i, v) in vals.iter().take(N).enumerate() {
            v.store(i, Ordering::Relaxed);
        }
        let runs_v: Vec<_> = (1..=S)
            .map(|s| {
                let (vals, bonus) = (&vals, &bonus);
                move |_: usize, a: usize, b: usize| {
                    for i in a..b {
                        let prev = vals[(s - 1) * N + (i * 7 + 3) % N].load(Ordering::Relaxed);
                        vals[s * N + i].fetch_add(prev + 1 + bonus[s - 1].load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                }
            })
            .collect();
        let tails: Vec<_> = (1..=S)
            .map(|s| {
                let bonus = &bonus;
                move || bonus[s].store(s, Ordering::Relaxed)
            })
            .collect();
        let stages: Vec<Stage> = (0..S)
            .map(|k| Stage { n: N, chunks: block_chunks(N, parts) + k % 3, kind: prof::REST, run: &runs_v[k], tail: Some(&tails[k]) })
            .collect();
        run_block(&stages, parts);
        runs += 1;
        // The same values computed on one thread, in order.
        let mut want: Vec<usize> = (0..N).collect();
        for s in 1..=S {
            let b = if s == 1 { 0 } else { s - 1 };
            want = (0..N).map(|i| want[(i * 7 + 3) % N] + 1 + b).collect();
            for (i, w) in want.iter().enumerate() {
                let got = vals[s * N + i].load(Ordering::Relaxed);
                if got != *w {
                    return Err(alloc::format!("bloc pe {} fire: etapa {}, elementul {} = {}, asteptat {}", parts, s, i, got, w));
                }
            }
        }
        for b in &bonus {
            b.store(0, Ordering::Relaxed);
        }
    }
    Ok(runs)
}

/// P6-43: many splits in a row with a changing number of parts (1..=max_parts, up and down, so workers without a part
/// in one job get one in the next): every element of every job must be written exactly once, by the right part.
/// Returns the number of jobs.
pub fn self_test_pool(max_parts: usize) -> Result<usize, String> {
    let max_parts = max_parts.clamp(1, MAX_THREADS);
    const N: usize = 97;
    let hits: Vec<AtomicUsize> = (0..N).map(|_| AtomicUsize::new(0)).collect();
    let mut jobs = 0;
    for round in 0..200 {
        let parts = if round % 2 == 0 { 1 + round / 2 % max_parts } else { max_parts - round / 2 % max_parts };
        for h in &hits {
            h.store(0, Ordering::Relaxed);
        }
        par_parts(N, parts, &|part, a, b| {
            for i in a..b {
                hits[i].fetch_add(1 + (part << 8), Ordering::Relaxed);
            }
        });
        jobs += 1;
        let parts_used = parts.min(N);
        if PIECES.load(Ordering::Relaxed) > 1 && parts_used > 1 {
            // Pieces: any thread may compute any element, but each exactly once, by a thread of this job.
            for (i, h) in hits.iter().enumerate() {
                let v = h.load(Ordering::Relaxed);
                if v & 0xff != 1 || v >> 8 >= parts_used {
                    return Err(alloc::format!("lucrarea {} ({} parti, bucati): elementul {} = {:#x}", jobs, parts, i, v));
                }
            }
            continue;
        }
        for p in 0..parts_used {
            let (a, b) = split(N, parts_used, p);
            for (i, h) in hits.iter().enumerate().take(b).skip(a) {
                let v = h.load(Ordering::Relaxed);
                if v != 1 + (p << 8) {
                    return Err(alloc::format!("lucrarea {} ({} parti): elementul {} = {:#x}, asteptat {:#x}", jobs, parts, i, v, 1 + (p << 8)));
                }
            }
        }
    }
    Ok(jobs)
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

    /// Events counted by `count` (P6-43): finished waits for threads at the end of a split, and `yield`s of waiting threads.
    pub const BARRIERS: usize = 0;
    pub const YIELDS: usize = 1;
    static COUNTS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

    pub fn count(what: usize) {
        COUNTS[what].fetch_add(1, Ordering::Relaxed);
    }

    pub fn counted(what: usize) -> u64 {
        COUNTS[what].load(Ordering::Relaxed)
    }

    /// P6-45: the waits split in three (TSC cycles, summed over every barrier): publish -> last worker started (waking),
    /// first part done -> last part done (waiting for the slowest), last part done -> seen by `par` (gathering).
    pub const WAKE: usize = 0;
    pub const SLOWEST: usize = 1;
    pub const GATHER: usize = 2;
    static WAIT_PARTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

    pub fn split_wait(wake: u64, slowest: u64, gather: u64) {
        WAIT_PARTS[WAKE].fetch_add(wake, Ordering::Relaxed);
        WAIT_PARTS[SLOWEST].fetch_add(slowest, Ordering::Relaxed);
        WAIT_PARTS[GATHER].fetch_add(gather, Ordering::Relaxed);
    }

    pub fn wait_part(which: usize) -> u64 {
        WAIT_PARTS[which].load(Ordering::Relaxed)
    }

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
        for c in COUNTS.iter().chain(WAIT_PARTS.iter()) {
            c.store(0, Ordering::Relaxed);
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
    /// P6-39 (C-48): threads of the attention's output projection while generating (0 = as many as heads, at most
    /// `threads`). On the workstation the projection took 46.6 % of the time at 2.1 GB/s: it is the only product that
    /// comes right after the qkv + attention split, which uses one part per head (12) while the other threads wait;
    /// sending the small projection [E, E] to the threads that just worked keeps it from waiting for the idle ones.
    /// Every output is still computed by one thread with the same code: the same bits for any value.
    pub proj_threads: usize,
    /// P6-45 (C-59): pieces per part of every split (0 or 1 = one fixed range per part). See `set_pieces`.
    pub pieces: usize,
    /// P6-45 step 4 (C-59): a generated token as ONE block (`run_block`, no bell between computations); tuning file
    /// `planificare = bloc` (absent = `azi`, a split per product as before).
    pub block: bool,
    /// P6-45 step 3 (C-59): threads per team (0 or 1 = none; 4 = 9×4, 6 = 6×6 on 36 threads); `echipe = N`. See `set_team`.
    pub team: usize,
}

impl Tuning {
    /// What the engine uses without a tuning file: every processor, vectors when present, order t-j.
    pub fn default_for(cpus: usize) -> Tuning {
        Tuning { threads: cpus.clamp(1, MAX_THREADS), simd: has_avx2_fma(), order: Order::TJ, proj_threads: 0, pieces: 0, block: false, team: 0 }
    }

    /// Threads of the projection while generating, for a model with `heads` heads (see `proj_threads`).
    pub fn proj_for(&self, heads: usize) -> usize {
        let t = self.threads.clamp(1, MAX_THREADS);
        if self.proj_threads > 0 { self.proj_threads.min(t) } else { heads.clamp(1, t) }
    }

    pub fn describe(&self) -> String {
        alloc::format!(
            "{}, {} {}, ordine {}{}{}{}",
            if self.simd { "vectori AVX2+FMA" } else { "fara vectori (scalar)" },
            self.threads,
            if self.threads == 1 { "fir" } else { "fire" },
            self.order.name(),
            if self.pieces > 1 { alloc::format!(", {} bucati pe fir", self.pieces) } else { String::new() },
            if self.block { ", un bloc pe simbol" } else { "" },
            if self.team > 1 { alloc::format!(", echipe de {}", self.team) } else { String::new() }
        )
    }

    /// The tuning file: `nume = valoare` lines, `#` comments.
    pub fn to_text(&self, processor: &str, note: &str) -> String {
        alloc::format!(
            "# Reglajul motorului KGR1 pentru acest procesor (kronos-calcul --cauta, P6-28).\n\
             # Il citeste `kronos` la pornire. Se poate sterge: atunci se folosesc valorile implicite.\n\
             procesor = {}\nvectori = {}\nfire = {}\nordine = {}\n{}{}{}{}{}",
            processor,
            if self.simd { "avx2" } else { "scalar" },
            self.threads,
            self.order.name(),
            if self.proj_threads > 0 { alloc::format!("fire_proiectie = {}\n", self.proj_threads) } else { String::new() },
            if self.pieces > 1 { alloc::format!("bucati = {}\n", self.pieces) } else { String::new() },
            if self.block { "planificare = bloc\n" } else { "" },
            if self.team > 1 { alloc::format!("echipe = {}\n", self.team) } else { String::new() },
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
                "fire_proiectie" => match v.parse::<usize>() {
                    Ok(n) if n >= 1 => t.proj_threads = n.min(MAX_THREADS),
                    _ => notes.push(alloc::format!("fire_proiectie = `{}` nu e un numar", v)),
                },
                "bucati" => match v.parse::<usize>() {
                    Ok(n) => t.pieces = n.min(64),
                    _ => notes.push(alloc::format!("bucati = `{}` nu e un numar", v)),
                },
                "echipe" => match v.parse::<usize>() {
                    Ok(n) => t.team = n.min(MAX_THREADS),
                    _ => notes.push(alloc::format!("echipe = `{}` nu e un numar", v)),
                },
                "planificare" => match v {
                    "bloc" => t.block = true,
                    "azi" => t.block = false,
                    _ => notes.push(alloc::format!("planificare = `{}` necunoscuta (bloc / azi)", v)),
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

/// One output of a product without bias (for `kronos-calcul --cauta`, P6-45: the products of a block).
pub fn output_one(w: &Mat, x: &[f32], j: usize, tune: Tuning) -> f32 {
    output(w, None, x, j, tune)
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
            tune: Tuning { threads: 1, simd: false, order: Order::TJ, proj_threads: 0, pieces: 0, block: false, team: 0 },
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
        if t == 1 && self.tune.block && self.tune.threads > 1 {
            return Ok(self.forward_block(ids[0], p0));
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
            if t == 1 {
                // P6-39 (C-48): the projection on the threads of the heads split just before (see `proj_threads`).
                let tune = Tuning { threads: self.tune.proj_for(h), ..self.tune };
                let c0 = prof::now();
                linear_act(&l.proj, Some(l.proj_b), &att, t, &mut tmp, tune, false);
                prof::add(prof::PROJ, prof::now() - c0, l.proj.bytes() as u64);
            } else {
                self.linear(prof::PROJ, &l.proj, Some(l.proj_b), &att, t, &mut tmp);
            }
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

    /// P6-45 step 4 (C-59): one generated token as ONE block (see `run_block`): per layer the stages qkv (columns) ->
    /// attention (heads, k and v into the cache) -> projection (tail: residual + ln2) -> fc + GELU -> MLP output (tail:
    /// residual + the next layer's ln1, or the final LayerNorm), then the head. Each value comes from the same function
    /// as in `forward` (`output`, `attend`, `layernorm`, the same element-wise adds): the same bits.
    fn forward_block(&mut self, id: u32, p0: usize) -> Vec<f32> {
        let (e, h) = (self.cfg.emb, self.cfg.heads);
        let d = e / h;
        let parts = self.tune.threads.clamp(1, MAX_THREADS);
        let mut x = vec![0.0f32; e];
        let mut row = vec![0.0f32; e];
        self.tok.row(id as usize, &mut x);
        self.pos.row(p0, &mut row);
        for (a, b) in x.iter_mut().zip(&row) {
            *a += b;
        }
        let mut hbuf = vec![0.0f32; e];
        let c0 = prof::now();
        layernorm(&x, self.layers[0].ln1, &mut hbuf, 1, e, self.cfg.eps);
        prof::add(prof::REST, prof::now() - c0, 0);
        let mut qkv = vec![0.0f32; 3 * e];
        let mut att = vec![0.0f32; e];
        let mut tmp = vec![0.0f32; e];
        let mut big = vec![0.0f32; 4 * e];
        let mut scores = vec![0.0f32; parts * (p0 + 1)];
        let mut logits = vec![0.0f32; self.cfg.vocab];
        let scale = 1.0 / math::sqrt(d as f32);
        let (tune, eps, nl) = (self.tune, self.cfg.eps, self.layers.len());
        let (xo, ho, qo, ao, to, bo, so, lo) = (
            Out(x.as_mut_ptr()),
            Out(hbuf.as_mut_ptr()),
            Out(qkv.as_mut_ptr()),
            Out(att.as_mut_ptr()),
            Out(tmp.as_mut_ptr()),
            Out(big.as_mut_ptr()),
            Out(scores.as_mut_ptr()),
            Out(logits.as_mut_ptr()),
        );
        let caches: Vec<(Out, Out)> =
            self.kcache.iter_mut().zip(self.vcache.iter_mut()).map(|(k, v)| (Out(k.as_mut_ptr()), Out(v.as_mut_ptr()))).collect();
        // SAFETY (for every `sl` / `sl_mut` below): each buffer above lives until `run_block` returns; inside a stage,
        // `run` writes only elements a..b of the stage's output (its pieces are disjoint) and reads buffers written by
        // EARLIER stages only (complete before this stage opened: Acquire on `open`); a tail runs alone, after every
        // piece of its stage and before the next stage opens, so its exclusive slices alias nothing in use.
        let sl = |o: Out, n: usize| unsafe { core::slice::from_raw_parts(o.0 as *const f32, n) };
        #[allow(clippy::mut_from_ref)]
        let sl_mut = |o: Out, n: usize| unsafe { core::slice::from_raw_parts_mut(o.0, n) };
        let layers = &self.layers;
        let (tok, lnf) = (&self.tok, self.lnf);
        // Closures of every stage, per layer (they must outlive the Stage list).
        let qkv_run: Vec<_> = (0..nl)
            .map(|li| {
                move |_: usize, a: usize, b: usize| {
                    let (qo, l) = (qo, &layers[li]);
                    let hb = sl(ho, e);
                    for col in a..b {
                        // SAFETY: column `col` of qkv belongs to this piece only.
                        unsafe { *qo.0.add(col) = output(&l.qkv, Some(l.qkv_b), hb, col, tune) };
                    }
                }
            })
            .collect();
        let attn_run: Vec<_> = (0..nl)
            .map(|li| {
                let (kc, vc) = caches[li];
                move |part: usize, a: usize, b: usize| {
                    let (kc, vc, qo, ao, so) = (kc, vc, qo, ao, so);
                    // SAFETY: the scores row `part` is this thread's (one walk per part at a time); head j's cache columns
                    // at position p0 and its output columns are written only by the piece that has head j.
                    let sc = unsafe { core::slice::from_raw_parts_mut(so.0.add(part * (p0 + 1)), p0 + 1) };
                    for j in a..b {
                        unsafe {
                            core::ptr::copy_nonoverlapping(qo.0.add(e + j * d), kc.0.add(p0 * e + j * d), d);
                            core::ptr::copy_nonoverlapping(qo.0.add(2 * e + j * d), vc.0.add(p0 * e + j * d), d);
                            let q = core::slice::from_raw_parts(qo.0.add(j * d), d);
                            let out = core::slice::from_raw_parts_mut(ao.0.add(j * d), d);
                            attend(q, kc.0, vc.0, e, j * d, p0, scale, sc, out);
                        }
                    }
                }
            })
            .collect();
        let proj_run: Vec<_> = (0..nl)
            .map(|li| {
                move |_: usize, a: usize, b: usize| {
                    let (to, l) = (to, &layers[li]);
                    let at = sl(ao, e);
                    for j in a..b {
                        // SAFETY: output j belongs to this piece only.
                        unsafe { *to.0.add(j) = output(&l.proj, Some(l.proj_b), at, j, tune) };
                    }
                }
            })
            .collect();
        let proj_tail: Vec<_> = (0..nl)
            .map(|li| {
                move || {
                    let (x, t) = (sl_mut(xo, e), sl(to, e));
                    for (a, b) in x.iter_mut().zip(t) {
                        *a += b;
                    }
                    layernorm(x, layers[li].ln2, sl_mut(ho, e), 1, e, eps);
                }
            })
            .collect();
        let fc_run: Vec<_> = (0..nl)
            .map(|li| {
                move |_: usize, a: usize, b: usize| {
                    let (bo, l) = (bo, &layers[li]);
                    let hb = sl(ho, e);
                    for j in a..b {
                        // SAFETY: output j belongs to this piece only.
                        unsafe { *bo.0.add(j) = gelu1(output(&l.fc, Some(l.fc_b), hb, j, tune)) };
                    }
                }
            })
            .collect();
        let out_run: Vec<_> = (0..nl)
            .map(|li| {
                move |_: usize, a: usize, b: usize| {
                    let (to, l) = (to, &layers[li]);
                    let bg = sl(bo, 4 * e);
                    for j in a..b {
                        // SAFETY: output j belongs to this piece only.
                        unsafe { *to.0.add(j) = output(&l.out, Some(l.out_b), bg, j, tune) };
                    }
                }
            })
            .collect();
        let out_tail: Vec<_> = (0..nl)
            .map(|li| {
                move || {
                    let (x, t) = (sl_mut(xo, e), sl(to, e));
                    for (a, b) in x.iter_mut().zip(t) {
                        *a += b;
                    }
                    let norm = if li + 1 < nl { layers[li + 1].ln1 } else { lnf };
                    layernorm(x, norm, sl_mut(ho, e), 1, e, eps);
                }
            })
            .collect();
        let head_run = move |_: usize, a: usize, b: usize| {
            let lo = lo;
            let hb = sl(ho, e);
            for j in a..b {
                // SAFETY: logit j belongs to this piece only.
                unsafe { *lo.0.add(j) = output(tok, None, hb, j, tune) };
            }
        };
        let mut stages: Vec<Stage> = Vec::with_capacity(5 * nl + 1);
        for li in 0..nl {
            stages.push(Stage { n: 3 * e, chunks: block_chunks(3 * e, parts), kind: prof::QKV, run: &qkv_run[li], tail: None });
            stages.push(Stage { n: h, chunks: h, kind: prof::ATTN, run: &attn_run[li], tail: None });
            stages.push(Stage { n: e, chunks: block_chunks(e, parts), kind: prof::PROJ, run: &proj_run[li], tail: Some(&proj_tail[li]) });
            stages.push(Stage { n: 4 * e, chunks: block_chunks(4 * e, parts), kind: prof::FC, run: &fc_run[li], tail: None });
            stages.push(Stage { n: e, chunks: block_chunks(e, parts), kind: prof::OUT, run: &out_run[li], tail: Some(&out_tail[li]) });
        }
        stages.push(Stage { n: self.cfg.vocab, chunks: block_chunks(self.cfg.vocab, parts), kind: prof::HEAD, run: &head_run, tail: None });
        run_block(&stages, parts);
        drop(stages);
        for l in &self.layers {
            prof::add(prof::QKV, 0, l.qkv.bytes() as u64);
            prof::add(prof::PROJ, 0, l.proj.bytes() as u64);
            prof::add(prof::FC, 0, l.fc.bytes() as u64);
            prof::add(prof::OUT, 0, l.out.bytes() as u64);
        }
        prof::add(prof::HEAD, 0, self.tok.bytes() as u64);
        self.len = p0 + 1;
        logits
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
