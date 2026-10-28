//! P6-27 (command C-25): `kronos` — the real Kronos model in the OS, through our KGR1 engine (`src/kgr.rs`).
//!
//!   kronos [optiuni] --simboluri 1,2,3     continues the text given as token ids (greedy), shows tokens per second
//!   kronos --verifica <model.kgr> <ref>    compares the engine with a reference file (logits + greedy continuation)
//! Options: `--model <cale>` (default: the first *.kgr in /usb/kronos), `--n <simboluri>` (default 32), `--fire <n>`
//! (default: every processor), `--fara-vector` (scalar arithmetic even when AVX2 + FMA exist), `--suma <hex>` (the
//! SHA-256 the file must have; otherwise `<model>.sha256` next to it, if present), `--reglaj <cale>` (the tuning file;
//! default `/usb/kronos/reglaj-<procesor>.txt` when it exists, written by `kronos-calcul --cauta`, P6-28). `--fire` and
//! `--fara-vector` win over the tuning file.
//! Text instead of ids comes with the tokenizer (P6-27b: its description is not in the KGR1 specification yet).
//!
//! Exit codes: 0 ok, 1 reference mismatch, 2 bad arguments, 3 wrong SHA-256, 4 cannot read or parse the model.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use cyber_crypto::sha2::Sha256;
#[path = "../kgr.rs"]
#[allow(dead_code)] // the search part (Tuning::to_text, Mat::from_q8) is used by kronos-calcul
mod kgr;

use kgr::{File, Model};
use cyber_user::println;

cyber_user::entry!(main);
cyber_user::heap!(32 * 1024 * 1024);

/// Declared tolerance against the float64 reference: |engine - ref| <= TOL * (1 + |ref|) for every logit.
const TOL: f32 = 1e-4;

fn now_ms() -> u64 {
    cyber_user::mem_info().map(|m| m.uptime_ms).unwrap_or(0)
}

/// The whole file in a fresh memory region (aligned to 2 MiB, so every tensor is aligned too).
fn read_all(path: &str) -> Result<&'static [u8], String> {
    let size = cyber_user::stat(path);
    if size < 0 {
        return Err(alloc::format!("{}: {}", path, cyber_user::error_text(size)));
    }
    let size = size as usize;
    let buf = cyber_user::mem_map(size.max(1)).map_err(|e| alloc::format!("memorie: {}", cyber_user::error_text(e)))?;
    // SAFETY: a fresh region of at least `size` bytes owned by this program, never freed while the program runs.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf, size) };
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(alloc::format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let mut at = 0;
    while at < size {
        let n = cyber_user::read(fd, &mut buf[at..]);
        if n < 0 {
            cyber_user::close(fd);
            return Err(alloc::format!("{}: {}", path, cyber_user::error_text(n)));
        }
        if n == 0 {
            break;
        }
        at += n as usize;
    }
    cyber_user::close(fd);
    if at != size {
        return Err(alloc::format!("{}: am citit {} octeti din {}", path, at, size));
    }
    Ok(buf)
}

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| alloc::format!("{:02x}", b)).collect()
}

/// First *.kgr file in /usb/kronos.
fn default_model() -> Option<String> {
    let mut buf = [0u8; 4096];
    let n = cyber_user::list("/usb/kronos", &mut buf);
    if n <= 0 {
        return None;
    }
    core::str::from_utf8(&buf[..n as usize])
        .ok()?
        .split(['\n', '\0'])
        .find(|l| l.to_ascii_lowercase().ends_with(".kgr"))
        .map(|l| alloc::format!("/usb/kronos/{}", l))
}

struct Opts {
    model: Option<String>,
    reference: Option<String>,
    ids: Vec<u32>,
    n: usize,
    threads: Option<usize>,
    scalar: bool,
    sum: Option<String>,
    tuning: Option<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        model: None,
        reference: None,
        ids: Vec::new(),
        n: 32,
        threads: None,
        scalar: false,
        sum: None,
        tuning: None,
    };
    let mut args = cyber_user::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| alloc::format!("{} cere o valoare", a));
        match a {
            "--model" => o.model = Some(val()?.into()),
            "--verifica" => {
                o.model = Some(val()?.into());
                o.reference = Some(val()?.into());
            }
            "--simboluri" => {
                for s in val()?.split(',').filter(|s| !s.is_empty()) {
                    o.ids.push(s.trim().parse().map_err(|_| alloc::format!("simbol gresit: `{}`", s))?);
                }
            }
            "--n" => o.n = val()?.parse().map_err(|_| String::from("--n cere un numar"))?,
            "--fire" => o.threads = Some(val()?.parse().map_err(|_| String::from("--fire cere un numar"))?),
            "--fara-vector" => o.scalar = true,
            "--suma" => o.sum = Some(val()?.into()),
            "--reglaj" => o.tuning = Some(val()?.into()),
            _ => return Err(alloc::format!(
                "argument necunoscut `{}` (textul direct vine cu tokenizerul; acum: --simboluri 1,2,3)", a
            )),
        }
    }
    Ok(o)
}

