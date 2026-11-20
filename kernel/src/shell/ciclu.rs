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
//!
//! P6-96 (command C-143, Adrian's "Da chei", DECIZII.md 2026-11-18 17:48): the keys are on. The test is now the chain that
//! passes on the real board — I9 `c135` by default (`ciclu-test os-<N> --i9 c125|c133|c135|os302` for another one) — and,
//! when #513 is on the stick, `kronos --gpu --n 16 Buna ziua` in the same chain (the keys stand for the `DA`, for that one
//! program). When `gpu recupereaza` does not bring the card back (WPR2 still raised; it never ran on the real board before),
//! the cycle stops cleanly with "trebuie pornire rece (Adrian)" and does not try again. The same test runs from the
//! automatic list (`test-placa` in `auto/teste.txt`, auto.rs) with the same keys: one boot from the stick, the test, the
//! journal up to the hub, back to Windows — no restart in between (C-143 completion of 17:58).
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

/// A cycle in progress: its phase, the hub command, the release and (P6-96) the I9 of the chain (`c135` when absent).
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct State {
    pub phase: Phase,
    pub id: String,
    pub tag: String,
    pub i9: String,
}

/// P6-96: the I9 the cycle uses when the command does not name one (the one that passed on the board at os-305/os-306).
pub const I9_DEFAULT: &str = "c135";
/// P6-96: the model the cycle runs after the chain when it is on the stick (`kronos --gpu`, C-141).
pub const K513: &str = "/usb/kronos/kronos_513_q8_0.kgr";
/// P6-100 (C-148 point 5): the same model on the computer's own disk (a partition marked `cyber.rw`, mounted at /disk).
pub const K513_DISK: &str = "/disk/kronos/kronos_513_q8_0.kgr";

/// P6-100: where the cycle finds #513: the own disk when it is marked `cyber.rw` and has it, else the stick
/// (`/bin/kronos` picks its folder the same way: /disk/kronos when /disk has cyber.rw and a model there).
pub fn k513_path() -> Option<&'static str> {
    if fs::own_disk_writable() && fs::kind(K513_DISK).is_ok() {
        return Some(K513_DISK);
    }
    fs::kind(K513).is_ok().then_some(K513)
}
/// P6-96: what `kronos --gpu` generates in the cycle (the command of CITESTE.txt).
pub const K513_ARGS: [&str; 6] = ["kronos", "--gpu", "--n", "16", "Buna", "ziua"];

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

fn release_name(tag: &str) -> bool {
    tag.strip_prefix("os-").is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `--i9 <x>` checked against the chain's own list (`gpu_golden::i9_option`).
fn i9_ok(x: &str) -> bool {
    crate::drivers::gpu_golden::i9_option(x).is_some()
}

/// `ciclu-test os-<N> [--i9 c125|c133|c135|os302]` -> (the release's name, the I9).
pub fn target(command: &str) -> Result<(String, String), &'static str> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["ciclu-test", tag] if release_name(tag) => Ok((String::from(*tag), String::from(I9_DEFAULT))),
        ["ciclu-test", tag, "--i9", x] if release_name(tag) && i9_ok(x) => Ok((String::from(*tag), String::from(*x))),
        _ => Err("forma corecta: `ciclu-test os-<numar>` sau `ciclu-test os-<numar> --i9 c135` (c125, c133, c135, os302)"),
    }
}

/// P6-96: `test-placa [--i9 …]` of the automatic list -> the I9.
pub fn board_command(command: &str) -> Option<Result<String, &'static str>> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["test-placa"] => Some(Ok(String::from(I9_DEFAULT))),
        ["test-placa", "--i9", x] if i9_ok(x) => Some(Ok(String::from(*x))),
        // P6-101 (C-152 point 1): the FLR trial = the usual test, then the chain a second time in the same power-on
        // (it starts with the reset from inside, since WPR2 is up after the first chain).
        ["test-placa", "--flr-proba"] => Some(Ok(format!("{}{}", I9_DEFAULT, FLR_PROBA))),
        ["test-placa", "--i9", x, "--flr-proba"] if i9_ok(x) => Some(Ok(format!("{}{}", x, FLR_PROBA))),
        // P6-105 (C-152 point 5, C-151 point 1): `--tablou` (last) = the board's dashboard at the end of the test's chain.
        ["test-placa", "--tablou"] => Some(Ok(format!("{}{}", I9_DEFAULT, TABLOU))),
        ["test-placa", "--i9", x, "--tablou"] if i9_ok(x) => Some(Ok(format!("{}{}", x, TABLOU))),
        ["test-placa", "--flr-proba", "--tablou"] => Some(Ok(format!("{}{}{}", I9_DEFAULT, FLR_PROBA, TABLOU))),
        ["test-placa", "--i9", x, "--flr-proba", "--tablou"] if i9_ok(x) => Some(Ok(format!("{}{}{}", x, FLR_PROBA, TABLOU))),
        ["test-placa", ..] => Some(Err(
            "forma corecta: `test-placa` sau `test-placa --i9 c135` (c125, c133, c135, os302), plus optional `--flr-proba` si/sau `--tablou` la sfarsit (in aceasta ordine)",
        )),
        _ => None,
    }
}

