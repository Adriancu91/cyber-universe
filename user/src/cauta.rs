//! P6-28 (command C-27): `kronos-calcul --cauta` — the first step of "the OS that tunes itself": tries variants of the
//! KGR1 engine's product (`kgr::linear`: vectors or scalar, number of threads, loop order), **checks every variant
//! against a float64 control** (a wrong variant is thrown away, however fast), times the correct ones with the
//! processor's cycle counter (TSC, calibrated against the 10 ms clock) and writes the fastest one for this processor
//! to `<dir>/reglaj-<processor>.txt` (default dir: /usb/kronos), which `kronos` reads at start. The idea is
//! "autotuning" (as in tinygrad's BEAM search or ATLAS); the code is ours. Nothing is written to hardware: the one
//! write is that file.
//!
//! Workload: a Q8_0 matrix [4E, E] (the MLP's first product, the largest of a layer) on 1 row (generating a token) and
//! on 16 rows (reading the text); score = 16 x time(1 row) + time(16 rows).
//! Options: `--emb E` (default 512, a multiple of 32), `--iesire <dir>`, `--test-stricat` (adds the deliberately wrong
//! variant, which must be rejected: used by the QEMU test).
//!
//! P6-29 (C-29): the small problem above is too small for a real model (on the workstation it picked "1 thread", the
//! wrong choice for model #459). When a KGR1 model is on the stick (`--model <cale>`, default: the first *.kgr in the
//! output dir), only its first megabytes are read (the keys: layers, E, vocabulary, F32 or Q8_0) and every correct
//! variant is also timed on **the products of one generated token of that model** (every layer's qkv, projection,
//! fc and MLP output, then the output head), on pseudo-random weights of the same sizes (the real ones would take the
//! whole file). The output head is checked against the float64 control too. With a model, the choice is the fastest
//! variant on the model; the thread counts tried are 1, 2, 4, ..., half the processors and all of them.
//! P6-36 (C-40): the time on the model is the average of 5 passes (it was the fastest of 3), so the cost of waiting for
//! more threads counts in the choice.
//! P6-39 (C-48): the pass on the model is now timed as `kronos` runs it: every layer's qkv on as many threads as heads
//! (the qkv + attention split), then the projection on its own thread count (`Tuning::proj_threads`, default as many
//! as heads), fc and MLP output on all. After the best variant is chosen, the projection's thread count is searched
//! too (1, 2, 4, ..., heads, half, all), each checked against the float64 control, and written as `fire_proiectie`.

use crate::kgr::{self, File, Mat, Order, Tuning};
use alloc::{string::String, vec, vec::Vec};
use cyber_user::println;

/// Declared tolerance against the float64 control: |y - y64| <= TOL * (1 + Σ|w·x|) for every output.
const TOL: f64 = 1e-5;
const ROWS: usize = 16;

use kgr::prof::{calibrate, now as rdtsc};

/// Deterministic pseudo-random numbers in [-1, 1).
fn fill(v: &mut [f32], mut seed: u64) {
    for x in v.iter_mut() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *x = ((seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
    }
}

/// Q8_0 bytes of a [rows, cols] f32 matrix (scale = max|x| / 127 per block of 32, as in the KGR1 specification).
fn quantize(w: &[f32], rows: usize, cols: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * cols / 32 * 36);
    for blk in w.chunks_exact(32) {
        let max = blk.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let s = max / 127.0;
        out.extend_from_slice(&s.to_le_bytes());
        for &v in blk {
            let q = if s == 0.0 { 0.0 } else { v / s };
            let q = if q >= 0.0 { q + 0.5 } else { q - 0.5 } as i32;
            out.push(q.clamp(-127, 127) as i8 as u8);
        }
    }
    out
}

