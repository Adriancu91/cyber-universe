//! The "black box" (task P6-22, command C-22): `raport` writes one text file on the stick with everything a hardware
//! test needs (version, `hw`, `lspci`, `net`, the `net`/`usb`/`pci`/`gpu` logs with the drivers' register lines, the
//! whole kept log), so results no longer travel as photos of the screen. The shell also writes one automatically,
//! before its prompt, when a new ERR line appeared in the log (at most `AUTO_MAX` per boot).
//!
//! Secrets never reach the file: the contents of the key files are replaced, and so is anything that looks like a
//! key or token (known prefixes, `Bearer`, `Authorization:`), line by line, before writing.
//!
//! A panic is not reported here: a panicked kernel may hold the disk or USB locks, and writing a FAT volume from
//! that state could damage the stick. The panic message stays on the screen and on the serial port.
//!
//! Idea: crash-dump / "pstore" style reports (Hubris `jefe` keeps failures for later reading); our own code.

use super::{emit_capture_begin, emit_capture_end};
use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Files whose contents are secret; they are read only to hide them.
const SECRET_FILES: &[&str] =
    &["/usb/claude.key", "/usb/github.token", "/disk/claude.key", "/disk/github.token", "/etc/claude.key", "/etc/github.token",
      "/usb/remote.cheie", "/disk/remote.cheie", "/etc/remote.cheie"];
/// Prefixes of keys and tokens: the prefix and the rest of the word are hidden.
/// Header names (ending in ':') come first: their whole value, to the end of the line, is hidden.
const SECRET_PREFIXES: &[&str] = &[
    "Authorization:", "authorization:", "x-api-key:", "X-Api-Key:", "sk-ant-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_",
    "github_pat_", "Bearer ", "bearer ",
];
const HIDDEN: &str = "[ascuns]";
/// Automatic reports per boot.
const AUTO_MAX: usize = 3;

/// ERR lines already covered by a report (manual or automatic).
static REPORTED_ERRORS: AtomicUsize = AtomicUsize::new(0);
static AUTO_DONE: AtomicUsize = AtomicUsize::new(0);
/// One report at a time (the shells of several terminal windows may ask together).
static WRITING: crate::sched::Mutex<()> = crate::sched::Mutex::new(());

/// Where reports go: the stick; in the `raport` test build also the test disk (QEMU has no writable stick there).
fn destinations() -> &'static [&'static str] {
    #[cfg(feature = "test-raport")]
    return &["/usb", "/disk"];
    #[cfg(not(feature = "test-raport"))]
    return &["/usb"];
}

/// `raport`: writes a report now; prints where; then sends the new reports to the hub (when there is a token).
pub fn manual() {
    match write_report("manual (comanda raport)") {
        Ok((path, bytes)) => {
            super::emit(&format!("raport: scris {} ({} octeti); cheile si tokenurile sunt ascunse\n", path, bytes));
            send_to_hub();
        }
        Err(why) => super::emit(&format!("raport: nu s-a scris: {}\n", why)),
    }
}

/// The reports folder and the token file `send_to_hub` looks at (the `ghpush` test build has no stick: /etc).
fn hub_paths() -> (&'static str, &'static str) {
    #[cfg(feature = "test-ghpush")]
    return ("/etc/raport", "/etc/github.token");
    #[cfg(not(feature = "test-ghpush"))]
    return ("/usb/raport", "/usb/github.token");
}

/// C-84 item 1 (P6-61): the reports on the stick not yet sent go to the hub (`rezultate/os-automat/<date>/`) with
/// `/bin/gh-trimite --rapoarte`, so Adrian does not copy them in Windows. Only with a token on the stick (made by Adrian,
/// minimal rights); without one, or without network, it says so and the reports stay on the stick. Called after
/// `raport`, after `gpu lant`, and by the command `trimite`.
pub fn send_to_hub() {
    let (dir, token) = hub_paths();
    if fs::kind(dir).is_err() {
        super::emit(&format!("trimite: {} nu exista: niciun raport de trimis\n", dir));
        return;
    }
    if fs::kind(token).is_err() {
        super::emit(&format!("trimite: fara {}: rapoartele raman pe stick in {} (pasii pentru token: docs/USB.md)\n", token, dir));
        return;
    }
    if crate::net::config().is_none() || !crate::net::wait_ready(10_000) {
        super::emit("trimite: reteaua nu are adresa: rapoartele raman pe stick (mai tarziu: comanda `trimite`)\n");
        return;
    }
    super::emit(&format!("trimite: trimit rapoartele noi din {} in hub...\n", dir));
    match crate::proc::spawn_path("/bin/gh-trimite", &["gh-trimite", "--rapoarte", dir]) {
        Ok(id) => {
            let code = crate::proc::wait(id);
            crate::info!("trimite: gh-trimite --rapoarte a iesit cu codul {}", code);
        }
        Err(e) => super::emit(&format!("trimite: /bin/gh-trimite nu porneste: {}\n", e.message())),
    }
}

