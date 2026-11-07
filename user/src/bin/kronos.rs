//! P6-27 (command C-25): `kronos` — the real Kronos model in the OS, through our KGR1 engine (`src/kgr.rs`).
//!
//!   kronos [optiuni] <text>                continues the text (greedy), writes it as it comes, shows tokens per second
//!   kronos [optiuni] --simboluri 1,2,3     the same from token ids
//!   kronos [optiuni] --codeaza <text>      only the tokenizer: the ids of the text and the text decoded back
//!   kronos --verifica <model.kgr> <ref>    compares the engine with a reference file (logits + greedy continuation)
//!   kronos --vectori <vectori.json> <fel>  the official Kronos test vectors (hub `integrare/kronos/vectori/`, P6-27b),
//!                                          `fel` = f32 or q8_0; the model file is next to the JSON
//!   kronos --control                       P6-38 (C-46): the 3 control texts of the model (recognised by its SHA-256)
//!                                          against the tokens Kronos published for it (hub `integrare/kronos/MODEL.md`)
//! Options: `--model <cale>` (default, P6-38: kronos_477_q8_0.kgr in /usb/kronos, else kronos_459_q8_0.kgr (the
//! fallback), else the first *.kgr there), `--dosar <cale>` (look there instead of /usb/kronos), `--n <simboluri>` (default 32), `--fire <n>`
//! (default: every processor), `--fara-vector` (scalar arithmetic even when AVX2 + FMA exist), `--suma <hex>` (the
//! SHA-256 the file must have; otherwise `<model>.sha256` next to it, if present; otherwise, P6-38, the sum of the
//! known model with that file name), `--reglaj <cale>` (the tuning file;
//! default `/usb/kronos/reglaj-<procesor>.txt` when it exists, written by `kronos-calcul --cauta`, P6-28). `--fire` and
//! `--fara-vector` win over the tuning file. `--fire-proiectie <n>` (P6-39): threads of the attention's projection while
//! generating (default: `fire_proiectie` from the tuning file, else as many as the model's heads). `--profil` (P6-29, C-29): after the text, where the time of the generated
//! tokens went (TSC cycles per kind of work: the products per layer kind, the output head, attention, the rest, the
//! wait for threads, writing the text on the screen), with ms per token and GB/s of weights for the products.
//! `--fara-cedari` (P6-47, C-67): waiting threads never yield their processor (refused with more threads than processors).
//! With `--profil` and one block per token, every stage is also shown as seen by all threads (work, threads that worked,
//! how late the last one started, yields).
//! `--fara-ecran` (P6-36): the generated text is written only at the end (otherwise whole lines, in one block at most
//! once a second, P6-39).
//! `--repetitie <p>` (P6-29, optional, off by default): divides (or multiplies, when negative) the logit of every token
//! already in the text by p > 1 before the greedy choice; without it the result is the plain greedy one.
//! The shell splits on spaces, so the words of the text are joined back with one space (surrounding `"` removed).
//! The tokenizer is the one inside the model file (`tokenizer.bpe`, `src/bpe.rs`, P6-27b). Generation stops at a
//! special token (`<|endoftext|>`), at `--n` tokens or at the end of the context.
//!
//! `--intreaba <intrebare>` (P6-61, C-84 item 3): Kronos as an assistant. The prompt is the last exchanges of the history
//! (`--istoric <cale>`, default `<dosar>/conversatie.txt` on the stick) plus `Intrebare: <q>\nRaspuns:`; generation stops
//! at the end of the answer's first line; the exchange is appended to the history. The model only continues text (it
//! was not trained for conversation), and the program says so.
//!
//! `--gpu` (P6-62 step 3, C-83): the greedy continuation on the video card. Started only from the shell, which asks
//! `DA` first (the whole window chain of `gpu lant` runs once, the text at its `i14-simbol` step, every token compared
//! in the kernel with the processor). Any error (no `DA`, a build that cannot write to the card, the chain stopped,
//! a token that does not match, time out) -> the same text on the processor, with a note. `--gpu-sim` = the same on
//! the gpu-sim models (test builds only). Plain greedy only (not with `--repetitie` or `--intreaba`).
//!
//! Exit codes: 0 ok, 1 reference mismatch, 2 bad arguments, 3 wrong SHA-256, 4 cannot read or parse the model.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use cyber_crypto::sha2::Sha256;
#[path = "../unicode.rs"]
mod unicode;
#[path = "../jscan.rs"]
mod jscan;
#[path = "../bpe.rs"]
mod bpe;
#[path = "../kgr.rs"]
#[allow(dead_code)] // the search part (Tuning::to_text, Mat::from_q8) is used by kronos-calcul
mod kgr;

use bpe::Tokenizer;
use jscan::Scan;
use kgr::{File, Model};
use cyber_user::println;

cyber_user::entry!(main);
cyber_user::heap!(32 * 1024 * 1024);

/// Declared tolerance against the float64 reference: |engine - ref| <= TOL * (1 + |ref|) for every logit.
const TOL: f32 = 1e-4;

