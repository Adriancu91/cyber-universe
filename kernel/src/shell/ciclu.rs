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

/// P6-122 (C-158 point 3): Kronos #651 (24 layers, hub `integrare/kronos/MODEL_651.md`), run by the cycle only when the
/// list asks for it (`test-placa … --k651`); #513 stays the default.
pub const K651: &str = "/usb/kronos/kronos_651_q8_0.kgr";
pub const K651_DISK: &str = "/disk/kronos/kronos_651_q8_0.kgr";

/// P6-122: where the cycle finds #651 (as `k513_path`: the own disk first, then the stick).
pub fn k651_path() -> Option<&'static str> {
    if fs::own_disk_writable() && fs::kind(K651_DISK).is_ok() {
        return Some(K651_DISK);
    }
    fs::kind(K651).is_ok().then_some(K651)
}

/// P6-122: the same text as #513 (`K513_ARGS`), with #651's file named.
pub fn k651_args(path: &str) -> [&str; 8] {
    ["kronos", "--gpu", "--model", path, "--n", "16", "Buna", "ziua"]
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
    const FORM: &str = "forma corecta: `test-placa` sau `test-placa --i9 c135` (c125, c133, c135, os302), plus optional `--flr-proba`, `--tablou`, `--lipit`, `--simplu`, `--ln-nou`, `--ln-vechi`, `--lipita-ln`, `--sferturi` sau `--ln-32`, apoi `--k651`, apoi `--argmax-placa` sau `--argmax-fara-copie`, apoi `--dieta2`, apoi `--p8`, apoi `--colsum`, apoi `--pas-aur`, apoi `--mm-tc`, apoi `--atentie-placi` la sfarsit (in aceasta ordine; doar una din `--lipit`, `--simplu`, `--ln-nou`, `--ln-vechi`, `--lipita-ln`, `--sferturi`, `--ln-32`; `--k651` nu merge cu `--lipit` sau `--simplu`; `--argmax-fara-copie` nu merge cu `--lipit`, `--ln-nou`, `--sferturi` sau `--ln-32`)";
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some(("test-placa", rest)) = words.split_first().map(|(a, b)| (*a, b)) else { return None };
    let (i9, flags) = match rest {
        ["--i9", x, flags @ ..] if i9_ok(x) => (*x, flags),
        ["--i9", ..] => return Some(Err(FORM)),
        flags => (I9_DEFAULT, flags),
    };
    // P6-101 (C-152 point 1): `--flr-proba` = the usual test, then the chain a second time in the same power-on.
    // P6-105 (C-152 point 5): `--tablou` = the board's dashboard at the end of the test's chain.
    // P6-110 (C-155 point 1): `--lipit` = #513 a second time with P3's glued list, compared bit for bit with the plain one.
    // P6-112: `--simplu` = #513 with the plain list alone (as in os-324); without either, the glued list alone.
    // P6-114 (C-155 point 2): `--ln-nou` = the glued list with the old LayerNorm, then with the new one, compared.
    // P6-120 (center's proposal 21 Nov 19:25): without any of them, the new LayerNorm alone (`FusedLn`); `--ln-vechi` =
    // the glued list with the old LayerNorm alone (as in os-329 … os-335).
    // P6-124 (C-155 point 4): `--sferturi` = the glued list, then the glued list on quarters (`Fused32`), compared.
    // P6-122 (C-158 point 3): `--k651` = Kronos #651 instead of #513, with the glued lists only (the plain one does not
    // fit #651's pushbuffer area and records' stride: 388 launches).
    // P6-123 (C-155 point 3): `--argmax-placa` = the argmax program after the copy of the logits, checked (the trial).
    // P6-126 (C-155 point 4): `--argmax-fara-copie` = in its place, the argmax program alone (the token chosen on the
    // board, checked after it); not with the two-run lists (they need the logits). P6-138 (cyber-gpu note C-157_P2j):
    // with `--dieta2` too (only its parts A+B run then: two records, the planned wait).
    // P6-119 (C-157 point 3): `--dieta2` = the driver's second diet for #513 (nothing new for the board).
    // P6-117 (C-161 point 2): `--p8` = after #513's texts, P8 (the first tensor-core computation).
    // P6-134 (C-161 point 4): `--colsum` = after everything, the first backward program of the training.
    // P6-136 (C-161 point 4): `--pas-aur` = after everything (after `--colsum`), the small golden step on the board.
    // P6-137 (C-161 point 4): `--mm-tc` = after everything (after `--pas-aur`), the `mm_tc` probe (training matmul).
    // P6-141 (C-161 point 4): `--atentie-placi` = after everything (after `--mm-tc`), the tiled training attention probe.
    // Each at most once, in this order (the I9 text carries them as suffixes, read back in the same order).
    let mut out = String::from(i9);
    let mut next = 0;
    // P6-128: `--ln-32` = `lipita-ln`, then `lipita-ln-32` (every product on quarters), compared; one more list choice.
    // P6-135 (note C-157_P2i, point 1): `--lipita-ln` = the default until now (`lipita-ln` alone); without any list
    // choice the cycle now runs `lipita-ln-32` alone. The checks below go by name, not by position.
    let all = [FLR_PROBA, TABLOU, LIPIT, SIMPLU, LN_NOU, LN_VECHI, LIPITA_LN, SFERTURI, LN_32, K651_OPT, ARGMAX_PLACA, ARGMAX_TOP, DIETA2, P8, COLSUM, PAS_AUR, MM_TC, ATTN_TILE];
    let pos = |x: &str| all.iter().position(|y| *y == x).unwrap_or(0);
    let lists = pos(LIPIT)..=pos(LN_32);
    // The two-run lists: the argmax without the copy reads no logits, they need both runs' logits.
    let two_runs = [LIPIT, LN_NOU, SFERTURI, LN_32];
    for f in flags {
        let Some(at) = all.iter().position(|x| x.trim() == *f) else { return Some(Err(FORM)) };
        if at < next {
            return Some(Err(FORM));
        }
        if all[at] == K651_OPT && (out.ends_with(LIPIT) || out.ends_with(SIMPLU)) {
            return Some(Err(FORM));
        }
        if all[at] == ARGMAX_TOP && two_runs.iter().any(|x| out.split(' ').any(|w| w == x.trim())) {
            return Some(Err(FORM));
        }
        out.push_str(all[at]);
        // The list choices exclude each other (each picks the lists #513 runs with); likewise `--argmax-placa` and
        // `--argmax-fara-copie`.
        next = if lists.contains(&at) {
            pos(K651_OPT)
        } else if all[at] == ARGMAX_PLACA {
            pos(DIETA2)
        } else {
            at + 1
        };
    }
    Some(Ok(out))
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
    let nth = match board_slot() {
        Ok(n) => n,
        Err(why) => {
            clear_state();
            return format!("CICLU OPRIT: {}; nimic scris in placa\n", why);
        }
    };
    let (text, result) = board_test(&s.i9, nth);
    board_finished(&result);
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

/// P6-122 (C-158 point 3, cyber-gpu note C-157_P4): #651's figures from the gpu journal of this test, next to what
/// Cyber-GPU expects (DEDUS): ~5.1-5.6 ms per token without the check (~180-195 tokens/s); far above ~6 ms = something
/// does not scale with the layers as expected.
fn k651_comparison() -> String {
    let Some(path) = crate::drivers::gpu_i6::last_journal() else {
        return String::from("\n-- #651: fara gpu-jurnal\n");
    };
    let now = String::from_utf8_lossy(&fs::read_all(&path).unwrap_or_default()).into_owned();
    k651_block(&path, &now)
}

fn k651_block(path: &str, journal: &str) -> String {
    let v = |n: &str| journal_value(journal, n);
    let token = match v("kt median simbol fara verificare (us)") {
        Some(us) => format!(
            "{} us ({}; asteptat ~5 100-5 600 us)",
            us,
            if us <= 6_000 { "in asteptari" } else { "PESTE ~6 ms -> de vazut de Cyber-GPU" }
        ),
        None => String::from("necitit (randul lipseste din jurnal)"),
    };
    let x100 = |x: u32| format!("{}.{:02}", x / 100, x % 100);
    let host = v("kv simboluri pe secunda x100 (procesor)").map_or(String::from("necitit"), x100);
    let driver = v("kt median driver (us)").map_or(String::from("?"), |w| format!("{}", w));
    let print = v("kt amprenta logit-urilor (FNV-1a, toate pozitiile)").map_or(String::from("?"), |w| format!("{:#010x}", w));
    format!(
        "\n-- Kronos #651 (jurnalul {})\nsimbolul fara verificare (kt median): {}\ndriverul pe simbol (kt median): {} us; simboluri/s cu driverul: {}\namprenta logit-urilor (prima la #651, pentru testele urmatoare): {}\n(asteptat de Cyber-GPU: VRAM ~270 MiB, urcarea in 25 de bucati, 172 de lansari pe simbol cu LayerNorm nou; simbolurile = cele ale procesorului)\n",
        path, token, driver, host, print
    )
}

/// P6-107 (C-154 point 3): the os-322 figures (20 Nov 15:31, `gpu-jurnal-017.txt` + `ciclu-auto-20261120-1531.txt`):
/// kt driver 3 081 us, kt token without the check 5 802 us, 94.84 tokens/s with the driver (`kv … (procesor)`), the
/// worst deviation 1.6212463e-5 (f32 bits 0x37880000) and the 16 tokens.
const OS322_DRIVER_US: u32 = 3_081;
const OS322_TOKEN_US: u32 = 5_802;
const OS322_HOST_X100: u32 = 9_484;
const OS322_WORST_BITS: u32 = 0x3788_0000;
const OS322_TOKENS: &str = "[44, 10, 73, 2524, 10766, 1438, 1505, 1428, 1765, 99, 1547, 3644, 44, 10, 15879, 1505]";

/// P6-107: the block "viteza fata de os-322" from the gpu journal of this test and the cycle's text so far.
fn diet_comparison(text: &str) -> String {
    let Some(path) = crate::drivers::gpu_i6::last_journal() else {
        return String::from("\n-- viteza fata de os-322: fara gpu-jurnal\n");
    };
    let now = String::from_utf8_lossy(&fs::read_all(&path).unwrap_or_default()).into_owned();
    diet_block(&path, &now, text)
}

/// The last value of the journal row `name` (`None`: missing, or u32::MAX = not measured).
fn journal_value(journal: &str, name: &str) -> Option<u32> {
    journal
        .lines()
        .filter_map(nvidia_ga104::kronos_perf::value_row)
        .filter(|(n, _)| *n == name)
        .map(|(_, v)| v)
        .last()
        .filter(|&v| v != u32::MAX)
}

fn diet_block(path: &str, journal: &str, text: &str) -> String {
    let v = |n: &str| journal_value(journal, n);
    let us = |what: &str, before: u32, now: Option<u32>| match now {
        Some(a) => format!("{}: os-322 {} -> acum {} ({} la mie)\n", what, before, a, u64::from(a) * 1000 / u64::from(before.max(1))),
        None => format!("{}: necitit (randul lipseste din jurnal)\n", what),
    };
    let x100 = |x: u32| format!("{}.{:02}", x / 100, x % 100);
    let host = match v("kv simboluri pe secunda x100 (procesor)") {
        Some(a) => format!("simboluri/s cu driverul: os-322 {} -> acum {}\n", x100(OS322_HOST_X100), x100(a)),
        None => String::from("simboluri/s cu driverul: necitit\n"),
    };
    let diet = match v("kt dieta (0/1)") {
        Some(1) => "pornita",
        Some(_) => "OPRITA (testul nu a folosit dieta)",
        None => "necitita (randul lipseste)",
    };
    let words = v("kt cuvinte VRAM pe simbol (median)").map_or(String::from("?"), |w| format!("{}", w));
    let read = v("kt median citirea logit-urilor (us)").map_or(String::from("?"), |w| format!("{}", w));
    let argmax = v("kt median argmax (us)").map_or(String::from("?"), |w| format!("{}", w));
    let tokens = text.lines().find(|l| l.contains("kronos: simboluri generate: ")).and_then(|l| l.split("simboluri generate: ").nth(1)).map(str::trim);
    let same_tokens = tokens == Some(OS322_TOKENS);
    // P6-135 (cyber-gpu note C-157_P2i, point 3): the bit-for-bit question is asked against the reference print of the
    // list that ran last (each kind of list adds in its own order, so os-322's bits are only the plain list's).
    let kind = v(crate::drivers::gpu_i14::KT_LIST);
    let print_now = v("kt amprenta logit-urilor (FNV-1a, toate pozitiile)");
    let list_ref = match kind {
        Some(6) | Some(7) => Some(("lipita-ln-32", REF_PRINT_LN32)),
        Some(3) | Some(4) => Some(("lipita-ln", REF_PRINT_LN_NEW)),
        Some(1) | Some(2) => Some(("lista lipita", REF_PRINT_LN_OLD)),
        _ => None,
    };
    let bits_question = match list_ref {
        Some((name, _)) => format!("cu referinta listei care a rulat ({})", name),
        None => String::from("cu os-322"),
    };
    let bits = match (list_ref, print_now) {
        (Some((_, want)), Some(w)) => format!(
            "{}: amprenta {:#010x}, referinta {:#010x}; simbolurile {} fata de os-322",
            if w == want { "DA" } else { "NU -> de vazut de Cyber-GPU" },
            w,
            want,
            if same_tokens { "aceleasi" } else { "DIFERITE" }
        ),
        (Some(_), None) => String::from("necitit (amprenta lipseste din jurnal)"),
        (None, _) => match v("kt abaterea maxima a logit-urilor (f32, biti)") {
        Some(b) if b == OS322_WORST_BITS && same_tokens => format!(
            "DA, dupa cat se poate vedea: abaterea maxima fata de procesor {:#010x} = exact cea din os-322 si aceleasi 16 simboluri (os-322 nu a scris amprenta tuturor logit-urilor)",
            b
        ),
        Some(b) => format!(
            "NU: abaterea maxima {:#010x} (os-322 {:#010x}), simbolurile {} -> de vazut de Cyber-GPU",
            b,
            OS322_WORST_BITS,
            if same_tokens { "aceleasi" } else { "DIFERITE" }
        ),
        None => String::from("necitit (randul lipseste din jurnal)"),
        },
    };
    // P6-126: without the copy the logits are not read: no deviation, no print (they are in the runs with the copy).
    let top = v(crate::drivers::gpu_i14::KT_TOP_ON) == Some(1);
    let bits = if top { String::from("nu se poate spune: argmax fara copie (logit-urile nu se citesc; simbolurile placii = procesorului, vezi randul kt argmax fara copie)") } else { bits };
    let print_v = if top { None } else { v("kt amprenta logit-urilor (FNV-1a, toate pozitiile)") };
    let print = if top { String::from("- (argmax fara copie)") } else { print_v.map_or(String::from("?"), |w| format!("{:#010x}", w)) };
    // P6-120 (cyber-gpu note C-157_P2d, point 1): the print against the reference of the same kind of LayerNorm (the
    // last run's list: 3 = the old one then the new one, 4 = the new one alone).
    let reference = match v(crate::drivers::gpu_i14::KT_LIST) {
        // P6-124: 5 = the glued list then quarters; quarters have no reference print yet (this is the first).
        Some(5) => {
            let last = if text.contains("kt rularile:") { "ultima rulare = " } else { "" };
            format!("amprenta fata de referinta: {}lista lipita pe sferturi (lipita-32): fara referinta inca (aceasta e prima; abaterea fata de lista lipita e in randul kt sferturi)\n", last)
        }
        Some(_) if top => String::from("amprenta fata de referinta: fara amprenta in aceasta rulare (argmax fara copie: logit-urile raman pe placa)\n"),
        // P6-128: 6 = `lipita-ln` then `lipita-ln-32`; P6-135: 7 = `lipita-ln-32` alone; its reference is the cycle of
        // 22 Nov 10:06 (note C-157_P2i, point 1).
        Some(k) => {
            let (what, want) = match k {
                6 | 7 => ("lipita-ln-32", REF_PRINT_LN32),
                3 | 4 => ("LayerNorm nou", REF_PRINT_LN_NEW),
                _ => ("LayerNorm vechi", REF_PRINT_LN_OLD),
            };
            let last = if text.contains("kt rularile:") { "ultima rulare = " } else { "" };
            match print_v {
                Some(w) => format!("amprenta fata de referinta: {}{} (referinta {:#010x}): {}\n", last, what, want, if w == want { "ACEEASI" } else { "ALTA -> de vazut de Cyber-GPU" }),
                None => format!("amprenta fata de referinta: necitita ({} {:#010x})\n", what, want),
            }
        }
        None => String::from("amprenta fata de referinta: felul listei necitit (randul kt lista lipseste)\n"),
    };
    // P6-120 (note C-157_P2d, point 2): with two runs, which rows cover which.
    let runs_note = if text.contains("kt rularile:") {
        "atentie, doua rulari pe acelasi text: simboluri/s cu driverul = ambele rulari; driverul si simbolul (randurile kt), cuvintele VRAM, citirea, argmax si amprenta = ultima rulare\n"
    } else {
        ""
    };
    format!(
        "\n-- viteza fata de os-322 (dieta driverului {}; jurnalul {})\n{}{}{}cuvinte VRAM pe simbol (median): {}; citirea logit-urilor {} us, argmax {} us (median)\nlogit-urile identice bit cu bit {}? {}\namprenta logit-urilor de acum (pentru testele urmatoare): {}\n{}{}",
        diet,
        path,
        us("driverul pe simbol (us, kt median driver)", OS322_DRIVER_US, v("kt median driver (us)")),
        us("simbolul fara verificare (us, kt)", OS322_TOKEN_US, v("kt median simbol fara verificare (us)")),
        host,
        words,
        read,
        argmax,
        bits_question,
        bits,
        print,
        reference,
        runs_note
    )
}

/// P6-120 (cyber-gpu note C-157_P2d, point 1): #513's logit print on the board with the old LayerNorm (os-328 … os-335)
/// and with the new one (os-331, `layernorm256`).
const REF_PRINT_LN_OLD: u32 = 0xda9a_e084;
const REF_PRINT_LN_NEW: u32 = 0x46a0_7914;
/// P6-135 (cyber-gpu note C-157_P2i, point 1): #513's logit print with `lipita-ln-32` (cycle 22 Nov 10:06, os-346).
const REF_PRINT_LN32: u32 = 0x6c85_01f0;

/// Suffix of the I9 text from `board_command` when the FLR trial is asked (`test-placa … --flr-proba`).
pub const FLR_PROBA: &str = " --flr-proba";

/// P6-105: suffix of the I9 text when the dashboard is asked (`test-placa … --tablou`, always last).
pub const TABLOU: &str = " --tablou";

/// P6-110: suffix of the I9 text when P3's glued list is asked (`test-placa … --lipit`, always last).
pub const LIPIT: &str = " --lipit";

/// P6-112: suffix of the I9 text when the plain list alone is asked (`test-placa … --simplu`, always last).
pub const SIMPLU: &str = " --simplu";

/// P6-114: suffix of the I9 text when the new LayerNorm is asked (`test-placa … --ln-nou`, always last).
pub const LN_NOU: &str = " --ln-nou";

/// P6-120: suffix of the I9 text when the old LayerNorm alone is asked (`test-placa … --ln-vechi`, as in os-329 … os-335).
pub const LN_VECHI: &str = " --ln-vechi";

/// P6-124 (C-155 point 4): suffix of the I9 text when the race on quarters is asked (`test-placa … --sferturi`, in the
/// place of the other list choices).
pub const SFERTURI: &str = " --sferturi";

/// P6-128 (C-157 point 2, note C-157_P2g): suffix of the I9 text when the chosen list `lipita-ln-32` is asked against
/// `lipita-ln` (`test-placa … --ln-32`, in the place of the other list choices).
pub const LN_32: &str = " --ln-32";

/// P6-135 (note C-157_P2i, point 1): suffix of the I9 text when `lipita-ln` alone is asked (`test-placa … --lipita-ln`,
/// the cycle's default from os-336 to os-349; in the place of the other list choices).
pub const LIPITA_LN: &str = " --lipita-ln";

/// P6-119 (C-157 point 3): suffix of the I9 text when the second diet is asked (`test-placa … --dieta2`, before `--p8`).
pub const DIETA2: &str = " --dieta2";

/// P6-122 (C-158 point 3): suffix of the I9 text when #651 is asked (`test-placa … --k651`, before `--dieta2`).
pub const K651_OPT: &str = " --k651";

/// P6-123 (C-155 point 3): suffix of the I9 text when the argmax on the board is asked (`test-placa … --argmax-placa`,
/// after `--k651`, before `--dieta2`).
pub const ARGMAX_PLACA: &str = " --argmax-placa";

/// P6-126 (C-155 point 4): suffix of the I9 text when the argmax without the copy is asked (`test-placa …
/// --argmax-fara-copie`, in the place of `--argmax-placa`).
pub const ARGMAX_TOP: &str = " --argmax-fara-copie";

/// P6-117 (C-161 point 2): suffix of the I9 text when P8 is asked (`test-placa … --p8`, always last).
pub const P8: &str = " --p8";

/// P6-134 (C-161 point 4): suffix of the I9 text when the `colsum` probe is asked (`test-placa … --colsum`, always last).
pub const COLSUM: &str = " --colsum";

/// P6-136 (C-161 point 4): suffix of the I9 text when the small golden step on the board is asked (`test-placa …
/// --pas-aur`, always last).
pub const PAS_AUR: &str = " --pas-aur";

/// P6-137 (C-161 point 4): suffix of the I9 text when the `mm_tc` probe is asked (`test-placa … --mm-tc`, always last).
pub const MM_TC: &str = " --mm-tc";

/// P6-141 (C-161 point 4): suffix of the I9 text when the tiled attention probe is asked (`test-placa … --atentie-placi`,
/// always last).
pub const ATTN_TILE: &str = " --atentie-placi";

/// P6-139 (C-158 point 5): at most this many board tests in one power-on (the list, the live mode and the hub cycle
/// together). The first one starts with `gpu recupereaza`; the next ones with the reset from inside (FLR) at the start of
/// their chain, the path that passed on the board in os-323 (21 Nov 08:32).
pub const MAX_BOARD_TESTS: usize = 4;
/// P6-139: board tests started in this power-on.
static BOARD_RUNS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// P6-139: set when a test left the board where no other test may start (a cold start is needed): why.
static BOARD_STUCK: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);

