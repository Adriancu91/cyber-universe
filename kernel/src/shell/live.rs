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
//! P6-109: the loop (`live <minute>`, and the line `live <minute>` in `auto/teste.txt`): opens `live/<sesiune>/` on the
//! hub branch `os/live` (`sesiune.json`), asks every 2.5 s for the folder with a conditional request (`gh-ia
//! --live-dosar`, ETag: an unchanged folder costs nothing), takes `c-NNNN.json` in order (`gh-ia --live-ia`), passes it
//! through the gate, notes it in the stick journal BEFORE it runs, runs it with every printed line captured and sends
//! `r-NNNN.json` (`gh-trimite --live`, never over another file). It ends at `gata`, after <minute> without commands, or
//! with Esc (by hand). Every command file number is read once: a refused one is used up too (the next one is N+1).
//!
//! Idea: signed, numbered remote commands (like job queues of hardware test labs); our own code.

use super::auto;
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use cyber_crypto::sha2::Sha256;
use cyber_json::Value;
use crate::drivers::keyboard::Key;

/// The version tag at the start of the signed text (a change of form changes it).
pub const FORM: &str = "cyber-live-1";
/// Largest command file accepted (bytes).
pub const MAX_COMMAND: usize = 64 * 1024;
/// The kinds that run: step 1 (`comanda`, `stare`, `gata`) and P6-111 (C-159 step 2, part 1: the live terminal's keys and
/// its screen text; P6-127: `captura`, `muta`, `clic`; P6-130: `actualizeaza`).
pub const KINDS: [&str; 12] =
    ["comanda", "stare", "gata", "scrie", "tasta", "ecran", "captura", "muta", "clic", "actualizeaza", "citeste", "salveaza"];
/// P6-127: `captura` = the screen reduced this many times in each direction, gray PNG; the largest file sent.
const CAPTURE_SCALE: usize = 4;
const CAPTURE_MAX: usize = 900 * 1024;
/// Longest text of one `scrie` (characters).
pub const MAX_TYPED: usize = 1024;

/// P6-111: the key names of `tasta`, separated by spaces (`ctrl+c enter`, `sus sus enter`). `alt+…` does not exist
/// yet (no window to switch to in the live terminal) and is refused with that reason, like any unknown name.
pub fn parse_keys(text: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for name in text.split_whitespace() {
        let low = name.to_ascii_lowercase();
        let key = match low.as_str() {
            "enter" => Key::Enter,
            "esc" | "escape" => Key::Escape,
            "tab" => Key::Tab,
            "backspace" | "bksp" => Key::Backspace,
            "spatiu" | "space" => Key::Char(' '),
            "sus" | "up" => Key::Up,
            "jos" | "down" => Key::Down,
            "stanga" | "left" => Key::Left,
            "dreapta" | "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "delete" | "del" => Key::Delete,
            other => match other.strip_prefix("ctrl+") {
                Some(l) if l.len() == 1 && l.as_bytes()[0].is_ascii_lowercase() => Key::Ctrl(l.as_bytes()[0] as char),
                _ if other.starts_with("alt+") => return Err(format!("tasta {}: alt+… nu exista inca in terminalul viu (nicio fereastra de schimbat)", name)),
                _ => return Err(format!("tasta necunoscuta: {} (enter, esc, tab, backspace, spatiu, sus, jos, stanga, dreapta, home, end, delete, ctrl+<litera>)", name)),
            },
        };
        keys.push(key);
    }
    if keys.is_empty() {
        return Err(String::from("nicio tasta"));
    }
    if keys.len() > 64 {
        return Err(String::from("prea multe taste intr-o comanda (cel mult 64)"));
    }
    Ok(keys)
}

/// P6-127: where `muta` / `clic` go and which button: `muta x y`, `clic`, `clic stanga|dreapta`, `clic x y [stanga|dreapta]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseOrder {
    pub to: Option<(usize, usize)>,
    /// 0 = only a move; 1 = left button, 2 = right button (the bits of `MouseEvent::buttons`).
    pub button: u8,
}

/// The largest coordinate accepted (screens up to 8K).
const MAX_XY: usize = 8192;

pub fn parse_mouse(kind: &str, text: &str) -> Result<MouseOrder, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let xy = |a: &str, b: &str| -> Result<(usize, usize), String> {
        match (a.parse::<usize>(), b.parse::<usize>()) {
            (Ok(x), Ok(y)) if x <= MAX_XY && y <= MAX_XY => Ok((x, y)),
            _ => Err(format!("{}: x si y = numere intregi de la 0 la {} (pixeli de la coltul din stanga sus)", kind, MAX_XY)),
        }
    };
    let button = |w: &str| match w {
        "stanga" | "left" => Ok(1),
        "dreapta" | "right" => Ok(2),
        _ => Err(format!("clic: butonul e stanga sau dreapta, nu {}", w)),
    };
    match (kind, words.as_slice()) {
        ("muta", [x, y]) => Ok(MouseOrder { to: Some(xy(x, y)?), button: 0 }),
        ("muta", _) => Err(String::from("muta: forma e `muta x y`")),
        ("clic", []) => Ok(MouseOrder { to: None, button: 1 }),
        ("clic", [b]) => Ok(MouseOrder { to: None, button: button(b)? }),
        ("clic", [x, y]) => Ok(MouseOrder { to: Some(xy(x, y)?), button: 1 }),
        ("clic", [x, y, b]) => Ok(MouseOrder { to: Some(xy(x, y)?), button: button(b)? }),
        _ => Err(String::from("clic: forma e `clic [x y] [stanga|dreapta]`")),
    }
}

/// P6-111: the keys of a `scrie` text (a line end = Enter, a tab = Tab); other control characters are refused.
pub fn typed_keys(text: &str) -> Result<Vec<Key>, String> {
    if text.is_empty() {
        return Err(String::from("text gol"));
    }
    if text.chars().count() > MAX_TYPED {
        return Err(format!("textul e prea lung (cel mult {} de caractere)", MAX_TYPED));
    }
    text.chars()
        .map(|c| match c {
            '\n' => Ok(Key::Enter),
            '\t' => Ok(Key::Tab),
            c if c.is_control() => Err(String::from("textul are caractere de control (doar rand nou si tab)")),
            c => Ok(Key::Char(c)),
        })
        .collect()
}

/// P6-111: a line typed in the live terminal, checked by its shell before it runs. Nothing that writes to the video card
/// goes by keys (only as `comanda`, through the gate, its keys and the one-per-session rule), and no nested `live`.
pub fn typed_line(line: &str) -> Result<(), &'static str> {
    if line.split_whitespace().next() == Some("live") {
        return Err("o sesiune vie nu porneste alta sesiune vie");
    }
    if writes_to_board(line) {
        return Err("prin taste nu ruleaza nimic ce scrie in placa video (doar ca `comanda`, cu cheile ei)");
    }
    Ok(())
}

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
        if !KINDS.contains(&kind) {
            return refuse(Some(nr), "fel necunoscut (comanda, stare, gata, scrie, tasta, ecran, captura, muta, clic, actualizeaza, citeste, salveaza)");
        }
        if kind == "actualizeaza" {
            if let Err(why) = update_tag(text) {
                return Verdict::Refused { nr: Some(nr), why };
            }
        }
        // P6-132: the paths of `citeste` / `salveaza` (and the content of `salveaza`) are checked before anything runs.
        if kind == "citeste" {
            if let Err(why) = read_path(text, stick_root()) {
                return Verdict::Refused { nr: Some(nr), why };
            }
        }
        if kind == "salveaza" {
            if let Err(why) = save_order(text, stick_root()) {
                return Verdict::Refused { nr: Some(nr), why };
            }
        }
        if kind == "scrie" {
            if let Err(why) = typed_keys(text) {
                return Verdict::Refused { nr: Some(nr), why: format!("scrie: {}", why) };
            }
        }
        if kind == "tasta" {
            if let Err(why) = parse_keys(text) {
                return Verdict::Refused { nr: Some(nr), why };
            }
        }
        if kind == "muta" || kind == "clic" {
            if let Err(why) = parse_mouse(kind, text) {
                return Verdict::Refused { nr: Some(nr), why };
            }
        }
        if kind == "comanda" && text.trim().is_empty() {
            return refuse(Some(nr), "comanda goala");
        }
        if kind == "comanda" && text.split_whitespace().next() == Some("live") {
            return refuse(Some(nr), "o sesiune vie nu porneste alta sesiune vie");
        }
        if kind == "comanda" && writes_to_board(text) && !board_key {
            return refuse(Some(nr), "comanda scrie in placa video: fara cheia permite-live-placa (pusa doar cu da-ul lui Adrian) nu ruleaza");
        }
        Verdict::Run(Command { nr, session: String::from(session), kind: String::from(kind), text: String::from(text) })
    }
}