/// Float64 control: y64 and Σ|w·x| per output, from the dequantized weights.
fn control(m: &Mat, x: &[f32], rows: usize) -> (Vec<f64>, Vec<f64>) {
    let mut wrow = vec![0.0f32; m.cols];
    let (mut y, mut mag) = (vec![0.0f64; rows * m.rows], vec![0.0f64; rows * m.rows]);
    for j in 0..m.rows {
        m.row(j, &mut wrow);
        for t in 0..rows {
            let xr = &x[t * m.cols..(t + 1) * m.cols];
            let (mut s, mut a) = (0.0f64, 0.0f64);
            for (w, x) in wrow.iter().zip(xr) {
                let p = *w as f64 * *x as f64;
                s += p;
                a += if p < 0.0 { -p } else { p };
            }
            y[t * m.rows + j] = s;
            mag[t * m.rows + j] = a;
        }
    }
    (y, mag)
}

fn bad(y: &[f32], c: &(Vec<f64>, Vec<f64>)) -> usize {
    y.iter().zip(c.0.iter().zip(&c.1)).filter(|(v, (r, m))| !((**v as f64 - **r).abs() <= TOL * (1.0 + **m))).count()
}

/// Fewest cycles of 3 runs (after one warm-up run).
/// Empty splits timed for the cost of one wait for the threads (P6-43).
const BARRIER_ROUNDS: u64 = 2000;

fn time(m: &Mat, x: &[f32], rows: usize, y: &mut [f32], t: Tuning) -> u64 {
    kgr::linear(m, None, x, rows, y, t);
    (0..3)
        .map(|_| {
            let c0 = rdtsc();
            kgr::linear(m, None, x, rows, y, t);
            rdtsc() - c0
        })
        .min()
        .unwrap()
}

/// "1 fir" / "n fire".
fn fire(n: usize) -> String {
    if n == 1 { "1 fir".into() } else { alloc::format!("{} fire", n) }
}

fn us(cycles: u64, per_ms: u64) -> u64 {
    cycles * 1000 / per_ms
}

/// Writes `text` to `path` (replacing it). Returns 0 or a negative error.
fn write_file(path: &str, text: &str) -> i64 {
    let r = cyber_user::remove(path);
    let r = if r < 0 && r != -3 { r } else { cyber_user::create(path, false) };
    if r < 0 {
        return r;
    }
    let fd = cyber_user::open(path);
    if fd < 0 {
        return fd;
    }
    let n = cyber_user::write(fd as u64, text.as_bytes());
    cyber_user::close(fd);
    cyber_user::sync(path);
    if n < 0 { n } else if n as usize != text.len() { -5 } else { 0 }
}

/// Bytes read from the start of a model file for its keys (the tokenizer text is among them).
const HEAD_BYTES: usize = 4 << 20;

