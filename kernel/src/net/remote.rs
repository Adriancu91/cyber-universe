//! The remote text console (task P6-49, F2 of the independence plan): the shell over the local network.
//!
//! `remote porneste [port]` (default 2323) reads the key from `remote.cheie` on the stick or on the disk (never in the
//! repository; without a key of at least 16 characters the service does not start), then listens on the TCP port.
//! A connection gets the line `CYBER-REMOTE 1`, then `cheie: `; the client sends the key and a newline. A wrong key is
//! answered with `GRESIT` and the connection is closed; after 3 wrong keys in a row every connection is refused for a
//! while (60 s; 3 s in test builds), and after 10 wrong keys in total the service stops itself. The key is compared in
//! constant time. With the right key (`OK ...`) the client gets its own shell, like a terminal window: what the shell
//! prints goes to the client, the bytes the client sends are keys typed in that shell. That shell runs only the
//! allow-listed commands (`shell::remote_allowed`: the automatic mode's read-only list — nothing that writes to the
//! video card, no `reboot`); every command and every refusal is logged (`log remote`). One session at a time;
//! `exit` ends it, `remote opreste` (local or remote) stops the service, 30 minutes without input close the session.
//! At boot the service starts by itself only if the file `remote.porneste` exists next to the key (the virtual machine).
//!
//! No encryption yet: meant for the virtual machine (QEMU `hostfwd` on 127.0.0.1) and the home network; TLS comes later.
//! Idea of a line-based protocol: RFC 854 (Telnet), much simplified. Our own code.

use super::tcp;
use crate::sched::{self, Mutex};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

pub const PORT_DEFAULT: u16 = 2323;
/// Where the key is looked for, in order.
pub const KEY_FILES: &[&str] = &["/usb/remote.cheie", "/disk/remote.cheie"];
const KEY_MIN: usize = 16;
const KEY_MAX: usize = 256;
const FAILS_IN_ROW: u32 = 3;
const FAILS_MAX: u32 = 10;
#[cfg(not(feature = "qemu-test"))]
const PAUSE_MS: u64 = 60_000;
#[cfg(feature = "qemu-test")]
const PAUSE_MS: u64 = 3_000;
const HANDSHAKE_MS: u64 = 15_000;
const IDLE_MS: u64 = 30 * 60_000;
pub const GREETING: &str = "CYBER-REMOTE 1\n";

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static PORT: AtomicU16 = AtomicU16::new(0);
static FAILS: AtomicU32 = AtomicU32::new(0);
static SESSIONS: AtomicU32 = AtomicU32::new(0);
static KEY: Mutex<Vec<u8>> = Mutex::new(Vec::new());
/// Where the key came from, and the client of the open session.
static KEY_FROM: Mutex<&'static str> = Mutex::new("");
static CLIENT: Mutex<Option<(super::Ipv4, u16)>> = Mutex::new(None);

fn ip(a: super::Ipv4) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

fn now_ms() -> u64 {
    crate::time::uptime_ms().unwrap_or(0)
}

/// The key from the first key file that exists (surrounding spaces and newlines removed).
fn read_key() -> Result<(Vec<u8>, &'static str), String> {
    for &path in KEY_FILES {
        let Ok(raw) = crate::fs::read_all(path) else { continue };
        let key: Vec<u8> = raw.trim_ascii().to_vec();
        if key.len() < KEY_MIN || key.len() > KEY_MAX || key.iter().any(|&b| !(0x21..0x7f).contains(&b)) {
            return Err(format!(
                "remote: cheia din {} nu e buna: trebuie {}-{} caractere vizibile, fara spatii (nu pornesc)",
                path, KEY_MIN, KEY_MAX
            ));
        }
        return Ok((key, path));
    }
    Err(format!(
        "remote: lipseste cheia: pune un fisier remote.cheie (cel putin {} caractere) pe stick sau pe disc (nu pornesc)",
        KEY_MIN
    ))
}

/// Starts the service on `port`. The text says what happened (for the shell).
pub fn start(port: u16) -> Result<String, String> {
    if RUNNING.load(Ordering::Acquire) {
        return Err(format!("remote: merge deja pe portul {}", PORT.load(Ordering::Relaxed)));
    }
    let (key, from) = read_key()?;
    tcp::listen(port).map_err(|e| format!("remote: nu pot asculta pe portul {}: {}", port, e.message()))?;
    *KEY.lock() = key;
    *KEY_FROM.lock() = from;
    PORT.store(port, Ordering::Relaxed);
    FAILS.store(0, Ordering::Relaxed);
    STOP.store(false, Ordering::Release);
    RUNNING.store(true, Ordering::Release);
    sched::spawn(service);
    let addr = super::config().map(|c| ip(c.ip)).unwrap_or_default();
    crate::info!("remote: pornit pe {}:{} (cheia din {})", addr, port, from);
    Ok(format!("remote: pornit pe {}:{}, cheia din {}; `remote opreste` il opreste", addr, port, from))
}