/// P6-132: the largest file `citeste` sends to the hub (as `captura`), the largest content `salveaza` takes (the command
/// file is at most 64 KiB, base64 included) and the largest text shown whole in `iesire`.
pub const READ_MAX: usize = 900 * 1024;
pub const SAVE_MAX: usize = 48 * 1024;
const SHOW_MAX: usize = 32 * 1024;
/// The only folder of the stick `salveaza` writes to (`/usb/schimb/`).
pub const SAVE_DIR: &str = "schimb";

/// The root of the stick (`/usb`; `/etc` in the QEMU tests) = the folder that holds the hub folder.
pub fn stick_root() -> &'static str {
    dir().rsplit_once('/').map_or("/usb", |(root, _)| root)
}

/// `path` without empty or `.` parts, in its own letter case; `None` if it is relative or has a `..` part.
fn plain_path(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut out = String::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => return None,
            p => {
                out.push('/');
                out.push_str(p);
            }
        }
    }
    Some(if out.is_empty() { String::from("/") } else { out })
}

/// Whether `path` is `root` or below it (letter case ignored, as on the FAT stick).
fn below(path: &str, root: &str) -> bool {
    let (p, r) = (path.to_ascii_lowercase(), root.to_ascii_lowercase());
    p == r || (p.starts_with(&r) && p.as_bytes().get(r.len()) == Some(&b'/'))
}

/// P6-132: the path `citeste` may read — on the stick, without `..`, never a secret file (the list of `cat`).
pub fn read_path(text: &str, root: &str) -> Result<String, String> {
    let Some(path) = plain_path(text.trim()) else { return Err(String::from("citeste: calea trebuie sa fie intreaga (incepe cu /, fara ..)")) };
    if !below(&path, root) {
        return Err(format!("citeste: doar de pe stick (sub {})", root));
    }
    if crate::proc::rights::canonical(&path).is_some_and(|c| crate::proc::rights::is_secret(&c)) {
        return Err(String::from("citeste: fisier secret (nu pleaca niciodata in hub)"));
    }
    Ok(path)
}

/// P6-132: `salveaza` = `<path>\n<content in base64>`; the path directly in `<stick>/schimb/`, a plain name.
pub fn save_order(text: &str, root: &str) -> Result<(String, Vec<u8>), String> {
    let Some((first, b64)) = text.split_once('\n') else { return Err(String::from("salveaza: forma: <cale>, un rand nou, apoi continutul in base64")) };
    let Some(path) = plain_path(first.trim()) else { return Err(String::from("salveaza: calea trebuie sa fie intreaga (incepe cu /, fara ..)")) };
    let allowed = format!("{}/{}", root, SAVE_DIR);
    let Some((parent, name)) = path.rsplit_once('/') else { return Err(String::from("salveaza: calea")) };
    if !parent.eq_ignore_ascii_case(&allowed) {
        return Err(format!("salveaza: doar direct in {}/ (nimic in alt dosar al stick-ului)", allowed));
    }
    let good = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    if name.is_empty() || name.len() > 64 || name.starts_with('.') || !name.chars().all(good) {
        return Err(String::from("salveaza: numele: litere, cifre, . - _ (cel mult 64, nu incepe cu .)"));
    }
    if crate::proc::rights::canonical(&path).is_some_and(|c| crate::proc::rights::is_secret(&c)) {
        return Err(String::from("salveaza: nume de fisier secret"));
    }
    let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(data) = cyber_json::base64::decode(&clean) else { return Err(String::from("salveaza: continutul nu e base64 valid")) };
    if data.len() > SAVE_MAX {
        return Err(format!("salveaza: {} octeti (cel mult {})", data.len(), SAVE_MAX));
    }
    Ok((path, data))
}

/// What the journal and the screen show of a command (`salveaza`: the path and the size, not the base64).
fn shown_text(kind: &str, text: &str) -> String {
    if kind == "salveaza" {
        let (path, rest) = text.split_once('\n').unwrap_or((text, ""));
        return format!("{} ({} caractere base64)", path.trim(), rest.len());
    }
    text.replace('\n', " ")
}

/// Whether `data` carries the bytes of a key or of the token (the values of the secret files, also as hex).
fn carries_secret(data: &[u8], secrets: &[String]) -> bool {
    secrets.iter().any(|s| !s.is_empty() && data.windows(s.len()).any(|w| w == s.as_bytes()))
        || core::str::from_utf8(data).is_ok_and(|t| super::raport::hide_secrets(t, secrets) != t)
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

/// P6-130 (C-159 step 3): the release a live `actualizeaza` asks for — `os-<N>` (or `--versiune os-<N>`), that exact
/// GitHub Release (every file checked against SHA256SUMS by `/bin/actualizeaza`).
pub fn update_tag(text: &str) -> Result<String, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let tag = match words.as_slice() {
        [t] | ["--versiune", t] => *t,
        _ => "",
    };
    match tag.strip_prefix("os-") {
        Some(n) if !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()) => Ok(String::from(tag)),
        _ => Err(String::from("actualizeaza: forma e `os-<numar>` (Release-ul exact de pe GitHub)")),
    }
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
            // P6-113: the text that leaves for the hub never carries a key or token (the values of the secret files,
            // also as hex, and anything shaped like a key), whatever the command did.
            let secrets = super::raport::secrets();
            let clean = |t: &str| super::raport::hide_secrets(t, &secrets);
            field("stare", Value::str("ok"), &mut out);
            field("cod", Value::int(o.code), &mut out);
            field("durata_ms", Value::int(o.ms.min(i64::MAX as u64) as i64), &mut out);
            field("iesire", Value::str(&clean(&o.output)), &mut out);
            field("ecran", Value::str(&clean(&o.screen)), &mut out);
            field("sistem", Value::str(&clean(&o.system)), &mut out);
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

/// How often the hub folder is asked for (ms), and the longest session (minutes without commands).
pub const POLL_MS: u64 = 2_500;
pub const MAX_MINUTES: u64 = 120;
/// Tries for sending one answer before it stays only on the stick.
const SEND_TRIES: u32 = 3;
/// P6-129 (C-159 step 3, part 1): how often the session writes `live/<sesiune>/puls.json` (the only file the OS rewrites).
pub const PULS_MS: u64 = 60_000;

/// One live session at a time.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// P6-127: the session was started by hand (`live <minute>` typed): Esc on the real keyboard stops it.
static BY_HAND: AtomicBool = AtomicBool::new(false);

/// P6-118 (C-161 point 1): a board command (`test-placa …` with a correct form) already came in this session. At most one
/// per session until "several tests in one power-on" works (C-158 point 5); the second is refused with the reason.
static BOARD_TAKEN: AtomicBool = AtomicBool::new(false);

/// P6-118 (C-161 point 1): whether a live session is running. A `DA` question asked meanwhile is answered at once and
/// never with `DA` (nobody is at the keyboard; keys sent through the channel never answer it — `hubcmd.rs`: "DA il
/// tasteaza doar Adrian, la calculator").
pub fn in_session() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// The reason a second board command in one session is refused.
pub const BOARD_ONCE: &str = "a doua comanda pe placa in aceeasi sesiune: cel mult una pe sesiune pana merge „mai multe teste intr-o pornire” (C-158 punctul 5); deschide o sesiune noua";

/// What a session did.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub session: String,
    pub ran: u32,
    pub refused: u32,
    pub unsent: u32,
    pub end: String,
    /// P6-130: `actualizeaza` put this release on the stick: the machine restarts into it and the session resumes.
    pub restart: Option<String>,
}

impl Summary {
    pub fn line(&self) -> String {
        format!(
            "live: sesiunea {} incheiata ({}): {} rulate, {} refuzate{}\n",
            self.session,
            self.end,
            self.ran,
            self.refused,
            if self.unsent > 0 { format!(", {} raspunsuri netrimise (raman pe stick)", self.unsent) } else { String::new() }
        )
    }
}

/// `live <minute>` in the shell.
pub fn command(args: &[&str]) {
    let Some(minutes) = parse_minutes(args) else {
        super::emit(&format!("folosire: live <minute>   (1-{}; canalul viu prin hub: comenzile din live/<sesiune>/ pe ramura os/live, docs/LIVE.md)\n", MAX_MINUTES));
        return;
    };
    match run(minutes, POLL_MS, PULS_MS, true) {
        Ok(s) => {
            super::emit(&s.line());
            if let Some(tag) = &s.restart {
                // P6-130: by hand the restart comes right away (the list restarts at its own end).
                auto::say(&format!("live: repornesc in {}; sesiunea {} se reia dupa pornire\n", tag, s.session));
                crate::sched::sleep_ms(1_000);
                crate::power::reboot(None);
            }
        }
        Err(why) => super::emit(&format!("live: {}\n", why)),
    }
}

fn parse_minutes(args: &[&str]) -> Option<u64> {
    match args {
        [m] => m.parse::<u64>().ok().filter(|m| (1..=MAX_MINUTES).contains(m)),
        _ => None,
    }
}

