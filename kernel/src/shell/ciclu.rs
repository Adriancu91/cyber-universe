//! The automatic test cycle from the hub (task P6-75, command C-105) — PREPARED, DISABLED BY DEFAULT.
//!
//! A hub command `K-<nr>: ciclu-test os-<N>` (taken by `hub ruleaza` / `hub asculta`) does, on the workstation:
//! (1) `actualizeaza --versiune os-<N>` (that exact release, every file checked against SHA256SUMS, the running version
//! kept in `boot/vechi/`), (2) a restart into it (trial boot + confirmation, revenire.rs), (3) `gpu recupereaza`
//! (Booter Unload; skipped by itself when WPR2 is already down) and `gpu lant` WITHOUT the typed `DA`, (4) the report on
//! the stick, `trimite`, and the answer `integrare/cyber-universe/raspunsuri/K-<nr>.txt` in the hub, (5) back to
//! listening. When the cycle itself fails (the update fails, the new version never confirms, the system falls over
//! during the test) the old version comes back from `boot/vechi/` and the report says so. The chain's own result
//! (all steps passed / stopped at Ix) is a RESULT, not a failure of the cycle.
//!
//! It runs ONLY with both keys, checked at every command: (a) the file `/usb/hub/permite-test-placa`, put on the stick by
//! Adrian; (b) a decision line (`- …`) in the hub's `DECIZII.md` containing `ciclu-test activat`. Adrian's rule today
//! is "writes to the card only with Adrian present" (DECIZII.md, 2026-11-11 14:36): without both keys the command is
//! refused and nothing happens.
//!
//! The phases live in `/usb/hub/ciclu.txt` (they survive the restart): `actualizare` -> `test` -> `test-pornit`
//! (written just before the card is touched) -> done (file removed).
//! Idea: hardware test labs with job queues, A/B updates; our own code.

use super::{auto, hubcmd, raport};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

/// The words a decision line in the hub's DECIZII.md must contain (key b).
pub const PHRASE: &str = "ciclu-test activat";
/// The stick the system started from (where `actualizeaza` writes and revenire.rs keeps the old version).
const STICK: &str = "/usb";

/// The phase of a cycle in progress.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Phase {
    /// `actualizeaza` is running (or was cut off).
    Update,
    /// The new version is in place; the test runs after the restart.
    Test,
    /// The test started (the card may have been touched).
    TestStarted,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Update => "actualizare",
            Phase::Test => "test",
            Phase::TestStarted => "test-pornit",
        }
    }
}

/// A cycle in progress: its phase, the hub command and the release.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct State {
    pub phase: Phase,
    pub id: String,
    pub tag: String,
}

/// What to do at boot with the state found on the stick.
#[derive(Debug, PartialEq, Eq)]
pub enum AtBoot {
    /// No cycle in progress.
    Nothing,
    /// The new version is on trial: wait for its confirmation, then test.
    WaitThenTest,
    /// The new version is confirmed (or there was nothing to update): test now.
    Test,
    /// The new version never confirmed and revenire.rs already put the old one back: report.
    NotConfirmed,
    /// The machine restarted in the middle of `actualizeaza`: report (actualizeaza repairs its files at the next run).
    UpdateCut,
    /// The machine restarted during the test: the old version back, report.
    FellDuringTest,
}

/// The decision, kept pure so the test covers every case. `on_trial` = `boot/incercare.txt` exists;
/// `returned` = `boot/revenit.txt` names this release.
pub fn decide(state: Option<Phase>, on_trial: bool, returned: bool) -> AtBoot {
    match state {
        None => AtBoot::Nothing,
        Some(Phase::Update) => AtBoot::UpdateCut,
        Some(Phase::Test) if returned => AtBoot::NotConfirmed,
        Some(Phase::Test) if on_trial => AtBoot::WaitThenTest,
        Some(Phase::Test) => AtBoot::Test,
        Some(Phase::TestStarted) => AtBoot::FellDuringTest,
    }
}

/// True when the hub command is a test cycle (it then goes through `gate`, never through the allow-list).
pub fn is_cycle(command: &str) -> bool {
    command.split_whitespace().next() == Some("ciclu-test")
}

/// `ciclu-test os-<N>` -> the release's name.
pub fn target(command: &str) -> Result<String, &'static str> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["ciclu-test", tag]
            if tag.strip_prefix("os-").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            Ok(String::from(*tag))
        }
        _ => Err("forma corecta: `ciclu-test os-<numar>` (de exemplu ciclu-test os-281)"),
    }
}