/// P6-96 (C-143 point 3): `gpu recupereaza` brought the card back when its last check says the I6 pre-check passes again
/// (WPR2 down, read after the program — or it was never raised). Anything else = a cold start is needed.
pub fn recovery_passed(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("pregatirea I6 trece din nou"))
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
    let (mut phase, mut id, mut tag, mut i9) = (None, None, None, String::from(I9_DEFAULT));
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
            "i9" if i9_ok(v) => i9 = String::from(v),
            _ => {}
        }
    }
    Some(State { phase: phase?, id: id?, tag: tag?, i9 })
}

fn read_state() -> Option<State> {
    let bytes = fs::read_all(&state_path()).ok()?;
    parse_state(&String::from_utf8_lossy(&bytes))
}

fn write_state(s: &State) -> Result<(), String> {
    let p = state_path();
    let text = format!("faza={}\ncomanda={}\nversiune={}\ni9={}\n", s.phase.name(), s.id, s.tag, s.i9);
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

/// Both keys, the command's form and no other cycle in progress. `Ok` = the release's name and the I9.
pub fn gate(command: &str) -> Result<(String, String), String> {
    let target = target(command).map_err(String::from)?;
    keys()?;
    if let Some(s) = read_state() {
        return Err(format!("un ciclu e deja in lucru ({} pentru {}, faza {})", s.id, s.tag, s.phase.name()));
    }
    Ok(target)
}

/// Adrian's two keys: (a) the file on the stick, (b) the decision line in the hub's DECIZII.md (read now, over the network).
pub fn keys() -> Result<(), String> {
    if fs::kind(&permit_path()).is_err() {
        return Err(format!(
            "ciclul de test e DEZACTIVAT: lipseste {} (il pune doar Adrian pe stick) si o decizie a lui in DECIZII.md cu `{}`",
            permit_path(),
            PHRASE
        ));
    }
    if !crate::net::wait_ready(30_000) {
        return Err(String::from("reteaua nu are adresa: decizia lui Adrian din DECIZII.md nu se poate verifica; nimic scris in placa"));
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
    Ok(())
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
pub fn start(id: &str, tag: &str, i9: &str) -> String {
    let mut s = State { phase: Phase::Update, id: String::from(id), tag: String::from(tag), i9: String::from(i9) };
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
        let o = hubcmd::Order { id: String::from(id), command: command_of(&s) };
        let _ = hubcmd::send_answer(&o, &text, None);
        crate::info!("ciclu-test {}: versiunea {} pusa; repornesc", id, tag);
        crate::sched::sleep_ms(1000);
        crate::power::reboot(None);
    }
    text.push_str(&format!("\nstick-ul are deja {}: testul porneste acum\n", tag));
    text.push_str(&test(&mut s));
    text
}

/// The hub command a cycle answers (the same words Adrian's command had).
fn command_of(s: &State) -> String {
    if s.i9 == I9_DEFAULT {
        format!("ciclu-test {}", s.tag)
    } else {
        format!("ciclu-test {} --i9 {}", s.tag, s.i9)
    }
}

/// Step 3: the card back to its start state, then the chain without `DA` (both keys were checked when the command came).
fn test(s: &mut State) -> String {
    s.phase = Phase::TestStarted;
    if let Err(e) = write_state(s) {
        clear_state();
        return format!("CICLU OPRIT: starea nu se poate scrie ({}); nimic scris in placa\n", e);
    }
    crate::info!("ciclu-test {}: testul pe placa ({}, I9 {}): gpu recupereaza, apoi lantul (fara DA: cheile ciclului sunt puse)", s.id, s.tag, s.i9);
    let (text, result) = board_test(&s.i9);
    clear_state();
    format!("{}\nCICLU GATA ({}): {}\n", text, s.tag, result)
}

/// P6-96: the board test of a cycle (from the hub or from the automatic list; both keys already checked): `gpu recupereaza`;
/// a cold start is needed when it does not bring the card back (no second try); else the chain with `i9` and — when #513
/// is on the stick — `kronos --gpu --n 16 Buna ziua` at its I14 (one chain). Returns (the whole text, the result line).
/// P6-106 (C-147 point 4): the cycle's `kp toate`, `kc` and `kp ln1` against `os-313` (20 Nov 08:07, no boost:
/// 22 968 us per token, 9 165 MB/s, 2 260 us), from the gpu journal of this test (`PerfGain::compare`).
fn speed_comparison() -> String {
    use nvidia_ga104::kronos_perf::{copy_rows, parse_journal, rows, PerfGain};
    let Some(path) = crate::drivers::gpu_i6::last_journal() else {
        return String::from("\n-- viteza fata de os-313: fara gpu-jurnal\n");
    };
    let now = String::from_utf8_lossy(&fs::read_all(&path).unwrap_or_default()).into_owned();
    let row = |n: &str, v: u32| format!("valoare {} = {:#010x}\n", n, v);
    let os313 = [
        row(rows::TOKENS, 16),
        row(rows::TOTAL, 22_968),
        row("kp ln1 simbol (us)", 2_260),
        row(copy_rows::KIB, 65_536),
        row(copy_rows::COPY_MB, 9_165),
    ]
    .concat();
    let g = PerfGain::compare(&parse_journal(&os313), &parse_journal(&now));
    let f = |what: &str, x: Option<(u32, u32, u32)>| match x {
        Some((b, a, r)) => format!("{}: os-313 {} -> acum {} ({} la mie)\n", what, b, a, r),
        None => format!("{}: necitit (randul lipseste din jurnal)\n", what),
    };
    format!(
        "\n-- viteza fata de os-313 (fara cerere de viteza; jurnalul {})\n{}{}{}",
        path,
        f("timpul placii pe simbol (us, kp toate)", g.token_us),
        f("copierea VRAM (MB/s, kc)", g.copy_mb_s),
        f("ln1 pe simbol (us, legat de ceasuri)", g.ln1_us)
    )
}

/// Suffix of the I9 text from `board_command` when the FLR trial is asked (`test-placa … --flr-proba`).
pub const FLR_PROBA: &str = " --flr-proba";

/// P6-105: suffix of the I9 text when the dashboard is asked (`test-placa … --tablou`, always last).
pub const TABLOU: &str = " --tablou";

pub fn board_test(i9: &str) -> (String, String) {
    let (i9, dashboard) = match i9.strip_suffix(TABLOU) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, flr_trial) = match i9.strip_suffix(FLR_PROBA) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let choice = crate::drivers::gpu_golden::i9_option(i9).unwrap_or(crate::drivers::gpu_golden::I9Choice::C135);
    // P6-100 (C-148 point 5): the disks the OS sees on this computer (reads only), before anything else.
    let mut text = String::from("\n-- discurile vazute de OS (doar citire)\n");
    for row in crate::block::inventory() {
        text.push_str(&row);
        text.push('\n');
    }
    // P6-104 (C-152 point 4): the processor's temperature; at or above TjMax - 10 the test does not start.
    text.push_str(&format!("{}\n", crate::hw::temp::line()));
    // P6-103 (C-152 point 3): the chipset's watchdog; this Release only reads it on a real computer.
    text.push_str(&format!("{}\n", crate::hw::tco::for_cycle()));
    if let Some(why) = crate::hw::temp::too_hot() {
        let why = format!("CICLU OPRIT: {}; testul pe placa nu porneste", why);
        crate::warn!("ciclu: {}", why);
        text.push_str(&format!("\n{}\n", why));
        return (text, why);
    }
    text.push_str("\n-- gpu recupereaza\n");
    let rec = crate::drivers::gpu_rec::recover();
    for l in &rec {
        text.push_str(l);
        text.push('\n');
    }
    if !recovery_passed(&rec) {
        let why = String::from(
            "CICLU OPRIT: trebuie pornire rece (Adrian): gpu recupereaza nu a adus placa la starea de pornire (pregatirea I6 nu trece); lantul nu a pornit, nicio alta incercare",
        );
        crate::warn!("ciclu: {}", why);
        text.push_str(&format!("\n{}\n", why));
        return (text, why);
    }
    // P6-106 (C-148 completion 1): only the cycle's test asks the board for its maximum speed (not the FLR trial's chain).
    crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
    // P6-105: the dashboard only when the list asked for it, and only for this chain (not the FLR trial's second one).
    crate::drivers::gpu_tur::RUSD_ARMED.store(dashboard, core::sync::atomic::Ordering::SeqCst);
    if dashboard {
        text.push_str("\n(tabloul de bord al placii: cerut de lista, la sfarsitul lantului, dupa randurile ks)\n");
    }
    let result = if let Some(model) = k513_path() {
        if model != K513 {
            text.push_str(&format!("\n(#513 de pe discul propriu: {})\n", model));
        }
        text.push_str(&format!("\n-- run {} (lantul cu I9 {}, apoi #513 la i14-simbol; DA dat de cheile ciclului)\n", K513_ARGS.join(" "), i9));
        *crate::drivers::gpu_tur::KRONOS_I9.lock() = Some(choice);
        crate::drivers::gpu_tur::KRONOS_ARMED.store(true, core::sync::atomic::Ordering::SeqCst);
        let (code, printed) = run_program("/bin/kronos", &K513_ARGS);
        // The permission never outlives this one program (as in the shell).
        crate::drivers::gpu_tur::KRONOS_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
        *crate::drivers::gpu_tur::KRONOS_I9.lock() = None;
        text.push_str(&printed);
        text.push_str(&format!("(kronos a iesit cu codul {})\n", code));
        let chain = printed.lines().rev().find(|l| l.contains("rezultat lant:")).map(String::from);
        let board = printed.lines().find(|l| l.contains("kronos: PE PLACA")).map(String::from);
        match (chain, board) {
            (Some(c), Some(b)) => format!("{}; {}", c.trim(), b.trim()),
            (Some(c), None) => format!("{}; #513 NU a rulat pe placa (vezi randurile kronos de mai sus)", c.trim()),
            (None, _) => String::from("(lantul nu a dat un rezultat; vezi randurile kronos de mai sus)"),
        }
    } else {
        text.push_str(&format!("\n-- gpu lant --i9 {} ({} lipseste: doar lantul)\n", i9, K513));
        let chain = crate::drivers::gpu_tur::chain(None, choice);
        for l in &chain {
            text.push_str(l);
            text.push('\n');
        }
        chain.iter().rev().find(|l| l.contains("rezultat lant:")).cloned().unwrap_or_else(|| String::from("(lantul nu a dat un rezultat)"))
    };
    crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::RUSD_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    text.push_str(&speed_comparison());
    if !flr_trial {
        return (text, result);
    }
    // P6-101 (C-152 point 1): the chain a second time in this power-on; it starts with the reset from inside (the
    // `kf` row). Whatever the outcome, there is no third try; the cycle ends as usual.
    text.push_str(&format!("
-- proba FLR (C-152 punctul 1): gpu lant --i9 {} a doua oara, cu resetul din interior la inceput
", i9));
    crate::drivers::gpu_tur::FLR_ARMED.store(true, core::sync::atomic::Ordering::SeqCst);
    let second = crate::drivers::gpu_tur::chain(None, choice);
    crate::drivers::gpu_tur::FLR_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    for l in &second {
        text.push_str(l);
        text.push('\n');
    }
    let kf = second.iter().find(|l| l.contains(": kf ")).map_or("(fara rand kf)", |l| l.as_str());
    let r2 = second.iter().rev().find(|l| l.contains("rezultat lant:")).map_or("(fara rezultat)", |l| l.as_str());
    (text, format!("{}; PROBA FLR: {}; al doilea lant: {}", result, kf.trim(), r2.trim()))
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
    let o = hubcmd::Order { id: s.id.clone(), command: command_of(s) };
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
    assert!(target("ciclu-test os-281") == Ok((String::from("os-281"), String::from("c135"))), "ciclu: implicit I9 c135 (P6-96)");
    assert!(target("ciclu-test os-307 --i9 c133") == Ok((String::from("os-307"), String::from("c133"))));
    assert!(target("ciclu-test os-307 --i9 os302") == Ok((String::from("os-307"), String::from("os302"))));
    for bad in ["ciclu-test", "ciclu-test 281", "ciclu-test os-", "ciclu-test os-28a", "ciclu-test os-1 DA", "ciclu-test os-1234567", "ciclu-test os-1 --i9", "ciclu-test os-1 --i9 c999", "ciclu-test os-1 --i9 c135 DA"] {
        assert!(target(bad).is_err(), "ciclu: `{}` trebuia refuzat", bad);
    }
    assert!(board_command("test-placa") == Some(Ok(String::from("c135"))));
    assert!(board_command("test-placa --i9 c125") == Some(Ok(String::from("c125"))));
    assert!(board_command("test-placa --i9 x").is_some_and(|r| r.is_err()) && board_command("test-placa DA").is_some_and(|r| r.is_err()));
    assert!(board_command("gpu lant").is_none() && board_command("test-placax").is_none());
    // P6-106: the comparison with os-313 from a journal with the rows (Cyber-GPU's own example: twice as fast).
    let _ = fs::create("/etc/kp-test.txt", Kind::File);
    let rows = "x valoare kp simboluri = 0x00000010\nx valoare kp toate simbol (us) = 0x00002cdc\nx valoare kc copiere VRAM KiB = 0x00010000\nx valoare kc copiere MB/s = 0x0002cc04\n";
    let _ = fs::write("/etc/kp-test.txt", 0, rows.as_bytes());
    crate::drivers::gpu_i6::set_last_journal(Some(String::from("/etc/kp-test.txt")));
    let c = speed_comparison();
    crate::drivers::gpu_i6::set_last_journal(None);
    assert!(c.contains("os-313 22968 -> acum 11484 (500 la mie)") && c.contains("os-313 9165 -> acum 183300 (20000 la mie)") && c.contains("ln1 pe simbol (us, legat de ceasuri): necitit"), "test: comparatia cu os-313: {}", c);
    // P6-101: the FLR trial form.
    assert!(board_command("test-placa --flr-proba") == Some(Ok(String::from("c135 --flr-proba"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba") == Some(Ok(String::from("c133 --flr-proba"))));
    assert!(board_command("test-placa --flr-proba --i9 c133").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --tablou") == Some(Ok(String::from("c135 --tablou"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou") == Some(Ok(String::from("c133 --flr-proba --tablou"))));
    assert!(board_command("test-placa --tablou --flr-proba").is_some_and(|r| r.is_err()));
    let rec = |s: &str| alloc::vec![String::from("gpu0: Bus Master pornit"), String::from(s)];
    assert!(recovery_passed(&rec("gpu0: C-103: pregatirea I6 trece din nou: `gpu lant` poate rula in aceeasi pornire")));
    assert!(!recovery_passed(&rec("gpu0: C-103: pregatirea I6 NU trece: tot pornire rece (cablul scos 30 s)")));
    assert!(!recovery_passed(&rec("gpu0: gpu recupereaza: exista doar pe stick-ul de test (optiunea gpu-scrie)")));
    assert!(decision_found("# Decizii\n- 2026-11-20 — ciclu-test activat (Adrian)\n"));
    // P6-96 (C-143 point 1): the exact form of Adrian's line in the hub's DECIZII.md of 18 Nov (bold, long dash).
    assert!(decision_found(
        "## 2026-11-18 17:48 · Adrian · teste pe placă fără el (ciclul automat C-105)\n- 2026-11-18 17:48 — **ciclu-test activat** (Adrian: „Da chei”, după ce `os-306` a trecut tot lanțul I0→I14 pe placa reală): înlocuiește explicit regula\n"
    ));
    assert!(!decision_found("## 2026-11-18 17:48 · ciclu-test activat\n"), "ciclu: un titlu nu e o decizie");
    assert!(!decision_found("# Decizii\n- Teste complet automate: directia aprobata, fara activare\n"));
    assert!(!decision_found("ciclu-test activat\n"), "ciclu: doar un rand de decizie (`- `) conteaza");
    let s = State { phase: Phase::TestStarted, id: String::from("K-12"), tag: String::from("os-300"), i9: String::from("c133") };
    let text = format!("faza={}\ncomanda={}\nversiune={}\ni9={}\n", s.phase.name(), s.id, s.tag, s.i9);
    assert!(parse_state(&text) == Some(s.clone()), "ciclu: starea citita inapoi");
    assert!(command_of(&s) == "ciclu-test os-300 --i9 c133");
    // A state written by os-306 (no i9 line) reads back with the default.
    let old = parse_state("faza=test\ncomanda=K-3\nversiune=os-306\n");
    assert!(old.as_ref().is_some_and(|o| o.i9 == "c135") && old.as_ref().is_some_and(|o| command_of(o) == "ciclu-test os-306"));
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
    crate::info!("test: ciclu-test: forma (cu --i9, implicit c135), test-placa, recuperarea, randul real din DECIZII.md, starea si fazele dupa repornire OK");
}