/// The line `live <minute>` of the automatic list: the text for its report.
pub fn auto_line(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().skip(1).collect();
    let Some(minutes) = parse_minutes(&words) else {
        return format!("REFUZAT: forma corecta: `live <minute>` (1-{})\n", MAX_MINUTES);
    };
    match run(minutes, POLL_MS, PULS_MS, false) {
        Ok(s) if s.restart.is_some() => format!("{}live: repornirea vine la sfarsitul listei; sesiunea se reia dupa pornire\n", s.line()),
        Ok(s) => s.line(),
        Err(why) => format!("live: {}\n", why),
    }
}

/// QEMU (`ghpush`): a whole session against the test server, asking every second.
#[cfg(feature = "test-ghpush")]
pub fn test_session(minutes: u64) -> Result<Summary, String> {
    run(minutes, 1_000, 5_000, false)
}

fn dir() -> &'static str {
    super::hubcmd::dir()
}

/// Runs a program to the end; its exit code.
fn spawn(path: &str, args: &[&str]) -> i64 {
    match crate::proc::spawn_path(path, args) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => {
            auto::say(&format!("live: {} nu porneste: {}\n", path, e.message()));
            -1
        }
    }
}

/// Sends a local file to `live/<path>` (a few tries; the same content again is harmless).
fn send(local: &str, remote: &str) -> bool {
    for t in 0..SEND_TRIES {
        if t > 0 {
            crate::sched::sleep_ms(2_000);
        }
        if spawn("/bin/gh-trimite", &["gh-trimite", "--live", local, remote]) == 0 {
            return true;
        }
    }
    false
}

fn write_new(path: &str, data: &[u8]) -> bool {
    let _ = fs::remove(path);
    fs::create(path, Kind::File).is_ok() && fs::write(path, 0, data).is_ok() && fs::flush(path).is_ok()
}

/// One line at the end of the stick journal, on the stick before anything else happens.
fn journal(path: &str, line: &str) {
    if fs::kind(path).is_err() {
        let _ = fs::create(path, Kind::File);
    }
    let at = fs::size(path).unwrap_or(0);
    let _ = fs::write(path, at, format!("{}\n", line).as_bytes());
    let _ = fs::flush(path);
}

/// The state of the machine, for every answer (and the kind `stare`).
pub fn system_text() -> String {
    use crate::mem::{frame, map::PAGE_SIZE};
    let stick = fs::read_all("/usb/VERSIUNE").map(|v| String::from_utf8_lossy(&v).lines().next().unwrap_or("").trim().into()).unwrap_or_else(|_| String::from("-"));
    format!(
        "cyber-universe {} (cyber-gpu {}); /usb/VERSIUNE: {}; pornit de {} s; ora {}; memorie libera {} MiB; {}",
        env!("CARGO_PKG_VERSION"),
        crate::drivers::gpu::CYBER_GPU_REV,
        stick,
        crate::time::uptime_ms().unwrap_or(0) / 1000,
        auto::now_text(),
        frame::free_frames() * PAGE_SIZE / (1024 * 1024),
        crate::hw::temp::line()
    )
}

/// The field `ecran` of every answer: the text on the real screen (the console, row by row); while the desktop owns the
/// screen, a note that only `captura` shows it; without a screen, the last lines of the system log.
fn real_screen() -> String {
    if crate::ui::running() || !crate::gfx::console::text_on_screen() {
        return String::from("(desktopul are ecranul: imaginea lui = fel captura)");
    }
    match crate::gfx::console::screen_text() {
        Some(t) => t,
        None => {
            let lines = crate::log::recent();
            lines[lines.len().saturating_sub(25)..].join("\n")
        }
    }
}

/// How long the live terminal is watched after keys are sent: until it has been quiet for `QUIET_MS`, at most `WAIT_MS`.
const QUIET_MS: u64 = 500;
const WAIT_MS: u64 = 10_000;
/// Lines of the live terminal kept for `ecran` (and their longest length).
const SCREEN_LINES: usize = 40;
const SCREEN_COLS: usize = 200;
/// The two parts of the answer to `ecran`.
pub const TERMINAL_HEAD: &str = "== terminalul viu (scrie / tasta) ==";
pub const SCREEN_HEAD: &str = "== ecranul adevarat ==";

/// P6-111 (C-159 step 2, part 1): the live session's own terminal — a whole shell on its own thread (`term::open_live`),
/// fed by `scrie` / `tasta`, whose text `ecran` shows. Opened at the first of them; closed with the session.
#[derive(Default)]
pub struct Terminal {
    id: Option<u64>,
    lines: Vec<String>,
}

impl Terminal {
    /// Adds text the terminal printed: a line end starts a new line, Backspace erases the last character of the line.
    pub fn feed(&mut self, text: &str) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        for c in text.chars() {
            match c {
                '\n' => {
                    self.lines.push(String::new());
                    if self.lines.len() > SCREEN_LINES {
                        self.lines.remove(0);
                    }
                }
                '\x08' => {
                    if let Some(l) = self.lines.last_mut() {
                        l.pop();
                    }
                }
                '\r' => {}
                c => {
                    if let Some(l) = self.lines.last_mut()
                        && l.chars().count() < SCREEN_COLS
                    {
                        l.push(c);
                    }
                }
            }
        }
    }

    /// The text on the terminal's screen now (the last `SCREEN_LINES` lines).
    pub fn screen(&self) -> String {
        self.lines.join("\n")
    }

    /// Opens the terminal if it is not open (again after `exit`). `None` = it could not be opened.
    fn ensure(&mut self) -> u64 {
        let me = crate::sched::current_id();
        match self.id {
            Some(id) if crate::term::is_open(me, id) => id,
            _ => {
                if self.id.is_some() {
                    self.feed("\n[terminalul viu s-a inchis (exit); am deschis altul]\n");
                }
                let id = crate::term::open_live(me);
                self.id = Some(id);
                id
            }
        }
    }

    /// Reads what the terminal printed until it has been quiet for `QUIET_MS` (at most `WAIT_MS`); returns that text.
    fn settle(&mut self, id: u64) -> String {
        let me = crate::sched::current_id();
        let start = crate::time::uptime_ms().unwrap_or(0);
        let mut quiet_since = start;
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let now = crate::time::uptime_ms().unwrap_or(0);
            match crate::term::read_output(me, id, &mut buf) {
                Some(n) if n > 0 => {
                    got.extend_from_slice(&buf[..n]);
                    quiet_since = now;
                    continue;
                }
                Some(_) => {}
                None => break, // closed and everything read
            }
            if now.saturating_sub(quiet_since) >= QUIET_MS || now.saturating_sub(start) >= WAIT_MS {
                break;
            }
            crate::sched::sleep_ms(50);
        }
        let text = String::from_utf8_lossy(&got).into_owned();
        self.feed(&text);
        text
    }

    /// `scrie` / `tasta`: sends the keys and returns what the terminal printed after them. P6-127: while the desktop runs
    /// (started from the live terminal: `scrie "desktop\n"`), the keys go to its window in front, like the real keyboard.
    fn keys(&mut self, keys: &[Key]) -> String {
        if crate::ui::running() {
            if BY_HAND.load(Ordering::Acquire) && keys.contains(&Key::Escape) {
                super::LAST_EXIT.store(1, Ordering::Relaxed);
                return String::from("esc: sesiunea pornita de mana se opreste cu Esc; cat ruleaza desktopul, Esc nu se trimite prin canal\n");
            }
            for &k in keys {
                crate::drivers::keyboard::push_key(k);
                crate::sched::sleep_ms(5);
            }
            crate::sched::sleep_ms(QUIET_MS);
            return format!("{} taste trimise desktopului (fereastra din fata){}\n", keys.len(), if crate::ui::running() { "" } else { "; desktopul s-a inchis" });
        }
        let id = self.ensure();
        let mut before = self.settle(id); // the welcome text and prompt of a new terminal stay on the screen
        before.clear();
        if !crate::term::send_key_list(crate::sched::current_id(), id, keys) {
            super::LAST_EXIT.store(1, Ordering::Relaxed);
            return String::from("terminalul viu nu primeste taste (inchis)\n");
        }
        self.settle(id)
    }

    /// `ecran`: the live terminal's text now (what arrived meanwhile included), then the real screen's text.
    fn look(&mut self) -> String {
        let id = self.ensure();
        self.settle(id);
        format!("{}\n{}\n{}\n{}\n", TERMINAL_HEAD, self.screen(), SCREEN_HEAD, real_screen())
    }

    /// The session ended: the terminal closes, so its shell ends too.
    fn close(&mut self) {
        if self.id.take().is_some() {
            crate::term::close_owned(crate::sched::current_id());
        }
    }
}

