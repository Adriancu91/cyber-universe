//! The automatic test mode (task P6-41, command C-53): the workstation runs read-only tests on its own.
//!
//! At boot, before the desktop, if `<stick>/auto/teste.txt` exists, the OS runs its commands one by one — only the ones on
//! the allow-list (nothing that writes to the video card: `gpu i6 --scrie` and any `--scrie` are refused) — writes
//! `auto/raport-<date>.txt` as it goes (a line "in lucru: <command>" before each command, then its output), renames the list
//! to `teste-facut-<date>.txt` so it never runs twice, and restarts the machine (Windows comes back: the stick was a one-time
//! boot).
//!
//! The safety clock: a system thread restarts the machine when the list has not finished in `ceas N` minutes (default 10).
//! The report already holds everything up to the stuck command. A marker `auto/in-lucru.txt` exists while the list runs:
//! if a boot finds it next to `teste.txt`, the last run was cut off — the list is not run again, the report gets
//! "INTRERUPT", the list is renamed, and the machine restarts (no endless loop).
//!
//! List format: one command per line; `#` starts a comment; `ceas N` = minutes for the safety clock (1-120).
//! Idea: unattended test runners that boot once into a test image (CI hardware labs); our own code.

use super::raport;
use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

/// Where the list and the reports live: the stick; in the `auto` test builds the test disk.
#[cfg(not(feature = "test-auto"))]
const DIR: &str = "/usb/auto";
#[cfg(feature = "test-auto")]
const DIR: &str = "/disk/auto";

const DEFAULT_MINUTES: u64 = 10;
const MAX_MINUTES: u64 = 120;
const MAX_COMMANDS: usize = 100;

/// Uptime (ms) after which the safety clock restarts the machine; 0 = not armed.
static DEADLINE_MS: AtomicU64 = AtomicU64::new(0);

