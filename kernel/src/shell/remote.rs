//! Remote text access (task P6-49, command C-68 F2): the shell's commands over the network, without Windows in between.
//!
//! A system thread listens on TCP port 2323. A client connects, sends the key (the first line), then command lines;
//! each allowed command runs like at the console and its output goes back. Rules:
//! - the service starts only when `/usb/remote.cheie` exists (a key of at least 16 characters, kept only on the
//!   stick / the VM disk; `remote cheie-noua` at the console makes one); without it nothing listens;
//! - the key is compared in constant time; a wrong key closes the connection and every further wrong key waits longer
//!   (2 s, 4 s, ...; from the third on, 60 s); the key never reaches the log or the reports;
//! - only the allow-list of the automatic mode (`auto::allowed`: commands that only read; nothing that writes to the
//!   video card, no `reboot`), plus `help`; `iesire` ends the session; one session at a time, closed after 10 minutes
//!   without a command.
//! In the virtual machine the port is reachable only from Windows itself (`hostfwd` on 127.0.0.1, vm-pornire.ps1).
//! Idea: line-based remote consoles (Telnet, RFC 854, only as the idea of a plain text protocol); our own code.

use crate::fs::{self, Kind};
use crate::net::tcp;
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const PORT: u16 = 2323;
pub const KEY_PATH: &str = "/usb/remote.cheie";
const KEY_MIN: usize = 16;
const MAX_LINE: usize = 256;
const KEY_WAIT_MS: u64 = 30_000;
const IDLE_MS: u64 = 10 * 60_000;

static RUNNING: AtomicBool = AtomicBool::new(false);
static KEY: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);
/// Wrong keys in a row (reset by a good one).
static FAILS: AtomicU32 = AtomicU32::new(0);
/// Sessions that ended (`iesire` or closed), for `remote` and the test.
pub static SESSIONS_DONE: AtomicU32 = AtomicU32::new(0);
static CLIENT: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);

/// The key from the text of a key file: the first line, trimmed; at least 16 characters, printable, no spaces.
pub fn parse_key(bytes: &[u8]) -> Result<String, &'static str> {
    let text = core::str::from_utf8(bytes).map_err(|_| "cheia nu e text")?;
    let key = text.lines().next().unwrap_or("").trim();
    if key.len() < KEY_MIN {
        return Err("cheia e prea scurta (minim 16 caractere)");
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("cheia are spatii sau caractere neobisnuite");
    }
    Ok(String::from(key))
}

/// At boot: starts the service when the stick / VM disk has a key file.
pub fn start_if_configured() {
    let Ok(bytes) = fs::read_all(KEY_PATH) else { return };
    match parse_key(&bytes) {
        Ok(key) => start(key),
        Err(why) => crate::warn!("remote: {}: {}; serviciul nu porneste", KEY_PATH, why),
    }
}

/// Starts the listening thread with `key` (once; a later call only replaces the key).
pub fn start(key: String) {
    *KEY.lock() = Some(key);
    if !RUNNING.swap(true, Ordering::AcqRel) {
        crate::sched::spawn_system(serve);
    }
}

fn serve() {
    if !crate::net::wait_ready(30_000) {
        crate::warn!("remote: reteaua nu are adresa; incerc din nou cand apare");
        while !crate::net::wait_ready(60_000) {}
    }
    if let Err(e) = tcp::listen(PORT) {
        crate::warn!("remote: portul {} nu se deschide: {}", PORT, e.message());
        RUNNING.store(false, Ordering::Release);
        return;
    }
    crate::info!("remote: astept conexiuni pe portul {} (cheia din {})", PORT, KEY_PATH);
    loop {
        let (id, ip, port) = match tcp::accept(PORT, 60_000) {
            Ok(c) => c,
            Err(tcp::Error::Timeout) => continue,
            Err(e) => {
                crate::warn!("remote: {}", e.message());
                crate::sched::sleep_ms(1000);
                continue;
            }
        };
        let who = format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port);
        session(id, &who);
        tcp::close(id, 2000);
        *CLIENT.lock() = None;
        SESSIONS_DONE.fetch_add(1, Ordering::Release);
    }
}