/// Runs one shell line with everything it prints captured (inside the automatic mode's own capture, which is kept).
fn run_captured(line: &str) -> String {
    let outer = super::CAPTURE.lock().take();
    let outer_programs = super::PROGRAM_CAPTURE.lock().take();
    super::emit_capture_begin();
    super::program_capture_begin();
    super::dispatch(line);
    let mut text = super::emit_capture_end();
    text.push_str(&super::program_capture_end());
    *super::CAPTURE.lock() = outer;
    *super::PROGRAM_CAPTURE.lock() = outer_programs;
    text
}

/// P6-118 (C-161 point 1, C-159 note of 21 Nov 16:35): `test-placa […]` in live mode — the same forms as in
/// `auto/teste.txt`, the same `board_test` (Adrian's keys: `permite-test-placa` + the line in DECIZII.md; the gate has
/// already asked for `permite-live-placa`), no `DA` from the keyboard; the whole report in `iesire` and sent to the hub
/// as from the list. A wrong form or a missing key = `REFUZAT: …` in `iesire` with code 1, nothing written to the board.
fn board_command(parsed: Result<String, &'static str>) -> String {
    let outer = super::CAPTURE.lock().take();
    let outer_programs = super::PROGRAM_CAPTURE.lock().take();
    let text = match auto::board_test(parsed) {
        Ok(text) => text,
        Err(why) => {
            super::LAST_EXIT.store(1, Ordering::Relaxed);
            format!("REFUZAT: {} (nimic nu s-a scris in placa)\n", why)
        }
    };
    *super::CAPTURE.lock() = outer;
    *super::PROGRAM_CAPTURE.lock() = outer_programs;
    text
}

/// The session's name: `s-AAAALLZZ-HHMM` (`-2`, `-3` … when that folder is already on the stick).
fn session_name(base: &str) -> String {
    let stamp = auto::date_stamp();
    let first = format!("s-{}", stamp);
    if fs::kind(&format!("{}/{}", base, first)).is_err() {
        return first;
    }
    (2..100).map(|n| format!("s-{}-{}", stamp, n)).find(|s| fs::kind(&format!("{}/{}", base, s)).is_err()).unwrap_or(first)
}

/// A whole session. `by_hand` = Esc stops it.
fn run(minutes: u64, poll_ms: u64, puls_ms: u64, by_hand: bool) -> Result<Summary, String> {
    if ACTIVE.swap(true, Ordering::Acquire) {
        return Err(String::from("o sesiune vie ruleaza deja"));
    }
    BOARD_TAKEN.store(false, Ordering::SeqCst);
    BY_HAND.store(by_hand, Ordering::Release);
    let r = session(minutes, poll_ms, puls_ms, by_hand);
    ACTIVE.store(false, Ordering::Release);
    r
}

/// What a running session needs (P6-130: the same for a new session and for one resumed after a restart).
struct Ctx {
    s: String,
    local: String,
    remote: String,
    key: Vec<u8>,
    board_key: bool,
    minutes: u64,
    poll_ms: u64,
    puls_ms: u64,
    by_hand: bool,
    journal_path: String,
}

/// The checks before any session (new or resumed): the key, the token, the network. Returns the key and the board key.
fn preflight() -> Result<(Vec<u8>, bool), String> {
    let d = dir();
    let key = fs::read_all(&format!("{}/live.cheie", d)).unwrap_or_default();
    if key.len() < 32 {
        return Err(format!("cheia {}/live.cheie lipseste sau e prea scurta (cel putin 32 de octeti): modul viu nu porneste", d));
    }
    let (_, token) = super::raport::hub_paths();
    if fs::kind(token).is_err() {
        return Err(format!("fara {}: nu pot vorbi cu hub-ul; modul viu nu porneste", token));
    }
    if crate::net::config().is_none() || !crate::net::wait_ready(15_000) {
        return Err(String::from("reteaua nu are adresa: modul viu nu porneste"));
    }
    Ok((key, fs::kind(&format!("{}/permite-live-placa", d)).is_ok()))
}

fn session(minutes: u64, poll_ms: u64, puls_ms: u64, by_hand: bool) -> Result<Summary, String> {
    let d = dir();
    let (key, board_key) = preflight()?;
    let base = format!("{}/live", d);
    if fs::kind(d).is_err() {
        let _ = fs::create(d, Kind::Dir);
    }
    if fs::kind(&base).is_err() && fs::create(&base, Kind::Dir).is_err() {
        return Err(format!("nu pot face {} (stick doar citire? lipseste cyber.rw)", base));
    }
    let s = session_name(&base);
    let local = format!("{}/{}", base, s);
    if fs::create(&local, Kind::Dir).is_err() {
        return Err(format!("nu pot face {}", local));
    }
    let remote = format!("live/{}", s);
    let mut head = String::new();
    cyber_json::Value::object([
        ("forma", cyber_json::Value::str(FORM)),
        ("sesiune", cyber_json::Value::str(&s)),
        ("pornit", cyber_json::Value::str(&auto::now_text())),
        ("minute", cyber_json::Value::int(minutes as i64)),
        ("intrebare_ms", cyber_json::Value::int(poll_ms as i64)),
        ("puls_ms", cyber_json::Value::int(puls_ms as i64)),
        ("placa", cyber_json::Value::Bool(board_key)),
        ("sistem", cyber_json::Value::str(&system_text())),
    ])
    .write_to(&mut head);
    let head_path = format!("{}/sesiune.json", local);
    if !write_new(&head_path, head.as_bytes()) || !send(&head_path, &format!("{}/sesiune.json", remote)) {
        return Err(format!("sesiunea {} nu s-a putut deschide in hub (ramura os/live)", s));
    }
    let journal_path = format!("{}/jurnal.txt", local);
    journal(&journal_path, &format!("# sesiunea {} pornita {} ({} minute fara comenzi = sfarsit)", s, auto::now_text(), minutes));
    auto::say(&format!(
        "live: sesiunea {} deschisa in hub ({}/, ramura os/live); astept comenzi la {} s; {} minute fara comenzi = sfarsit{}; placa video: {}\n",
        s,
        remote,
        poll_ms / 1000,
        minutes,
        if by_hand { "; Esc opreste" } else { "" },
        if board_key { "PERMISA (permite-live-placa)" } else { "oprita" }
    ));
    let ctx = Ctx { s: s.clone(), local, remote, key, board_key, minutes, poll_ms, puls_ms, by_hand, journal_path };
    let sum = Summary { session: s.clone(), ..Summary::default() };
    Ok(serve(&ctx, Gate::new(&s), sum, 0))
}

/// The loop of a session, until `gata`, `<minute>` without commands, Esc (by hand) or a restart asked by `actualizeaza`.
/// `puls_count` = the beats already sent (a resumed session goes on from there).
fn serve(ctx: &Ctx, mut gate: Gate, mut sum: Summary, puls_count: u32) -> Summary {
    let s = &ctx.s;
    let idle_ms = ctx.minutes * 60_000;
    let mut last = crate::time::uptime_ms().unwrap_or(0);
    let list_path = format!("{}/lista.txt", ctx.local);
    let mut errors = 0u32;
    let mut terminal = Terminal::default();
    let mut puls = Puls {
        local: format!("{}/puls.json", ctx.local),
        remote: format!("{}/puls.json", ctx.remote),
        every_ms: ctx.puls_ms,
        count: puls_count,
        ..Puls::default()
    };
    puls.at = last;
    'outer: loop {
        if ctx.by_hand && crate::drivers::keyboard::take_escape() {
            sum.end = String::from("oprita cu Esc");
            break;
        }
        let now = crate::time::uptime_ms().unwrap_or(0);
        if now.saturating_sub(last) >= idle_ms {
            sum.end = format!("{} minute fara comenzi", ctx.minutes);
            break;
        }
        auto::keep_alive(idle_ms.saturating_sub(now - last) + 5 * 60_000);
        puls.maybe(s, &gate, &sum, now);
        match spawn("/bin/gh-ia", &["gh-ia", "--live-dosar", &ctx.remote, &list_path]) {
            0 => {
                errors = 0;
                let names = String::from_utf8_lossy(&fs::read_all(&list_path).unwrap_or_default()).into_owned();
                loop {
                    let want = gate.next;
                    let name = file_name('c', want);
                    if !names.lines().any(|l| l.trim() == name) {
                        break;
                    }
                    let copy = format!("{}/{}", ctx.local, name);
                    if spawn("/bin/gh-ia", &["gh-ia", "--live-ia", &format!("{}/{}", ctx.remote, name), &copy]) != 0 {
                        break; // not readable yet: at the next look
                    }
                    let raw = String::from_utf8_lossy(&fs::read_all(&copy).unwrap_or_default()).into_owned();
                    let end = one(ctx, &raw, want, &mut gate, &mut sum, &mut terminal);
                    last = crate::time::uptime_ms().unwrap_or(0);
                    if end {
                        sum.end = match &sum.restart {
                            Some(tag) => format!("repornire in {} (actualizeaza); sesiunea se reia dupa pornire", tag),
                            None => String::from("gata (cerut de centrul de comanda)"),
                        };
                        break 'outer;
                    }
                }
            }
            3 | 4 => errors = 0,
            code => {
                errors += 1;
                if errors == 1 || errors % 20 == 0 {
                    auto::say(&format!("live: hub-ul nu raspunde (gh-ia codul {}, de {} ori la rand); mai incerc\n", code, errors));
                }
            }
        }
        let mut waited = 0;
        while waited < ctx.poll_ms {
            if ctx.by_hand && crate::drivers::keyboard::take_escape() {
                sum.end = String::from("oprita cu Esc");
                break 'outer;
            }
            let step = (ctx.poll_ms - waited).min(200);
            crate::sched::sleep_ms(step);
            waited += step;
        }
    }
    terminal.close();
    puls.send(s, &gate, &sum, Some(&sum.end));
    journal(&ctx.journal_path, &format!("# sfarsit {}: {}", auto::now_text(), sum.end));
    auto::say(&sum.line());
    sum
}