/// Runs the list if there is one (never returns then: the machine restarts). Without a list: returns at once.
pub fn run_if_present() {
    let list = format!("{}/teste.txt", DIR);
    let Ok(bytes) = fs::read_all(&list) else { return };
    let marker = format!("{}/in-lucru.txt", DIR);
    let stamp = date_stamp();
    if let Ok(prev) = fs::read_all(&marker) {
        interrupted(&list, &marker, String::from_utf8_lossy(&prev).trim(), &stamp);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let plan = parse(&text);

    let report = free_name(&format!("{}/raport-{}", DIR, stamp));
    let mut out = Report::create(&report);
    say(&format!("auto: lista {}: {} comenzi, ceasul de siguranta {} {}\n", list, plan.commands.len(), plan.limit, plan.unit));
    arm(plan.limit_ms);
    let _ = fs::create(&marker, Kind::File).and_then(|_| fs::write(&marker, 0, report.as_bytes()).map(|_| ())).and_then(|_| fs::flush(&marker));

    out.add(&format!(
        "== bucla automata: raport Cyber Universe OS ==\nlista: {}\nnucleu: cyber-universe {} (cyber-gpu {})\n",
        list,
        env!("CARGO_PKG_VERSION"),
        crate::drivers::gpu::CYBER_GPU_REV
    ));
    if let Ok(v) = fs::read_all("/usb/VERSIUNE") {
        out.add(&format!("/usb/VERSIUNE: {}\n", String::from_utf8_lossy(&v).lines().next().unwrap_or("").trim()));
    }
    out.add(&format!("pornit: {} · ceasul de siguranta: {} {}\n", now_text(), plan.limit, plan.unit));
    for w in &plan.warnings {
        out.add(&format!("atentie: {}\n", w));
    }

    let start = crate::time::uptime_ms().unwrap_or(0);
    let (mut ran, mut refused) = (0usize, 0usize);
    let total = plan.commands.len();
    for (i, cmd) in plan.commands.iter().enumerate() {
        out.add(&format!("\n== [{}/{}] {} ==\n", i + 1, total, cmd));
        if let Err(why) = allowed(cmd) {
            refused += 1;
            say(&format!("auto: [{}/{}] REFUZAT: {} ({})\n", i + 1, total, cmd, why));
            out.add(&format!("REFUZAT: {} (modul automat doar citeste; nimic nu s-a rulat)\n", why));
            continue;
        }
        say(&format!("auto: [{}/{}] {}\n", i + 1, total, cmd));
        out.add(&format!("in lucru: {} (de la {})\n", cmd, now_text()));
        let t0 = crate::time::uptime_ms().unwrap_or(0);
        super::emit_capture_begin();
        super::program_capture_begin();
        super::dispatch(cmd);
        let mut text = super::emit_capture_end();
        text.push_str(&super::program_capture_end());
        let ms = crate::time::uptime_ms().unwrap_or(0).saturating_sub(t0);
        crate::print!("{}", text);
        crate::serial_print!("{}", text);
        out.add(&text);
        out.add(&format!("-- gata in {} ms\n", ms));
        ran += 1;
    }
    let secs = crate::time::uptime_ms().unwrap_or(0).saturating_sub(start) / 1000;
    let summary = format!("REZULTAT: {} rulate, {} refuzate, {} s", ran, refused, secs);
    out.add(&format!("\n== {} ==\n", summary));
    out.add(&raport::build("bucla automata (dupa lista)"));
    say(&format!("auto: {}; raport: {}\n", summary, report));
    out.add(&format!("\n{}\n", summary));
    finish(&list, &marker, &stamp);
}

/// The parsed list: commands, and the safety clock.
struct Plan {
    commands: Vec<String>,
    limit: u64,
    unit: &'static str,
    limit_ms: u64,
    warnings: Vec<String>,
}

fn parse(text: &str) -> Plan {
    let mut p = Plan { commands: Vec::new(), limit: DEFAULT_MINUTES, unit: "min", limit_ms: DEFAULT_MINUTES * 60_000, warnings: Vec::new() };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["ceas", n] => match n.parse::<u64>() {
                Ok(m) if (1..=MAX_MINUTES).contains(&m) => {
                    p.limit = m;
                    p.unit = "min";
                    p.limit_ms = m * 60_000;
                }
                _ => p.warnings.push(format!("`{}`: minute intre 1 si {}; raman {} min", line, MAX_MINUTES, p.limit)),
            },
            // Test builds: seconds, so the scenario with a stuck command ends quickly.
            #[cfg(feature = "test-auto")]
            ["ceas-test-secunde", n] if n.parse::<u64>().is_ok() => {
                p.limit = n.parse().unwrap_or(5);
                p.unit = "s";
                p.limit_ms = p.limit * 1000;
            }
            _ if p.commands.len() >= MAX_COMMANDS => {
                p.warnings.push(format!("peste {} comenzi; `{}` nu se ruleaza", MAX_COMMANDS, line));
            }
            _ => p.commands.push(String::from(line)),
        }
    }
    p
}

