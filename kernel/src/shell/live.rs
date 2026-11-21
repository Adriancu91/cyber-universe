//! The live channel (C-159 step 1, task P6-108): the command centre drives the OS one command at a time through the hub
//! (`live/<session>/c-NNNN.json` → `r-NNNN.json`, branch `os/live`). The form of the files is in `docs/LIVE.md`.
//!
//! This file is the gate every command goes through before it runs, and the answer it gets — no network here:
//!   - the exact JSON form (`nr`, `sesiune`, `fel`, `text`, `semnatura`);
//!   - the signature: HMAC-SHA-256 with the key `/usb/hub/live.cheie` over [`canonical`], 64 hex digits;
//!   - the number must be exactly the next one; a number already run is never run again (the stick journal keeps
//!     every number, written BEFORE the command runs, so a crash in the middle never repeats it);
//!   - a command that writes to the video card is `REFUZAT` without `/usb/hub/permite-live-placa`;
//!   - the answer always carries the exact echo of what came in.
//!
//! Idea: signed, numbered remote commands (like job queues of hardware test labs); our own code.

#![cfg_attr(not(feature = "test-ghpush"), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use cyber_crypto::sha2::Sha256;
use cyber_json::Value;

/// The version tag at the start of the signed text (a change of form changes it).
pub const FORM: &str = "cyber-live-1";
/// Largest command file accepted (bytes).
pub const MAX_COMMAND: usize = 64 * 1024;
/// The kinds of step 1. The others (step 2: `scrie`, `tasta`, `clic`, `muta`, `ecran`, `captura`) are refused with a clear reason.
pub const KINDS: [&str; 3] = ["comanda", "stare", "gata"];
const LATER: [&str; 6] = ["scrie", "tasta", "clic", "muta", "ecran", "captura"];

/// A command that passed the gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub nr: u32,
    pub session: String,
    pub kind: String,
    pub text: String,
}

/// What the gate says about one command file.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Run(Command),
    /// `nr` = the number the file claimed (if it could be read); `why` = the reason written in the answer.
    Refused { nr: Option<u32>, why: String },
}

/// The text that is signed: the form tag, the session, the number, the kind and the text, one per line (the text last,
/// as it is, with its own line breaks).
pub fn canonical(session: &str, nr: u32, kind: &str, text: &str) -> String {
    format!("{}\n{}\n{}\n{}\n{}", FORM, session, nr, kind, text)
}

/// The signature of a command: HMAC-SHA-256 with `key`, 64 lowercase hex digits.
pub fn sign(key: &[u8], session: &str, nr: u32, kind: &str, text: &str) -> String {
    let mut tag = [0u8; 32];
    cyber_crypto::hmac::hmac::<Sha256>(key, canonical(session, nr, kind, text).as_bytes(), &mut tag);
    tag.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Equal without an early exit (the time does not tell how many digits were right).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether a shell line would write to the video card (refused in live mode without `permite-live-placa`).
pub fn writes_to_board(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.iter().any(|w| w.contains("--scrie") || *w == "--gpu" || *w == "--gpu-sim") {
        return true;
    }
    match words.as_slice() {
        ["timp", rest @ ..] if !rest.is_empty() => writes_to_board(&rest.join(" ")),
        ["test-placa", ..] | ["ciclu-test", ..] => true,
        ["gpu", "tur", rest @ ..] => rest.contains(&"--trezire"),
        ["gpu", sub, ..] => !matches!(*sub, "identifica" | "gfw" | "vbios" | "fwsec" | "firmware" | "pregatire-i6" | "tur" | "plan" | "variante" | "stare"),
        _ => false,
    }
}

/// What the stick journal of a session says: the next number expected and every number already taken.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Gate {
    pub session: String,
    pub next: u32,
    pub taken: Vec<u32>,
}

impl Gate {
    pub fn new(session: &str) -> Self {
        Gate { session: String::from(session), next: 1, taken: Vec::new() }
    }

    /// From the journal lines `<nr> <incepe|gata|refuz> …` (the first word of a line is the number).
    pub fn from_journal(session: &str, text: &str) -> Self {
        let mut g = Gate::new(session);
        for nr in text.lines().filter_map(|l| l.split_whitespace().next()?.parse::<u32>().ok()) {
            if !g.taken.contains(&nr) {
                g.taken.push(nr);
            }
            g.next = g.next.max(nr.saturating_add(1));
        }
        g
    }

    /// Marks `nr` taken (the caller writes the journal line first).
    pub fn take(&mut self, nr: u32) {
        if !self.taken.contains(&nr) {
            self.taken.push(nr);
        }
        self.next = self.next.max(nr.saturating_add(1));
    }