/// Called by the shell before its prompt: a report when new ERR lines appeared since the last one.
pub fn auto_if_errors() {
    let errors = crate::log::error_count();
    if errors <= REPORTED_ERRORS.load(Ordering::Relaxed) || AUTO_DONE.load(Ordering::Relaxed) >= AUTO_MAX {
        return;
    }
    if !destinations().iter().any(|d| fs::list(d).is_ok()) {
        return; // no stick: nothing to write on (tried again after the next command)
    }
    AUTO_DONE.fetch_add(1, Ordering::Relaxed);
    let last = crate::log::recent().into_iter().rev().find(|l| l.contains(" ERR ")).unwrap_or_default();
    let reason = format!("automat (eroare in jurnal: {})", last.trim());
    match write_report(&reason) {
        Ok((path, _)) => super::emit(&format!("raport automat (eroare in jurnal): scris {}\n", path)),
        Err(why) => super::emit(&format!("raport automat: nu s-a scris: {}\n", why)),
    }
}

fn write_report(reason: &str) -> Result<(String, usize), String> {
    let _one = WRITING.lock();
    let errors_now = crate::log::error_count();
    let (dir, name) = pick_file()?;
    let text = hide_secrets(&build(reason), &secrets());
    let path = format!("{}/{}", dir, name);
    fs::create(&path, Kind::File).map_err(|e| format!("{}: {}", path, e.message()))?;
    let written = fs::write(&path, 0, text.as_bytes()).map_err(|e| format!("{}: {}", path, e.message()))?;
    fs::flush(&path).map_err(|e| format!("{}: {}", path, e.message()))?;
    REPORTED_ERRORS.fetch_max(errors_now, Ordering::Relaxed);
    crate::info!("raport: {} scris ({} octeti, {})", path, written, reason);
    Ok((path, written))
}

/// `<mount>/raport/` (created if missing) and the first free `AAAALLZZ-<nr>.txt`.
fn pick_file() -> Result<(String, String), String> {
    let mut last_err = String::from("stick-ul nu e montat (/usb)");
    for mount in destinations() {
        if fs::list(mount).is_err() {
            continue;
        }
        let dir = format!("{}/raport", mount);
        match fs::create(&dir, Kind::Dir) {
            Ok(()) | Err(FsError::AlreadyExists) => {}
            Err(FsError::ReadOnly) => {
                last_err = format!("{} e doar pentru citire (lipseste cyber.rw)", mount);
                continue;
            }
            Err(e) => {
                last_err = format!("{}: {}", dir, e.message());
                continue;
            }
        }
        let date = crate::drivers::rtc::now()
            .map(|t| format!("{:04}{:02}{:02}", t.year, t.month, t.day))
            .unwrap_or_else(|| String::from("00000000"));
        let existing: Vec<String> = fs::list(&dir).map_err(|e| e.message())?.into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
        for nr in 1..1000 {
            let name = format!("{}-{}.txt", date, nr);
            if !existing.contains(&name) {
                return Ok((dir, name));
            }
        }
        return Err(format!("{}: prea multe rapoarte (1000)", dir));
    }
    Err(last_err)
}