fn send(id: u32, text: &str) -> bool {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    tcp::send(id, crlf.as_bytes(), 10_000).is_ok()
}

/// Reads one line (without the line end). `None` = closed, timed out or too long.
fn read_line(id: u32, pending: &mut Vec<u8>, timeout_ms: u64) -> Option<String> {
    let deadline = crate::time::uptime_ms().unwrap_or(0) + timeout_ms;
    loop {
        // Line ends left over from the previous line (`\r\n`, or empty lines) are skipped.
        while matches!(pending.first(), Some(b'\r' | b'\n')) {
            pending.remove(0);
        }
        if let Some(i) = pending.iter().position(|&b| b == b'\n' || b == b'\r') {
            let line: Vec<u8> = pending.drain(..=i).take(i).collect();
            return Some(clean(&line));
        }
        if pending.len() > MAX_LINE {
            return None;
        }
        let left = deadline.saturating_sub(crate::time::uptime_ms().unwrap_or(0));
        if left == 0 {
            return None;
        }
        match tcp::recv(id, 512, left) {
            Ok(d) if d.is_empty() => return None,
            Ok(d) => pending.extend_from_slice(&d),
            Err(_) => return None,
        }
    }
}

/// Printable text of a line: Telnet negotiation (0xFF + 2 bytes) dropped, backspace applied.
fn clean(raw: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            0xFF => i += 2,
            0x08 | 0x7F => {
                out.pop();
            }
            b if (0x20..0x7F).contains(&b) => out.push(b as char),
            _ => {}
        }
        i += 1;
    }
    String::from(out.trim())
}