/// Writes the whole file again (the history is small).
fn save(path: &str, data: &[u8]) -> Result<(), String> {
    let _ = cyber_user::remove(path);
    let r = cyber_user::create(path, false);
    if r < 0 {
        return Err(alloc::format!("{}: {}", path, cyber_user::error_text(r)));
    }
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(alloc::format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let w = cyber_user::write(fd as u64, data);
    cyber_user::close(fd);
    if w != data.len() as i64 { Err(alloc::format!("{}: scriere incompleta", path)) } else { Ok(()) }
}

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

/// One control text of a known model: its tokens and the first greedy tokens Kronos published for it.
struct Control {
    text: &'static str,
    ids: &'static [u32],
    greedy: &'static [u32],
    greedy_text: &'static str,
}

/// A model the OS knows (P6-38, C-46): the file name expected on the stick, its SHA-256 and its control texts.
struct Known {
    name: &'static str,
    file: &'static str,
    sha256: &'static str,
    controls: [Control; 3],
}

const TEXTS: [&str; 3] = ["Ana are mere.", "Capitala României este", "def suma(a, b):"];
/// The tokens of the 3 texts with the w2 tokenizer (the same in every model below: hub `MODEL.md`, `vectori.json`).
const IDS: [&[u32]; 3] = [&[65, 1988, 1585, 2717, 46], &[7132, 1373, 2710, 5734, 1744], &[2082, 15746, 40, 97, 44, 1376, 1754]];

const fn control(i: usize, greedy: &'static [u32], greedy_text: &'static str) -> Control {
    Control { text: TEXTS[i], ids: IDS[i], greedy, greedy_text }
}

/// Hub `integrare/kronos/MODEL.md` (S54, C-44) for #477 and #459; `integrare/kronos/vectori/vectori.json` (q8_0) for
/// the tiny random model of the official vectors (on the test disk, so the control path is tested in QEMU).
const KNOWN: [Known; 3] = [
    Known {
        name: "#477",
        file: "kronos_477_q8_0.kgr",
        sha256: "bcbe8187087dbb3e531a8a7b10991cd24ea14a6e067d07c0fcbb8d3edd753963",
        controls: [
            control(0, &[2233, 1585, 32, 475, 50], " Acum are 1192"),
            control(1, &[8369, 1497, 2207, 117, 44], " orașul Bacău,"),
            control(2, &[1510, 1947, 1358, 1646, 1376], "\n    return a + b"),
        ],
    },
    Known {
        name: "#459 (rezerva)",
        file: "kronos_459_q8_0.kgr",
        sha256: "e80618b4ef555983085c3127acb10114e4f4915d939734207aa849ba0f6d382e",
        controls: [
            control(0, &[2233, 1585, 32, 475, 50], " Acum are 1192"),
            control(1, &[8369, 1497, 4444, 6882, 46], " orașul Baia Mare."),
            control(2, &[1510, 1947, 1358, 1646, 1376], "\n    return a + b"),
        ],
    },
    Known {
        name: "modelul mic al vectorilor oficiali (greutati la intamplare)",
        file: "mic_q8_0.kgr",
        sha256: "b08f4c2976b95d9e1cc3058766b41e13d1ab1a8466b5eb06fa59ff665dd692a0",
        controls: [
            control(0, &[4978, 3604, 4649, 13843, 3604], "aționale bunny ep rule bunny"),
            control(1, &[3604, 3604, 3604, 13843, 3604], " bunny bunny bunny rule bunny"),
            control(2, &[8117, 8117, 3604, 3604, 3604], " atac atac bunny bunny bunny"),
        ],
    },
];

fn known_by_file(path: &str) -> Option<&'static Known> {
    let base = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    KNOWN.iter().find(|k| k.file == base)
}

fn known_by_sum(sum: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.sha256 == sum)
}

/// P6-38 (C-46): the model in `dir` (default /usb/kronos): kronos_477_q8_0.kgr, else kronos_459_q8_0.kgr (the
/// fallback Kronos keeps), else the first *.kgr; with a line saying which and why.
fn default_model(dir: &str) -> Option<(String, String)> {
    let mut buf = [0u8; 4096];
    let n = cyber_user::list(dir, &mut buf);
    if n <= 0 {
        return None;
    }
    let names: Vec<&str> = core::str::from_utf8(&buf[..n as usize])
        .ok()?
        .split(['\n', '\0'])
        .filter(|l| l.to_ascii_lowercase().ends_with(".kgr"))
        .collect();
    let has = |f: &str| names.iter().find(|l| l.to_ascii_lowercase() == f).copied();
    let (name, why) = if let Some(l) = has(KNOWN[0].file) {
        (l, String::from("#477, modelul Kronos pentru OS"))
    } else if let Some(l) = has(KNOWN[1].file) {
        (l, alloc::format!("#459, rezerva: {} lipseste", KNOWN[0].file))
    } else {
        let l = *names.first()?;
        (l, alloc::format!("primul .kgr; nici {}, nici {}", KNOWN[0].file, KNOWN[1].file))
    };
    Some((alloc::format!("{}/{}", dir.trim_end_matches('/'), name), why))
}

/// `--control`: the 3 control texts with the key/value cache, greedy, against the published tokens. Also the smallest
/// gap between the first and the second logit on the greedy steps (Kronos publishes it: below ~0.015 a tiny numeric
/// difference could change a token).
fn run_controls(m: &mut Model, tok: &Tokenizer, k: &Known) -> Result<bool, String> {
    let mut all_ok = true;
    for (n, c) in k.controls.iter().enumerate() {
        let ids = tok.encode(c.text);
        let enc_ok = ids == c.ids;
        m.reset();
        let mut logits = m.forward(c.ids, false)?;
        let (mut got, mut gap) = (Vec::new(), f32::INFINITY);
        for i in 0..c.greedy.len() {
            let next = kgr::argmax(&logits);
            let second = logits.iter().enumerate().filter(|&(j, _)| j as u32 != next).map(|(_, &v)| v).fold(f32::NEG_INFINITY, f32::max);
            gap = gap.min(logits[next as usize] - second);
            got.push(next);
            if i + 1 < c.greedy.len() {
                logits = m.forward(&[next], false)?;
            }
        }
        let text = tok.decode(&got);
        let ok = enc_ok && got == c.greedy && text == c.greedy_text;
        all_ok &= ok;
        println!(
            "kronos: control {} `{}`: simboluri {:?} {}; greedy {:?} {}, textul `{}`; marja minima {:.3}",
            n + 1,
            c.text,
            ids,
            if enc_ok { "= asteptat" } else { "DIFERIT" },
            got,
            if got == c.greedy { "= asteptat" } else { "DIFERIT" },
            text.replace('\n', "\\n"),
            gap
        );
        if !ok {
            println!("kronos: asteptat {:?} {:?} `{}`", c.ids, c.greedy, c.greedy_text.replace('\n', "\\n"));
        }
    }
    Ok(all_ok)
}