/// P6-130 (C-159 step 3): the marker of a session that waits for the restart after `actualizeaza` —
/// `<hub>/live/reluare.txt`, one `name value` per line. Written right after the new version is on the stick (before the
/// answer leaves), removed at the first boot that reads it (a crash during the resumed session never loops).
const RESUME: &str = "reluare.txt";

/// What the marker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    pub session: String,
    pub minutes: u64,
    /// The session was started from the automatic list (`true`: the machine restarts when the resumed session ends).
    pub from_list: bool,
    pub tag: String,
    pub nr: u32,
}

impl Resume {
    pub fn text(&self) -> String {
        format!(
            "sesiune {}\nminute {}\norigine {}\nversiune {}\nnr {}\n",
            self.session,
            self.minutes,
            if self.from_list { "lista" } else { "mana" },
            self.tag,
            self.nr
        )
    }

    /// The marker read back; `None` when a line is missing or wrong (the session name must be one of ours).
    #[cfg_attr(all(feature = "qemu-test", not(feature = "test-ghpush")), allow(dead_code))]
    pub fn parse(text: &str) -> Option<Resume> {
        let get = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k)?.strip_prefix(' ').map(str::trim));
        let session = get("sesiune")?;
        if !session.starts_with("s-") || session.len() > 40 || !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        let minutes = get("minute")?.parse::<u64>().ok().filter(|m| (1..=MAX_MINUTES).contains(m))?;
        let from_list = match get("origine")? {
            "lista" => true,
            "mana" => false,
            _ => return None,
        };
        let tag = update_tag(get("versiune")?).ok()?;
        let nr = get("nr")?.parse::<u32>().ok().filter(|&n| n > 0)?;
        Some(Resume { session: String::from(session), minutes, from_list, tag, nr })
    }
}

/// What the stick journal of a session says about it: (ran, refused, resumes so far, a board command already taken).
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-ghpush")), allow(dead_code))]
pub fn journal_counts(text: &str) -> (u32, u32, u32, bool) {
    let (mut ran, mut refused, mut resumes, mut board) = (0, 0, 0, false);
    for l in text.lines() {
        let mut w = l.split_whitespace();
        let (Some(first), Some(second)) = (w.next(), w.next()) else { continue };
        if first == "#" && second == "reluata" {
            resumes += 1;
        } else if first.parse::<u32>().is_ok() {
            match second {
                "gata" => ran += 1,
                "refuz" => refused += 1,
                "incepe" if l.contains(" comanda test-placa") => board = true,
                _ => {}
            }
        }
    }
    (ran, refused, resumes, board)
}

/// P6-130: at boot, before the automatic list — a session that asked for the restart (`actualizeaza`) is resumed: the
/// same name, the same folder in the hub, the next number from the stick journal, `reluare-<k>.json` sent with the
/// version that now runs. Returns `true` when the machine should restart after it (the session came from the list).
#[cfg(not(feature = "qemu-test"))]
pub fn resume_if_asked() -> bool {
    let path = format!("{}/live/{}", dir(), RESUME);
    if fs::kind(&path).is_err() {
        return false;
    }
    // The new version on trial is confirmed while the session runs (it reached the hub): it is ready.
    crate::revenire::system_ready();
    match resume(POLL_MS, PULS_MS) {
        Ok((sum, from_list)) => {
            auto::say(&sum.line());
            if let Some(tag) = &sum.restart {
                auto::say(&format!("live: repornesc in {}; sesiunea {} se reia dupa pornire\n", tag, sum.session));
                crate::sched::sleep_ms(1_000);
                crate::power::reboot(None);
            }
            from_list
        }
        Err(why) => {
            auto::say(&format!("live: sesiunea nu s-a reluat: {}\n", why));
            false
        }
    }
}

/// QEMU (`ghpush`): the resume, right after a session that asked for the restart (no real restart in the test).
#[cfg(feature = "test-ghpush")]
pub fn test_resume() -> Result<(Summary, bool), String> {
    resume(1_000, 5_000)
}

#[cfg_attr(all(feature = "qemu-test", not(feature = "test-ghpush")), allow(dead_code))]
fn resume(poll_ms: u64, puls_ms: u64) -> Result<(Summary, bool), String> {
    let path = format!("{}/live/{}", dir(), RESUME);
    let text = String::from_utf8_lossy(&fs::read_all(&path).unwrap_or_default()).into_owned();
    // Read once: removed before anything else, so a crash below never resumes again.
    let _ = fs::remove(&path);
    let _ = fs::flush(&format!("{}/live", dir()));
    let r = Resume::parse(&text).ok_or_else(|| format!("{} nu se poate citi (sters, nimic reluat)", path))?;
    if ACTIVE.swap(true, Ordering::Acquire) {
        return Err(String::from("o sesiune vie ruleaza deja"));
    }
    BY_HAND.store(false, Ordering::Release);
    let out = resumed(&r, poll_ms, puls_ms);
    ACTIVE.store(false, Ordering::Release);
    out.map(|sum| (sum, r.from_list))
}

#[cfg_attr(all(feature = "qemu-test", not(feature = "test-ghpush")), allow(dead_code))]
fn resumed(r: &Resume, poll_ms: u64, puls_ms: u64) -> Result<Summary, String> {
    let (key, board_key) = preflight()?;
    let s = r.session.clone();
    let local = format!("{}/live/{}", dir(), s);
    let journal_path = format!("{}/jurnal.txt", local);
    let jtext = String::from_utf8_lossy(&fs::read_all(&journal_path).map_err(|_| format!("{} lipseste: nimic reluat", journal_path))?).into_owned();
    let gate = Gate::from_journal(&s, &jtext);
    if !gate.taken.contains(&r.nr) {
        return Err(format!("jurnalul sesiunii {} nu are comanda nr {} (actualizeaza): nimic reluat", s, r.nr));
    }
    let (ran, refused, resumes, board) = journal_counts(&jtext);
    BOARD_TAKEN.store(board, Ordering::SeqCst);
    let k = resumes + 1;
    let version = fs::read_all("/usb/VERSIUNE")
        .ok()
        .and_then(|v| String::from_utf8_lossy(&v).lines().next().map(|l| String::from(l.trim())))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| String::from("necunoscuta (/usb/VERSIUNE lipseste)"));
    journal(&journal_path, &format!("# reluata {} {} in versiunea {} (dupa actualizeaza {}, nr {}); astept nr {}", k, auto::now_text(), version, r.tag, r.nr, gate.next));
    let remote = format!("live/{}", s);
    let mut note = String::new();
    cyber_json::Value::object([
        ("forma", cyber_json::Value::str(FORM)),
        ("sesiune", cyber_json::Value::str(&s)),
        ("reluare", cyber_json::Value::int(k as i64)),
        ("ora", cyber_json::Value::str(&auto::now_text())),
        ("ceruta", cyber_json::Value::str(&r.tag)),
        ("versiune", cyber_json::Value::str(&version)),
        ("dupa_nr", cyber_json::Value::int(r.nr as i64)),
        ("astept_nr", cyber_json::Value::int(gate.next as i64)),
        ("placa", cyber_json::Value::Bool(board_key)),
        ("sistem", cyber_json::Value::str(&system_text())),
    ])
    .write_to(&mut note);
    let name = format!("reluare-{}.json", k);
    let note_path = format!("{}/{}", local, name);
    if !write_new(&note_path, note.as_bytes()) || !send(&note_path, &format!("{}/{}", remote, name)) {
        return Err(format!("sesiunea {} nu s-a putut relua in hub ({} nu a plecat)", s, name));
    }
    auto::say(&format!(
        "live: sesiunea {} RELUATA dupa repornire (a {}-a oara) in versiunea {}; astept nr {} la {} s\n",
        s,
        k,
        version,
        gate.next,
        poll_ms / 1000
    ));
    let puls_count = fs::read_all(&format!("{}/puls.json", local))
        .ok()
        .and_then(|b| cyber_json::parse(&String::from_utf8_lossy(&b)).ok())
        .and_then(|v| v.get("puls").and_then(Value::as_i64))
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let ctx = Ctx { s: s.clone(), local, remote, key, board_key, minutes: r.minutes, poll_ms, puls_ms, by_hand: false, journal_path };
    let sum = Summary { session: s, ran, refused, ..Summary::default() };
    Ok(serve(&ctx, gate, sum, puls_count))
}