/// Files that ask for the service at boot (opt-in, next to the key); the content may be a port number.
pub const AUTOSTART_FILES: &[&str] = &["/usb/remote.porneste", "/disk/remote.porneste"];

/// At boot: starts the service in the background (once the network has an address) if `remote.porneste` exists,
/// so the virtual machine can be reached without typing anything in it. Without that file: nothing.
pub fn start_if_asked() {
    if AUTOSTART_FILES.iter().any(|p| crate::fs::kind(p).is_ok()) {
        sched::spawn(autostart);
    }
}

fn autostart() {
    let port = AUTOSTART_FILES
        .iter()
        .find_map(|p| crate::fs::read_all(p).ok())
        .and_then(|b| String::from_utf8_lossy(&b).trim().parse::<u16>().ok())
        .filter(|&p| p != 0)
        .unwrap_or(PORT_DEFAULT);
    if !super::wait_ready(30_000) {
        crate::warn!("remote: pornirea automata: reteaua nu are adresa (nu pornesc)");
        return;
    }
    crate::info!("remote: pornire automata (remote.porneste)");
    if let Err(msg) = start(port) {
        crate::warn!("{}", msg);
    }
}

/// Asks the service to stop (it closes the open session too). `false` if it was not running.
pub fn stop() -> bool {
    if !RUNNING.load(Ordering::Acquire) {
        return false;
    }
    STOP.store(true, Ordering::Release);
    true
}

pub fn running() -> bool {
    RUNNING.load(Ordering::Acquire)
}

pub fn status() -> String {
    if !running() {
        return String::from("remote: oprit (`remote porneste` il porneste; cheia: remote.cheie pe stick sau pe disc)");
    }
    let client = match *CLIENT.lock() {
        Some((a, p)) => format!("sesiune deschisa: {}:{}", ip(a), p),
        None => String::from("nicio sesiune deschisa"),
    };
    format!(
        "remote: merge pe portul {} (cheia din {}); {}; sesiuni: {}; chei gresite: {}",
        PORT.load(Ordering::Relaxed),
        *KEY_FROM.lock(),
        client,
        SESSIONS.load(Ordering::Relaxed),
        FAILS.load(Ordering::Relaxed)
    )
}

/// Compares the key without stopping at the first difference (the time does not tell how much was right).
fn same_key(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u32;
    for i in 0..KEY_MAX {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u32;
    }
    diff == 0
}

/// The service thread: takes one connection at a time until `remote opreste`.
fn service() {
    let port = PORT.load(Ordering::Relaxed);
    let mut in_row = 0u32;
    let mut paused_until = 0u64;
    while !STOP.load(Ordering::Acquire) {
        match tcp::accept(port, 200) {
            Ok(id) => connection(id, &mut in_row, &mut paused_until),
            Err(tcp::Error::Timeout) => {}
            Err(_) => sched::sleep_ms(200),
        }
    }
    tcp::unlisten(port);
    KEY.lock().fill(0);
    KEY.lock().clear();
    *CLIENT.lock() = None;
    RUNNING.store(false, Ordering::Release);
    crate::info!("remote: oprit");
}

fn send(id: u32, text: &str) -> bool {
    tcp::send(id, text.as_bytes(), 5000).is_ok()
}

/// Reads the key line. Returns the key and whatever came after its newline (keys typed early).
fn read_key_line(id: u32) -> Option<(Vec<u8>, Vec<u8>)> {
    let deadline = now_ms() + HANDSHAKE_MS;
    let mut got = Vec::new();
    while now_ms() < deadline && !STOP.load(Ordering::Acquire) {
        match tcp::recv(id, 512, 100) {
            Ok(d) if d.is_empty() => return None,
            Ok(d) => {
                got.extend_from_slice(&d);
                if let Some(i) = got.iter().position(|&b| b == b'\n') {
                    let rest = got[i + 1..].to_vec();
                    got.truncate(i);
                    return Some((got.trim_ascii().to_vec(), rest));
                }
                if got.len() > KEY_MAX + 2 {
                    return Some((got, Vec::new())); // too long: certainly wrong
                }
            }
            Err(tcp::Error::Timeout) => {}
            Err(_) => return None,
        }
    }
    None
}