struct Opts {
    model: Option<String>,
    /// `--test-fire N` (P6-43): only the thread pool's self-test, with up to N parts.
    pool_test: Option<usize>,
    /// `--dosar` (P6-38): where the default model is looked for.
    dir: Option<String>,
    /// `--control` (P6-38): the control texts of the known model.
    control: bool,
    reference: Option<String>,
    ids: Vec<u32>,
    n: usize,
    threads: Option<usize>,
    /// `--fire-proiectie <n>` (P6-39): threads of the projection while generating (wins over the tuning file).
    proj_threads: Option<usize>,
    /// `--bucati <n>` (P6-45, C-59): pieces per thread in every split (wins over the tuning file; 1 = fixed ranges).
    pieces: Option<usize>,
    /// `--bloc` (P6-45 step 4, C-59): a generated token as one block (wins over the tuning file).
    block: bool,
    /// `--echipe <n>` (P6-45 step 3, C-59): threads per team, the wait in two steps (wins over the tuning file).
    team: Option<usize>,
    scalar: bool,
    sum: Option<String>,
    tuning: Option<String>,
    words: Vec<String>,
    encode_only: bool,
    profile: bool,
    /// `--fara-cedari` (P6-47, C-67): waiting threads never give their processor away (only if threads <= processors).
    no_yield: bool,
    /// `--fara-ecran` (P6-36): the generated text is written only at the end (nothing on the screen while generating).
    no_screen: bool,
    /// Repetition penalty (> 1), None = off.
    penalty: Option<f32>,
    /// (vectori.json, fel)
    vectors: Option<(String, String)>,
    /// `--intreaba`: the words are a question; answer one line, keep the history.
    ask: bool,
    /// `--istoric <cale>`: the conversation file (default `<dosar>/conversatie.txt`).
    history: Option<String>,
    /// `--gpu` / `--gpu-sim` (P6-62 step 3): Some(false) = the board, Some(true) = the simulated board.
    gpu: Option<bool>,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        model: None,
        pool_test: None,
        dir: None,
        control: false,
        reference: None,
        ids: Vec::new(),
        n: 32,
        threads: None,
        proj_threads: None,
        pieces: None,
        block: false,
        team: None,
        scalar: false,
        sum: None,
        tuning: None,
        words: Vec::new(),
        encode_only: false,
        profile: false,
        no_yield: false,
        no_screen: false,
        penalty: None,
        vectors: None,
        ask: false,
        history: None,
        gpu: None,
    };
    let mut args = cyber_user::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| alloc::format!("{} cere o valoare", a));
        match a {
            "--model" => o.model = Some(val()?.into()),
            "--dosar" => o.dir = Some(val()?.into()),
            "--control" => o.control = true,
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
            "--bucati" => o.pieces = Some(val()?.parse().map_err(|_| String::from("--bucati cere un numar"))?),
            "--fire-proiectie" => o.proj_threads = Some(val()?.parse().map_err(|_| String::from("--fire-proiectie cere un numar"))?),
            "--fire" => o.threads = Some(val()?.parse().map_err(|_| String::from("--fire cere un numar"))?),
            "--fara-vector" => o.scalar = true,
            "--bloc" => o.block = true,
            "--echipe" => o.team = Some(val()?.parse().map_err(|_| String::from("--echipe cere un numar"))?),
            "--test-fire" => o.pool_test = Some(val()?.parse().map_err(|_| String::from("--test-fire cere un numar"))?),
            "--suma" => o.sum = Some(val()?.into()),
            "--reglaj" => o.tuning = Some(val()?.into()),
            "--codeaza" => o.encode_only = true,
            "--profil" => o.profile = true,
            "--fara-cedari" => o.no_yield = true,
            "--fara-ecran" => o.no_screen = true,
            "--intreaba" => o.ask = true,
            "--gpu" => o.gpu = Some(false),
            "--gpu-sim" => o.gpu = Some(true),
            "--istoric" => o.history = Some(val()?.into()),
            "--repetitie" => {
                let p: f32 = val()?.parse().map_err(|_| String::from("--repetitie cere un numar (de ex. 1.3)"))?;
                if !(p >= 1.0 && p <= 10.0) {
                    return Err("--repetitie cere un numar intre 1 si 10".into());
                }
                o.penalty = (p > 1.0).then_some(p);
            }
            "--vectori" => {
                let j = val()?;
                o.vectors = Some((j.into(), val()?.into()));
            }
            _ if a.starts_with("--") => return Err(alloc::format!("argument necunoscut `{}`", a)),
            _ => o.words.push(a.into()),
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

/// The words of the text joined with one space, without surrounding quotes.
fn text_of(words: &[String]) -> String {
    let t = words.join(" ");
    match t.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        Some(inner) => inner.into(),
        None => t,
    }
}

/// Declared tolerance of the official vectors (hub `integrare/kronos/README.md`): absolute, on the logits of the
/// last position; F32 1e-4, Q8_0 1e-3 (against the Q8_0 vectors).
fn vectors_tolerance(kind: &str) -> f32 {
    if kind == "q8_0" { 1e-3 } else { 1e-4 }
}

/// One text of the official vectors.
#[derive(Default)]
struct Entry {
    text: String,
    ids: Vec<u32>,
    argmax: Vec<u32>,
    greedy: Vec<u32>,
    greedy_text: String,
    logits: Vec<f32>,
}

fn id_list(sc: &mut Scan) -> jscan::R<Vec<u32>> {
    let mut v = Vec::new();
    sc.array(|sc| {
        let n = sc.int()?;
        v.push(u32::try_from(n).map_err(|_| String::from("simbol gresit"))?);
        Ok(())
    })?;
    Ok(v)
}