/// P6-129 (C-159 step 3, part 1): the heartbeat of a session — `live/<sesiune>/puls.json`, rewritten every `PULS_MS`
/// (60 s) while the session lives and once more at its end (`stare: incheiata: …`). It is the only file the OS rewrites
/// (`gh-trimite --live` replaces only `…/puls.json`). One try per beat: a beat that does not leave is not repeated (the
/// next one comes in a minute); the copy on the stick is always the latest.
#[derive(Default)]
struct Puls {
    local: String,
    remote: String,
    every_ms: u64,
    at: u64,
    count: u32,
    unsent: u32,
}

impl Puls {
    fn maybe(&mut self, s: &str, gate: &Gate, sum: &Summary, now: u64) {
        if self.every_ms > 0 && now.saturating_sub(self.at) >= self.every_ms {
            self.send(s, gate, sum, None);
        }
    }

    fn send(&mut self, s: &str, gate: &Gate, sum: &Summary, end: Option<&str>) {
        self.count += 1;
        let text = puls_text(s, self.count, gate.next, sum, end);
        let ok = write_new(&self.local, text.as_bytes()) && spawn("/bin/gh-trimite", &["gh-trimite", "--live", &self.local, &self.remote]) == 0;
        if !ok {
            self.unsent += 1;
            if self.unsent == 1 || self.unsent % 10 == 0 {
                auto::say(&format!("live: pulsul {} nu a plecat ({} netrimise); mai incerc la urmatorul\n", self.count, self.unsent));
            }
        }
        self.at = crate::time::uptime_ms().unwrap_or(0);
    }
}

/// The text of `puls.json` (P6-129): which beat, the time, the number the OS waits for, what ran, and the system line.
pub fn puls_text(s: &str, count: u32, next: u32, sum: &Summary, end: Option<&str>) -> String {
    let state = match end {
        Some(why) if why.starts_with("repornire") => String::from(why),
        Some(why) => format!("incheiata: {}", why),
        None => String::from("merge"),
    };
    let mut out = String::new();
    cyber_json::Value::object([
        ("forma", cyber_json::Value::str(FORM)),
        ("sesiune", cyber_json::Value::str(s)),
        ("puls", cyber_json::Value::int(count as i64)),
        ("ora", cyber_json::Value::str(&auto::now_text())),
        ("stare", cyber_json::Value::str(&state)),
        ("astept_nr", cyber_json::Value::int(next as i64)),
        ("rulate", cyber_json::Value::int(sum.ran as i64)),
        ("refuzate", cyber_json::Value::int(sum.refused as i64)),
        ("sistem", cyber_json::Value::str(&system_text())),
    ])
    .write_to(&mut out);
    out
}

/// One command file (number `want`): the gate, the journal BEFORE it runs, the run, the answer. `true` = the session
/// ends (`gata`, or P6-130: `actualizeaza` put a new version on the stick — `sum.restart`).
fn one(ctx: &Ctx, raw: &str, want: u32, gate: &mut Gate, sum: &mut Summary, terminal: &mut Terminal) -> bool {
    let (local, remote, journal_path) = (ctx.local.as_str(), ctx.remote.as_str(), ctx.journal_path.as_str());
    let mut verdict = gate.check(&ctx.key, ctx.board_key, raw);
    // P6-118 (C-161 point 1): one board command per session (a correct `test-placa` form counts, whatever its keys say).
    if let Verdict::Run(cmd) = &verdict {
        if cmd.kind == "comanda" && matches!(super::ciclu::board_command(&cmd.text), Some(Ok(_))) && BOARD_TAKEN.swap(true, Ordering::SeqCst) {
            verdict = Verdict::Refused { nr: Some(cmd.nr), why: String::from(BOARD_ONCE) };
        }
    }
    let (answer_text, end) = match verdict {
        Verdict::Refused { nr, why } => {
            journal(journal_path, &format!("{} refuz {} ({})", want, auto::now_text(), why));
            gate.take(want);
            sum.refused += 1;
            auto::say(&format!("live: c-{:04} REFUZAT: {}\n", want, why));
            (answer(&gate.session, nr, raw, Err(&why)), false)
        }
        Verdict::Run(cmd) => {
            journal(journal_path, &format!("{} incepe {} {} {}", want, auto::now_text(), cmd.kind, shown_text(&cmd.kind, &cmd.text)));
            gate.take(want);
            auto::say(&format!("live: c-{:04} {}: {}\n", want, cmd.kind, shown_text(&cmd.kind, &cmd.text)));
            let t0 = crate::time::uptime_ms().unwrap_or(0);
            super::LAST_EXIT.store(0, Ordering::Relaxed);
            let output = match cmd.kind.as_str() {
                "comanda" => match super::ciclu::board_command(&cmd.text) {
                    Some(parsed) => board_command(parsed),
                    None => run_captured(&cmd.text),
                },
                "stare" => format!("{}\n", system_text()),
                // P6-111: the gate has already checked the text and the key names.
                "scrie" => terminal.keys(&typed_keys(&cmd.text).unwrap_or_default()),
                "tasta" => terminal.keys(&parse_keys(&cmd.text).unwrap_or_default()),
                "ecran" => terminal.look(),
                "captura" => capture(cmd.nr, local, remote),
                "muta" | "clic" => mouse(parse_mouse(&cmd.kind, &cmd.text).unwrap_or(MouseOrder { to: None, button: 0 })),
                "actualizeaza" => update(ctx, cmd.nr, &update_tag(&cmd.text).unwrap_or_default(), sum),
                "citeste" => read_file(cmd.nr, &read_path(&cmd.text, stick_root()).unwrap_or_default(), local, remote),
                "salveaza" => match save_order(&cmd.text, stick_root()) {
                    Ok((path, data)) => save_file(&path, &data),
                    Err(why) => why,
                },
                _ => String::from("sesiunea se incheie (gata)\n"),
            };
            let o = Outcome {
                code: super::LAST_EXIT.load(Ordering::Relaxed),
                ms: crate::time::uptime_ms().unwrap_or(0).saturating_sub(t0),
                output,
                screen: real_screen(),
                system: system_text(),
            };
            journal(journal_path, &format!("{} gata {} ms cod {}", want, o.ms, o.code));
            sum.ran += 1;
            (answer(&gate.session, Some(cmd.nr), raw, Ok(&o)), cmd.kind == "gata" || sum.restart.is_some())
        }
    };
    let name = file_name('r', want);
    let path = format!("{}/{}", local, name);
    if !(write_new(&path, answer_text.as_bytes()) && send(&path, &format!("{}/{}", remote, name))) {
        sum.unsent += 1;
        journal(journal_path, &format!("# {} netrimis (ramane pe stick: {})", name, path));
        auto::say(&format!("live: {} nu a plecat in hub; ramane pe stick\n", name));
    }
    end
}

/// P6-130 (C-159 step 3): `actualizeaza os-<N>` — `/bin/actualizeaza --versiune os-<N>` (that exact Release, every file
/// checked against SHA256SUMS, the running version kept in `boot/vechi/` for the automatic return). When the new version
/// is on the stick (on trial), the resume marker is written BEFORE the answer leaves and the session ends asking for the
/// restart; the stick already having that version (or a failed update) = only the answer, the session goes on.
fn update(ctx: &Ctx, nr: u32, tag: &str, sum: &mut Summary) -> String {
    let (code, printed, placed) = run_update(tag);
    let mut text = format!("-- actualizeaza --versiune {} (codul {})\n{}", tag, code, printed);
    super::LAST_EXIT.store(code, Ordering::Relaxed);
    if code != 0 {
        text.push_str("actualizarea nu s-a facut: versiunea de pe stick a ramas cea veche; sesiunea merge mai departe\n");
        return text;
    }
    if !placed {
        text.push_str(&format!("stick-ul are deja {}: nicio repornire; sesiunea merge mai departe\n", tag));
        return text;
    }
    let r = Resume { session: ctx.s.clone(), minutes: ctx.minutes, from_list: !ctx.by_hand, tag: String::from(tag), nr };
    let path = format!("{}/live/{}", dir(), RESUME);
    if !write_new(&path, r.text().as_bytes()) {
        super::LAST_EXIT.store(1, Ordering::Relaxed);
        text.push_str(&format!("{} nu s-a putut scrie: NU repornesc (versiunea noua porneste la urmatoarea pornire, fara reluarea sesiunii)\n", path));
        return text;
    }
    journal(&ctx.journal_path, &format!("# repornire ceruta {}: {} pus pe stick; reluarea in {}", auto::now_text(), tag, path));
    sum.restart = Some(String::from(tag));
    text.push_str(&format!(
        "versiunea {} e pe stick (in incercare: daca nu se confirma, revine singura cea veche); repornesc{}; sesiunea {} se reia dupa pornire cu nr {} (reluare-<k>.json in hub)\n",
        tag,
        if ctx.by_hand { " acum" } else { " la sfarsitul listei automate" },
        ctx.s,
        nr + 1
    ));
    text
}