/// Key b: a decision line (a list item) of DECIZII.md with the exact words.
pub fn decision_found(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("- ") && l.contains(PHRASE))
}

fn state_path() -> String {
    format!("{}/ciclu.txt", hubcmd::dir())
}

fn permit_path() -> String {
    format!("{}/permite-test-placa", hubcmd::dir())
}

pub fn parse_state(text: &str) -> Option<State> {
    let (mut phase, mut id, mut tag) = (None, None, None);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "faza" => {
                phase = match v {
                    "actualizare" => Some(Phase::Update),
                    "test" => Some(Phase::Test),
                    "test-pornit" => Some(Phase::TestStarted),
                    _ => None,
                }
            }
            "comanda" => id = Some(String::from(v)),
            "versiune" => tag = Some(String::from(v)),
            _ => {}
        }
    }
    Some(State { phase: phase?, id: id?, tag: tag? })
}

fn read_state() -> Option<State> {
    let bytes = fs::read_all(&state_path()).ok()?;
    parse_state(&String::from_utf8_lossy(&bytes))
}

fn write_state(s: &State) -> Result<(), String> {
    let p = state_path();
    let text = format!("faza={}\ncomanda={}\nversiune={}\n", s.phase.name(), s.id, s.tag);
    let _ = fs::remove(&p);
    fs::create(&p, Kind::File).map_err(|e| format!("{}: {}", p, e.message()))?;
    fs::write(&p, 0, text.as_bytes()).map_err(|e| format!("{}: {}", p, e.message()))?;
    fs::flush(&p).map_err(|e| format!("{}: {}", p, e.message()))
}

fn clear_state() {
    let p = state_path();
    let _ = fs::remove(&p);
    let _ = fs::flush(hubcmd::dir());
}

/// Both keys, the command's form and no other cycle in progress. `Ok` = the release's name.
pub fn gate(command: &str) -> Result<String, String> {
    let tag = target(command).map_err(String::from)?;
    if fs::kind(&permit_path()).is_err() {
        return Err(format!(
            "ciclul de test e DEZACTIVAT: lipseste {} (il pune doar Adrian pe stick) si o decizie a lui in DECIZII.md cu `{}`",
            permit_path(),
            PHRASE
        ));
    }
    let local = format!("{}/decizii.md", hubcmd::dir());
    let got = match crate::proc::spawn_path("/bin/gh-ia", &["gh-ia", "--hub-ia", "DECIZII.md", &local]) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => return Err(format!("/bin/gh-ia nu porneste ({}): nu pot verifica decizia lui Adrian", e.message())),
    };
    if got != 0 {
        return Err(String::from("DECIZII.md nu s-a putut lua din hub: nu pot verifica decizia lui Adrian"));
    }
    let text = String::from_utf8_lossy(&fs::read_all(&local).unwrap_or_default()).into_owned();
    if !decision_found(&text) {
        return Err(format!(
            "ciclul de test e DEZACTIVAT: in DECIZII.md din hub nu e nicio decizie cu `{}` (regula de azi: scrieri pe placa doar cu Adrian de fata)",
            PHRASE
        ));
    }
    if let Some(s) = read_state() {
        return Err(format!("un ciclu e deja in lucru ({} pentru {}, faza {})", s.id, s.tag, s.phase.name()));
    }
    Ok(tag)
}

/// Runs a program and returns (exit code, what it printed).
fn run_program(path: &str, args: &[&str]) -> (i64, String) {
    super::program_capture_begin();
    let code = match crate::proc::spawn_path(path, args) {
        Ok(id) => crate::proc::wait(id),
        Err(e) => {
            let _ = super::program_capture_end();
            return (-1, format!("{} nu porneste: {}\n", path, e.message()));
        }
    };
    (code, super::program_capture_end())
}

fn on_trial() -> bool {
    fs::read_all(&format!("{}/boot/incercare.txt", STICK)).is_ok()
}