/// P6-139: may another board test start, after `done` in this power-on (`stuck` = why the board cannot go on)? `Ok(n)` =
/// it is the n-th (1 = `gpu recupereaza` first, 2.. = the reset from inside); `Err` = why not (nothing touched).
pub fn slot_decision(done: usize, stuck: Option<&str>) -> Result<usize, String> {
    if let Some(why) = stuck {
        return Err(format!(
            "placa nu a revenit dupa un test de mai devreme din aceasta pornire ({}); restul testelor pe placa nu ruleaza, fara alta incercare (trebuie pornire rece)",
            why
        ));
    }
    if done >= MAX_BOARD_TESTS {
        return Err(format!("cel mult {} teste pe placa intr-o pornire (C-158 punctul 5); acesta ar fi al {}-lea", MAX_BOARD_TESTS, done + 1));
    }
    Ok(done + 1)
}

/// P6-139: takes the next board-test slot of this power-on (see [`slot_decision`]).
pub fn board_slot() -> Result<usize, String> {
    use core::sync::atomic::Ordering;
    let stuck = BOARD_STUCK.lock().clone();
    let n = slot_decision(BOARD_RUNS.load(Ordering::SeqCst), stuck.as_deref())?;
    BOARD_RUNS.store(n, Ordering::SeqCst);
    Ok(n)
}