fn connection(id: u32, in_row: &mut u32, paused_until: &mut u64) {
    let (addr, port) = tcp::peer(id).unwrap_or(([0; 4], 0));
    let who = format!("{}:{}", ip(addr), port);
    crate::info!("remote: conexiune de la {}", who);
    if now_ms() < *paused_until {
        let secs = (*paused_until - now_ms()).div_ceil(1000);
        send(id, &format!("{}REFUZAT: prea multe chei gresite; mai incearca peste {} s\n", GREETING, secs));
        tcp::close(id, 1000);
        crate::warn!("remote: {} refuzat: pauza dupa {} chei gresite ({} s ramase)", who, FAILS_IN_ROW, secs);
        return;
    }
    if !send(id, &format!("{}cheie: ", GREETING)) {
        tcp::close(id, 500);
        return;
    }
    let Some((key, early)) = read_key_line(id) else {
        crate::warn!("remote: {} nu a trimis cheia la timp", who);
        tcp::close(id, 500);
        return;
    };
    let good = same_key(&key, &KEY.lock());
    drop(key);
    if !good {
        *in_row += 1;
        let total = FAILS.fetch_add(1, Ordering::Relaxed) + 1;
        send(id, "GRESIT\n");
        tcp::close(id, 1000);
        crate::warn!("remote: cheie gresita de la {} ({} la rand, {} in total)", who, in_row, total);
        if *in_row >= FAILS_IN_ROW {
            *in_row = 0;
            *paused_until = now_ms() + PAUSE_MS;
            crate::warn!("remote: pauza {} s: refuz orice conexiune", PAUSE_MS / 1000);
        }
        if total >= FAILS_MAX {
            crate::warn!("remote: {} chei gresite: opresc serviciul (`remote porneste` il reporneste)", total);
            STOP.store(true, Ordering::Release);
        }
        return;
    }
    *in_row = 0;
    SESSIONS.fetch_add(1, Ordering::Relaxed);
    *CLIENT.lock() = Some((addr, port));
    crate::info!("remote: sesiune deschisa pentru {}", who);
    if send(id, "OK: Cyber Universe OS. Doar comenzile din lista alba (`help`); `exit` inchide sesiunea.\n") {
        session(id, &early);
    }
    *CLIENT.lock() = None;
    tcp::close(id, 2000);
    crate::info!("remote: sesiune inchisa ({})", who);
}

/// Bytes from the client become keys typed in the session's shell (`\r` dropped, invalid UTF-8 replaced).
fn type_keys(me: u64, term: u64, bytes: &[u8]) {
    let text: String = String::from_utf8_lossy(bytes).chars().filter(|&c| c != '\r').collect();
    if !text.is_empty() {
        crate::term::send_keys(me, term, &text);
    }
}

/// The shell of one client: its output goes out, the client's bytes come in as keys.
fn session(id: u32, early: &[u8]) {
    let me = sched::current_id();
    let term = crate::term::open_restricted(me);
    type_keys(me, term, early);
    let mut buf = [0u8; 2048];
    let mut last_input = now_ms();
    'session: loop {
        loop {
            match crate::term::read_output(me, term, &mut buf) {
                Some(0) => break,
                Some(n) => {
                    if tcp::send(id, &buf[..n], 5000).is_err() {
                        break 'session;
                    }
                }
                None => break 'session, // `exit`: the shell ended and everything was sent
            }
        }
        if STOP.load(Ordering::Acquire) {
            send(id, "\nremote: serviciul a fost oprit\n");
            break;
        }
        match tcp::recv(id, 512, 50) {
            Ok(d) if d.is_empty() => break, // the client closed
            Ok(d) => {
                last_input = now_ms();
                type_keys(me, term, &d);
            }
            Err(tcp::Error::Timeout) => {}
            Err(_) => break,
        }
        if now_ms() - last_input > IDLE_MS {
            send(id, "\nremote: 30 de minute fara nimic tastat: inchid sesiunea\n");
            break;
        }
    }
    crate::term::close_owned(me);
}

/// Self-test of the pieces that need no network: the key comparison and the allow-list.
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    assert!(same_key(b"cheie-lunga-de-test", b"cheie-lunga-de-test"));
    assert!(!same_key(b"cheie-lunga-de-test", b"cheie-lunga-de-tesT"));
    assert!(!same_key(b"cheie-lunga-de-test", b"cheie-lunga-de-tes"));
    assert!(!same_key(b"", b"x"));
    use crate::shell::remote_allowed;
    for ok in ["help", "uptime", "mem", "ls /disk", "log remote", "gpu identifica", "remote stare", "remote opreste", "exit", "log --sumar", "log --avertismente", "timp uptime", "timp remote stare", "compara /usb/a /usb/b"] {
        assert!(remote_allowed(ok).is_ok(), "remote: `{}` trebuia permisa", ok);
    }
    for bad in ["gpu i6 --scrie", "gpu lant", "reboot", "remote porneste", "run desktop", "mod install x", "tastatura us", "rtl porneste", "timp reboot", "timp gpu lant", "timp exit", "timp timp mem", "timp remote porneste"] {
        assert!(remote_allowed(bad).is_err(), "remote: `{}` trebuia refuzata", bad);
    }
    crate::info!("test: remote: cheia si lista alba OK");
}