/// Step 1, from the hub command (both keys already checked): the update. Returns the answer's text; when a new version
/// was put in place the machine restarts into it and this never returns.
pub fn start(id: &str, tag: &str) -> String {
    let mut s = State { phase: Phase::Update, id: String::from(id), tag: String::from(tag) };
    if let Err(e) = write_state(&s) {
        return format!("CICLU OPRIT inainte de actualizare: starea nu se poate scrie ({}); nimic schimbat\n", e);
    }
    auto::say(&format!("ciclu-test {}: {}: actualizez la {}\n", id, auto::now_text(), tag));
    let (code, printed) = run_program("/bin/actualizeaza", &["actualizeaza", "--versiune", tag]);
    let mut text = format!("== ciclu-test {} ==\n-- actualizeaza --versiune {} (codul {})\n{}", tag, tag, code, printed);
    if code != 0 {
        clear_state();
        text.push_str("\nCICLU OPRIT la actualizare: versiunea de pe stick a ramas cea veche; nicio scriere in placa\n");
        return text;
    }
    s.phase = Phase::Test;
    if let Err(e) = write_state(&s) {
        clear_state();
        text.push_str(&format!("\nCICLU OPRIT: starea nu se poate scrie ({}); testul nu porneste\n", e));
        return text;
    }
    if on_trial() {
        text.push_str(&format!("\nversiunea {} e pe stick; repornesc in ea; testul si raportul vin dupa repornire, in acelasi fisier\n", tag));
        let o = hubcmd::Order { id: String::from(id), command: format!("ciclu-test {}", tag) };
        let _ = hubcmd::send_answer(&o, &text, None);
        crate::info!("ciclu-test {}: versiunea {} pusa; repornesc", id, tag);
        crate::sched::sleep_ms(1000);
        crate::power::reboot(None);
    }
    text.push_str(&format!("\nstick-ul are deja {}: testul porneste acum\n", tag));
    text.push_str(&test(&mut s));
    text
}

/// Step 3: the card back to its start state, then the chain without `DA` (both keys were checked when the command came).
fn test(s: &mut State) -> String {
    s.phase = Phase::TestStarted;
    if let Err(e) = write_state(s) {
        clear_state();
        return format!("CICLU OPRIT: starea nu se poate scrie ({}); nimic scris in placa\n", e);
    }
    crate::info!("ciclu-test {}: testul pe placa ({}): gpu recupereaza, apoi gpu lant (fara DA: cheile ciclului sunt puse)", s.id, s.tag);
    let mut text = String::from("\n-- gpu recupereaza\n");
    for l in crate::drivers::gpu_rec::recover() {
        text.push_str(&l);
        text.push('\n');
    }
    text.push_str("\n-- gpu lant\n");
    let chain = crate::drivers::gpu_tur::chain();
    for l in &chain {
        text.push_str(l);
        text.push('\n');
    }
    clear_state();
    let result = chain.iter().rev().find(|l| l.contains("rezultat lant:")).cloned().unwrap_or_else(|| String::from("(lantul nu a dat un rezultat)"));
    text.push_str(&format!("\nCICLU GATA ({}): {}\n", s.tag, result));
    text
}

/// After the restart: the report of a cycle in progress, the old version back when the cycle itself failed.
fn report(s: &State, body: &str, ms: Option<u64>) {
    let (reports, _) = raport::hub_paths();
    if fs::kind(reports).is_err() {
        let _ = fs::create(reports, Kind::Dir);
    }
    let path = auto::free_name(&format!("{}/ciclu-{}", reports, auto::date_stamp()));
    let mut out = auto::Report::create(&path);
    out.add(&format!(
        "== ciclu de test din hub: raport Cyber Universe OS ==\ncomanda: {} (ciclu-test {})\nnucleu: cyber-universe {} (cyber-gpu {})\nora: {}\n{}",
        s.id,
        s.tag,
        env!("CARGO_PKG_VERSION"),
        crate::drivers::gpu::CYBER_GPU_REV,
        auto::now_text(),
        body
    ));
    if !crate::net::wait_ready(30_000) {
        crate::warn!("ciclu-test {}: reteaua nu are adresa: raportul ramane pe stick ({})", s.id, path);
        return;
    }
    let o = hubcmd::Order { id: s.id.clone(), command: format!("ciclu-test {}", s.tag) };
    let sent = hubcmd::send_answer(&o, body, ms);
    crate::info!("ciclu-test {}: raspuns in hub: {}", s.id, sent);
    raport::send_to_hub();
}