/// Reads vectori.json (only the texts of `kind`); returns them with the model path (the file named in
/// `fisiere.<fel>.nume`, next to the JSON) and the SHA-256 it must have.
fn load_vectors(json_path: &str, kind: &str) -> Result<(Vec<Entry>, String, String), String> {
    let b = read_all(json_path)?;
    let text = core::str::from_utf8(b).map_err(|_| alloc::format!("{}: nu e UTF-8", json_path))?;
    let (mut name, mut sum, mut entries) = (None, None, None);
    let mut sc = Scan::new(text);
    sc.object(|sc, key| match key {
        "fisiere" => sc.object(|sc, k| {
            if k != kind {
                return sc.skip();
            }
            sc.object(|sc, f| match f {
                "nume" => sc.string().map(|v| name = Some(v)),
                "sha256" => sc.string().map(|v| sum = Some(v)),
                _ => sc.skip(),
            })
        }),
        k if k == kind => {
            let mut list = Vec::new();
            sc.array(|sc| {
                let mut e = Entry::default();
                sc.object(|sc, f| {
                    match f {
                        "text" => e.text = sc.string()?,
                        "simboluri" => e.ids = id_list(sc)?,
                        "argmax_pe_pozitii" => e.argmax = id_list(sc)?,
                        "greedy_5" => e.greedy = id_list(sc)?,
                        "greedy_5_text" => e.greedy_text = sc.string()?,
                        "logits_ultima_pozitie" => {
                            e.logits.reserve(16384);
                            sc.array(|sc| sc.float().map(|x| e.logits.push(x)))?
                        }
                        _ => sc.skip()?,
                    }
                    Ok(())
                })?;
                list.push(e);
                Ok(())
            })?;
            entries = Some(list);
            Ok(())
        }
        _ => sc.skip(),
    })
    .and_then(|_| sc.end())
    .map_err(|e| alloc::format!("{}: {}", json_path, e))?;
    let name = name.ok_or_else(|| alloc::format!("vectorii nu au fisierul `{}`", kind))?;
    let sum = sum.ok_or("vectorii: lipseste `sha256`")?;
    let entries = entries.ok_or_else(|| alloc::format!("vectorii nu au textele `{}`", kind))?;
    let dir = json_path.rfind('/').map(|i| &json_path[..i]).unwrap_or(".");
    let base = name.rsplit('/').next().unwrap_or(&name);
    Ok((entries, alloc::format!("{}/{}", dir, base), sum))
}

/// The official vectors of one kind: per text, the tokenizer both ways, argmax on every position, the logits of the
/// last position within the tolerance, the greedy tokens with and without the key/value cache, and their text.
fn verify_vectors(m: &mut Model, tok: &Tokenizer, list: &[Entry], kind: &str) -> Result<bool, String> {
    let tol = vectors_tolerance(kind);
    let vocab = m.cfg.vocab;
    let mut all_ok = true;
    for (n, e) in list.iter().enumerate() {
        let text = e.text.as_str();
        if e.logits.len() != vocab || e.ids.is_empty() {
            return Err(alloc::format!("vectorii au {} logit-uri, modelul are vocabularul {}", e.logits.len(), vocab));
        }
        let ids = tok.encode(text);
        let enc_ok = ids == e.ids;
        let dec_ok = tok.decode(&e.ids) == text;
        // Every position at once, without the cache.
        m.reset();
        let l = m.forward(&e.ids, true)?;
        let t = e.ids.len();
        let argmax: Vec<u32> = (0..t).map(|i| kgr::argmax(&l[i * vocab..(i + 1) * vocab])).collect();
        let arg_ok = argmax == e.argmax;
        let last = &l[(t - 1) * vocab..];
        let worst = last.iter().zip(&e.logits).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        let log_ok = worst <= tol;
        // Greedy with the cache: one new position per step.
        let mut cached = Vec::new();
        let mut next = kgr::argmax(last);
        for k in 0..e.greedy.len() {
            cached.push(next);
            if k + 1 < e.greedy.len() {
                next = kgr::argmax(&m.forward(&[next], false)?);
            }
        }
        // Greedy without the cache: the whole sequence again at every step.
        let mut seq = e.ids.clone();
        for _ in 0..e.greedy.len() {
            m.reset();
            let l = m.forward(&seq, false)?;
            seq.push(kgr::argmax(&l));
        }
        let uncached = &seq[t..];
        let greedy_ok = cached == e.greedy && uncached == e.greedy.as_slice();
        let gen_text = tok.decode(&cached);
        let text_ok = gen_text == e.greedy_text;
        let ok = enc_ok && dec_ok && arg_ok && log_ok && greedy_ok && text_ok;
        all_ok &= ok;
        let yes = |b: bool| if b { "= referinta" } else { "DIFERIT" };
        println!(
            "kronos: vectori {} text {} `{}`: simboluri {:?} {}, decodare {}; argmax pe {} pozitii {}; logit-uri abatere maxima {:e} (toleranta {:e}) {}",
            kind,
            n + 1,
            text,
            ids,
            yes(enc_ok),
            yes(dec_ok),
            t,
            yes(arg_ok),
            worst,
            tol,
            if log_ok { "OK" } else { "PESTE TOLERANTA" }
        );
        println!(
            "kronos: vectori {} text {}: greedy {:?} {} (cu si fara cache), textul `{}` {}",
            kind,
            n + 1,
            cached,
            yes(greedy_ok),
            gen_text.replace('\n', "\\n"),
            yes(text_ok)
        );
        if !enc_ok {
            println!("kronos: referinta simboluri: {:?}", e.ids);
        }
        if !arg_ok {
            println!("kronos: argmax {:?}, referinta {:?}", argmax, e.argmax);
        }
        if !greedy_ok {
            println!("kronos: greedy cu cache {:?}, fara cache {:?}, referinta {:?}", cached, uncached, e.greedy);
        }
        if !text_ok {
            println!("kronos: referinta text `{}`", e.greedy_text.replace('\n', "\\n"));
        }
    }
    Ok(all_ok)
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
    if let Some(n) = o.proj_threads {
        t.proj_threads = n.clamp(1, kgr::MAX_THREADS);
    }
    if let Some(n) = o.pieces {
        t.pieces = n.min(64);
    }
    kgr::set_pieces(t.pieces);
    if o.block {
        t.block = true;
    }
    if let Some(n) = o.team {
        t.team = n.min(kgr::MAX_THREADS);
    }
    kgr::set_team(t.team);
    if let Some(n) = o.threads {
        t.threads = n.clamp(1, kgr::MAX_THREADS);
    }
    if o.scalar {
        t.simd = false;
    }
    // P6-47 (C-67): only when every thread has its processor (otherwise a spinning thread could keep the one it waits
    // for from running until the timer takes the processor away).
    let cpus = cyber_user::cpus().max(1) as usize;
    if o.no_yield && t.threads > cpus {
        println!("kronos: --fara-cedari refuzat: {} fire pe {} procesoare (firele care asteapta trebuie sa cedeze)", t.threads, cpus);
    }
    kgr::set_no_yield(o.no_yield && t.threads <= cpus);
    t
}