/// Reference file "KREF" (tools/kgr-mini.py): checks logits and the greedy continuation of every sequence.
fn verify(m: &mut Model, path: &str) -> Result<bool, String> {
    let b = read_all(path)?;
    let u = |at: usize| -> Result<u32, String> {
        b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| "referinta trunchiata".into())
    };
    if b.get(..4) != Some(b"KREF") || u(4)? != 1 {
        return Err("referinta: nu e un fisier KREF versiunea 1".into());
    }
    let (vocab, nseq) = (u(8)? as usize, u(12)? as usize);
    if vocab != m.cfg.vocab {
        return Err(alloc::format!("referinta are vocabularul {}, modelul {}", vocab, m.cfg.vocab));
    }
    let mut at = 16;
    let (mut worst, mut bad, mut total, mut ok) = (0.0f32, 0usize, 0usize, true);
    for s in 0..nseq {
        let (len, ngen) = (u(at)? as usize, u(at + 4)? as usize);
        at += 8;
        let ids: Vec<u32> = (0..len).map(|i| u(at + 4 * i)).collect::<Result<_, _>>()?;
        at += 4 * len;
        let refl: Vec<f32> = (0..len * vocab).map(|i| u(at + 4 * i).map(f32::from_bits)).collect::<Result<_, _>>()?;
        at += 4 * len * vocab;
        let want: Vec<u32> = (0..ngen).map(|i| u(at + 4 * i)).collect::<Result<_, _>>()?;
        at += 4 * ngen;
        m.reset();
        let got = m.forward(&ids, true)?;
        for (g, r) in got.iter().zip(&refl) {
            let d = (g - r).abs();
            worst = worst.max(d / (1.0 + r.abs()));
            if !(d <= TOL * (1.0 + r.abs())) {
                bad += 1;
            }
        }
        total += refl.len();
        // Greedy continuation with the key/value cache: one new position per step.
        let mut next = kgr::argmax(&got[(len - 1) * vocab..]);
        let mut mine = Vec::new();
        for _ in 0..ngen {
            mine.push(next);
            if mine.len() == ngen {
                break;
            }
            let l = m.forward(&[next], false)?;
            next = kgr::argmax(&l);
        }
        let same = mine == want;
        ok &= same;
        println!(
            "kronos: secventa {}: {} simboluri, continuarea greedy {:?} {}",
            s + 1,
            len,
            mine,
            if same { "= referinta" } else { "DIFERITA de referinta" }
        );
        if !same {
            println!("kronos: referinta: {:?}", want);
        }
    }
    println!(
        "kronos: logit-uri: {} din {} in afara tolerantei {:e}*(1+|ref|), abaterea maxima {:e}*(1+|ref|)",
        bad, total, TOL, worst
    );
    Ok(ok && bad == 0)
}

/// Defaults, then the tuning file (if any), then `--fire` / `--fara-vector`.
fn tuning(o: &Opts) -> kgr::Tuning {
    let mut t = kgr::Tuning::default_for(cyber_user::cpus().max(1) as usize);
    let cpu = kgr::processor_name();
    let path = o.tuning.clone().unwrap_or_else(|| alloc::format!("/usb/kronos/reglaj-{}.txt", cpu));
    match read_all(&path).ok().and_then(|b| core::str::from_utf8(b).ok()) {
        Some(text) => {
            let (got, notes) = kgr::Tuning::parse(text, t, &cpu);
            for n in notes {
                println!("kronos: reglaj: {}", n);
            }
            println!("kronos: reglaj din {}: {}", path, got.describe());
            t = got;
        }
        None if o.tuning.is_some() => println!("kronos: reglaj: nu pot citi {}; valorile implicite", path),
        None => println!("kronos: reglaj: niciun fisier pentru `{}` (kronos-calcul --cauta il scrie); valorile implicite", cpu),
    }
    if let Some(n) = o.threads {
        t.threads = n.clamp(1, kgr::MAX_THREADS);
    }
    if o.scalar {
        t.simd = false;
    }
    t
}