    /// The gate for one command file. `board_key` = `/usb/hub/permite-live-placa` exists.
    pub fn check(&self, key: &[u8], board_key: bool, raw: &str) -> Verdict {
        let refuse = |nr: Option<u32>, why: &str| Verdict::Refused { nr, why: String::from(why) };
        if key.len() < 32 {
            return refuse(None, "cheia live.cheie lipseste sau e prea scurta (cel putin 32 de octeti)");
        }
        if raw.len() > MAX_COMMAND {
            return refuse(None, "fisierul comenzii e prea mare");
        }
        let Ok(v) = cyber_json::parse_limited(raw, MAX_COMMAND) else { return refuse(None, "fisierul comenzii nu e JSON valid") };
        let nr = v.get("nr").and_then(Value::as_i64).and_then(|n| u32::try_from(n).ok()).filter(|&n| n > 0);
        let (Some(session), Some(kind), Some(text), Some(sig)) = (
            v.get("sesiune").and_then(Value::as_str),
            v.get("fel").and_then(Value::as_str),
            v.get("text").and_then(Value::as_str),
            v.get("semnatura").and_then(Value::as_str),
        ) else {
            return refuse(nr, "lipseste un camp (nr, sesiune, fel, text, semnatura)");
        };
        let Some(nr) = nr else { return refuse(None, "nr trebuie sa fie un numar intreg pozitiv") };
        if session != self.session {
            return refuse(Some(nr), "alta sesiune");
        }
        if !same(sign(key, session, nr, kind, text).as_bytes(), sig.to_ascii_lowercase().as_bytes()) {
            return refuse(Some(nr), "semnatura gresita");
        }
        if self.taken.contains(&nr) {
            return Verdict::Refused { nr: Some(nr), why: format!("comanda repetata: nr {} s-a executat deja (o singura data)", nr) };
        }
        if nr != self.next {
            return Verdict::Refused { nr: Some(nr), why: format!("numar gresit: astept nr {}", self.next) };
        }
        if LATER.contains(&kind) {
            return refuse(Some(nr), "felul acesta vine la pasul 2 al agentului viu (inca nu exista)");
        }
        if !KINDS.contains(&kind) {
            return refuse(Some(nr), "fel necunoscut (comanda, stare, gata)");
        }
        if kind == "comanda" && text.trim().is_empty() {
            return refuse(Some(nr), "comanda goala");
        }
        if kind == "comanda" && writes_to_board(text) && !board_key {
            return refuse(Some(nr), "comanda scrie in placa video: fara cheia permite-live-placa (pusa doar cu da-ul lui Adrian) nu ruleaza");
        }
        Verdict::Run(Command { nr, session: String::from(session), kind: String::from(kind), text: String::from(text) })
    }
}

/// What a command did (for the answer).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Outcome {
    pub code: i64,
    pub ms: u64,
    pub output: String,
    pub screen: String,
    pub system: String,
}

/// The answer file `r-NNNN.json`: always the exact echo of the command file that came in (`ecou`, as text, so the
/// command centre compares it byte for byte), then either the outcome or `REFUZAT` with the reason.
pub fn answer(session: &str, nr: Option<u32>, raw: &str, result: Result<&Outcome, &str>) -> String {
    let mut out = String::from("{");
    let field = |name: &str, v: Value, out: &mut String| {
        if out.len() > 1 {
            out.push(',');
        }
        cyber_json::write_string(name, out);
        out.push(':');
        v.write_to(out);
    };
    field("forma", Value::str(FORM), &mut out);
    field("sesiune", Value::str(session), &mut out);
    field("nr", nr.map_or(Value::Null, |n| Value::int(i64::from(n))), &mut out);
    field("ecou", Value::str(raw), &mut out);
    match result {
        Ok(o) => {
            field("stare", Value::str("ok"), &mut out);
            field("cod", Value::int(o.code), &mut out);
            field("durata_ms", Value::int(o.ms.min(i64::MAX as u64) as i64), &mut out);
            field("iesire", Value::str(&o.output), &mut out);
            field("ecran", Value::str(&o.screen), &mut out);
            field("sistem", Value::str(&o.system), &mut out);
        }
        Err(why) => {
            field("stare", Value::str("REFUZAT"), &mut out);
            field("motiv", Value::str(why), &mut out);
        }
    }
    out.push('}');
    out
}

/// The file name of command / answer `nr` (`c-0001.json`, `r-0001.json`).
pub fn file_name(prefix: char, nr: u32) -> String {
    format!("{}-{:04}.json", prefix, nr)
}

