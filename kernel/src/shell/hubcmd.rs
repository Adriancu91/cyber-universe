//! Commands from the Cyber Hub, run by the OS (task P6-64, command C-87 item 2).
//!
//! Adrian (or the command centre for him) writes lines `- [ ] K-<nr>: <command>` in the hub file
//! `integrare/cyber-universe/os-comenzi.md`. `hub ruleaza` (or, at boot, `/usb/hub/la-pornire`) takes that file with
//! `/bin/gh-ia --hub-ia`, keeps the commands whose ID has not run yet (the IDs that ran are in `/usb/hub/facute.txt`),
//! runs only the allow-listed ones (the automatic mode's read-only list + `test tot` + `trimite`), each once, writes
//! `/usb/raport/hub-<date>.txt` as it goes and sends it to the hub (`trimite`). Nothing writes to the video card, deletes
//! or updates: those are not on the list (the `DA` Adrian types at the keyboard is never given from here).
//!
//! `hub ruleaza --arata` only shows what would run and what is refused; it notes nothing.
//! Idea: remotely queued jobs of hardware test labs; our own code.

use super::{auto, raport};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

/// The hub file with the commands (in our folder of the hub).
pub const HUB_FILE: &str = "integrare/cyber-universe/os-comenzi.md";
/// At most this many commands per run (the rest wait for the next one).
const MAX_RUN: usize = 20;

/// Where the local copy and the list of IDs that ran live: the stick; in the `ghpush` test build /etc (no stick there).
fn dir() -> &'static str {
    #[cfg(feature = "test-ghpush")]
    return "/etc/hub";
    #[cfg(not(feature = "test-ghpush"))]
    return "/usb/hub";
}

/// One run at a time (the boot thread and a shell may ask together).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// One command of the hub file.
#[derive(Debug, PartialEq)]
pub struct Order {
    pub id: String,
    pub command: String,
}

/// The open commands of the file: `- [ ] K-12: test tot`. `[x]` (done by hand), other lines and bad IDs are skipped.
pub fn parse(text: &str) -> Vec<Order> {
    let mut out: Vec<Order> = Vec::new();
    for raw in text.lines() {
        let Some(rest) = raw.trim().strip_prefix("- [ ] ") else { continue };
        let Some((id, command)) = rest.split_once(':') else { continue };
        let id = id.trim().trim_matches('`');
        let command = command.trim().trim_matches('`').trim();
        let good_id = id.strip_prefix("K-").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()));
        if !good_id || command.is_empty() || out.iter().any(|o| o.id == id) {
            continue;
        }
        out.push(Order { id: String::from(id), command: String::from(command) });
    }
    out
}

/// The allow-list for commands that come from the hub: what the automatic mode may run (it only reads), plus the
/// self-test and sending the reports. `Err` = why not.
pub fn allowed(line: &str) -> Result<(), &'static str> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.iter().any(|w| w.eq_ignore_ascii_case("da")) {
        return Err("`DA` il tasteaza doar Adrian, la calculator");
    }
    match words.as_slice() {
        ["test", "tot"] | ["trimite"] => Ok(()),
        ["hub", ..] => Err("o comanda din hub nu ruleaza alte comenzi din hub"),
        ["timp", "test", "tot"] => Ok(()),
        _ => auto::allowed(line),
    }
}

/// The IDs that already ran (first word of every line of `facute.txt`).
fn done_ids() -> Vec<String> {
    let text = fs::read_all(&format!("{}/facute.txt", dir())).unwrap_or_default();
    String::from_utf8_lossy(&text).lines().filter_map(|l| l.split_whitespace().next()).map(String::from).collect()
}

fn note_done(id: &str, what: &str) {
    let path = format!("{}/facute.txt", dir());
    if fs::kind(&path).is_err() {
        let _ = fs::create(&path, Kind::File);
    }
    let at = fs::size(&path).unwrap_or(0);
    let _ = fs::write(&path, at, format!("{} {} {}\n", id, auto::date_stamp(), what).as_bytes());
    let _ = fs::flush(&path);
}

/// `hub ruleaza [--arata]`.
pub fn command(args: &[&str]) {
    match args {
        [] => run(false),
        ["--arata"] => run(true),
        _ => super::emit("folosire: hub ruleaza [--arata]   (comenzile din hub, integrare/cyber-universe/os-comenzi.md)\n"),
    }
}

/// At boot, when `/usb/hub/la-pornire` exists: a system thread runs the hub's commands once the network has an address.
pub fn start_if_asked() {
    if fs::kind(&format!("{}/la-pornire", dir())).is_ok() {
        crate::info!("hub: {}/la-pornire: rulez comenzile din hub cand reteaua e gata", dir());
        crate::sched::spawn_system(|| run(false));
    }
}

fn run(only_show: bool) {
    if RUNNING.swap(true, Ordering::Acquire) {
        super::emit("hub ruleaza: deja ruleaza (alt terminal sau pornirea)\n");
        return;
    }
    run_inner(only_show);
    RUNNING.store(false, Ordering::Release);
}