/// Repetition penalty (as in the CTRL paper's rule): for every distinct token already in the text, a positive logit is
/// divided by `p` and a negative one multiplied by it.
fn penalize(logits: &mut [f32], seen: &[u32], p: f32) {
    let mut done = Vec::with_capacity(seen.len());
    for &id in seen {
        if done.contains(&id) {
            continue;
        }
        done.push(id);
        if let Some(l) = logits.get_mut(id as usize) {
            *l = if *l > 0.0 { *l / p } else { *l * p };
        }
    }
}

/// P6-47: short names of the block's stages in `--profil` (index = `prof` kind).
const PROFILE_SHORT: [&str; kgr::prof::N] = ["qkv", "proiectie", "fc 4E", "iesire MLP", "cap", "atentie", "restul", "asteptare"];

/// P6-62 step 3: `--gpu`. `Some(code)` = done on the board; `None` = continue on the processor (the note says why).
fn on_gpu(o: &Opts, path: &str, n: usize, sim: bool, tok: Option<&Tokenizer>) -> Option<i64> {
    let back = |why: &str| {
        println!("kronos: --gpu: {} -> continui pe procesor", why);
        None
    };
    if o.penalty.is_some() || o.ask {
        return back("pe placa doar alegerea greedy simpla (fara --repetitie si --intreaba)");
    }
    if n == 0 {
        return back("nimic de generat");
    }
    let ids: Vec<String> = o.ids.iter().map(|x| alloc::format!("{}", x)).collect();
    let req = alloc::format!("{}\n{}\n{}\n{}\n", if sim { "sim" } else { "placa" }, path, n, ids.join(","));
    println!("kronos: --gpu{}: lantul ferestrei porneste, apoi {} simboluri pe placa (pot trece cateva minute)", if sim { " (SIMULARE)" } else { "" }, n);
    let mut buf = alloc::vec![0u8; 64 * 1024];
    let t0 = now_ms();
    let got = cyber_user::gpu_kronos(&req, &mut buf);
    let ms = now_ms() - t0;
    if got < 0 {
        return back(&alloc::format!("apelul a fost refuzat (cod {})", got));
    }
    let text = String::from_utf8_lossy(&buf[..got as usize]).into_owned();
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    // The chain's last lines (its result) say where it stopped.
    let rest: Vec<&str> = lines.collect();
    for l in rest.iter().filter(|l| l.contains("kronos --gpu") || l.contains("rezultat lant")) {
        println!("kronos: lant: {}", l);
    }
    let Some(list) = first.strip_prefix("ok ") else {
        return back(first.strip_prefix("eroare ").unwrap_or(first));
    };
    let parsed: Option<Vec<u32>> = list.split(',').map(|t| t.trim().parse::<u32>().ok()).collect();
    let Some(mut out) = parsed.filter(|v| v.len() == n) else {
        return back(&alloc::format!("raspuns de neinteles de la nucleu: {}", first));
    };
    let mut ended = false;
    if let Some(t) = tok {
        if let Some(cut) = out.iter().position(|&x| t.is_special(x)) {
            out.truncate(cut);
            ended = true;
        }
        println!("kronos: continuare: {}", t.decode(&out));
        println!(
            "kronos: textul generat `{}`{}",
            t.decode(&out).replace('\n', "\\n"),
            if ended { " (oprit la simbolul de sfarsit)" } else { "" }
        );
        println!("kronos: simboluri generate: {:?}", out);
    } else {
        println!("kronos: continuare: {:?}", out);
    }
    println!(
        "kronos: PE PLACA{}: {} simboluri, fiecare comparat in nucleu cu procesorul (lantul intreg + textul in {} ms)",
        if sim { " (SIMULARE)" } else { "" },
        out.len(),
        ms
    );
    Some(0)
}

/// `--profil`: where the cycles of the generated tokens went.
/// P6-39 (C-48): the generated text goes to the screen at most this often (whole lines, one block).
const FLUSH_MS: u64 = 1000;