/// The allow-list: commands that only read (from the card, the computer, the stick). `Err` = why not.
pub fn allowed(line: &str) -> Result<(), &'static str> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.iter().any(|w| w.contains("--scrie")) {
        return Err("scrierile in placa video se fac doar cu Adrian de fata");
    }
    match words.as_slice() {
        [] => Err("rand gol"),
        ["hw" | "lspci" | "lsusb" | "net" | "mem" | "uptime" | "raport" | "analiza", ..] => Ok(()),
        ["log" | "ls" | "cat" | "compara", ..] => Ok(()),
        ["timp", "timp" | "exit", ..] => Err("timp masoara o singura comanda obisnuita"),
        ["timp", rest @ ..] if !rest.is_empty() => allowed(&rest.join(" ")),
        ["cpu", ..] => Ok(()),
        ["gpu"] => Ok(()),
        // P6-85 (C-116): the minimal wake-up writes the PCI Command register: only with Adrian there.
        ["gpu", "tur", rest @ ..] if rest.contains(&"--trezire") => {
            Err("gpu tur --trezire scrie in configuratia PCI a placii: doar cu Adrian de fata")
        }
        ["gpu", sub, ..] => match *sub {
            "identifica" | "gfw" | "vbios" | "fwsec" | "firmware" | "pregatire-i6" | "tur" | "plan" | "variante" => Ok(()),
            "lant" => Err("gpu lant scrie in placa video: doar cu Adrian de fata"),
            "i6" => Err("gpu i6 scrie in placa video: doar cu Adrian de fata"),
            "recupereaza" => Err("gpu recupereaza scrie in placa video: doar cu Adrian de fata"),
            _ => Err("comanda gpu nu e pe lista alba"),
        },
        ["run", "kronos", rest @ ..] if rest.iter().any(|a| matches!(*a, "--gpu" | "--gpu-sim")) => {
            Err("kronos --gpu scrie in placa video: doar cu Adrian de fata")
        }
        ["run", "kronos", rest @ ..] if rest.contains(&"--conversatie") => Err("kronos --conversatie asteapta tastatura: doar cu Adrian de fata"),
        ["run", "kronos" | "kronos-calcul", ..] => Ok(()),
        ["run", ..] => Err("programul nu e pe lista alba (doar kronos, kronos-calcul)"),
        ["reboot", ..] => Err("repornirea o face bucla, la final"),
        #[cfg(feature = "test-auto")]
        ["test-blocheaza" | "test-jurnal", ..] => Ok(()),
        _ => Err("comanda nu e pe lista alba"),
    }
}

/// The last run was cut off (safety clock or a hang): note it, rename the list, restart. Never runs the list again.
fn interrupted(list: &str, marker: &str, report: &str, stamp: &str) -> ! {
    say(&format!("auto: rularea trecuta a fost INTRERUPTA (raport {}); lista nu se mai ruleaza\n", report));
    let path = if report.starts_with(DIR) && fs::kind(report).is_ok() { String::from(report) } else { free_name(&format!("{}/raport-{}", DIR, stamp)) };
    let mut out = Report::open(&path);
    out.add(&format!(
        "\n== INTRERUPT ==\nrularea nu a terminat (ceasul de siguranta sau o blocare); comanda de mai sus marcata `in lucru` nu s-a incheiat.\n\
         gasit la pornirea din {}; lista redenumita, nu se reia.\n",
        now_text()
    ));
    finish(list, marker, stamp)
}

/// List renamed, marker removed, restart.
fn finish(list: &str, marker: &str, stamp: &str) -> ! {
    DEADLINE_MS.store(0, Ordering::Relaxed);
    let done = free_name(&format!("{}/teste-facut-{}", DIR, stamp));
    match fs::rename(list, &done) {
        Ok(()) => say(&format!("auto: lista redenumita: {}\n", done)),
        Err(e) => {
            // Without the rename the list would run at every boot from the stick: remove it instead.
            say(&format!("auto: redenumirea a esuat ({}); sterg lista\n", e.message()));
            let _ = fs::remove(list);
        }
    }
    let _ = fs::remove(marker);
    let _ = fs::flush(DIR);
    crate::info!("auto: lista terminata, repornesc");
    say("auto: repornesc\n");
    crate::power::reboot(None)
}

/// Starts the safety clock thread.
fn arm(ms: u64) {
    let now = crate::time::uptime_ms().unwrap_or(0);
    DEADLINE_MS.store(now + ms, Ordering::Relaxed);
    crate::sched::spawn_system(watchdog);
}

fn watchdog() {
    loop {
        crate::sched::sleep_ms(250);
        let deadline = DEADLINE_MS.load(Ordering::Relaxed);
        if deadline != 0 && crate::time::uptime_ms().unwrap_or(0) >= deadline {
            crate::error!("auto: ceasul de siguranta: lista nu a terminat la timp, repornesc");
            crate::serial_println!("auto: CEASUL DE SIGURANTA: repornesc");
            crate::power::reboot(None);
        }
    }
}