fn run_inner(only_show: bool) {
    let (reports, token) = raport::hub_paths();
    if fs::kind(token).is_err() {
        super::emit(&format!("hub ruleaza: fara {}: nu pot citi hub-ul (privat); pasii pentru token: docs/USB.md\n", token));
        return;
    }
    if crate::net::config().is_none() || !crate::net::wait_ready(15_000) {
        super::emit("hub ruleaza: reteaua nu are adresa: nimic rulat (mai tarziu: `hub ruleaza`)\n");
        return;
    }
    let d = dir();
    if fs::kind(d).is_err() && fs::create(d, Kind::Dir).is_err() {
        super::emit(&format!("hub ruleaza: nu pot face {} (stick doar citire? lipseste cyber.rw)\n", d));
        return;
    }
    let local = format!("{}/os-comenzi.md", d);
    let got = match crate::proc::spawn_path("/bin/gh-ia", &["gh-ia", "--hub-ia", HUB_FILE, &local]) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => {
            super::emit(&format!("hub ruleaza: /bin/gh-ia nu porneste: {}\n", e.message()));
            return;
        }
    };
    if got != 0 {
        super::emit(&format!("hub ruleaza: {} nu s-a putut lua din hub: nimic rulat\n", HUB_FILE));
        return;
    }
    let text = String::from_utf8_lossy(&fs::read_all(&local).unwrap_or_default()).into_owned();
    let done = done_ids();
    let orders: Vec<Order> = parse(&text).into_iter().filter(|o| !done.contains(&o.id)).collect();
    if orders.is_empty() {
        super::emit("hub ruleaza: nicio comanda noua in hub\n");
        return;
    }
    let (now, later) = orders.split_at(orders.len().min(MAX_RUN));
    if only_show {
        super::emit(&format!("hub ruleaza --arata: {} comenzi noi (nimic rulat, nimic notat):\n", orders.len()));
        for o in now {
            match allowed(&o.command) {
                Ok(()) => super::emit(&format!("  {}: {}  -> s-ar rula\n", o.id, o.command)),
                Err(why) => super::emit(&format!("  {}: {}  -> REFUZAT: {}\n", o.id, o.command, why)),
            }
        }
        if !later.is_empty() {
            super::emit(&format!("  (+{} la rularea urmatoare)\n", later.len()));
        }
        return;
    }

    if fs::kind(reports).is_err() {
        let _ = fs::create(reports, Kind::Dir);
    }
    let path = auto::free_name(&format!("{}/hub-{}", reports, auto::date_stamp()));
    let mut out = auto::Report::create(&path);
    out.add(&format!(
        "== comenzi din hub: raport Cyber Universe OS ==\nfisier: {} (hub)\nnucleu: cyber-universe {} (cyber-gpu {})\npornit: {}\n\
         {} comenzi noi; ruleaza doar cele de pe lista alba (doar citiri, test tot, trimite)\n",
        HUB_FILE,
        env!("CARGO_PKG_VERSION"),
        crate::drivers::gpu::CYBER_GPU_REV,
        auto::now_text(),
        orders.len()
    ));
    let (mut ran, mut refused) = (0usize, 0usize);
    for (i, o) in now.iter().enumerate() {
        out.add(&format!("\n== [{}/{}] {}: {} ==\n", i + 1, now.len(), o.id, o.command));
        if let Err(why) = allowed(&o.command) {
            refused += 1;
            auto::say(&format!("hub: [{}/{}] {} REFUZAT: {} ({})\n", i + 1, now.len(), o.id, o.command, why));
            out.add(&format!("REFUZAT: {} (nimic nu s-a rulat)\n", why));
            note_done(&o.id, "refuzat");
            continue;
        }
        auto::say(&format!("hub: [{}/{}] {}: {}\n", i + 1, now.len(), o.id, o.command));
        // Noted before it runs: a command that hangs or restarts the machine is not run again at the next boot.
        note_done(&o.id, "rulat");
        let t0 = crate::time::uptime_ms().unwrap_or(0);
        super::emit_capture_begin();
        super::program_capture_begin();
        super::dispatch(&o.command);
        let mut text = super::emit_capture_end();
        text.push_str(&super::program_capture_end());
        let ms = crate::time::uptime_ms().unwrap_or(0).saturating_sub(t0);
        auto::say(&text);
        out.add(&text);
        out.add(&format!("-- gata in {} ms\n", ms));
        ran += 1;
    }
    if !later.is_empty() {
        out.add(&format!("\n{} comenzi raman pentru rularea urmatoare (cel mult {} odata)\n", later.len(), MAX_RUN));
    }
    let summary = format!("REZULTAT: {} rulate, {} refuzate", ran, refused);
    out.add(&format!("\n== {} ==\n", summary));
    super::emit(&format!("hub ruleaza: {}; raport: {}\n", summary, path));
    raport::send_to_hub();
}

/// Boot-time self test of the parser and the allow-list (`ghpush` and `auto` test builds).
#[cfg(any(feature = "test-ghpush", feature = "test-auto"))]
pub fn self_test() {
    let text = "# Comenzi\n- [ ] K-1: test tot\n- [x] K-2: raport\n- [ ] K-3: `gpu lant`\n  - [ ] K-4 : hw  \n- [ ] K-1: raport\n\
                - [ ] X-5: hw\n- [ ] K-: hw\n- [ ] K-6:\ntext oarecare K-7: hw\n";
    let p = parse(text);
    let ids: Vec<&str> = p.iter().map(|o| o.id.as_str()).collect();
    assert!(ids == ["K-1", "K-3", "K-4"], "hub: parse: {:?}", ids);
    assert!(p[1].command == "gpu lant" && p[2].command == "hw", "hub: parse: {:?}", p);
    for ok in ["test tot", "trimite", "raport", "hw", "gpu tur", "run kronos --profil --n 64 Buna", "run kronos --control", "log gpu 40", "timp test tot"] {
        assert!(allowed(ok).is_ok(), "hub: `{}` trebuia permis", ok);
    }
    for bad in ["gpu lant", "gpu lant DA", "gpu i6 --scrie", "run kronos --gpu Ana", "reboot", "hub ruleaza", "hub", "run actualizeaza", "run files", "mod remove x", "test", "remote porneste", "da", "run kronos --conversatie"] {
        assert!(allowed(bad).is_err(), "hub: `{}` trebuia refuzat", bad);
    }
    crate::info!("test: comenzile din hub: citire si lista alba OK");
}