fn profile_report(m: &Model, passes: usize, total: u64, screen: u64, pick: u64, per_ms: u64) {
    use kgr::prof;
    let ms = |c: u64| c as f64 / per_ms.max(1) as f64;
    let pct = |c: u64| c as f64 * 100.0 / total.max(1) as f64;
    let per = passes.max(1) as f64;
    println!(
        "kronos: profil: {} treceri prin model (un simbol nou fiecare), {}, TSC {} cicluri/ms; total {:.1} ms = {:.2} ms/simbol",
        passes,
        m.tune.describe(),
        per_ms,
        ms(total),
        ms(total) / per
    );
    let mut known = 0u64;
    for k in 0..prof::N {
        let (c, b) = prof::get(k);
        if k != prof::SYNC {
            known += c;
        }
        let rate = if b > 0 && c > 0 {
            alloc::format!(", {:.2} GB/s ({:.2} MiB/simbol)", b as f64 / (ms(c) * 1e6), b as f64 / per / 1048576.0)
        } else {
            String::new()
        };
        println!("kronos: profil: {:<34} {:>9.1} ms {:>7.3} ms/simbol {:>5.1} %{}", prof::NAMES[k], ms(c), ms(c) / per, pct(c), rate);
    }
    // P6-43 (C-59): how many waits for the threads, and what one costs on average.
    let (barriers, yields) = (prof::counted(prof::BARRIERS), prof::counted(prof::YIELDS));
    println!(
        "kronos: profil: bariere (asteptari ale firelor): {} = {:.1} pe simbol, {:.1} us in medie pe bariera; cedari ale procesorului: {}",
        barriers,
        barriers as f64 / per,
        ms(prof::get(prof::SYNC).0) * 1000.0 / barriers.max(1) as f64,
        yields
    );
    // P6-45 (C-59): where the wait goes, per barrier on average: waking the workers, waiting for the slowest part,
    // gathering (the last part done until the main thread sees it). Several can overlap the main thread's own part.
    let us = |c: u64| ms(c) * 1000.0 / barriers.max(1) as f64;
    println!(
        "kronos: profil: bariera in medie: trezire {:.1} us, asteptat cel mai lent {:.1} us, adunare {:.1} us{}",
        us(prof::wait_part(prof::WAKE)),
        us(prof::wait_part(prof::SLOWEST)),
        us(prof::wait_part(prof::GATHER)),
        if m.tune.block { " (un bloc pe simbol: barierele = asteptarile intre etape, fara despartire)" } else { "" }
    );
    // P6-47 (C-67): how the waiting threads behave, and (one block per token) every stage seen from ALL threads.
    let cpus = cyber_user::cpus();
    println!(
        "kronos: profil: {} procesoare, {} {}; firele care asteapta: {}",
        cpus,
        m.tune.threads,
        if m.tune.threads == 1 { "fir" } else { "fire" },
        match kgr::spin_policy() {
            None => String::from("nu cedeaza niciodata procesorul (--fara-cedari)"),
            Some(n) => alloc::format!("cedeaza procesorul dupa {} pauze", n),
        }
    );
    if m.tune.block {
        for k in [prof::QKV, prof::ATTN, prof::PROJ, prof::FC, prof::OUT, prof::HEAD] {
            let [work, pieces, yields, lag, helpers, stages] = prof::block(k);
            if stages == 0 {
                continue;
            }
            let (wall, bytes) = prof::get(k);
            let st = stages as f64;
            let us_st = |c: u64| ms(c) * 1000.0 / st;
            let rate = if bytes > 0 && work > 0 {
                alloc::format!("; cu toate firele ocupate: {:.2} GB/s", bytes as f64 * m.tune.threads as f64 / (ms(work) * 1e6))
            } else {
                String::new()
            };
            println!(
                "kronos: profil: etapa {:<11} {:>7.1} us/etapa (firul 0); lucrul / {} fire = {:>6.1} us{}; au lucrat {:.1} fire, {:.1} bucati; ultimul fir a inceput la {:.1} us; cedari {:.1}/etapa",
                PROFILE_SHORT[k],
                us_st(wall),
                m.tune.threads,
                us_st(work) / m.tune.threads.max(1) as f64,
                rate,
                helpers as f64 / st,
                pieces as f64 / st,
                us_st(lag),
                yields as f64 / st
            );
        }
    }
    for (name, c) in [("afisarea textului pe ecran", screen), ("alegerea simbolului (argmax)", pick)] {
        known += c;
        println!("kronos: profil: {:<34} {:>9.1} ms {:>7.3} ms/simbol {:>5.1} %", name, ms(c), ms(c) / per, pct(c));
    }
    let other = total.saturating_sub(known);
    println!("kronos: profil: {:<34} {:>9.1} ms {:>7.3} ms/simbol {:>5.1} %", "nemasurat (alocari, intreruperi)", ms(other), ms(other) / per, pct(other));
    let (mut wc, mut wb) = (0u64, 0u64);
    for k in [prof::QKV, prof::PROJ, prof::FC, prof::OUT, prof::HEAD] {
        let (c, b) = prof::get(k);
        wc += c;
        wb += b;
    }
    if wc > 0 {
        println!(
            "kronos: profil: toate produsele: {:.2} GB/s de greutati; fara ecran ar fi {:.1} simboluri/s",
            wb as f64 / (ms(wc) * 1e6),
            per * 1000.0 / ms(total.saturating_sub(screen)).max(1e-3)
        );
    }
}

/// The history's exchanges, oldest first: `(question, answer)` from `Intrebare: ...` / `Raspuns: ...` lines.
fn history_turns(text: &str) -> Vec<(String, String)> {
    let mut turns = Vec::new();
    let mut q: Option<String> = None;
    for line in text.lines() {
        if let Some(x) = line.strip_prefix(ASK_Q) {
            q = Some(x.trim().into());
        } else if let (Some(x), Some(qq)) = (line.strip_prefix(ASK_A), q.take()) {
            turns.push((qq, String::from(x.trim())));
        }
    }
    turns
}

/// The prompt: as many of the latest exchanges as fit in about 3/4 of the context, then the question.
fn ask_prompt(tok: &Tokenizer, turns: &[(String, String)], question: &str, context: usize) -> (String, usize) {
    let tail = alloc::format!("{}{}\n{}", ASK_Q, question, ASK_A.trim_end());
    let mut first = 0;
    loop {
        let mut p = String::new();
        for (q, a) in &turns[first..] {
            p += &alloc::format!("{}{}\n{}{}\n\n", ASK_Q, q, ASK_A, a);
        }
        p += &tail;
        if first == turns.len() || tok.encode(&p).len() <= context * 3 / 4 {
            return (p, turns.len() - first);
        }
        first += 1;
    }
}

const ASK_Q: &str = "Intrebare: ";
const ASK_A: &str = "Raspuns: ";