fn main() -> i64 {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            println!("kronos: {}", e);
            return 2;
        }
    };
    let Some(path) = o.model.clone().or_else(default_model) else {
        println!("kronos: niciun model: pune fisierul .kgr pe stick in /usb/kronos/ (sau --model <cale>)");
        return 4;
    };
    let t0 = now_ms();
    let bytes = match read_all(&path) {
        Ok(b) => b,
        Err(e) => {
            println!("kronos: {}", e);
            return 4;
        }
    };
    let t_read = now_ms();
    let sum = hex(&Sha256::digest(bytes));
    let expected = o.sum.clone().or_else(|| {
        read_all(&alloc::format!("{}.sha256", path)).ok().and_then(|b| core::str::from_utf8(b).ok().map(String::from))
    });
    match expected.as_deref().map(|s| s.trim().split_whitespace().next().unwrap_or("").to_ascii_lowercase()) {
        Some(e) if e == sum => println!("kronos: SHA-256 {} = suma asteptata", sum),
        Some(e) => {
            println!("kronos: SHA-256 GRESIT: fisierul are {}, astept {} — fisierul e stricat sau altul", sum, e);
            return 3;
        }
        None => println!("kronos: SHA-256 {} (nicio suma asteptata: --suma sau {}.sha256)", sum, path),
    }
    let file = match File::parse(bytes) {
        Ok(f) => f,
        Err(e) => {
            println!("kronos: {}: {}", path, e);
            return 4;
        }
    };
    let mut m = match Model::new(&file) {
        Ok(m) => m,
        Err(e) => {
            println!("kronos: {}: {}", path, e);
            return 4;
        }
    };
    m.tune = tuning(&o);
    let c = m.cfg;
    println!(
        "kronos: {} ({}, {} MiB, citit in {} ms): {} straturi, {} capete, n_emb {}, context {}, vocabular {}, {} parametri",
        path,
        m.kind,
        bytes.len() >> 20,
        t_read - t0,
        c.layers,
        c.heads,
        c.emb,
        c.context,
        c.vocab,
        m.parameters()
    );
    println!("kronos: sursa `{}`, tokenizer {} octeti; {}", m.source, m.tokenizer.len(), m.tune.describe());
    if let Some(r) = &o.reference {
        return match verify(&mut m, r) {
            Ok(true) => {
                println!("kronos: motorul KGR1 = referinta (OK)");
                0
            }
            Ok(false) => {
                println!("kronos: motorul KGR1 NU se potriveste cu referinta");
                1
            }
            Err(e) => {
                println!("kronos: {}", e);
                1
            }
        };
    }
    if o.ids.is_empty() {
        println!("kronos: dati textul ca simboluri: kronos --simboluri 1,2,3 (textul direct vine cu tokenizerul)");
        return 2;
    }
    let n = o.n.min(c.context.saturating_sub(o.ids.len()));
    let t1 = now_ms();
    let mut logits = match m.forward(&o.ids, false) {
        Ok(l) => l,
        Err(e) => {
            println!("kronos: {}", e);
            return 2;
        }
    };
    let t2 = now_ms();
    let mut out = Vec::new();
    for _ in 0..n {
        let next = kgr::argmax(&logits);
        out.push(next);
        if out.len() == n {
            break;
        }
        logits = match m.forward(&[next], false) {
            Ok(l) => l,
            Err(e) => {
                println!("kronos: {}", e);
                break;
            }
        };
    }
    let t3 = now_ms();
    println!("kronos: continuare: {:?}", out);
    let gen_ms = (t3 - t2).max(1);
    println!(
        "kronos: textul dat ({} simboluri) in {} ms; {} simboluri noi in {} ms = {}.{} simboluri/s",
        o.ids.len(),
        t2 - t1,
        out.len(),
        t3 - t2,
        out.len() as u64 * 1000 / gen_ms,
        out.len() as u64 * 10000 / gen_ms % 10
    );
    0
}