/// At boot (before listening): finishes a cycle in progress. Runs on the system thread that then listens.
pub fn resume() {
    let Some(mut s) = read_state() else { return };
    let returned = fs::read_all(&format!("{}/boot/revenit.txt", STICK))
        .is_ok_and(|t| String::from_utf8_lossy(&t).contains(s.tag.as_str()));
    match decide(Some(s.phase), on_trial(), returned) {
        AtBoot::Nothing => {}
        AtBoot::UpdateCut => {
            clear_state();
            report(&s, "\nCICLU OPRIT: calculatorul a repornit in timpul actualizarii; nicio scriere in placa (actualizeaza isi repara fisierele la rularea urmatoare)\n", None);
        }
        AtBoot::NotConfirmed => {
            clear_state();
            report(&s, &format!("\nCICLU OPRIT: versiunea {} nu a ajuns la confirmare; s-a revenit singur la versiunea veche (boot/revenit.txt); nicio scriere in placa\n", s.tag), None);
        }
        AtBoot::FellDuringTest => {
            clear_state();
            let back = match crate::revenire::return_to_old(STICK, &format!("ciclu-test {}: sistemul a cazut in timpul testului pe placa", s.tag)) {
                Ok(()) => String::from("versiunea veche pusa la loc din boot/vechi/; repornesc in ea"),
                Err(e) => format!("revenirea la versiunea veche NU a mers ({}); raman pe versiunea de acum", e),
            };
            report(&s, &format!("\nCICLU ESUAT: calculatorul a repornit in timpul testului pe placa ({}); {}\n", s.tag, back), None);
            if back.starts_with("versiunea veche") {
                crate::sched::sleep_ms(1000);
                crate::power::reboot(None);
            }
        }
        AtBoot::WaitThenTest | AtBoot::Test => {
            // The trial version is tested only once it confirmed (revenire.rs: 60 s); a hang here is then the card's.
            let deadline = crate::time::uptime_ms().unwrap_or(0) + 6 * 60_000;
            while on_trial() {
                if crate::time::uptime_ms().unwrap_or(0) > deadline {
                    return; // revenire.rs restarts the machine; the next boot reports NotConfirmed
                }
                crate::sched::sleep_ms(1000);
            }
            let t0 = crate::time::uptime_ms().unwrap_or(0);
            let text = test(&mut s);
            report(&s, &text, Some(crate::time::uptime_ms().unwrap_or(0).saturating_sub(t0)));
        }
    }
}

/// True when a cycle is in progress (the boot thread finishes it before listening).
pub fn in_progress() -> bool {
    fs::kind(&state_path()).is_ok()
}

/// Boot-time self test (`ghpush`): the form, the decision line, the state file and every boot decision.
#[cfg(feature = "test-ghpush")]
pub fn self_test() {
    assert!(is_cycle("ciclu-test os-281") && is_cycle("  ciclu-test") && !is_cycle("gpu lant") && !is_cycle("ciclu-testx os-1"));
    assert!(target("ciclu-test os-281") == Ok(String::from("os-281")));
    for bad in ["ciclu-test", "ciclu-test 281", "ciclu-test os-", "ciclu-test os-28a", "ciclu-test os-1 DA", "ciclu-test os-1234567"] {
        assert!(target(bad).is_err(), "ciclu: `{}` trebuia refuzat", bad);
    }
    assert!(decision_found("# Decizii\n- 2026-11-20 — ciclu-test activat (Adrian)\n"));
    assert!(!decision_found("# Decizii\n- Teste complet automate: directia aprobata, fara activare\n"));
    assert!(!decision_found("ciclu-test activat\n"), "ciclu: doar un rand de decizie (`- `) conteaza");
    let s = State { phase: Phase::TestStarted, id: String::from("K-12"), tag: String::from("os-300") };
    let text = format!("faza={}\ncomanda={}\nversiune={}\n", s.phase.name(), s.id, s.tag);
    assert!(parse_state(&text) == Some(s), "ciclu: starea citita inapoi");
    assert!(parse_state("faza=altceva\ncomanda=K-1\nversiune=os-1\n").is_none() && parse_state("faza=test\n").is_none());
    use AtBoot::*;
    for (st, trial, ret, want) in [
        (None, false, false, Nothing),
        (None, true, true, Nothing),
        (Some(Phase::Update), false, false, UpdateCut),
        (Some(Phase::Update), true, false, UpdateCut),
        (Some(Phase::Test), true, false, WaitThenTest),
        (Some(Phase::Test), false, false, Test),
        (Some(Phase::Test), false, true, NotConfirmed),
        (Some(Phase::Test), true, true, NotConfirmed),
        (Some(Phase::TestStarted), false, false, FellDuringTest),
        (Some(Phase::TestStarted), true, false, FellDuringTest),
    ] {
        assert!(decide(st, trial, ret) == want, "ciclu: decide({:?}, {}, {})", st, trial, ret);
    }
    crate::info!("test: ciclu-test: forma, decizia, starea si fazele dupa repornire OK");
}