/// The report text (before hiding secrets).
pub(super) fn build(reason: &str) -> String {
    let mut t = String::new();
    t.push_str("== raport Cyber Universe OS ==\n");
    match crate::drivers::rtc::now() {
        Some(d) => t.push_str(&format!(
            "data (ceasul calculatorului): {:04}-{:02}-{:02} {:02}:{:02}:{:02}\n",
            d.year, d.month, d.day, d.hour, d.minute, d.second
        )),
        None => t.push_str("data: ceasul nu s-a citit\n"),
    }
    t.push_str(&format!("motiv: {}\n", reason));
    t.push_str(&format!("nucleu: cyber-universe {} (cyber-gpu {})\n", env!("CARGO_PKG_VERSION"), crate::drivers::gpu::CYBER_GPU_REV));
    for f in ["/usb/VERSIUNE", "/disk/VERSIUNE"] {
        if let Ok(v) = fs::read_all(f) {
            let v = String::from_utf8_lossy(&v);
            t.push_str(&format!("{}: {}\n", f, v.lines().next().unwrap_or("").trim()));
        }
    }
    t.push_str(&format!("erori in jurnal de la pornire: {}\n", crate::log::error_count()));
    section(&mut t, "hw", super::hw);
    section(&mut t, "lspci", super::lspci);
    section(&mut t, "net", super::net);
    let log = crate::log::recent();
    for (name, words) in [
        ("jurnal net (si registrele placilor de retea)", &["net:", "rtl", "e1000", "i219", "arp", "dhcp"][..]),
        ("jurnal usb", &["usb", "xhci"][..]),
        ("jurnal pci", &["pci:", "acpi:"][..]),
        ("jurnal gpu", &["gpu"][..]),
        ("erori si avertismente", &[" ERR ", " WARN "][..]),
    ] {
        t.push_str(&format!("\n== {} ==\n", name));
        let lines: Vec<&String> = log.iter().filter(|l| words.iter().any(|w| l.to_ascii_lowercase().contains(&w.to_ascii_lowercase()))).collect();
        if lines.is_empty() {
            t.push_str("(niciun rand)\n");
        }
        for l in lines {
            t.push_str(l);
            t.push('\n');
        }
    }
    t.push_str(&format!("\n== jurnal complet ({} randuri pastrate in memorie) ==\n", log.len()));
    for l in &log {
        t.push_str(l);
        t.push('\n');
    }
    t.push_str("\n== sfarsit ==\n");
    t
}

/// Runs a shell command with its output captured into the report.
fn section(t: &mut String, name: &str, f: fn()) {
    t.push_str(&format!("\n== {} ==\n", name));
    emit_capture_begin();
    f();
    t.push_str(&emit_capture_end());
}

/// The secret values (key and token files), trimmed; short ones are ignored (they would hide ordinary words).
pub(super) fn secrets() -> Vec<String> {
    let mut v = Vec::new();
    for f in SECRET_FILES {
        if let Ok(bytes) = fs::read_all(f) {
            for line in String::from_utf8_lossy(&bytes).lines() {
                let s = line.trim();
                if s.len() >= 8 {
                    v.push(String::from(s));
                }
            }
        }
    }
    v
}

/// Replaces every secret value, and every word that starts like a key or token, with `[ascuns]`.
pub fn hide_secrets(text: &str, secrets: &[String]) -> String {
    let mut out = String::from(text);
    for s in secrets {
        out = out.replace(s.as_str(), HIDDEN);
    }
    for p in SECRET_PREFIXES {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(p) {
            result.push_str(&rest[..i]);
            let after = &rest[i + p.len()..];
            // `Bearer `, `Authorization:` and `x-api-key:` keep their name; the value after optional spaces goes.
            let keep_name = p.ends_with(' ') || p.ends_with(':');
            let value_start = after.len() - after.trim_start_matches(' ').len();
            let value = &after[value_start..];
            let value_len = if p.ends_with(':') {
                value.find('\n').unwrap_or(value.len())
            } else {
                value.find(|c: char| c.is_whitespace() || c == '"' || c == '\'').unwrap_or(value.len())
            };
            if keep_name {
                result.push_str(p);
                result.push_str(&after[..value_start]);
            }
            result.push_str(HIDDEN);
            rest = &after[value_start + value_len..];
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// Boot-time self test of the filter (every test build).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let s = [String::from("cheie-secreta-1234")];
    let t = hide_secrets("a cheie-secreta-1234 b sk-ant-api03-XYZ c\nAuthorization: Bearer abc.def\nghp_abc123 ok", &s);
    assert!(
        t == "a [ascuns] b [ascuns] c\nAuthorization: [ascuns]\n[ascuns] ok",
        "raport: filtrul de chei a gresit: {:?}", t
    );
    crate::info!("test: filtrul de chei al raportului OK");
}
