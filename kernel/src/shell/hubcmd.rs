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
//!
//! C-104 (P6-73): `hub asculta [--la N]` = the same, every N minutes (default 2), until Esc; started at boot when
//! `/usb/hub/asculta` exists (then `hub opreste` stops it). Every command also gets its own answer file in the hub,
//! `integrare/cyber-universe/raspunsuri/K-<nr>.txt` (`gh-trimite --raspuns`), so the command centre reads it easily.
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
/// C-104: one listening loop at a time; `hub opreste` asks it to stop.
static LISTENING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
/// Default and allowed range of the pause between two looks at the hub, in minutes.
const LISTEN_DEFAULT_MIN: u64 = 2;
const LISTEN_MAX_MIN: u64 = 60;

/// One command of the hub file.
#[derive(Debug, PartialEq)]
pub struct Order {
    pub id: String,
    pub command: String,
}

/// The open commands of the file: `- [ ] K-12: test tot`. `[x]` (done by hand), other lines, bad IDs and anything inside a
/// ``` block are skipped.
pub fn parse(text: &str) -> Vec<Order> {
    let mut out: Vec<Order> = Vec::new();
    let mut in_code = false;
    for raw in text.lines() {
        // Examples inside ``` blocks never run.
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
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

/// `hub asculta [--la N]` (C-104): looks at the hub every N minutes (1..60, default 2) until Esc.
pub fn listen_command(args: &[&str]) {
    let minutes = match args {
        [] => LISTEN_DEFAULT_MIN,
        ["--la", n] => match n.parse::<u64>() {
            Ok(m) if (1..=LISTEN_MAX_MIN).contains(&m) => m,
            _ => {
                super::emit(&format!("hub asculta: --la cere minute intre 1 si {}\n", LISTEN_MAX_MIN));
                return;
            }
        },
        _ => {
            super::emit("folosire: hub asculta [--la N]   (la fiecare N minute, implicit 2; Esc opreste)\n");
            return;
        }
    };
    listen(minutes * 60_000, None, true, |_| {});
}

/// `hub opreste` (C-104): stops the listening loop started at boot (`/usb/hub/asculta`).
pub fn stop_command() {
    if LISTENING.load(Ordering::Acquire) {
        STOP.store(true, Ordering::Release);
        super::emit("hub opreste: bucla `hub asculta` se opreste (dupa comanda care ruleaza acum, daca e una)\n");
    } else {
        super::emit("hub opreste: nicio bucla `hub asculta` nu ruleaza\n");
    }
}

/// The listening loop. `by_hand` = started from the shell: Esc stops it (it is the only keyboard reader then); started
/// at boot it stops with `hub opreste`. `max_rounds` and `before_round` only for the tests.
fn listen(interval_ms: u64, max_rounds: Option<u32>, by_hand: bool, mut before_round: impl FnMut(u32)) {
    if LISTENING.swap(true, Ordering::Acquire) {
        super::emit("hub asculta: bucla ruleaza deja (pornita la pornire: `hub opreste`)\n");
        return;
    }
    STOP.store(false, Ordering::Release);
    let how = if by_hand { "Esc opreste" } else { "`hub opreste` opreste" };
    auto::say(&format!(
        "hub asculta: iau comenzile din hub la fiecare {} s ({}); doar lista alba, fiecare comanda o singura data\n",
        interval_ms / 1000,
        how
    ));
    let mut round = 0u32;
    'outer: loop {
        round += 1;
        before_round(round);
        if RUNNING.swap(true, Ordering::Acquire) {
            auto::say("hub asculta: `hub ruleaza` ruleaza acum; incerc la runda urmatoare\n");
        } else {
            run_inner(false, true);
            RUNNING.store(false, Ordering::Release);
        }
        if max_rounds.is_some_and(|m| round >= m) {
            break;
        }
        auto::say(&format!("hub asculta: runda {} gata; urmatoarea peste {} s ({})\n", round, interval_ms / 1000, how));
        let mut waited = 0u64;
        while waited < interval_ms {
            if STOP.load(Ordering::Acquire) || (by_hand && crate::drivers::keyboard::take_escape()) {
                break 'outer;
            }
            let step = (interval_ms - waited).min(200);
            crate::sched::sleep_ms(step);
            waited += step;
        }
    }
    LISTENING.store(false, Ordering::Release);
    auto::say(&format!("hub asculta: oprit dupa {} runde\n", round));
}

/// At boot, when `/usb/hub/la-pornire` exists: a system thread runs the hub's commands once the network has an address.
/// C-104: when `/usb/hub/asculta` exists, a system thread listens instead (minutes = the number in that file, default 2).
pub fn start_if_asked() {
    let d = dir();
    if let Ok(text) = fs::read_all(&format!("{}/asculta", d)) {
        let minutes = String::from_utf8_lossy(&text)
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|m| (1..=LISTEN_MAX_MIN).contains(m))
            .unwrap_or(LISTEN_DEFAULT_MIN);
        LISTEN_BOOT_MIN.store(minutes, Ordering::Relaxed);
        crate::info!("hub: {}/asculta: ascult comenzile din hub la fiecare {} min (oprire: `hub opreste`)", d, minutes);
        crate::sched::spawn_system(|| listen(LISTEN_BOOT_MIN.load(Ordering::Relaxed) * 60_000, None, false, |_| {}));
    } else if fs::kind(&format!("{}/la-pornire", d)).is_ok() {
        crate::info!("hub: {}/la-pornire: rulez comenzile din hub cand reteaua e gata", d);
        crate::sched::spawn_system(|| run(false));
    }
}