/// QEMU (`ghpush`): `actualizeaza` played without a release server (the real program is tested in the `update` build).
#[cfg(feature = "test-ghpush")]
pub static FAKE_UPDATE: AtomicBool = AtomicBool::new(false);

/// (exit code, what it printed, a new version was put in place).
fn run_update(tag: &str) -> (i64, String, bool) {
    #[cfg(feature = "test-ghpush")]
    if FAKE_UPDATE.load(Ordering::Acquire) {
        return (0, format!("(proba QEMU) actualizeaza: {} pus pe stick, in incercare\n", tag), true);
    }
    let outer_programs = super::PROGRAM_CAPTURE.lock().take();
    super::program_capture_begin();
    let code = match crate::proc::spawn_path("/bin/actualizeaza", &["actualizeaza", "--versiune", tag]) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => {
            let _ = super::program_capture_end();
            *super::PROGRAM_CAPTURE.lock() = outer_programs;
            return (-1, format!("/bin/actualizeaza nu porneste: {}\n", e.message()), false);
        }
    };
    let printed = super::program_capture_end();
    *super::PROGRAM_CAPTURE.lock() = outer_programs;
    (code, printed, code == 0 && fs::kind("/usb/boot/incercare.txt").is_ok())
}

/// P6-127: `muta` / `clic` — mouse events in the same queue as the real mouse, read by the window server; only while the
/// desktop runs (waits up to 3 s for it, right after `scrie "desktop\n"`). `iesire` says where the cursor ended up.
fn mouse(order: MouseOrder) -> String {
    use crate::input::{MouseEvent, push_mouse};
    let start = crate::time::uptime_ms().unwrap_or(0);
    while !crate::ui::running() && crate::time::uptime_ms().unwrap_or(0).saturating_sub(start) < 3_000 {
        crate::sched::sleep_ms(100);
    }
    let Some(at) = crate::ui::cursor().filter(|_| crate::ui::running()) else {
        super::LAST_EXIT.store(1, Ordering::Relaxed);
        return String::from("mouse: desktopul nu ruleaza (porneste-l din terminalul viu: scrie \"desktop\\n\")\n");
    };
    let wait_cursor = |want: (usize, usize)| {
        let t0 = crate::time::uptime_ms().unwrap_or(0);
        while crate::ui::cursor() != Some(want) && crate::time::uptime_ms().unwrap_or(0).saturating_sub(t0) < 1_000 {
            crate::sched::sleep_ms(20);
        }
    };
    if let Some((x, y)) = order.to {
        let (mut dx, mut dy) = (x as i64 - at.0 as i64, y as i64 - at.1 as i64);
        while dx != 0 || dy != 0 {
            let (sx, sy) = (dx.clamp(-1000, 1000), dy.clamp(-1000, 1000));
            push_mouse(MouseEvent { dx: sx as i16, dy: sy as i16, buttons: 0 });
            dx -= sx;
            dy -= sy;
        }
        let (w, h) = crate::ui::screen_size().unwrap_or((usize::MAX, usize::MAX));
        wait_cursor((x.min(w.saturating_sub(1)), y.min(h.saturating_sub(1))));
    }
    if order.button != 0 {
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: order.button });
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 0 });
        crate::sched::sleep_ms(QUIET_MS);
    }
    let end = crate::ui::cursor().unwrap_or(at);
    let what = match order.button {
        1 => "; clic stanga",
        2 => "; clic dreapta",
        _ => "",
    };
    format!("mouse: cursorul la {} {}{}\n", end.0, end.1, what)
}

/// P6-127 (C-159 step 2): `captura` — the screen as a gray PNG reduced `CAPTURE_SCALE` times, on the stick and in the hub
/// next to the answers (`captura-NNNN.png`); `iesire` names it. Code 1 (and no file) without a screen or when too big.
/// P6-132: `citeste` — a folder = its list; a file = sent to the hub as `fisier-NNNN.bin` (a small text also shown whole).
fn read_file(nr: u32, path: &str, local: &str, remote: &str) -> String {
    let fail = |why: String| {
        super::LAST_EXIT.store(1, Ordering::Relaxed);
        format!("citeste: {}: {}\n", path, why)
    };
    match fs::kind(path) {
        Err(e) => return fail(String::from(e.message())),
        Ok(Kind::Dir) => {
            let mut entries = match fs::list(path) {
                Ok(v) => v,
                Err(e) => return fail(String::from(e.message())),
            };
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = format!("citeste: dosarul {} ({} intrari)\n", path, entries.len());
            for (name, kind) in entries {
                let full = if path.ends_with('/') { format!("{}{}", path, name) } else { format!("{}/{}", path, name) };
                match kind {
                    Kind::Dir => out.push_str(&format!("  {}/\n", name)),
                    Kind::File => out.push_str(&format!("  {}  {} octeti\n", name, fs::size(&full).unwrap_or(0))),
                }
            }
            return out;
        }
        Ok(Kind::File) => {}
    }
    let size = fs::size(path).unwrap_or(0) as usize;
    if size > READ_MAX {
        return fail(format!("{} octeti (cel mult {} prin canal)", size, READ_MAX));
    }
    let Ok(data) = fs::read_all(path) else { return fail(String::from("nu se poate citi")) };
    if carries_secret(&data, &super::raport::secrets()) {
        return fail(String::from("poarta octetii unei chei sau ai token-ului; nu pleaca in hub"));
    }
    let name = format!("fisier-{:04}.bin", nr);
    let copy = format!("{}/{}", local, name);
    if !write_new(&copy, &data) {
        return fail(format!("nu pot scrie copia {}", copy));
    }
    if !send(&copy, &format!("{}/{}", remote, name)) {
        return fail(format!("{} nu a plecat in hub (ramane pe stick: {})", name, copy));
    }
    let mut out = format!("citeste: {} -> {} ({} octeti, sha256 {})\n", path, name, data.len(), super::antrenare::sha256_hex(&data));
    match core::str::from_utf8(&data) {
        Ok(text) if data.len() <= SHOW_MAX => {
            out.push_str("== continut ==\n");
            out.push_str(text);
        }
        _ => out.push_str("(continutul doar in fisier: binar sau mai mare de 32 KiB)\n"),
    }
    out
}

/// P6-132: `salveaza` — the file written in `<stick>/schimb/` (the folder made if missing) and read back.
fn save_file(path: &str, data: &[u8]) -> String {
    let fail = |why: &str| {
        super::LAST_EXIT.store(1, Ordering::Relaxed);
        format!("salveaza: {}: {}\n", path, why)
    };
    let folder = path.rsplit_once('/').map_or("", |(d, _)| d);
    if fs::kind(folder).is_err() && fs::create(folder, Kind::Dir).is_err() {
        return fail("nu pot face dosarul");
    }
    let old = match fs::kind(path) {
        Ok(Kind::Dir) => return fail("e un dosar"),
        Ok(Kind::File) => Some(fs::size(path).unwrap_or(0)),
        Err(_) => None,
    };
    if !write_new(path, data) || fs::read_all(path).ok().as_deref() != Some(data) {
        return fail("scrierea nu s-a putut verifica");
    }
    let before = old.map_or(String::from("fisier nou"), |n| format!("inlocuit, avea {} octeti", n));
    format!("salveaza: {} ({} octeti, sha256 {}; {})\n", path, data.len(), super::antrenare::sha256_hex(data), before)
}

fn capture(nr: u32, local: &str, remote: &str) -> String {
    let fail = |why: &str| {
        super::LAST_EXIT.store(1, Ordering::Relaxed);
        format!("captura: {}\n", why)
    };
    let Some((w, h, gray)) = crate::gfx::console::capture_gray(CAPTURE_SCALE) else { return fail("fara ecran (nicio imagine de la pornire)") };
    let png = crate::gfx::png::gray(w, h, &gray);
    if png.len() > CAPTURE_MAX {
        return fail(&format!("imaginea are {} octeti (cel mult {})", png.len(), CAPTURE_MAX));
    }
    let name = format!("captura-{:04}.png", nr);
    let path = format!("{}/{}", local, name);
    if !write_new(&path, &png) {
        return fail(&format!("nu pot scrie {}", path));
    }
    if !send(&path, &format!("{}/{}", remote, name)) {
        return fail(&format!("{} nu a plecat in hub (ramane pe stick: {})", name, path));
    }
    format!("captura: {} ({} x {} pixeli, tonuri de gri, {} octeti; ecranul micsorat de {} ori)\n", name, w, h, png.len(), CAPTURE_SCALE)
}

