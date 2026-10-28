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

use crate::kgr::{self, Mat, Order, Tuning};
use alloc::{string::String, vec, vec::Vec};
use cyber_user::println;

/// Declared tolerance against the float64 control: |y - y64| <= TOL * (1 + Σ|w·x|) for every output.
const TOL: f64 = 1e-5;
const ROWS: usize = 16;

fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: RDTSC only reads the time-stamp counter (allowed in ring 3: the kernel leaves CR4.TSD clear).
    unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) };
    (hi as u64) << 32 | lo as u64
}

fn uptime_ms() -> u64 {
    cyber_user::mem_info().map(|m| m.uptime_ms).unwrap_or(0)
}

/// TSC cycles per millisecond, measured over ~200 ms of the kernel clock (from one clock edge to another).
fn calibrate() -> u64 {
    let edge = |after: u64| loop {
        let t = uptime_ms();
        if t > after {
            return t;
        }
        core::hint::spin_loop();
    };
    let a = edge(uptime_ms());
    let c0 = rdtsc();
    let b = edge(a + 199);
    let c1 = rdtsc();
    ((c1 - c0) / (b - a).max(1)).max(1)
}

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

pub fn run(args: &[&str]) -> i64 {
    let (mut emb, mut dir, mut broken) = (512usize, String::from("/usb/kronos"), false);
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
                let t = Tuning { threads: th, simd, order };
                tried += 1;
                let a = time(&m, &x[..inp], 1, &mut y1, t);
                let b = time(&m, &x, ROWS, &mut y16, t);
                let wrong = bad(&y1, &c1) + bad(&y16, &c16);
                if wrong > 0 {
                    rejected += 1;
                    println!("cauta: {}: RESPINSA ({} rezultate in afara tolerantei)", t.describe(), wrong);
                    continue;
                }
                let score = 16 * a + b;
                println!("cauta: {}: 1 rand {} us, {} randuri {} us: corecta", t.describe(), us(a, per_ms), ROWS, us(b, per_ms));
                if score < best_score {
                    best_score = score;
                    best = Some((t, a, b));
                }
            }
        }
    }
    let Some((t, a, b)) = best else {
        println!("cauta: nicio varianta corecta (ar fi o greseala in motor)");
        return 1;
    };
    let base = Tuning { threads: 1, simd: false, order: Order::TJ };
    let (a0, b0) = (time(&m, &x[..inp], 1, &mut y1, base), time(&m, &x, ROWS, &mut y16, base));
    println!(
        "cauta: {} variante incercate, {} respinse; cea mai buna: {} ({}x fata de scalar pe 1 fir)",
        tried,
        rejected,
        t.describe(),
        (16 * a0 + b0) / best_score.max(1)
    );
    let note = alloc::format!(
        "# masurat: 1 rand {} us, {} randuri {} us (scalar pe 1 fir: {} us, {} us), E = {}, TSC {} cicluri/ms\n",
        us(a, per_ms),
        ROWS,
        us(b, per_ms),
        us(a0, per_ms),
        us(b0, per_ms),
        emb,
        per_ms
    );
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