/// P6-139: does this result line leave the board where no other test may start? `Some(why)`.
pub fn stuck_after(result: &str) -> Option<&'static str> {
    if result.contains("trebuie pornire rece") {
        Some("gpu recupereaza nu a adus placa la starea de pornire")
    } else if result.contains("oprit la resetul din interior") {
        Some("resetul din interior nu a coborat WPR2")
    } else {
        None
    }
}

/// P6-139: after a board test: remember when the board cannot go on.
pub fn board_finished(result: &str) {
    if let Some(why) = stuck_after(result) {
        crate::warn!("ciclu: dupa testul pe placa: {}; restul testelor din aceasta pornire se refuza", why);
        *BOARD_STUCK.lock() = Some(String::from(why));
    }
}

/// P6-139: the board tests of this power-on so far.
pub fn board_runs() -> usize {
    BOARD_RUNS.load(core::sync::atomic::Ordering::SeqCst)
}

/// `nth` (P6-139) = which board test of this power-on this is (1 = the first: `gpu recupereaza`; 2.. = the reset from
/// inside at the start of the chain instead).
pub fn board_test(i9: &str, nth: usize) -> (String, String) {
    let (i9, attn_tile) = match i9.strip_suffix(ATTN_TILE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc) = match i9.strip_suffix(MM_TC) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, golden) = match i9.strip_suffix(PAS_AUR) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, colsum) = match i9.strip_suffix(COLSUM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, p8) = match i9.strip_suffix(P8) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, diet2) = match i9.strip_suffix(DIETA2) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, argmax_top) = match i9.strip_suffix(ARGMAX_TOP) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, argmax) = match i9.strip_suffix(ARGMAX_PLACA) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, k651) = match i9.strip_suffix(K651_OPT) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, ln32) = match i9.strip_suffix(LN_32) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, quarters) = match i9.strip_suffix(SFERTURI) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, lipita_ln) = match i9.strip_suffix(LIPITA_LN) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, ln_old) = match i9.strip_suffix(LN_VECHI) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, ln_new) = match i9.strip_suffix(LN_NOU) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, plain) = match i9.strip_suffix(SIMPLU) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, fused) = match i9.strip_suffix(LIPIT) {
        Some(x) => (x, true),
        None => (i9, false),
    };
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
    // P6-139 (C-158 point 5): the first test of the power-on brings the card back with `gpu recupereaza` (as before); the
    // next ones do not (Booter Unload over a running GSP never ran on the board) but arm the reset from inside for their
    // chain (os-323: WPR2 down in 0.13 s, the second chain passed whole). WPR2 already down = nothing written for it.
    let again = nth > 1;
    let rec = if again {
        text.push_str(&format!(
            "\n-- testul pe placa nr. {} in aceasta pornire: gpu recupereaza SARIT; resetul din interior (FLR, ca in proba os-323) la inceputul lantului\n",
            nth
        ));
        Vec::new()
    } else {
        text.push_str("\n-- gpu recupereaza\n");
        crate::drivers::gpu_rec::recover()
    };
    for l in &rec {
        text.push_str(l);
        text.push('\n');
    }
    if !again && !recovery_passed(&rec) {
        let why = String::from(
            "CICLU OPRIT: trebuie pornire rece (Adrian): gpu recupereaza nu a adus placa la starea de pornire (pregatirea I6 nu trece); lantul nu a pornit, nicio alta incercare",
        );
        crate::warn!("ciclu: {}", why);
        text.push_str(&format!("\n{}\n", why));
        return (text, why);
    }
    // P6-106 (C-148 completion 1): only the cycle's test asks the board for its maximum speed (not the FLR trial's chain).
    crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
    // P6-107 (C-154): the driver's diet, likewise only for the cycle's ordinary test.
    crate::drivers::gpu_tur::DIET_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
    // P6-105: the dashboard only when the list asked for it, and only for this chain (not the FLR trial's second one).
    crate::drivers::gpu_tur::RUSD_ARMED.store(dashboard, core::sync::atomic::Ordering::SeqCst);
    if dashboard {
        text.push_str("\n(tabloul de bord al placii: cerut de lista, la sfarsitul lantului, dupa randurile ks)\n");
    }
    // P6-110 (C-155 point 1): P3 only when the list asked for it, only for this chain's #513.
    crate::drivers::gpu_tur::FUSED_IN_CYCLE.store(fused, core::sync::atomic::Ordering::SeqCst);
    // P6-112 (center's note 21 Nov 13:00): otherwise the glued list alone, unless the list says `--simplu`.
    // P6-120 (center's proposal 21 Nov 19:25): the glued list with the old LayerNorm alone only with `--ln-vechi`;
    // without any list option, the new LayerNorm alone (it passed on the board in os-331 and os-332).
    crate::drivers::gpu_tur::FUSED_ONLY_IN_CYCLE.store(ln_old, core::sync::atomic::Ordering::SeqCst);
    // P6-135 (note C-157_P2i, point 1): the new LayerNorm alone only with `--lipita-ln`; without any list option,
    // `lipita-ln-32` alone (it passed on the board twice: 22 Nov 05:06 and 10:06).
    crate::drivers::gpu_tur::LN_ONLY_IN_CYCLE.store(lipita_ln, core::sync::atomic::Ordering::SeqCst);
    let default_list = !fused && !plain && !ln_new && !ln_old && !lipita_ln && !quarters && !ln32;
    crate::drivers::gpu_tur::LN32_ONLY_IN_CYCLE.store(default_list, core::sync::atomic::Ordering::SeqCst);
    // P6-124 (C-155 point 4): the race on quarters only when the list asked for it, only for this chain's model.
    crate::drivers::gpu_tur::QUARTERS_IN_CYCLE.store(quarters, core::sync::atomic::Ordering::SeqCst);
    // P6-128 (C-157 point 2): `lipita-ln` then `lipita-ln-32` only when the list asked for it.
    crate::drivers::gpu_tur::LN32_IN_CYCLE.store(ln32, core::sync::atomic::Ordering::SeqCst);
    // P6-114 (C-155 point 2): the new LayerNorm only when the list asked for it, only for this chain's #513.
    crate::drivers::gpu_tur::LN_NEW_IN_CYCLE.store(ln_new, core::sync::atomic::Ordering::SeqCst);
    // P6-117 (C-161 point 2): P8 only when the list asked for it, only after this chain's #513 texts.
    crate::drivers::gpu_tur::P8_IN_CYCLE.store(p8, core::sync::atomic::Ordering::SeqCst);
    // P6-134 (C-161 point 4): the `colsum` probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(colsum, core::sync::atomic::Ordering::SeqCst);
    // P6-136 (C-161 point 4): the small golden step only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(golden, core::sync::atomic::Ordering::SeqCst);
    // P6-137 (C-161 point 4): the `mm_tc` probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(mm_tc, core::sync::atomic::Ordering::SeqCst);
    // P6-141 (C-161 point 4): the tiled attention probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(attn_tile, core::sync::atomic::Ordering::SeqCst);
    // P6-119 (C-157 point 3): the second diet only when the list asked for it, only for this chain's #513.
    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(diet2, core::sync::atomic::Ordering::SeqCst);
    // P6-123 (C-155 point 3): the argmax on the board only when the list asked for it, only for this chain's model.
    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(argmax, core::sync::atomic::Ordering::SeqCst);
    // P6-126 (C-155 point 4): the argmax without the copy likewise only when the list asked for it.
    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(argmax_top, core::sync::atomic::Ordering::SeqCst);
    if argmax_top {
        text.push_str("\n(argmax fara copie: cerut de lista (--argmax-fara-copie); fiecare simbol se termina doar cu programul `argmax` (ca in os-340, fara copierea logit-urilor); OS-ul citeste cei 16 octeti si ia simbolul de acolo; dupa placa, procesorul calculeaza simbolul fiecarei pozitii: trebuie sa fie acelasi; logit-urile nu se mai citesc, deci fara amprenta si fara abaterea lor)\n");
    }
    if argmax {
        text.push_str("\n(argmax pe placa: cerut de lista (--argmax-placa); program nou `argmax` (256 de fire, o bariera, 16 octeti scrisi in RAM) dupa copierea logit-urilor, in acelasi simbol; la fiecare pozitie cele 4 cuvinte = argmax_result al logit-urilor copiate, prag 0; simbolul ales tot pe procesor)\n");
    }
    if diet2 && argmax_top {
        text.push_str("\n(dieta 2 a driverului: ceruta de lista (--dieta2), cu argmax fara copie: doar bucatile A+B (doua inregistrari citite pe cele mai multe pozitii, asteptarea planificata); nimic nou pentru placa; aceleasi simboluri, fara amprenta; Cyber-GPU asteapta driverul ~410-460 us in loc de ~608)\n");
    } else if diet2 {
        text.push_str("\n(dieta 2 a driverului: ceruta de lista (--dieta2); nimic nou pentru placa: doua inregistrari citite pe cele mai multe pozitii, asteptarea planificata, amprenta dupa ceas, argmax pe biti; aceleasi simboluri si aceeasi amprenta)\n");
    }
    if p8 {
        text.push_str("\n(P8, primul calcul cu circuitele speciale: cerut de lista (--p8); dupa textele lui #513, programul mma16_f16 de 6 ori cu 6 seturi fixe, judecat pe procesor cu toleranta scrisa; trece = 6 seturi in toleranta si identitate 256/256 exacte)\n");
    }
    if colsum {
        text.push_str("\n(primul program inapoi al antrenarii: cerut de lista (--colsum); dupa textele lui #513, programul colsum (sumele pe coloane ale unui gradient) de 5 ori cu 5 seturi fixe, judecat pe procesor cu marginea scrisa; trece = 5 seturi in margine, garzile neatinse; predictia Cyber-GPU: 3778 / 3778 elemente exacte fata de model)\n");
    }
    if golden {
        text.push_str("\n(pasul de aur mic pe placa: cerut de lista (--pas-aur); dupa textele lui #513, un pas intreg de antrenare al modelului mic (2 straturi, C 64, lot 2 x 32) de 10 ori NUMAI pe placa, pentru sgd (73 de lansari pe pas) si adamw_513 (75), cate o trimitere pe pas; 11 programe noi pentru placa (mm_train, xent, ln_fwd, ln_back, gelu_tr, attn_fwd, attn_back, emb_fwd, emb_back, gnorm, opt_step) + colsum + vadd; arena ~2,8 MiB in RAM; judecat fata de PyTorch in tolerantele hub-ului; trece = kt pas-aur verdict 1 la ambele retete, toate cifrele sub 1000 promile; datele de pe stick: antrenare/pas_de_aur_mic sau integrare/kronos/pas_de_aur_mic)\n");
    }
    if mm_tc {
        text.push_str("\n(proba mm_tc, inmultirea antrenarii pe circuitele speciale: ceruta de lista (--mm-tc); dupa textele lui #513, programul mm_tc (nou pentru placa: 64 de registre, blocuri de 128, 24 KiB de memorie partajata + o bariera, grila cu y > 1) de 5 ori cu 5 seturi fixe mici (judecate pe procesor cu marginea scrisa), apoi setul de timp qkv-513 (6144 x 2304 x 768, ~82,6 MB in VRAM, 5 lansari, judecat pe 3 benzi); trece = toate seturile rulate in margine, garzile neatinse; predictia Cyber-GPU: abaterea maxima sub 100 promile, qkv-513 la 8-17 TFLOP/s)\n");
    }
    if attn_tile {
        text.push_str("\n(proba atentiei pe placi: ceruta de lista (--atentie-placi); dupa textele lui #513, lantul atentiei de antrenare pe placi: attn_fwd_tile (P si att) -> attn_back_tile0 (randurile lui dS) -> attn_back_tile (dQ, dK, dV); 3 programe noi pentru placa (55 / 50 / 73 de registre, 37 / 32 / 32 KiB de memorie partajata + o bariera, blocuri de 256), pe 5 seturi mici (fiecare program judecat singur pe procesor: inainte in marginea MUFU, cele doua inapoi pe P si dS citite de pe placa, prezise bit cu bit), apoi setul atentie-513-b4 (4 texte x 512 pozitii, 12 capete; ~151 MB in VRAM, intrarile urcate si recitite inainte de orice lansare) de 5 ori fiecare program, pentru timp; trece = toate seturile in margine, garzile neatinse; Cyber-GPU estimeaza ~0,53 / ~0,49 / ~0,42-0,68 ms pe lansare)\n");
    }
    if ln32 {
        text.push_str("\n(lista aleasa de Cyber-GPU: ceruta de lista (--ln-32); #513 ruleaza de doua ori pe acelasi text: intai lipita-ln (implicitul, amprenta de referinta 0x46a07914), apoi lipita-ln-32 (LayerNorm nou + toate inmultirile pe sferturi; nimic nou pentru placa, aceleasi 88 de lansari si cbuf-uri); corect = aceleasi simboluri si abaterea sub 1e-4; timpul pe fel de lansare in ambele, un fel trece doar cu cel putin 50 la mie)\n");
    } else if quarters {
        text.push_str("\n(cursa inmultirilor pe sferturi: ceruta de lista (--sferturi); #513 ruleaza de doua ori pe acelasi text: intai lista lipita (LayerNorm vechi, ca in os-329), apoi lista lipita pe sferturi (programe noi q8_fused32 si q8_matvec32, aceleasi 88 de lansari si cbuf-uri); corect = aceleasi simboluri si abaterea sub 1e-4; un fel de lansare trece pe sferturi doar daca e mai rapid cu cel putin 50 la mie)\n");
    } else if ln_new {
        text.push_str("\n(LayerNorm nou: cerut de lista (--ln-nou); #513 ruleaza de doua ori pe acelasi text cu lista lipita: intai cu LayerNorm vechi, apoi cu layernorm256, comparate: aceleasi simboluri si abaterea sub 1e-4)\n");
    } else if fused {
        text.push_str("\n(P3, lista lipita: cerut de lista; #513 ruleaza de doua ori pe acelasi text: intai lista simpla, apoi cea lipita, comparate bit cu bit)\n");
    } else if plain {
        text.push_str("\n(lista simpla: cerut de lista (--simplu); #513 ca in os-324)\n");
    } else if ln_old {
        text.push_str("\n(lista lipita cu LayerNorm vechi: cerut de lista (--ln-vechi); #513 ca in os-329 ... os-335, amprenta de referinta 0xda9ae084)\n");
    } else if lipita_ln {
        text.push_str("\n(lista lipita-ln: ceruta de lista (--lipita-ln); #513 ca in os-336 ... os-349 (LayerNorm nou, 88 de lansari pe simbol), amprenta de referinta 0x46a07914)\n");
    } else {
        text.push_str("\n(lista lipita-ln-32 implicit (LayerNorm nou + toate inmultirile pe sferturi, 88 de lansari pe simbol, nimic nou pentru placa; amprenta de referinta 0x6c8501f0); --lipita-ln = implicitul de pana acum, --ln-32 = amandoua comparate, --ln-vechi = LayerNorm vechi, --simplu = lista simpla)\n");
    }
    // P6-122 (C-158 point 3): #651 only when the list asked for it; when its file is missing, #513 as usual (said in
    // the report and in the result line).
    let mut k651_note = "";
    let big = if k651 {
        match k651_path() {
            Some(path) => {
                text.push_str(&format!("\n(Kronos #651: cerut de lista (--k651); 24 de straturi, {}; aceleasi optiuni de lista ca la #513; #513 ramane implicit)\n", path));
                Some(path)
            }
            None => {
                text.push_str(&format!("\n(Kronos #651: cerut de lista (--k651), dar {} lipseste: rulez #513 ca de obicei)\n", K651));
                k651_note = "#651 LIPSESTE de pe stick (rulat #513); ";
                None
            }
        }
    } else {
        None
    };
    // P6-139: the reset from inside, taken by this test's chain (kronos's or the plain one).
    crate::drivers::gpu_tur::FLR_ARMED.store(again, core::sync::atomic::Ordering::SeqCst);
    let result = if let Some(model) = big.or_else(k513_path) {
        let args651 = big.map(k651_args);
        let args: &[&str] = match &args651 {
            Some(a) => a,
            None => &K513_ARGS,
        };
        let which = if big.is_some() { "#651" } else { "#513" };
        if big.is_none() && model != K513 {
            text.push_str(&format!("\n(#513 de pe discul propriu: {})\n", model));
        }
        text.push_str(&format!("\n-- run {} (lantul cu I9 {}, apoi {} la i14-simbol; DA dat de cheile ciclului)\n", args.join(" "), i9, which));
        *crate::drivers::gpu_tur::KRONOS_I9.lock() = Some(choice);
        crate::drivers::gpu_tur::KRONOS_ARMED.store(true, core::sync::atomic::Ordering::SeqCst);
        let (code, printed) = run_program("/bin/kronos", args);
        // The permission never outlives this one program (as in the shell).
        crate::drivers::gpu_tur::KRONOS_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
        *crate::drivers::gpu_tur::KRONOS_I9.lock() = None;
        text.push_str(&printed);
        text.push_str(&format!("(kronos a iesit cu codul {})\n", code));
        let chain = printed.lines().rev().find(|l| l.contains("rezultat lant:")).map(String::from);
        let board = printed.lines().find(|l| l.contains("kronos: PE PLACA")).map(String::from);
        match (chain, board) {
            (Some(c), Some(b)) => format!("{}; {}", c.trim(), b.trim()),
            (Some(c), None) => format!("{}; {} NU a rulat pe placa (vezi randurile kronos de mai sus)", c.trim(), which),
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
    // P6-139: never left armed when the program stopped before its chain.
    crate::drivers::gpu_tur::FLR_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::DIET_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::RUSD_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::FUSED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::FUSED_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN_NEW_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::QUARTERS_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN32_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN32_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::P8_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    // P6-126: the verdict of the argmax without the copy next to the chain's in the answer.
    let result = if argmax_top {
        let verdict = text.lines().rev().find(|l| l.contains("argmax fara copie (simbolul ales pe placa)")).map_or("argmax fara copie: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-123: the argmax trial's verdict next to the chain's in the answer.
    let result = if argmax {
        let verdict = text.lines().rev().find(|l| l.contains("argmax pe placa (proba")).map_or("argmax pe placa: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-117: P8's verdict next to the chain's in the answer (the rows themselves are in the report above).
    let result = if p8 {
        let verdict = text.lines().rev().find(|l| l.contains("P8 (mma16): ")).map_or("P8: niciun rezultat (vezi raportul)", |l| l.trim());
        // P6-125: the adder model's bit-for-bit count, when the probe got that far.
        match text.lines().rev().find(|l| l.contains("P8 model ga104")) {
            Some(fit) => format!("{}; {}; {}", result, verdict, fit.trim()),
            None => format!("{}; {}", result, verdict),
        }
    } else {
        result
    };
    // P6-134: the `colsum` probe's verdict next to the chain's in the answer.
    let result = if colsum {
        let verdict = text.lines().rev().find(|l| l.contains("colsum (primul program inapoi): ")).map_or("colsum: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-136: the small golden step's verdict next to the chain's in the answer.
    let result = if golden {
        let verdict = text
            .lines()
            .rev()
            .find(|l| l.contains("pasul de aur mic pe placa: ") && (l.contains("TRECE") || l.contains("NU A TERMINAT") || l.contains("NU A RULAT") || l.contains("oprit")))
            .map_or("pasul de aur mic: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-137: the `mm_tc` probe's verdict next to the chain's in the answer.
    let result = if mm_tc {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc: ")).map_or("proba mm_tc: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-141: the tiled attention probe's verdict next to the chain's in the answer.
    let result = if attn_tile {
        let verdict = text.lines().rev().find(|l| l.contains("proba atentiei pe placi: ")).map_or("proba atentiei pe placi: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    let result = format!("{}{}", k651_note, result);
    // P6-139: the n-th test of the power-on says so in its answer.
    let result = if again { format!("testul pe placa nr. {} in aceasta pornire (cu resetul din interior): {}", nth, result) } else { result };
    // P6-122: the comparisons with os-313 / os-322 are #513's; #651 gets its own block (Cyber-GPU's expected figures).
    if big.is_some() {
        text.push_str(&k651_comparison());
    } else {
        text.push_str(&speed_comparison());
        text.push_str(&diet_comparison(&text));
    }
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
    // P6-139 (C-158 point 5): the n-th board test of a power-on; refused past the limit or after a board that did not
    // come back; which results block the rest.
    assert!(slot_decision(0, None) == Ok(1) && slot_decision(1, None) == Ok(2) && slot_decision(3, None) == Ok(4));
    assert!(slot_decision(4, None).is_err_and(|w| w.contains("cel mult 4 teste") && w.contains("al 5-lea")));
    assert!(slot_decision(1, Some("x")).is_err_and(|w| w.contains("placa nu a revenit") && w.contains("(x)")));
    assert!(slot_decision(0, Some("x")).is_err());
    assert!(stuck_after("CICLU OPRIT: trebuie pornire rece (Adrian): gpu recupereaza nu a adus placa").is_some());
    assert!(stuck_after("testul pe placa nr. 2 in aceasta pornire (cu resetul din interior): gpu0: rezultat lant: oprit la resetul din interior (x); fara a doua incercare").is_some());
    assert!(stuck_after("testul pe placa nr. 2 in aceasta pornire (cu resetul din interior): gpu0: rezultat lant: toti pasii au trecut").is_none());
    assert!(stuck_after("CICLU OPRIT: procesorul e prea cald").is_none() && stuck_after("gpu0: rezultat lant: oprit la I9").is_none());
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
    // P6-107 (C-154): the block "viteza fata de os-322": the diet on, the driver halved, the same worst deviation and
    // tokens = "DA"; another worst deviation = "NU"; a journal without the rows = "necitit".
    let j = "x valoare kt median driver (us) = 0x00000604\nx valoare kt median simbol fara verificare (us) = 0x00000fa0\nx valoare kv simboluri pe secunda x100 (procesor) = 0x00006180\nx valoare kt dieta (0/1) = 0x00000001\nx valoare kt cuvinte VRAM pe simbol (median) = 0x000000d2\nx valoare kt median citirea logit-urilor (us) = 0x00000190\nx valoare kt median argmax (us) = 0x00000014\nx valoare kt abaterea maxima a logit-urilor (f32, biti) = 0x37880000\nx valoare kt amprenta logit-urilor (FNV-1a, toate pozitiile) = 0x1234abcd\n";
    let t = format!("kronos: simboluri generate: {}\n", OS322_TOKENS);
    let d = diet_block("/x", j, &t);
    assert!(d.contains("dieta driverului pornita") && d.contains("os-322 3081 -> acum 1540 (499 la mie)") && d.contains("os-322 5802 -> acum 4000 (689 la mie)"), "test: dieta: {}", d);
    assert!(d.contains("cu driverul: os-322 94.84 -> acum 249.60") && d.contains("(median): 210;") && d.contains("400 us, argmax 20 us"), "test: dieta: {}", d);
    assert!(d.contains("bit cu bit cu os-322? DA") && d.contains("0x1234abcd"), "test: dieta: {}", d);
    let d = diet_block("/x", &j.replace("0x37880000", "0x37880001"), &t);
    assert!(d.contains("bit cu bit cu os-322? NU: abaterea maxima 0x37880001") && d.contains("simbolurile aceleasi"), "test: dieta NU: {}", d);
    let d = diet_block("/x", &j.replace("0x37880000", "0x37880000"), "kronos: simboluri generate: [1]\n");
    assert!(d.contains("simbolurile DIFERITE"), "test: dieta simboluri: {}", d);
    let d = diet_block("/x", "", "");
    assert!(d.contains("dieta driverului necitita") && d.contains("driverul pe simbol (us, kt median driver): necitit") && d.contains("os-322? necitit"), "test: dieta gol: {}", d);
    // P6-101: the FLR trial form.
    assert!(board_command("test-placa --flr-proba") == Some(Ok(String::from("c135 --flr-proba"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba") == Some(Ok(String::from("c133 --flr-proba"))));
    assert!(board_command("test-placa --flr-proba --i9 c133").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --tablou") == Some(Ok(String::from("c135 --tablou"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou") == Some(Ok(String::from("c133 --flr-proba --tablou"))));
    assert!(board_command("test-placa --tablou --flr-proba").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit") == Some(Ok(String::from("c135 --lipit"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou --lipit") == Some(Ok(String::from("c133 --flr-proba --tablou --lipit"))));
    assert!(board_command("test-placa --tablou --lipit") == Some(Ok(String::from("c135 --tablou --lipit"))));
    assert!(board_command("test-placa --lipit --tablou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit --lipit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --tablou --simplu") == Some(Ok(String::from("c135 --tablou --simplu"))));
    assert!(board_command("test-placa --lipit --simplu").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --simplu --lipit").is_some_and(|r| r.is_err()));
    // P6-114: `--ln-nou` last, alone among the three list choices.
    assert!(board_command("test-placa --ln-nou") == Some(Ok(String::from("c135 --ln-nou"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou --ln-nou") == Some(Ok(String::from("c133 --flr-proba --tablou --ln-nou"))));
    assert!(board_command("test-placa --lipit --ln-nou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --simplu --ln-nou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-nou --tablou").is_some_and(|r| r.is_err()));
    // P6-117: `--p8` last, alone or after any other option.
    assert!(board_command("test-placa --p8") == Some(Ok(String::from("c135 --p8"))));
    assert!(board_command("test-placa --ln-nou --p8") == Some(Ok(String::from("c135 --ln-nou --p8"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou --lipit --p8") == Some(Ok(String::from("c133 --flr-proba --tablou --lipit --p8"))));
    assert!(board_command("test-placa --p8 --tablou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --p8 --p8").is_some_and(|r| r.is_err()));
    // P6-134: `--colsum` last, alone or after any other option (`--p8` included).
    assert!(board_command("test-placa --colsum") == Some(Ok(String::from("c135 --colsum"))));
    assert!(board_command("test-placa --tablou --p8 --colsum") == Some(Ok(String::from("c135 --tablou --p8 --colsum"))));
    assert!(board_command("test-placa --tablou --ln-32 --k651 --argmax-placa --dieta2 --p8 --colsum") == Some(Ok(String::from("c135 --tablou --ln-32 --k651 --argmax-placa --dieta2 --p8 --colsum"))));
    assert!(board_command("test-placa --colsum --p8").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --colsum --colsum").is_some_and(|r| r.is_err()));
    // P6-136: `--pas-aur` last, alone or after any other option (`--colsum` included).
    assert!(board_command("test-placa --pas-aur") == Some(Ok(String::from("c135 --pas-aur"))));
    assert!(board_command("test-placa --colsum --pas-aur") == Some(Ok(String::from("c135 --colsum --pas-aur"))));
    assert!(board_command("test-placa --tablou --p8 --colsum --pas-aur") == Some(Ok(String::from("c135 --tablou --p8 --colsum --pas-aur"))));
    assert!(board_command("test-placa --pas-aur --colsum").is_some_and(|r| r.is_err()));
    // P6-137: `--mm-tc` last, alone or after any other option (`--pas-aur` included).
    assert!(board_command("test-placa --mm-tc") == Some(Ok(String::from("c135 --mm-tc"))));
    assert!(board_command("test-placa --pas-aur --mm-tc") == Some(Ok(String::from("c135 --pas-aur --mm-tc"))));
    assert!(board_command("test-placa --tablou --p8 --colsum --pas-aur --mm-tc") == Some(Ok(String::from("c135 --tablou --p8 --colsum --pas-aur --mm-tc"))));
    assert!(board_command("test-placa --mm-tc --pas-aur").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc").is_some_and(|r| r.is_err()));
    // P6-141: `--atentie-placi` last, alone or after any other option (`--mm-tc` included).
    assert!(board_command("test-placa --atentie-placi") == Some(Ok(String::from("c135 --atentie-placi"))));
    assert!(board_command("test-placa --mm-tc --atentie-placi") == Some(Ok(String::from("c135 --mm-tc --atentie-placi"))));
    assert!(board_command("test-placa --tablou --pas-aur --mm-tc --atentie-placi") == Some(Ok(String::from("c135 --tablou --pas-aur --mm-tc --atentie-placi"))));
    assert!(board_command("test-placa --atentie-placi --mm-tc").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie-placi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --pas-aur --pas-aur").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --simplu --p8 --ln-nou").is_some_and(|r| r.is_err()));
    // P6-120: `--ln-vechi` in the same place as `--ln-nou`, alone among the four list choices.
    assert!(board_command("test-placa --ln-vechi") == Some(Ok(String::from("c135 --ln-vechi"))));
    assert!(board_command("test-placa --tablou --ln-vechi --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --ln-vechi --dieta2 --p8"))));
    assert!(board_command("test-placa --ln-nou --ln-vechi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-vechi --lipit").is_some_and(|r| r.is_err()));
    // P6-122 (C-158 point 3): `--k651` after the list choice, before `--dieta2`; never with the plain list.
    assert!(board_command("test-placa --k651") == Some(Ok(String::from("c135 --k651"))));
    assert!(board_command("test-placa --tablou --ln-nou --k651 --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --ln-nou --k651 --dieta2 --p8"))));
    assert!(board_command("test-placa --ln-vechi --k651") == Some(Ok(String::from("c135 --ln-vechi --k651"))));
    assert!(board_command("test-placa --simplu --k651").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit --k651").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --k651 --ln-nou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --dieta2 --k651").is_some_and(|r| r.is_err()));
    assert!(k651_args("/usb/kronos/kronos_651_q8_0.kgr").join(" ") == "kronos --gpu --model /usb/kronos/kronos_651_q8_0.kgr --n 16 Buna ziua");
    let b = k651_block("j.txt", "valoare kt median simbol fara verificare (us) = 0x000015e0\nvaloare kv simboluri pe secunda x100 (procesor) = 0x00004a38\n");
    assert!(b.contains("5600 us (in asteptari") && b.contains("simboluri/s cu driverul: 190.00"), "test: blocul #651: {}", b);
    assert!(k651_block("j.txt", "valoare kt median simbol fara verificare (us) = 0x00001b58\n").contains("PESTE ~6 ms"), "test: #651 lent");
    assert!(board_command("test-placa --ln-vechi --tablou").is_some_and(|r| r.is_err()));
    // P6-120: the reference print by the kind of LayerNorm (cyber-gpu note C-157_P2d, point 1).
    let jl = |kind: u32, print: u32| format!("x valoare {} = {:#010x}\nx valoare kt amprenta logit-urilor (FNV-1a, toate pozitiile) = {:#010x}\n", crate::drivers::gpu_i14::KT_LIST, kind, print);
    let d = diet_block("/x", &jl(4, 0x46a0_7914), "");
    assert!(d.contains("LayerNorm nou (referinta 0x46a07914): ACEEASI"), "test: amprenta ln nou: {}", d);
    let d = diet_block("/x", &jl(1, 0x46a0_7914), "");
    assert!(d.contains("LayerNorm vechi (referinta 0xda9ae084): ALTA"), "test: amprenta ln vechi: {}", d);
    let d = diet_block("/x", &jl(3, 0x46a0_7914), "kronos --gpu: kt rularile: 2 rulari\n");
    assert!(d.contains("ultima rulare = LayerNorm nou (referinta 0x46a07914): ACEEASI") && d.contains("ambele rulari"), "test: amprenta doua rulari: {}", d);
    // P6-123: `--argmax-placa` after `--k651`, before `--dieta2`.
    assert!(board_command("test-placa --argmax-placa") == Some(Ok(String::from("c135 --argmax-placa"))));
    assert!(board_command("test-placa --tablou --ln-nou --k651 --argmax-placa --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --ln-nou --k651 --argmax-placa --dieta2 --p8"))));
    assert!(board_command("test-placa --simplu --argmax-placa") == Some(Ok(String::from("c135 --simplu --argmax-placa"))));
    assert!(board_command("test-placa --argmax-placa --k651").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --dieta2 --argmax-placa").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-placa --argmax-placa").is_some_and(|r| r.is_err()));
    // P6-128: `--ln-32` in the place of the list choices, alone among them; before `--k651`; not with `--argmax-fara-copie`.
    assert!(board_command("test-placa --ln-32") == Some(Ok(String::from("c135 --ln-32"))));
    assert!(board_command("test-placa --tablou --ln-32 --k651 --argmax-placa --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --ln-32 --k651 --argmax-placa --dieta2 --p8"))));
    assert!(board_command("test-placa --sferturi --ln-32").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-32 --ln-vechi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --k651 --ln-32").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-32 --argmax-fara-copie").is_some_and(|r| r.is_err()));
    let d = diet_block("/x", &jl(6, 0x1234_5678), "kronos --gpu: kt rularile: 2 rulari\n");
    assert!(d.contains("ultima rulare = lipita-ln-32 (referinta 0x6c8501f0): ALTA"), "test: amprenta ln-32: {}", d);
    // P6-135 (note C-157_P2i): `lipita-ln-32` alone is the default; `--lipita-ln` = the default until now, a list choice;
    // the argmax without the copy goes with the default and with `--lipita-ln` (one run each).
    assert!(board_command("test-placa --lipita-ln") == Some(Ok(String::from("c135 --lipita-ln"))));
    assert!(board_command("test-placa --tablou --lipita-ln --k651 --argmax-fara-copie --p8 --colsum") == Some(Ok(String::from("c135 --tablou --lipita-ln --k651 --argmax-fara-copie --p8 --colsum"))));
    assert!(board_command("test-placa --lipita-ln --ln-32").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-vechi --lipita-ln").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --k651 --lipita-ln").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --k651 --argmax-fara-copie") == Some(Ok(String::from("c135 --k651 --argmax-fara-copie"))));
    let d = diet_block("/x", &jl(7, 0x6c85_01f0), "");
    assert!(d.contains("amprenta fata de referinta: lipita-ln-32 (referinta 0x6c8501f0): ACEEASI"), "test: amprenta ln-32 singura: {}", d);
    assert!(d.contains("bit cu bit cu referinta listei care a rulat (lipita-ln-32)? DA: amprenta 0x6c8501f0"), "test: bit cu bit ln-32: {}", d);
    let d = diet_block("/x", &jl(4, 0x1111_2222), "");
    assert!(d.contains("(lipita-ln)? NU -> de vazut de Cyber-GPU: amprenta 0x11112222, referinta 0x46a07914"), "test: bit cu bit ln: {}", d);
    // P6-126: `--argmax-fara-copie` in the place of `--argmax-placa`, not with the two-run lists; P6-138: with `--dieta2`.
    assert!(board_command("test-placa --argmax-fara-copie") == Some(Ok(String::from("c135 --argmax-fara-copie"))));
    assert!(board_command("test-placa --tablou --k651 --argmax-fara-copie --p8") == Some(Ok(String::from("c135 --tablou --k651 --argmax-fara-copie --p8"))));
    assert!(board_command("test-placa --ln-vechi --argmax-fara-copie") == Some(Ok(String::from("c135 --ln-vechi --argmax-fara-copie"))));
    assert!(board_command("test-placa --argmax-placa --argmax-fara-copie").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-fara-copie --argmax-placa").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --sferturi --argmax-fara-copie").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ln-nou --argmax-fara-copie").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit --argmax-fara-copie").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-fara-copie --dieta2") == Some(Ok(String::from("c135 --argmax-fara-copie --dieta2"))));
    assert!(board_command("test-placa --tablou --k651 --argmax-fara-copie --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --k651 --argmax-fara-copie --dieta2 --p8"))));
    assert!(board_command("test-placa --dieta2 --argmax-fara-copie").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-placa --dieta2 --p8") == Some(Ok(String::from("c135 --argmax-placa --dieta2 --p8"))));
    // P6-124: `--sferturi` in the place of the list choices, alone among them; before `--k651`.
    assert!(board_command("test-placa --sferturi") == Some(Ok(String::from("c135 --sferturi"))));
    assert!(board_command("test-placa --tablou --sferturi --k651 --argmax-placa --dieta2 --p8") == Some(Ok(String::from("c135 --tablou --sferturi --k651 --argmax-placa --dieta2 --p8"))));
    assert!(board_command("test-placa --ln-nou --sferturi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --sferturi --ln-vechi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --sferturi --lipit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --k651 --sferturi").is_some_and(|r| r.is_err()));
    let d = diet_block("/x", &jl(5, 0x1234_5678), "kronos --gpu: kt rularile: 2 rulari\n");
    assert!(d.contains("ultima rulare = lista lipita pe sferturi (lipita-32): fara referinta inca"), "test: amprenta sferturi: {}", d);
    // P6-119: `--dieta2` after the list choice, before `--p8`.
    assert!(board_command("test-placa --dieta2") == Some(Ok(String::from("c135 --dieta2"))));
    assert!(board_command("test-placa --ln-nou --dieta2 --p8") == Some(Ok(String::from("c135 --ln-nou --dieta2 --p8"))));
    assert!(board_command("test-placa --p8 --dieta2").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --dieta2 --simplu").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --i9 c999").is_some_and(|r| r.is_err()));
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
