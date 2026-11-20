//! The boot switch on the stick (task P6-102, command C-152 point 2).
//!
//! The stick's `boot/limine/limine.conf` has two entries: "Cyber Universe OS" (first) and "Windows" (Limine's
//! `efi_boot_entry`: it sets the firmware's `BootNext` to "Windows Boot Manager" and restarts). Which one starts
//! is the `default_entry:` line, which the OS rewrites with the `porneste` command:
//! - `porneste windows`: Windows from now on;
//! - `porneste os`: Cyber Universe, guarded. At every boot the OS first sets Windows as the default and, once the
//!   system reached the shell / desktop and ran 60 s, puts Cyber Universe back and writes the confirmation in
//!   `boot/pornire-ultima.txt`. A boot that never confirms (crash, hang, power cut) leaves Windows for the next boot;
//! - `porneste os --o-data`: Cyber Universe only at the next boot; at that boot the OS sets Windows right away;
//! - `porneste os --fara-garda`: Cyber Universe always, without the guard (as before P6-102).
//! The mode lives in `boot/porneste.txt` (`garda` or `o-data`; no file = no guard). A self-update (`actualizeaza`)
//! brings a fresh `limine.conf` (Cyber Universe first); the mode file then only re-arms the guard at the next boot.
//! Limit: a kernel that fails before it mounts the stick cannot switch anything; with `garda` the default is already
//! Windows by then only if a previous boot of this mode ran far enough.
//! Sources: Limine CONFIG.md (`default_entry`, `efi_boot_entry`), UEFI 2.10 §3.1 (BootNext). Our own code.

use crate::fs;
use crate::revenire::{now_text, write_file};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

pub const CONF: &str = "boot/limine/limine.conf";
const MODE: &str = "boot/porneste.txt";
const LAST: &str = "boot/pornire-ultima.txt";
const WINDOWS: &str = "Windows";
const CONFIRM_AFTER_MS: u64 = 60_000;

static CONFIRM_MS: AtomicU64 = AtomicU64::new(CONFIRM_AFTER_MS);
static GUARD_DIR: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);

/// What `porneste` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Windows,
    OsGuarded,
    OsOnce,
    OsUnguarded,
}

fn path(dir: &str, rel: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), rel)
}

/// The top-level entry names, in order (Limine numbers them from 1).
fn entries(conf: &str) -> Vec<&str> {
    conf.lines().filter(|l| l.starts_with('/') && !l.starts_with("//")).map(|l| l[1..].trim()).collect()
}

/// The 1-based index of the default entry (`default_entry:`; Limine's default is 1).
fn default_index(conf: &str) -> usize {
    for l in conf.lines() {
        if l.starts_with('/') {
            break;
        }
        let t = l.trim();
        if let Some(v) = t.strip_prefix("default_entry:") {
            return v.trim().parse().unwrap_or(1);
        }
    }
    1
}