/// The sizes of a model: (layers, E, vocabulary, Q8_0?, heads), from the keys at the start of its file.
fn model_sizes(path: &str) -> Result<(usize, usize, usize, bool, usize), String> {
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(alloc::format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let buf = cyber_user::mem_map(HEAD_BYTES).map_err(|e| alloc::format!("memorie: {}", cyber_user::error_text(e)))?;
    // SAFETY: a fresh region of HEAD_BYTES bytes owned by this program (kept until it exits).
    let buf = unsafe { core::slice::from_raw_parts_mut(buf, HEAD_BYTES) };
    let mut at = 0;
    while at < buf.len() {
        let n = cyber_user::read(fd, &mut buf[at..]);
        if n <= 0 {
            break;
        }
        at += n as usize;
    }
    cyber_user::close(fd);
    let f = File::parse_keys(&buf[..at]).map_err(|e| alloc::format!("{}: {}", path, e))?;
    let (l, e, v) = (f.int("n_straturi")?, f.int("n_emb")?, f.int("vocabular")?);
    let h = f.int("n_capete").unwrap_or(1);
    if e % 32 != 0 || e > 16384 || l > 256 || v > 1 << 20 || h == 0 || h > e {
        return Err(alloc::format!("{}: dimensiuni nerezonabile (E {}, {} straturi, vocabular {})", path, e, l, v));
    }
    Ok((l, e, v, f.text("fel") == Some("q8_0"), h))
}

/// First *.kgr in `dir`.
fn first_model(dir: &str) -> Option<String> {
    let mut buf = [0u8; 4096];
    let n = cyber_user::list(dir, &mut buf);
    if n <= 0 {
        return None;
    }
    core::str::from_utf8(&buf[..n as usize])
        .ok()?
        .split(['\n', '\0'])
        .find(|l| l.to_ascii_lowercase().ends_with(".kgr"))
        .map(|l| alloc::format!("{}/{}", dir, l))
}

/// The products of one generated token of a model (rows, cols): per layer qkv, projection, fc, MLP output; then the
/// head. Pseudo-random weights in one region (Q8_0 blocks with scale 1/127, or f32 in [-1, 1)).
struct Workload {
    mats: Vec<Mat<'static>>,
    bytes: usize,
    heads: usize,
}

fn workload(layers: usize, e: usize, vocab: usize, q8: bool, heads: usize) -> Result<Workload, String> {
    let mut shapes = Vec::new();
    for _ in 0..layers {
        shapes.extend_from_slice(&[(3 * e, e), (e, e), (4 * e, e), (e, 4 * e)]);
    }
    shapes.push((vocab, e));
    let size = |(r, c): (usize, usize)| if q8 { r * (c / 32) * 36 } else { r * c * 4 };
    let bytes: usize = shapes.iter().map(|&s| size(s)).sum();
    let region = cyber_user::mem_map(bytes).map_err(|err| alloc::format!("memorie pentru {} MiB: {}", bytes >> 20, cyber_user::error_text(err)))?;
    // SAFETY: a fresh region of `bytes` bytes, page aligned, owned by this program until it exits; split below into
    // disjoint matrices that are only read.
    let all: &'static mut [u8] = unsafe { core::slice::from_raw_parts_mut(region, bytes) };
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    if q8 {
        let scale = (1.0f32 / 127.0).to_le_bytes();
        for blk in all.chunks_exact_mut(36) {
            blk[..4].copy_from_slice(&scale);
            for q in blk[4..].chunks_exact_mut(8) {
                let r = next().to_le_bytes();
                for (o, b) in q.iter_mut().zip(r) {
                    *o = (b as i8).max(-127) as u8;
                }
            }
        }
    } else {
        for w in all.chunks_exact_mut(4) {
            let v = ((next() >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0;
            w.copy_from_slice(&v.to_le_bytes());
        }
    }
    let mut mats = Vec::new();
    let mut rest: &'static [u8] = all;
    for &(r, c) in &shapes {
        let (m, tail) = rest.split_at(size((r, c)));
        rest = tail;
        mats.push(if q8 {
            Mat::from_q8(m, r, c)
        } else {
            // SAFETY: the region is page aligned and every matrix size is a multiple of 4 bytes, so `m` is f32-aligned.
            Mat::from_f32(unsafe { core::slice::from_raw_parts(m.as_ptr() as *const f32, m.len() / 4) }, r, c)
        });
    }
    Ok(Workload { mats, bytes, heads })
}

/// Passes averaged by `time_model`.
const PASSES: u64 = 5;

/// Cycles of one token's products, **the average of 5 runs** after a warm-up; `y` ends with the head's output.
/// P6-36 (C-40): one run = the products of one generated token one after another, each followed by the wait for every
/// thread, as in `kronos` (4 per layer since qkv and attention share one split, + the head). The average, not the
/// fastest run: a thread that comes late (the cost of synchronizing more threads) must count in the choice.
fn time_model(w: &Workload, x: &[f32], y: &mut [f32], t: Tuning) -> u64 {
    let last = w.mats.len() - 1;
    let pass = |y: &mut [f32]| {
        for (i, m) in w.mats.iter().enumerate() {
            // P6-39: as in `kronos` while generating: qkv split by heads, the projection on its own thread count.
            let threads = match i % 4 {
                _ if i == last => t.threads,
                0 => w.heads.clamp(1, t.threads),
                1 => t.proj_for(w.heads),
                _ => t.threads,
            };
            kgr::linear(m, None, &x[..m.cols], 1, &mut y[..m.rows], Tuning { threads, ..t });
        }
    };
    pass(y);
    let c0 = rdtsc();
    for _ in 0..PASSES {
        pass(y);
    }
    (rdtsc() - c0) / PASSES
}

/// P6-45 step 4 (C-59): as `time_model`, but the token's products as ONE block (`kgr::run_block`: no bell between
/// them, pieces from a shared counter). Each product writes `y[..rows]` from `x` (they run in order, so never at once).
fn time_block(w: &Workload, x: &[f32], y: &mut [f32], t: Tuning) -> u64 {
    #[derive(Clone, Copy)]
    struct Y(*mut f32);
    // SAFETY: each piece writes only its own outputs; the stages run one after another (see `run_block`).
    unsafe impl Sync for Y {}
    unsafe impl Send for Y {}
    let yo = Y(y.as_mut_ptr());
    let parts = t.threads.clamp(1, kgr::MAX_THREADS);
    let runs: Vec<_> = w
        .mats
        .iter()
        .map(|m| {
            move |_: usize, a: usize, b: usize| {
                let yo = yo;
                for j in a..b {
                    // SAFETY: output j of this product belongs to this piece only; y has at least m.rows floats.
                    unsafe { *yo.0.add(j) = kgr::output_one(m, &x[..m.cols], j, t) };
                }
            }
        })
        .collect();
    let stages: Vec<kgr::Stage> = w
        .mats
        .iter()
        .zip(&runs)
        .map(|(m, r)| kgr::Stage { n: m.rows, chunks: kgr::block_chunks(m.rows, parts), kind: kgr::prof::REST, run: r, tail: None })
        .collect();
    kgr::run_block(&stages, parts);
    let c0 = rdtsc();
    for _ in 0..PASSES {
        kgr::run_block(&stages, parts);
    }
    (rdtsc() - c0) / PASSES
}

pub fn run(args: &[&str]) -> i64 {
    let (mut emb, mut dir, mut broken, mut model) = (512usize, String::from("/usb/kronos"), false, None::<String>);
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--emb" if i + 1 < args.len() => {
                emb = args[i + 1].parse().unwrap_or(0);
                i += 1;
            }
            "--iesire" if i + 1 < args.len() => {
                dir = args[i + 1].into();
                i += 1;
            }
            "--test-stricat" => broken = true,
            "--model" if i + 1 < args.len() => {
                model = Some(args[i + 1].into());
                i += 1;
            }
            a => {
                println!("kronos-calcul --cauta: argument necunoscut `{}`", a);
                return 2;
            }
        }
        i += 1;
    }
    if emb < 64 || emb % 32 != 0 || emb > 8192 {
        println!("kronos-calcul --cauta: --emb trebuie sa fie multiplu de 32, intre 64 si 8192");
        return 2;
    }
    let cpus = cyber_user::cpus().max(1) as usize;
    let cpu = kgr::processor_name();
    let per_ms = calibrate();
    println!(
        "cauta: procesor `{}`, {} {}, AVX2+FMA {}; ceas fin: TSC {} cicluri/ms",
        cpu,
        cpus,
        if cpus == 1 { "nucleu" } else { "nuclee" },
        if kgr::has_avx2_fma() { "da" } else { "nu" },
        per_ms
    );
    let (out, inp) = (4 * emb, emb);
    let mut wf = vec![0.0f32; out * inp];
    fill(&mut wf, 27);
    let wq = quantize(&wf, out, inp);
    drop(wf);
    let m = Mat::from_q8(&wq, out, inp);
    let mut x = vec![0.0f32; ROWS * inp];
    fill(&mut x, 28);
    let c1 = control(&m, &x[..inp], 1);
    let c16 = control(&m, &x, ROWS);
    println!("cauta: produsul Q8_0 [{}, {}] pe 1 rand si pe {} randuri; control float64, toleranta {:e}*(1+suma|w*x|)", out, inp, ROWS, TOL);

    let mut threads = vec![1usize];
    let mut n = 2;
    while n < cpus.min(kgr::MAX_THREADS) {
        threads.push(n);
        n *= 2;
    }
    if cpus > 1 {
        threads.push(cpus.min(kgr::MAX_THREADS));
    }
    // Half the processors: with Hyper-Threading, one thread per physical core may beat two (P6-29).
    let half = (cpus / 2).min(kgr::MAX_THREADS);
    if half > 2 && !threads.contains(&half) {
        threads.push(half);
        threads.sort_unstable();
    }
    // The model on the stick, if any (P6-29).
    let model = model.or_else(|| first_model(&dir));
    let mut work = None;
    if let Some(path) = &model {
        match model_sizes(path).and_then(|(l, e, v, q8, h)| workload(l, e, v, q8, h).map(|w| (w, l, e, v, q8))) {
            Ok((w, l, e, v, q8)) => {
                println!(
                    "cauta: pe dimensiunile modelului {}: {} straturi, {} capete, E {}, vocabular {}, {}; {} produse pe simbol, {:.1} MiB de greutati (aleatoare, aceleasi marimi)",
                    path,
                    l,
                    w.heads,
                    e,
                    v,
                    if q8 { "Q8_0" } else { "F32" },
                    w.mats.len(),
                    w.bytes as f64 / 1048576.0
                );
                work = Some(w);
            }
            Err(err) => println!("cauta: modelul nu se poate folosi ({}); doar problema mica", err),
        }
    } else {
        println!("cauta: niciun model .kgr in {} (sau --model): doar problema mica", dir);
    }
    let (mut xm, mut ym, mut head_control) = (Vec::new(), Vec::new(), None);
    if let Some(w) = &work {
        let head = w.mats.last().unwrap();
        xm = vec![0.0f32; 4 * head.cols];
        fill(&mut xm, 29);
        ym = vec![0.0f32; w.mats.iter().map(|m| m.rows).max().unwrap()];
        head_control = Some(control(head, &xm[..head.cols], 1));
    }
    let simds: &[bool] = if kgr::has_avx2_fma() { &[false, true] } else { &[false] };
    let mut orders = vec![Order::TJ, Order::JT];
    if broken {
        orders.push(Order::Broken);
    }
    let (mut best, mut best_score, mut tried, mut rejected) = (None::<(Tuning, u64, u64)>, u64::MAX, 0, 0);
    let mut y1 = vec![0.0f32; out];
    let mut y16 = vec![0.0f32; ROWS * out];
    for &simd in simds {
        for &order in &orders {
            for &th in &threads {
                let t = Tuning { threads: th, simd, order, proj_threads: 0, pieces: 0, block: false, team: 0 };
                tried += 1;
                let a = time(&m, &x[..inp], 1, &mut y1, t);
                let b = time(&m, &x, ROWS, &mut y16, t);
                let wrong = bad(&y1, &c1) + bad(&y16, &c16);
                if wrong > 0 {
                    rejected += 1;
                    println!("cauta: {}: RESPINSA ({} rezultate in afara tolerantei)", t.describe(), wrong);
                    continue;
                }
                let mut score = 16 * a + b;
                let mut on_model = String::new();
                if let (Some(w), Some(hc)) = (&work, &head_control) {
                    let c = time_model(w, &xm, &mut ym, t);
                    let head = w.mats.last().unwrap();
                    let wrong = bad(&ym[..head.rows], hc);
                    if wrong > 0 {
                        rejected += 1;
                        println!("cauta: {}: RESPINSA pe model ({} rezultate ale capului in afara tolerantei)", t.describe(), wrong);
                        continue;
                    }
                    score = c;
                    on_model = alloc::format!(
                        "; model: {} us/simbol ({:.1} GB/s, ~{} simboluri/s doar produsele)",
                        us(c, per_ms),
                        w.bytes as f64 / (c as f64 / per_ms as f64 * 1e6),
                        per_ms * 1000 / c.max(1)
                    );
                }
                println!("cauta: {}: 1 rand {} us, {} randuri {} us: corecta{}", t.describe(), us(a, per_ms), ROWS, us(b, per_ms), on_model);
                if score < best_score {
                    best_score = score;
                    best = Some((t, a, b));
                }
            }
        }
    }
    let Some((mut t, a, b)) = best else {
        println!("cauta: nicio varianta corecta (ar fi o greseala in motor)");
        return 1;
    };
    // P6-39 (C-48): the projection's own thread count, on the best variant (each checked against the control).
    let mut proj_note = String::new();
    if let Some(w) = &work {
        let proj = &w.mats[1];
        let pc = control(proj, &xm[..proj.cols], 1);
        let mut counts: Vec<usize> = threads.iter().copied().filter(|&n| n <= t.threads).collect();
        counts.push(w.heads.min(t.threads));
        counts.push(t.threads);
        counts.sort_unstable();
        counts.dedup();
        let auto = t.proj_for(w.heads);
        let mut results: Vec<(usize, u64)> = Vec::new();
        for &n in &counts {
            let tp = Tuning { proj_threads: n, ..t };
            kgr::linear(proj, None, &xm[..proj.cols], 1, &mut ym[..proj.rows], Tuning { threads: n, ..t });
            let wrong = bad(&ym[..proj.rows], &pc);
            if wrong > 0 {
                rejected += 1;
                println!("cauta: proiectia pe {}: RESPINSA ({} rezultate in afara tolerantei)", fire(n), wrong);
                continue;
            }
            let c = time_model(w, &xm, &mut ym, tp);
            println!("cauta: proiectia pe {}: model {} us/simbol (~{} simboluri/s doar produsele)", fire(n), us(c, per_ms), per_ms * 1000 / c.max(1));
            results.push((n, c));
        }
        // Ties (within 1 %) keep the default (as many threads as heads), which needs no line in the file.
        let (mut best_p, mut best_c) = results.iter().copied().min_by_key(|r| r.1).unwrap_or((0, u64::MAX));
        if let Some(&(n, c)) = results.iter().find(|r| r.0 == auto)
            && c * 100 <= best_c * 101
        {
            (best_p, best_c) = (n, c);
        }
        if best_p > 0 {
            if best_p != auto {
                t.proj_threads = best_p;
            }
            best_score = best_score.min(best_c);
            println!("cauta: proiectia (dupa impartirea pe capete): cea mai buna pe {}", fire(best_p));
            proj_note = alloc::format!("# proiectia: {} fire, {} us/simbol pe model\n", best_p, us(best_c, per_ms));
        }
    }
    // P6-45 (C-59, Adrian's „bucățele”): the model pass with N pieces per thread (a shared counter, whoever is done takes
    // the next piece), each checked against the control; ties (within 1 %) keep fixed ranges (no line in the file).
    let mut pieces_note = String::new();
    if let Some(w) = &work
        && t.threads > 1
    {
        let proj = &w.mats[1];
        let pc = control(proj, &xm[..proj.cols], 1);
        let mut results: Vec<(usize, u64)> = Vec::new();
        for k in [1usize, 2, 4, 8] {
            kgr::set_pieces(k);
            kgr::linear(proj, None, &xm[..proj.cols], 1, &mut ym[..proj.rows], t);
            let wrong = bad(&ym[..proj.rows], &pc);
            if wrong > 0 {
                rejected += 1;
                println!("cauta: {} bucati pe fir: RESPINSA ({} rezultate in afara tolerantei)", k, wrong);
                continue;
            }
            let c = time_model(w, &xm, &mut ym, t);
            println!("cauta: {} bucati pe fir: model {} us/simbol (~{} simboluri/s doar produsele)", k, us(c, per_ms), per_ms * 1000 / c.max(1));
            results.push((k, c));
        }
        kgr::set_pieces(0);
        let (mut best_k, mut best_c) = results.iter().copied().min_by_key(|r| r.1).unwrap_or((1, u64::MAX));
        if let Some(&(k, c)) = results.iter().find(|r| r.0 == 1)
            && c * 100 <= best_c * 101
        {
            (best_k, best_c) = (k, c);
        }
        if best_c != u64::MAX {
            t.pieces = if best_k > 1 { best_k } else { 0 };
            best_score = best_score.min(best_c);
            println!("cauta: bucati: cea mai buna {} pe fir", best_k);
            pieces_note = alloc::format!("# bucati: {} pe fir, {} us/simbol pe model\n", best_k, us(best_c, per_ms));
        }
    }
    // P6-45 step 3 (C-59, Adrian's „echipe”): teams of 4 and of 6 neighbouring threads (9×4 / 6×6 on 36), the wait in two
    // steps, on the chosen variant (its pieces included); each checked against the head's control; ties (within 1 %)
    // keep no teams (no line in the file).
    let mut team_note = String::new();
    if let (Some(w), Some(hc)) = (&work, &head_control)
        && t.threads > 4
    {
        let head = w.mats.last().unwrap();
        kgr::set_pieces(t.pieces);
        let mut results: Vec<(usize, u64)> = Vec::new();
        for k in [4usize, 6] {
            kgr::set_team(k);
            let c = time_model(w, &xm, &mut ym, t);
            let wrong = bad(&ym[..head.rows], hc);
            if wrong > 0 {
                rejected += 1;
                println!("cauta: echipe de {}: RESPINSA ({} rezultate ale capului in afara tolerantei)", k, wrong);
                continue;
            }
            println!("cauta: echipe de {} ({} echipe): model {} us/simbol (~{} simboluri/s doar produsele)", k, t.threads.div_ceil(k), us(c, per_ms), per_ms * 1000 / c.max(1));
            results.push((k, c));
        }
        kgr::set_team(0);
        kgr::set_pieces(0);
        if let Some(&(k, c)) = results.iter().min_by_key(|r| r.1) {
            let wins = c * 101 < best_score * 100;
            println!(
                "cauta: echipe: cele mai bune de {}: {} us/simbol fata de {} us fara echipe: {}",
                k,
                us(c, per_ms),
                us(best_score, per_ms),
                if wins { "alese" } else { "nu sunt mai rapide, fara echipe" }
            );
            team_note = alloc::format!("# echipe: cele mai bune de {}, {} us/simbol pe model\n", k, us(c, per_ms));
            if wins {
                t.team = k;
                best_score = c;
            }
        }
    }
    // P6-45 steps 4-5 (C-59, Adrian's ideas 3 and 5): the token as ONE block (no bell between the products), with 1/2/4/8
    // pieces per thread, on the chosen threads and with one processor left free for the system (interrupts, other
    // threads); each checked against the head's control. It is written to the file only if it beats the variant above
    // by more than 1 %.
    let mut block_note = String::new();
    let all = cpus.min(kgr::MAX_THREADS);
    if let (Some(w), Some(hc)) = (&work, &head_control)
        && all > 1
    {
        let head = w.mats.last().unwrap();
        // On the chosen threads (every processor when the chosen variant uses one), and one fewer when that is all.
        let first = if t.threads > 1 { t.threads } else { all };
        let mut counts = vec![first];
        if first == all && first > 2 {
            counts.push(first - 1);
        }
        let mut found: Option<(usize, usize, u64)> = None;
        for &n in &counts {
            for k in [1usize, 2, 4, 8] {
                kgr::set_pieces(k);
                let tb = Tuning { threads: n, block: true, team: 0, ..t };
                let c = time_block(w, &xm, &mut ym, tb);
                let wrong = bad(&ym[..head.rows], hc);
                if wrong > 0 {
                    rejected += 1;
                    println!("cauta: un bloc pe simbol, {}, {} bucati pe fir: RESPINSA ({} rezultate ale capului in afara tolerantei)", fire(n), k, wrong);
                    continue;
                }
                println!(
                    "cauta: un bloc pe simbol, {}{}, {} bucati pe fir: model {} us/simbol (~{} simboluri/s doar produsele)",
                    fire(n),
                    if n < all { " (un procesor liber)" } else { "" },
                    k,
                    us(c, per_ms),
                    per_ms * 1000 / c.max(1)
                );
                if found.is_none_or(|f| c < f.2) {
                    found = Some((n, k, c));
                }
            }
        }
        kgr::set_pieces(0);
        if let Some((n, k, c)) = found {
            let wins = c * 101 < best_score * 100;
            println!(
                "cauta: un bloc pe simbol: cel mai bun pe {}, {} bucati pe fir: {} us/simbol fata de {} us fara bloc: {}",
                fire(n),
                k,
                us(c, per_ms),
                us(best_score, per_ms),
                if wins { "ales (planificare = bloc)" } else { "nu e mai rapid, raman la impartirea de acum" }
            );
            block_note = alloc::format!("# un bloc pe simbol: cel mai bun {}, {} bucati pe fir, {} us/simbol pe model\n", fire(n), k, us(c, per_ms));
            if wins {
                t.block = true;
                t.team = 0;
                t.threads = n;
                t.pieces = if k > 1 { k } else { 0 };
                best_score = c;
            }
        }
    }
    // P6-43 (C-59): what one wait for the threads costs, with no work (on the chosen threads, and on as many as heads).
    // On every processor too when the chosen variant uses one thread (so the line is always there with several).
    if all > 1 {
        let mut sizes = vec![if t.threads > 1 { t.threads } else { all }];
        if let Some(w) = &work
            && w.heads > 1
            && w.heads < sizes[0]
        {
            sizes.push(w.heads);
        }
        for &n in &sizes {
            kgr::par(n, n, &|_, _| {});
            let c0 = rdtsc();
            for _ in 0..BARRIER_ROUNDS {
                kgr::par(n, n, &|_, _| {});
            }
            let c = (rdtsc() - c0) / BARRIER_ROUNDS;
            let per_token = work.as_ref().map_or(String::new(), |w| {
                alloc::format!(" ({} pe simbol = {} us/simbol)", w.mats.len(), us(c * w.mats.len() as u64, per_ms))
            });
            println!("cauta: bariera goala pe {}: {:.2} us{}", fire(n), c as f64 * 1000.0 / per_ms as f64, per_token);
            proj_note += &alloc::format!("# bariera goala pe {}: {:.2} us\n", fire(n), c as f64 * 1000.0 / per_ms as f64);
        }
    }
    let base = Tuning { threads: 1, simd: false, order: Order::TJ, proj_threads: 0, pieces: 0, block: false, team: 0 };
    let (a0, b0) = (time(&m, &x[..inp], 1, &mut y1, base), time(&m, &x, ROWS, &mut y16, base));
    let base_score = match &work {
        Some(w) => time_model(w, &xm, &mut ym, base),
        None => 16 * a0 + b0,
    };
    println!(
        "cauta: {} variante incercate, {} respinse; cea mai buna{}: {} ({}x fata de scalar pe 1 fir)",
        tried,
        rejected,
        if work.is_some() { " pe model" } else { "" },
        t.describe(),
        base_score / best_score.max(1)
    );
    let mut note = alloc::format!(
        "# masurat: 1 rand {} us, {} randuri {} us (scalar pe 1 fir: {} us, {} us), E = {}, TSC {} cicluri/ms\n",
        us(a, per_ms),
        ROWS,
        us(b, per_ms),
        us(a0, per_ms),
        us(b0, per_ms),
        emb,
        per_ms
    );
    if let (Some(w), Some(path)) = (&work, &model) {
        note += &alloc::format!(
            "# pe model ({}, {:.1} MiB pe simbol): {} us/simbol (scalar pe 1 fir: {} us)\n",
            path,
            w.bytes as f64 / 1048576.0,
            us(best_score, per_ms),
            us(base_score, per_ms)
        );
    }
    note += &proj_note;
    note += &pieces_note;
    note += &team_note;
    note += &block_note;
    kgr::stop_pool();
    let mut probe = [0u8; 16];
    if cyber_user::list(&dir, &mut probe) < 0 && cyber_user::create(&dir, true) < 0 {
        println!("cauta: nu pot crea {} (stick-ul are cyber.rw?)", dir);
        return 3;
    }
    let path = alloc::format!("{}/reglaj-{}.txt", dir, cpu);
    match write_file(&path, &t.to_text(&cpu, &note)) {
        0 => {
            println!("cauta: scris {}", path);
            0
        }
        e => {
            println!("cauta: nu pot scrie {}: {}", path, cyber_user::error_text(e));
            3
        }
    }
}