/// QEMU test (P6-113, C-159 note of 21 Nov 13:00), with the live key already on the test disk at `<dir>/live.cheie`:
/// `cat` refuses it, a program cannot see it, and an answer whose text carries the key (raw or as hex) leaves without it.
#[cfg(feature = "test-ghpush")]
pub fn secret_test() {
    let path = format!("{}/live.cheie", dir());
    let raw = fs::read_all(&path).unwrap_or_default();
    let key = String::from(String::from_utf8_lossy(&raw).trim());
    assert!(key.len() >= 32, "test: live: cheia de proba lipseste");
    let shown = run_captured(&format!("cat {}", path));
    assert!(shown.contains("fisier secret") && !shown.contains(&key), "test: live: cat arata cheia: {}", shown);
    let canon = crate::proc::rights::canonical(&path).unwrap_or_default();
    assert!(crate::proc::rights::is_secret(&canon) && crate::proc::rights::is_kernel_only(&canon), "test: live: cheia nu e secreta");
    let hex: String = key.bytes().map(|b| format!("{:02x}", b)).collect();
    let o = Outcome { code: 0, ms: 1, output: format!("a {} b\n{}\n", key, hex), screen: format!("ecran {}", key), system: key.clone() };
    let text = answer("s-test", Some(1), "{}", Ok(&o));
    assert!(!text.contains(&key) && !text.contains(&hex) && !text.contains(&hex.to_ascii_uppercase()), "test: live: cheia in raspuns: {}", text);
    assert!(text.contains("[ascuns]"), "test: live: nimic ascuns: {}", text);
    crate::info!("test: live: cheia live.cheie e secreta (cat refuzat, ascunsa din raspuns, si ca hex) OK");
}

/// QEMU self-test (P6-108): the refusal probes the command centre counts (C-159 completion of 21 Nov 08:31) — a bad
/// signature, a wrong number, a repeated command, a board command without the key — and the echo of every answer.
#[cfg_attr(not(feature = "test-ghpush"), allow(dead_code))]
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
    // The same signature as tools/live-semneaza.py (Python's hmac), so the command centre's end matches ours.
    assert!(sign(&key, s, 1, "comanda", "hw") == "65628b225841c7f1d8e79dc3fdc4446005580ba569e6dd515534654e834bb2b4", "live: semnatura ca in Python");
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
    // P6-127: the forms of `muta` / `clic`.
    assert!(matches!(g.check(&key, false, &good(2, "clic", "10 10")), Verdict::Run(_)));
    assert!(parse_mouse("clic", "5 6 dreapta") == Ok(MouseOrder { to: Some((5, 6)), button: 2 }) && parse_mouse("clic", "") == Ok(MouseOrder { to: None, button: 1 }));
    assert!(parse_mouse("muta", "7 8") == Ok(MouseOrder { to: Some((7, 8)), button: 0 }));
    assert!(why(g.check(&key, false, &good(2, "muta", "10"))).starts_with("muta: forma"));
    assert!(why(g.check(&key, false, &good(2, "muta", "-1 5"))).starts_with("muta: x si y"));
    assert!(why(g.check(&key, false, &good(2, "clic", "mijloc"))).starts_with("clic: butonul"));
    // P6-111: the keys of `tasta` and `scrie`, checked at the gate; typed lines never write to the board.
    assert!(matches!(g.check(&key, false, &good(2, "tasta", "ctrl+c")), Verdict::Run(_)));
    assert!(parse_keys("ctrl+c Enter sus esc spatiu") == Ok(alloc::vec![Key::Ctrl('c'), Key::Enter, Key::Up, Key::Escape, Key::Char(' ')]));
    assert!(why(g.check(&key, false, &good(2, "tasta", "alt+tab"))).contains("alt+"));
    assert!(why(g.check(&key, false, &good(2, "tasta", "f13"))).starts_with("tasta necunoscuta: f13"));
    assert!(why(g.check(&key, false, &good(2, "tasta", " "))) == "nicio tasta");
    assert!(typed_keys("hw\n") == Ok(alloc::vec![Key::Char('h'), Key::Char('w'), Key::Enter]));
    assert!(why(g.check(&key, false, &good(2, "scrie", "a\u{1b}b"))).starts_with("scrie: textul are caractere de control"));
    assert!(why(g.check(&key, false, &good(2, "scrie", ""))) == "scrie: text gol");
    assert!(why(g.check(&key, false, &good(2, "scrie", &"x".repeat(MAX_TYPED + 1)))).starts_with("scrie: textul e prea lung"));
    assert!(matches!(g.check(&key, false, &good(2, "scrie", "gpu lant\n")), Verdict::Run(_)), "live: scrie trece poarta; randul e oprit de shell");
    assert!(typed_line("gpu lant").is_err() && typed_line("test-placa --p8").is_err() && typed_line("live 5").is_err() && typed_line("uptime").is_ok() && typed_line("gpu stare").is_ok());
    crate::gfx::png::self_test(); // P6-127
    let mut t = Terminal::default();
    t.feed("cyber> hx\x08w\npornit de 3 s\r\ncyber> ");
    assert!(t.screen() == "cyber> hw\npornit de 3 s\ncyber> ", "live: ecranul terminalului: {:?}", t.screen());
    for i in 0..60 {
        t.feed(&format!("rand {}\n", i));
    }
    assert!(t.lines.len() == SCREEN_LINES && t.screen().starts_with("rand 21\n"), "live: ecranul pastreaza ultimele randuri");
    assert!(why(g.check(&key, false, &good(2, "sterge", "x"))).starts_with("fel necunoscut"));
    // P6-132: the paths of `citeste` / `salveaza` (and the content of `salveaza`), checked at the gate.
    let root = stick_root();
    assert!(read_path(&format!("{}/auto/teste.txt", root), root) == Ok(format!("{}/auto/teste.txt", root)), "live: citeste");
    assert!(read_path(&format!("{}/hub/live.cheie", root), root).unwrap_err().contains("secret"), "live: citeste cheia");
    assert!(read_path(&format!("{}/HUB/LIVE.CHEIE", root), root).unwrap_err().contains("secret"), "live: citeste cheia (litere mari)");
    assert!(read_path(&format!("{}/hub/../hub/x", root), root).is_err() && read_path("auto/teste.txt", "/usb").is_err());
    assert!(read_path("/boot/x", "/usb").unwrap_err().contains("doar de pe stick") && read_path("/usbx/a", "/usb").is_err());
    let b64 = cyber_json::base64::encode(b"salut\n");
    assert!(save_order(&format!("/usb/schimb/nota.txt\n{}", b64), "/usb") == Ok((String::from("/usb/schimb/nota.txt"), b"salut\n".to_vec())));
    assert!(save_order(&format!("/USB/Schimb/n_1-a.txt\n{}\n", b64), "/usb").is_ok() && save_order("/usb/schimb/gol.txt\n", "/usb").is_ok());
    for bad in ["/usb/auto/teste.txt", "/usb/schimb/a/b.txt", "/usb/schimb/../auto/teste.txt", "/usb/schimb/.x", "/usb/schimb/a b", "/usb/schimb", "/usb/hub/schimb/x"] {
        assert!(save_order(&format!("{}\n{}", bad, b64), "/usb").is_err(), "live: salveaza {}", bad);
    }
    assert!(save_order("/usb/schimb/x.txt", "/usb").unwrap_err().starts_with("salveaza: forma"));
    assert!(save_order("/usb/schimb/x.txt\n@@@=", "/usb").unwrap_err().contains("base64"));
    assert!(save_order(&format!("/usb/schimb/x.txt\n{}", cyber_json::base64::encode(&[0u8; SAVE_MAX + 1])), "/usb").unwrap_err().contains("cel mult"));
    assert!(matches!(g.check(&key, false, &good(2, "salveaza", &format!("{}/schimb/n.txt\n{}", root, b64))), Verdict::Run(_)));
    assert!(why(g.check(&key, false, &good(2, "citeste", &format!("{}/hub/live.cheie", root)))).contains("secret"));
    assert!(why(g.check(&key, false, &good(2, "salveaza", &format!("{}/auto/teste.txt\n{}", root, b64)))).contains("doar direct in"));
    assert!(shown_text("salveaza", &format!("/usb/schimb/n.txt\n{}", b64)) == "/usb/schimb/n.txt (8 caractere base64)");
    assert!(carries_secret(b"x ghp_abc y", &[]) && carries_secret(b"aa-cheie-12345678-bb", &[String::from("cheie-12345678")]) && !carries_secret(b"salut", &[]));
    assert!(why(g.check(&key, false, &good(2, "comanda", "live 5"))).starts_with("o sesiune vie nu porneste"));
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