/// The report file, written in place: every piece is on the stick before the next command starts.
pub(super) struct Report {
    path: String,
    offset: u64,
    ok: bool,
}

impl Report {
    pub(super) fn create(path: &str) -> Report {
        let ok = fs::create(path, Kind::File).is_ok();
        if !ok {
            say(&format!("auto: raportul {} nu s-a putut crea (stick doar citire? lipseste cyber.rw)\n", path));
        }
        Report { path: String::from(path), offset: 0, ok }
    }

    fn open(path: &str) -> Report {
        match fs::size(path) {
            Ok(n) => Report { path: String::from(path), offset: n, ok: true },
            Err(_) => Report::create(path),
        }
    }

    pub(super) fn add(&mut self, text: &str) {
        if !self.ok {
            return;
        }
        let clean = raport::hide_secrets(text, &raport::secrets());
        match fs::write(&self.path, self.offset, clean.as_bytes()) {
            Ok(n) => self.offset += n as u64,
            Err(e) => {
                say(&format!("auto: scrierea raportului a esuat: {}\n", e.message()));
                self.ok = false;
                return;
            }
        }
        let _ = fs::flush(&self.path);
    }
}

/// `<base>.txt`, or `<base>-2.txt` … when it already exists.
pub(super) fn free_name(base: &str) -> String {
    let first = format!("{}.txt", base);
    if matches!(fs::kind(&first), Err(FsError::NotFound)) {
        return first;
    }
    for n in 2..100 {
        let name = format!("{}-{}.txt", base, n);
        if matches!(fs::kind(&name), Err(FsError::NotFound)) {
            return name;
        }
    }
    format!("{}-99.txt", base)
}

pub(super) fn date_stamp() -> String {
    crate::drivers::rtc::now()
        .map(|t| format!("{:04}{:02}{:02}-{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute))
        .unwrap_or_else(|| String::from("00000000-0000"))
}

pub(super) fn now_text() -> String {
    crate::drivers::rtc::now()
        .map(|t| format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second))
        .unwrap_or_else(|| String::from("ceasul nu s-a citit"))
}

/// Progress on the screen and the serial port (never into the report capture).
pub(super) fn say(text: &str) {
    crate::print!("{}", text);
    crate::serial_print!("{}", text);
}

/// Boot-time self test of the allow-list (every `auto` test build).
#[cfg(feature = "test-auto")]
pub fn self_test() {
    for ok in ["hw", "gpu", "gpu identifica", "gpu pregatire-i6", "gpu tur", "gpu plan", "run kronos --profil --n 64 Buna", "run kronos-calcul --cauta", "cpu porneste", "raport", "log gpu 40", "compara /usb/a.txt /usb/b.txt", "log --sumar", "log --avertismente 50", "timp uptime", "timp run kronos --n 16 Buna"] {
        assert!(allowed(ok).is_ok(), "auto: `{}` trebuia permis", ok);
    }
    for bad in ["gpu i6 --scrie", "gpu i6", "gpu lant", "gpu recupereaza", "gpu recupereaza --sim", "gpu lant --sim", "gpu pregatire-i6 --scrie", "reboot", "run fetch https://x", "http get x", "rtl", "run editor", "mod incarca x", "exit", "test-gata", "timp", "timp reboot", "timp gpu i6 --scrie", "timp timp uptime", "timp run editor", "run kronos --gpu Ana", "run kronos --n 3 --gpu-sim x", "timp run kronos --gpu", "run kronos --conversatie", "gpu tur --trezire", "timp gpu tur --trezire", "gpu tur --trezire --sim"] {
        assert!(allowed(bad).is_err(), "auto: `{}` trebuia refuzat", bad);
    }
    crate::info!("test: lista alba a modului automat OK");
}