/// QEMU self-test (P6-108): the refusal probes the command centre counts (C-159 completion of 21 Nov 08:31) — a bad
/// signature, a wrong number, a repeated command, a board command without the key — and the echo of every answer.
pub fn self_test() {
    let key = [7u8; 32];
    let s = "s-20261121-0900";
    let file = |nr: i64, kind: &str, text: &str, sig: &str| {
        let mut o = String::new();
        Value::object([("nr", Value::int(nr)), ("sesiune", Value::str(s)), ("fel", Value::str(kind)), ("text", Value::str(text)), ("semnatura", Value::str(sig))])
            .write_to(&mut o);
        o
    };
    let good = |nr: u32, kind: &str, text: &str| file(i64::from(nr), kind, text, &sign(&key, s, nr, kind, text));
    // HMAC-SHA-256 against RFC 4231 test case 2 (key "Jefe").
    let mut t = [0u8; 32];
    cyber_crypto::hmac::hmac::<Sha256>(b"Jefe", b"what do ya want for nothing?", &mut t);
    assert!(t[..4] == [0x5b, 0xdc, 0xc1, 0x46], "live: HMAC-SHA-256 RFC 4231");
    let mut g = Gate::new(s);
    let c1 = good(1, "comanda", "hw");
    assert!(g.check(&key, false, &c1) == Verdict::Run(Command { nr: 1, session: String::from(s), kind: String::from("comanda"), text: String::from("hw") }));
    g.take(1);
    let why = |v: Verdict| match v {
        Verdict::Refused { why, .. } => why,
        Verdict::Run(c) => format!("RULEAZA {}", c.nr),
    };
    // The four refusal probes.
    let w = why(g.check(&key, false, &file(2, "comanda", "mem", &"0".repeat(64))));
    assert!(w == "semnatura gresita", "live: {}", w);
    let w = why(g.check(&key, false, &good(3, "comanda", "mem")));
    assert!(w == "numar gresit: astept nr 2", "live: {}", w);
    let w = why(g.check(&key, false, &c1));
    assert!(w.starts_with("comanda repetata: nr 1"), "live: {}", w);
    for line in ["gpu lant", "gpu i6 --scrie", "run kronos --gpu --n 16 Buna ziua", "test-placa", "timp gpu recupereaza", "gpu tur --trezire"] {
        let w = why(g.check(&key, false, &good(2, "comanda", line)));
        assert!(w.starts_with("comanda scrie in placa video"), "live: `{}`: {}", line, w);
        assert!(matches!(g.check(&key, true, &good(2, "comanda", line)), Verdict::Run(_)), "live: `{}` cu cheia", line);
    }
    for line in ["gpu stare", "gpu tur", "hw", "net", "run kronos --n 4 Ana"] {
        assert!(!writes_to_board(line), "live: `{}` nu scrie in placa", line);
    }
    // A signature over another text, another session, a short key, bad JSON, later kinds.
    let mut forged = good(2, "comanda", "hw");
    forged = forged.replace("\"hw\"", "\"mem\"");
    assert!(why(g.check(&key, false, &forged)) == "semnatura gresita");
    assert!(why(g.check(&key, false, &good(2, "comanda", "hw").replace(s, "s-alta"))) == "alta sesiune");
    assert!(why(g.check(&key[..16], false, &good(2, "comanda", "hw"))).starts_with("cheia live.cheie"));
    assert!(why(g.check(&key, false, "{nr: 2")) == "fisierul comenzii nu e JSON valid");
    assert!(why(g.check(&key, false, &good(2, "tasta", "ctrl+c"))).contains("pasul 2"));
    assert!(why(g.check(&key, false, &good(2, "sterge", "x"))).starts_with("fel necunoscut"));
    assert!(matches!(g.check(&key, false, &good(2, "gata", "")), Verdict::Run(_)));
    // The journal: numbers taken before a crash are never run again.
    let j = Gate::from_journal(s, "1 incepe 09:00:01\n1 gata 09:00:02\n2 incepe 09:00:05\n");
    assert!(j.next == 3 && j.taken == [1, 2], "live: jurnalul {:?}", j);
    assert!(why(j.check(&key, false, &good(2, "comanda", "hw"))).starts_with("comanda repetata"));
    // Every answer carries the exact echo; a refusal carries the reason.
    let o = Outcome { code: 0, ms: 12, output: String::from("memorie: ok\n"), screen: String::from("cyber> "), system: String::from("os-325") };
    let a = answer(s, Some(1), &c1, Ok(&o));
    let back = cyber_json::parse(&a).expect("live: raspunsul e JSON");
    assert!(back.get("ecou").and_then(Value::as_str) == Some(c1.as_str()) && back.get("stare").and_then(Value::as_str) == Some("ok"));
    assert!(back.get("iesire").and_then(Value::as_str) == Some("memorie: ok\n") && back.get("cod").and_then(Value::as_i64) == Some(0));
    let r = answer(s, None, "{nr: 2", Err("fisierul comenzii nu e JSON valid"));
    let back = cyber_json::parse(&r).expect("live: refuzul e JSON");
    assert!(back.get("ecou").and_then(Value::as_str) == Some("{nr: 2") && back.get("stare").and_then(Value::as_str) == Some("REFUZAT"));
    assert!(back.get("nr").is_some_and(Value::is_null));
    assert!(file_name('c', 7) == "c-0007.json" && file_name('r', 12) == "r-0012.json");
    crate::info!("test: live: poarta comenzilor (semnatura, numarul, o singura data, placa fara cheie) si ecoul OK");
}