/// `conf` with exactly one `default_entry: <index>` line, just before the first entry.
fn with_default(conf: &str, index: usize) -> String {
    let mut out = String::new();
    let mut placed = false;
    for l in conf.lines() {
        let global = !placed && !l.starts_with('/');
        if global && l.trim().starts_with("default_entry:") {
            continue;
        }
        if !placed && l.starts_with('/') {
            out.push_str(&format!("default_entry: {}\n", index));
            placed = true;
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn read_text(p: &str) -> Option<String> {
    fs::read_all(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Rewrites the stick's limine.conf so that entry `name` starts by default: written as `.nou`, read back, swapped in.
fn set_default(dir: &str, name: &str) -> Result<(), String> {
    let conf_path = path(dir, CONF);
    let conf = read_text(&conf_path).ok_or_else(|| format!("{} nu se poate citi", conf_path))?;
    let list = entries(&conf);
    let index = list.iter().position(|e| *e == name).map(|i| i + 1).ok_or_else(|| {
        format!("{} nu are intrarea \"{}\" (stick vechi: pune pe el versiunea noua)", CONF, name)
    })?;
    if default_index(&conf) == index {
        return Ok(());
    }
    let new = with_default(&conf, index);
    let tmp = format!("{}.nou", conf_path);
    write_file(&tmp, new.as_bytes())?;
    if read_text(&tmp).as_deref() != Some(new.as_str()) {
        let _ = fs::remove(&tmp);
        return Err(format!("{}: ce am citit inapoi nu e ce am scris", tmp));
    }
    let _ = fs::remove(&conf_path);
    fs::rename(&tmp, &conf_path).map_err(|e| format!("{}: {}", conf_path, e.message()))?;
    fs::flush(&path(dir, "boot/limine")).map_err(|e| format!("golirea discului: {}", e.message()))
}

fn os_name(conf: &str) -> Option<String> {
    entries(conf).first().map(|s| String::from(*s))
}

fn set_mode(dir: &str, mode: Option<&str>) -> Result<(), String> {
    let p = path(dir, MODE);
    match mode {
        Some(m) => write_file(&p, format!("{}\n", m).as_bytes()),
        None => {
            let _ = fs::remove(&p);
            fs::flush(&path(dir, "boot")).map_err(|e| format!("golirea discului: {}", e.message()))
        }
    }
}

fn mode(dir: &str) -> Option<String> {
    read_text(&path(dir, MODE)).map(|t| String::from(t.lines().next().unwrap_or("").trim()))
}

/// `porneste <...>`: changes the switch on the stick mounted at `dir`. Returns the line to show.
pub fn choose(dir: &str, choice: Choice) -> Result<String, String> {
    let conf = read_text(&path(dir, CONF)).ok_or_else(|| format!("nu gasesc {} pe stick ({})", CONF, dir))?;
    let os = os_name(&conf).ok_or_else(|| format!("{} nu are nicio intrare", CONF))?;
    match choice {
        Choice::Windows => {
            set_default(dir, WINDOWS)?;
            set_mode(dir, None)?;
            Ok(String::from("porneste: de acum stick-ul porneste Windows"))
        }
        Choice::OsGuarded => {
            set_default(dir, &os)?;
            set_mode(dir, Some("garda"))?;
            Ok(String::from("porneste: stick-ul porneste Cyber Universe, cu garda (fara confirmare in 60 s, pornirea urmatoare intra in Windows)"))
        }
        Choice::OsOnce => {
            set_default(dir, &os)?;
            set_mode(dir, Some("o-data"))?;
            Ok(String::from("porneste: o singura data Cyber Universe, apoi Windows"))
        }
        Choice::OsUnguarded => {
            set_default(dir, &os)?;
            set_mode(dir, None)?;
            Ok(String::from("porneste: stick-ul porneste Cyber Universe, fara garda"))
        }
    }
}

/// `porneste` without arguments: the default entry, the mode and the last confirmation.
pub fn status(dir: &str) -> String {
    let Some(conf) = read_text(&path(dir, CONF)) else {
        return format!("porneste: nu gasesc {} pe stick ({})", CONF, dir);
    };
    let list = entries(&conf);
    let index = default_index(&conf);
    let name = list.get(index.wrapping_sub(1)).copied().unwrap_or("?");
    let mode = match mode(dir).as_deref() {
        Some("garda") => "cu garda (Windows daca pornirea nu se confirma)",
        Some("o-data") => "o singura data, apoi Windows",
        Some(_) => "necunoscut",
        None => "fara garda",
    };
    let last = read_text(&path(dir, LAST)).map(|t| String::from(t.trim())).unwrap_or_else(|| String::from("(niciuna)"));
    format!(
        "porneste: implicit \"{}\" ({} din {}: {}); mod: {}; ultima confirmare: {}",
        name,
        index,
        list.len(),
        list.join(", "),
        mode,
        last
    )
}

/// At boot, after the stick is mounted (before anything that may hang or restart): acts on the mode file.
pub fn at_boot(dir: &str) {
    let Some(m) = mode(dir) else { return };
    match m.as_str() {
        "o-data" => {
            match set_default(dir, WINDOWS).and_then(|_| set_mode(dir, None)) {
                Ok(()) => crate::info!("porneste: pornire o singura data; urmatoarea pornire intra in Windows"),
                Err(e) => crate::error!("porneste: nu pot pune Windows pentru pornirea urmatoare: {}", e),
            }
            confirm_line(dir, "o singura data");
        }
        "garda" => match set_default(dir, WINDOWS) {
            Ok(()) => {
                crate::info!(
                    "porneste: garda: Windows pana la confirmare (dupa {} s de mers revine Cyber Universe)",
                    CONFIRM_MS.load(Ordering::Relaxed) / 1000
                );
                *GUARD_DIR.lock() = Some(String::from(dir));
            }
            Err(e) => crate::error!("porneste: garda nu se poate arma: {}", e),
        },
        other => {
            crate::warn!("porneste: {} are un mod necunoscut ({}); il sterg", MODE, other);
            let _ = set_mode(dir, None);
        }
    }
}

fn confirm_line(dir: &str, how: &str) {
    let line = format!("{} · {}\n", now_text(), how);
    if let Err(e) = write_file(&path(dir, LAST), line.as_bytes()) {
        crate::warn!("porneste: confirmarea nu s-a scris: {}", e);
    }
}

/// The system reached the shell / desktop (called by `main`): with the guard armed, confirms after the delay.
pub fn system_ready() {
    if GUARD_DIR.lock().is_some() {
        crate::sched::spawn_system(guard);
    }
}

fn guard() {
    while crate::time::uptime_ms().unwrap_or(0) < CONFIRM_MS.load(Ordering::Relaxed) {
        crate::sched::sleep_ms(500);
    }
    let Some(dir) = GUARD_DIR.lock().take() else { return };
    let conf = read_text(&path(&dir, CONF)).unwrap_or_default();
    let Some(os) = os_name(&conf) else { return };
    match set_default(&dir, &os) {
        Ok(()) => {
            confirm_line(&dir, "garda");
            crate::info!("porneste: pornirea confirmata; urmatoarea pornire ramane Cyber Universe");
        }
        Err(e) => crate::error!("porneste: confirmarea nu s-a scris: {}", e),
    }
}

/// The scenario `revenire` (test builds): the switch on the writable test stick mounted at /usb.
#[cfg(feature = "test-revenire")]
pub fn self_test() {
    let d = "/usb";
    let _ = fs::create("/usb/boot", fs::Kind::Dir);
    let _ = fs::create("/usb/boot/limine", fs::Kind::Dir);
    let conf = "timeout: 3\n\n/Cyber Universe OS\n    protocol: limine\n    path: boot():/boot/kernel\n    module_path: boot():/boot/initrd.tar\n\n/Windows\n    protocol: efi_boot_entry\n    entry: Windows Boot Manager\n";
    write_file(&path(d, CONF), conf.as_bytes()).expect("test: porneste: scriere");
    let get = || read_text(&path(d, CONF)).unwrap_or_default();
    assert!(default_index(&get()) == 1 && entries(&get()) == ["Cyber Universe OS", "Windows"], "test: porneste: conf de start");

    // 1. Windows from now on: one default_entry line, the entries untouched.
    choose(d, Choice::Windows).expect("test: porneste windows");
    let c = get();
    assert!(default_index(&c) == 2 && c.matches("default_entry:").count() == 1, "test: porneste windows: implicit 2");
    assert!(c.contains("    module_path: boot():/boot/initrd.tar\n") && c.ends_with("entry: Windows Boot Manager\n"), "test: porneste: intrarile atinse");
    assert!(fs::kind("/usb/boot/limine/limine.conf.nou").is_err(), "test: porneste: a ramas limine.conf.nou");
    crate::info!("test: porneste windows: implicit Windows");

    // 2. Once: OS now, Windows set at that boot.
    choose(d, Choice::OsOnce).expect("test: porneste os --o-data");
    assert!(default_index(&get()) == 1 && mode(d).as_deref() == Some("o-data"), "test: o-data: implicit 1");
    at_boot(d);
    assert!(default_index(&get()) == 2 && mode(d).is_none(), "test: o-data: la pornire trebuia Windows");
    at_boot(d);
    assert!(default_index(&get()) == 2, "test: o-data: a doua pornire ramane Windows");
    crate::info!("test: porneste o singura data: Cyber Universe, apoi Windows");

    // 3. Guarded: Windows while unconfirmed, OS again after the confirmation.
    choose(d, Choice::OsGuarded).expect("test: porneste os");
    assert!(default_index(&get()) == 1, "test: garda: implicit 1");
    CONFIRM_MS.store(crate::time::uptime_ms().unwrap_or(0) + 1500, Ordering::Relaxed);
    at_boot(d);
    assert!(default_index(&get()) == 2, "test: garda: pana la confirmare trebuia Windows");
    system_ready();
    let deadline = crate::time::uptime_ms().unwrap_or(0) + 10_000;
    while default_index(&get()) != 1 {
        assert!(crate::time::uptime_ms().unwrap_or(0) < deadline, "test: garda: confirmarea nu a venit");
        crate::sched::sleep_ms(100);
    }
    assert!(mode(d).as_deref() == Some("garda") && read_text(&path(d, LAST)).is_some_and(|t| t.contains("garda")), "test: garda: confirmarea lipseste");
    crate::info!("test: porneste garda: Windows pana la confirmare, apoi Cyber Universe");

    // 4. Unguarded, and a stick without the Windows entry refuses `porneste windows` without touching anything.
    choose(d, Choice::OsUnguarded).expect("test: porneste os --fara-garda");
    assert!(default_index(&get()) == 1 && mode(d).is_none(), "test: fara garda");
    let old = "timeout: 0\n\n/Cyber Universe OS\n    protocol: limine\n    path: boot():/boot/kernel\n";
    write_file(&path(d, CONF), old.as_bytes()).expect("test: porneste: scriere");
    assert!(choose(d, Choice::Windows).is_err() && get() == old, "test: stick vechi: refuz, nimic atins");
    crate::info!("{}", status(d));
    crate::info!("test: porneste OK");
}