static LISTEN_BOOT_MIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(LISTEN_DEFAULT_MIN);

fn run(only_show: bool) {
    if RUNNING.swap(true, Ordering::Acquire) {
        super::emit("hub ruleaza: deja ruleaza (alt terminal sau pornirea)\n");
        return;
    }
    run_inner(only_show, false);
    RUNNING.store(false, Ordering::Release);
}

/// `quiet` (the listening loop): a round with nothing new says one short line and sends nothing.
fn run_inner(only_show: bool, quiet: bool) {
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
        if quiet {
            auto::say(&format!("hub asculta: {} nicio comanda noua\n", auto::now_text()));
        } else {
            super::emit("hub ruleaza: nicio comanda noua in hub\n");
        }
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
            let sent = send_answer(o, &format!("REFUZAT: {} (nimic nu s-a rulat)\n", why), None);
            out.add(&format!("-- raspuns in hub: {}\n", sent));
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
        let sent = send_answer(o, &text, Some(ms));
        out.add(&format!("-- raspuns in hub: {}\n", sent));
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

/// C-104: the answer to one command, its own file in the hub (`integrare/cyber-universe/raspunsuri/K-<nr>.txt`), kept
/// on the stick too (`<dir>/raspunsuri/`). Returns what to note in the report.
fn send_answer(o: &Order, body: &str, ms: Option<u64>) -> String {
    let folder = format!("{}/raspunsuri", dir());
    if fs::kind(&folder).is_err() && fs::create(&folder, Kind::Dir).is_err() {
        return format!("NU (nu pot face {})", folder);
    }
    let path = format!("{}/{}.txt", folder, o.id);
    let _ = fs::remove(&path);
    let mut text = format!(
        "== raspuns Cyber Universe OS: {} ==\ncomanda: {}\nora: {}\nnucleu: cyber-universe {} (cyber-gpu {})\n\n",
        o.id,
        o.command,
        auto::now_text(),
        env!("CARGO_PKG_VERSION"),
        crate::drivers::gpu::CYBER_GPU_REV
    );
    text.push_str(body);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    match ms {
        Some(ms) => text.push_str(&format!("\n-- gata in {} ms\n", ms)),
        None => text.push_str("\n-- nerulat\n"),
    }
    let mut file = auto::Report::create(&path); // hides secrets, as the reports
    file.add(&text);
    let code = match crate::proc::spawn_path("/bin/gh-trimite", &["gh-trimite", "--raspuns", &path, &o.id]) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => return format!("NU (/bin/gh-trimite nu porneste: {})", e.message()),
    };
    if code == 0 {
        format!("integrare/cyber-universe/raspunsuri/{}.txt", o.id)
    } else {
        format!("NU (gh-trimite codul {}; ramane pe stick in {})", code, path)
    }
}

/// C-104 test (`ghpush`): the listening loop, 3 rounds 1 s apart; `before_round` changes the hub file between them.
#[cfg(feature = "test-ghpush")]
pub fn listen_test(before_round: impl FnMut(u32)) {
    listen(1_000, Some(3), false, before_round);
}

/// Boot-time self test of the parser and the allow-list (`ghpush` and `auto` test builds).
#[cfg(any(feature = "test-ghpush", feature = "test-auto"))]
pub fn self_test() {
    let text = "# Comenzi\n- [ ] K-1: test tot\n- [x] K-2: raport\n- [ ] K-3: `gpu lant`\n  - [ ] K-4 : hw  \n- [ ] K-1: raport\n\
                - [ ] X-5: hw\n- [ ] K-: hw\n- [ ] K-6:\ntext oarecare K-7: hw\n```\n- [ ] K-8: hw\n```\n";
    let p = parse(text);
    let ids: Vec<&str> = p.iter().map(|o| o.id.as_str()).collect();
    assert!(ids == ["K-1", "K-3", "K-4"], "hub: parse: {:?}", ids);
    assert!(p[1].command == "gpu lant" && p[2].command == "hw", "hub: parse: {:?}", p);
    for ok in ["test tot", "trimite", "raport", "hw", "gpu tur", "run kronos --profil --n 64 Buna", "run kronos --control", "log gpu 40", "timp test tot"] {
        assert!(allowed(ok).is_ok(), "hub: `{}` trebuia permis", ok);
    }
    for bad in ["gpu lant", "gpu lant DA", "hub asculta", "hub opreste", "gpu i6 --scrie", "run kronos --gpu Ana", "reboot", "hub ruleaza", "hub", "run actualizeaza", "run files", "mod remove x", "test", "remote porneste", "da", "run kronos --conversatie"] {
        assert!(allowed(bad).is_err(), "hub: `{}` trebuia refuzat", bad);
    }
    crate::info!("test: comenzile din hub: citire si lista alba OK");
}