/// Same length and same bytes, without stopping at the first difference.
fn same_key(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn session(id: u32, who: &str) {
    let mut pending = Vec::new();
    if !send(id, "Cyber Universe - acces de la distanta (text).\nCheia: ") {
        return;
    }
    let Some(given) = read_line(id, &mut pending, KEY_WAIT_MS) else {
        crate::info!("remote: {} nu a trimis cheia", who);
        return;
    };
    let good = KEY.lock().as_deref().is_some_and(|k| same_key(k, &given));
    if !good {
        let n = FAILS.fetch_add(1, Ordering::AcqRel) + 1;
        let wait = if n >= 3 { 60_000 } else { 2000 * n as u64 };
        crate::warn!("remote: cheie gresita de la {} (a {}-a la rand); astept {} s", who, n, wait / 1000);
        send(id, "cheie gresita\n");
        crate::sched::sleep_ms(wait);
        return;
    }
    FAILS.store(0, Ordering::Release);
    *CLIENT.lock() = Some(String::from(who));
    crate::info!("remote: sesiune deschisa de la {}", who);
    send(id, "autentificat. Doar comenzile care citesc (ca bucla automata); `help` = lista, `iesire` = inchide.\n");
    loop {
        if !send(id, "cyber-remote> ") {
            break;
        }
        let Some(line) = read_line(id, &mut pending, IDLE_MS) else {
            crate::info!("remote: {}: conexiune inchisa sau 10 minute fara comanda", who);
            break;
        };
        match line.as_str() {
            "" => continue,
            "iesire" | "exit" | "quit" => {
                send(id, "la revedere\n");
                break;
            }
            "help" => {
                send(id, HELP);
                continue;
            }
            _ => {}
        }
        if let Err(why) = super::auto::allowed(&line) {
            crate::info!("remote: refuzat: {} ({})", line, why);
            send(id, &format!("REFUZAT: {}\n", why));
            continue;
        }
        crate::info!("remote: comanda: {}", line);
        super::emit_capture_begin();
        super::program_capture_begin();
        super::dispatch(&line);
        let mut text = super::emit_capture_end();
        text.push_str(&super::program_capture_end());
        let text = super::raport::hide_secrets(&text, &super::raport::secrets());
        if !send(id, &text) {
            break;
        }
    }
    crate::info!("remote: sesiune inchisa ({})", who);
}

const HELP: &str = "comenzi permise de la distanta (doar citire):\n  hw lspci lsusb net mem uptime raport\n  log [cuvant] [n]   ls [dir]   cat <fisier>\n  cpu [porneste]   gpu [identifica|plan|tur|gfw|vbios|fwsec|firmware|pregatire-i6]\n  run kronos ...   run kronos-calcul ...\n  iesire           inchide sesiunea\n";

/// The console command `remote [cheie-noua|porneste]`.
pub fn command(args: &[&str]) -> String {
    match args {
        [] => {
            let key = fs::kind(KEY_PATH).is_ok();
            let state = if RUNNING.load(Ordering::Acquire) { "pornit" } else { "oprit" };
            let client = CLIENT.lock().clone().unwrap_or_else(|| String::from("niciunul"));
            format!(
                "remote: serviciul {}, portul {}, cheie {}: {}; client acum: {}; sesiuni terminate: {}\n",
                state,
                PORT,
                KEY_PATH,
                if key { "exista" } else { "lipseste (remote cheie-noua)" },
                client,
                SESSIONS_DONE.load(Ordering::Acquire)
            )
        }
        ["cheie-noua"] => match new_key() {
            Ok(key) => {
                start(key.clone());
                format!(
                    "remote: cheie noua scrisa in {} si serviciul pornit pe portul {}.\nCHEIA (se arata o singura data, scrie-o undeva sigur): {}\n",
                    KEY_PATH, PORT, key
                )
            }
            Err(why) => format!("remote: cheia nu s-a putut scrie: {}\n", why),
        },
        ["porneste"] => match fs::read_all(KEY_PATH).map_err(|e| e.message()).and_then(|b| parse_key(&b)) {
            Ok(key) => {
                start(key);
                format!("remote: pornit pe portul {}\n", PORT)
            }
            Err(why) => format!("remote: {}: {}\n", KEY_PATH, why),
        },
        _ => String::from("folosire: remote [cheie-noua|porneste]\n"),
    }
}

/// 24 random characters from an alphabet without look-alikes, written to the key file.
fn new_key() -> Result<String, &'static str> {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzACDEFGHJKLMNPQRTUVWXY346789";
    let mut raw = [0u8; 24];
    crate::random::fill(&mut raw);
    let key: String = raw.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    let _ = fs::remove(KEY_PATH);
    fs::create(KEY_PATH, Kind::File).map_err(|e| e.message())?;
    fs::write(KEY_PATH, 0, key.as_bytes()).map_err(|e| e.message())?;
    fs::write(KEY_PATH, key.len() as u64, b"\n").map_err(|e| e.message())?;
    fs::flush(KEY_PATH).map_err(|e| e.message())?;
    Ok(key)
}

/// Boot-time self test (test builds): key parsing and comparison.
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    assert!(parse_key(b"scurta").is_err(), "remote: cheie scurta acceptata");
    assert!(parse_key(b"are spatii in mijloc 123").is_err(), "remote: cheie cu spatii acceptata");
    assert!(parse_key(b"  abcdefghijklmnop12  \r\na doua linie").as_deref() == Ok("abcdefghijklmnop12"), "remote: cheie citita gresit");
    assert!(same_key("abcdefghijklmnop", "abcdefghijklmnop"), "remote: aceeasi cheie");
    assert!(!same_key("abcdefghijklmnop", "abcdefghijklmnoq") && !same_key("abcdefghijklmnop", "abcdefghijklmno"), "remote: chei diferite");
    assert!(clean(b"\xff\xfb\x01upz\x08time ") == "uptime", "remote: curatarea randului");
    crate::info!("test: remote: cheia si randurile OK");
}