fn main() -> i64 {
    let mut o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            println!("kronos: {}", e);
            return 2;
        }
    };
    if let Some(n) = o.pool_test {
        kgr::set_pieces(o.pieces.unwrap_or(0));
        kgr::set_team(o.team.unwrap_or(0));
        if o.block {
            // P6-45 step 4: the block (stages that each need the whole stage before, tails between them).
            let r = kgr::self_test_block(n);
            kgr::stop_pool();
            return match r {
                Ok(runs) => {
                    println!("kronos: test bloc: {} blocuri pe 1..{} fire, 12 etape fiecare, fiecare element = calculul pe un fir (OK)", runs, n);
                    0
                }
                Err(e) => {
                    println!("kronos: test bloc: GRESIT: {}", e);
                    1
                }
            };
        }
        let r = kgr::self_test_pool(n);
        kgr::stop_pool();
        return match r {
            Ok(jobs) => {
                let mut mode = match o.pieces {
                    Some(k) if k > 1 => alloc::format!(", {} bucati pe fir", k),
                    _ => String::new(),
                };
                if let Some(k) = o.team.filter(|&k| k > 1) {
                    mode += &alloc::format!(", echipe de {}", k);
                }
                println!("kronos: test fire: {} lucrari cu 1..{} parti{}, fiecare element scris o data, firele fara parte nu sunt asteptate (OK)", jobs, n, mode);
                0
            }
            Err(e) => {
                println!("kronos: test fire: GRESIT: {}", e);
                1
            }
        };
    }
    let vectors = match &o.vectors {
        Some((json, kind)) => match load_vectors(json, kind) {
            Ok((j, model, sum)) => {
                o.model = Some(model);
                o.sum = Some(sum);
                Some((j, kind.clone()))
            }
            Err(e) => {
                println!("kronos: {}", e);
                return 4;
            }
        },
        None => None,
    };
    let dir = o.dir.clone().unwrap_or_else(|| String::from("/usb/kronos"));
    let path = match o.model.clone() {
        Some(p) => p,
        None => match default_model(&dir) {
            Some((p, why)) => {
                println!("kronos: model ales: {} ({})", p, why);
                p
            }
            None => {
                println!("kronos: niciun model: pune {} (si {}.sha256) pe stick in {}/ (sau --model <cale>)", KNOWN[0].file, KNOWN[0].file, dir);
                return 4;
            }
        },
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
    let expected = o
        .sum
        .clone()
        .or_else(|| read_all(&alloc::format!("{}.sha256", path)).ok().and_then(|b| core::str::from_utf8(b).ok().map(String::from)))
        .or_else(|| {
            known_by_file(&path).map(|k| {
                println!("kronos: fara {}.sha256: folosesc suma cunoscuta a lui {}", path, k.name);
                String::from(k.sha256)
            })
        });
    match expected.as_deref().map(|s| s.trim().split_whitespace().next().unwrap_or("").to_ascii_lowercase()) {
        Some(e) if e == sum => println!("kronos: SHA-256 {} = suma asteptata", sum),
        Some(e) => {
            println!("kronos: SHA-256 GRESIT: fisierul are {}, astept {} — fisierul e stricat sau altul", sum, e);
            return 3;
        }
        None => println!("kronos: SHA-256 {} (nicio suma asteptata: --suma sau {}.sha256)", sum, path),
    }
    let known = known_by_sum(&sum);
    match known {
        Some(k) => println!("kronos: modelul cunoscut {} ({})", k.name, k.file),
        None => println!("kronos: model necunoscut OS-ului (nu e #477 sau #459)"),
    }
    if o.control && known.is_none() {
        println!("kronos: --control: nu am textele de control pentru acest model (SHA-256 {}); CONTROL: NU TRECE", sum);
        return 1;
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
    println!(
        "kronos: proiectia la generare pe {} ({})",
        match m.tune.proj_for(c.heads) {
            1 => String::from("1 fir"),
            n => alloc::format!("{} fire", n),
        },
        if m.tune.proj_threads > 0 { "din reglaj sau --fire-proiectie" } else { "cate capete are modelul" }
    );
    let tok = if vectors.is_some() || !o.words.is_empty() || o.control {
        match Tokenizer::from_json(m.tokenizer, c.vocab) {
            Ok(t) => {
                println!(
                    "kronos: tokenizer {:?}: {} imbinari, {} simboluri (Unicode {})",
                    t.rules,
                    t.merges,
                    t.vocab(),
                    unicode::UNICODE_VERSION
                );
                Some(t)
            }
            Err(e) => {
                println!("kronos: {}: {}", path, e);
                return 4;
            }
        }
    } else {
        None
    };
    if let (Some((j, kind)), Some(tok)) = (&vectors, &tok) {
        return match verify_vectors(&mut m, tok, j, kind) {
            Ok(true) => {
                println!("kronos: vectorii oficiali Kronos ({}): tokenizer, logit-uri, argmax si greedy = referinta (OK)", kind);
                0
            }
            Ok(false) => {
                println!("kronos: vectorii oficiali Kronos ({}): NU se potrivesc", kind);
                1
            }
            Err(e) => {
                println!("kronos: {}", e);
                1
            }
        };
    }
    if o.control {
        let (Some(k), Some(tok)) = (known, &tok) else {
            return 1;
        };
        let ok = run_controls(&mut m, tok, k);
        kgr::stop_pool();
        return match ok {
            Ok(true) => {
                println!("kronos: CONTROL: TRECE ({}: 3 texte, simbolurile si textul = cele publicate de Kronos)", k.name);
                0
            }
            Ok(false) => {
                println!("kronos: CONTROL: NU TRECE ({}): motorul nu da simbolurile publicate de Kronos", k.name);
                1
            }
            Err(e) => {
                println!("kronos: {}", e);
                1
            }
        };
    }
    let mut ask_turns = 0;
    if let (true, Some(tok)) = (o.ask, &tok) {
        let question = text_of(&o.words).replace('\n', " ");
        let hist = o.history.clone().unwrap_or_else(|| alloc::format!("{}/conversatie.txt", dir));
        let old = read_all(&hist).ok().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        let turns = history_turns(&old);
        let (prompt, used) = ask_prompt(tok, &turns, &question, c.context);
        ask_turns = used;
        println!("kronos: intrebare cu {} schimburi din istoric ({}, {} in total)", used, hist, turns.len());
        println!("kronos: (modelul continua text; nu e antrenat pentru conversatie, raspunsul poate fi fara legatura)");
        o.words = alloc::vec![prompt];
        o.history = Some(hist);
    }
    if let Some(tok) = &tok {
        let text = text_of(&o.words);
        o.ids = tok.encode(&text);
        let back = tok.decode(&o.ids);
        if o.ask {
            println!("kronos: intrebarea cu istoricul = {} simboluri (context {})", o.ids.len(), c.context);
        } else {
        println!(
            "kronos: textul `{}` = {} simboluri {:?}; decodat inapoi {}",
            text,
            o.ids.len(),
            o.ids,
            if back == text { "identic" } else { "DIFERIT" }
        );
        }
        if o.encode_only {
            return if back == text { 0 } else { 1 };
        }
    }
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
        println!("kronos: dati un text (kronos Ana are) sau simboluri (kronos --simboluri 1,2,3)");
        return 2;
    }
    let n = o.n.min(c.context.saturating_sub(o.ids.len()));
    if let Some(sim) = o.gpu {
        if let Some(code) = on_gpu(&o, &path, n, sim, tok.as_ref()) {
            kgr::stop_pool();
            return code;
        }
    }
    let per_ms = if o.profile { kgr::prof::calibrate() } else { 0 };
    kgr::prof::set_detail(o.profile);
    let t1 = now_ms();
    let mut logits = match m.forward(&o.ids, false) {
        Ok(l) => l,
        Err(e) => {
            println!("kronos: {}", e);
            return 2;
        }
    };
    let t2 = now_ms();
    // The profile counts only the generated tokens (the given text runs as one batch, timed above).
    kgr::prof::reset();
    let (mut c_screen, mut c_pick, c_start) = (0u64, 0u64, kgr::prof::now());
    let mut out = Vec::new();
    let mut stream = bpe::TextStream::default();
    let mut pending = String::new();
    let mut ended = false;
    let mut last_flush = now_ms();
    if tok.is_some() {
        cyber_user::print!("kronos: continuare: ");
    }
    let mut seen = o.ids.clone();
    for _ in 0..n {
        let c0 = kgr::prof::now();
        if let Some(p) = o.penalty {
            penalize(&mut logits, &seen, p);
        }
        let next = kgr::argmax(&logits);
        let c1 = kgr::prof::now();
        c_pick += c1 - c0;
        if let Some(t) = &tok {
            if t.is_special(next) {
                ended = true;
                break;
            }
            // P6-36 (C-40): the text goes to the screen in whole lines, or only at the end (`--fara-ecran`).
            // P6-39 (C-48): on the workstation each line still cost ~59 ms (45.7 % of the time): every new line at the
            // bottom moves the whole screen up. Now the finished lines go out in one block at most once a second
            // (the console moves the screen once per block), and the rest at the end.
            pending.push_str(&stream.push(t.token_bytes(next)));
            if !o.no_screen && pending.contains('\n') && now_ms() >= last_flush + FLUSH_MS {
                let cut = pending.rfind('\n').unwrap() + 1;
                cyber_user::print!("{}", &pending[..cut]);
                pending.drain(..cut);
                last_flush = now_ms();
            }
        }
        c_screen += kgr::prof::now() - c1;
        out.push(next);
        seen.push(next);
        if out.len() == n {
            break;
        }
        // `--intreaba`: the answer is the first line (a new line after some text ends it).
        if let (true, Some(t)) = (o.ask, &tok) {
            let so_far = t.decode(&out);
            if so_far.trim_start().contains('\n') {
                break;
            }
        }
        logits = match m.forward(&[next], false) {
            Ok(l) => l,
            Err(e) => {
                println!("kronos: {}", e);
                break;
            }
        };
    }
    let c_total = kgr::prof::now() - c_start;
    let t3 = now_ms();
    if let Some(t) = &tok {
        println!("{}{}", pending, stream.finish());
        println!(
            "kronos: textul generat `{}`{}",
            t.decode(&out).replace('\n', "\\n"),
            if ended { " (oprit la simbolul de sfarsit)" } else { "" }
        );
        println!("kronos: simboluri generate: {:?}", out);
        if o.no_screen {
            println!("kronos: textul a fost scris doar la sfarsit (--fara-ecran)");
        }
        if let (true, Some(hist)) = (o.ask, &o.history) {
            let answer = String::from(t.decode(&out).trim_start().lines().next().unwrap_or("").trim());
            println!("kronos: raspuns: {}", answer);
            let mut all = read_all(hist).ok().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            all += &alloc::format!("{}{}\n{}{}\n\n", ASK_Q, text_of(&o.words).rsplit(ASK_Q).next().unwrap_or("").split('\n').next().unwrap_or(""), ASK_A, answer);
            match save(hist, all.as_bytes()) {
                Ok(()) => println!("kronos: istoric salvat in {} ({} schimburi; {} folosite acum)", hist, history_turns(&all).len(), ask_turns),
                Err(e) => println!("kronos: istoricul NU s-a salvat: {}", e),
            }
        }
    } else {
        println!("kronos: continuare: {:?}", out);
    }
    if let Some(p) = o.penalty {
        println!("kronos: penalizarea repetitiilor {} (fara --repetitie: alegerea greedy simpla)", p);
    }
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
    if o.profile {
        // `out.len() - 1` forward passes ran after the given text (the first new token comes from the text's pass).
        profile_report(&m, out.len().saturating_sub(1), c_total, c_screen, c_pick, per_ms);
    }
    kgr::stop_pool();
    0
}
