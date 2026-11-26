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

/// Every `test-placa` option in the one order the line must keep (each at most once); the I9 text carries them as
/// suffixes in this order. P6-236: shared with `board_refusal_hint`.
const BOARD_OPTIONS: &[&str] = &[FLR_PROBA, TABLOU, TABLOU_VITEZA, LIPIT, SIMPLU, LN_NOU, LN_VECHI, LIPITA_LN, SFERTURI, LN_32, K651_OPT, ARGMAX_PLACA, ARGMAX_TOP, DIETA2, LOT, TOKEN32, P8, P8_S8, BARIERA, BARIERA_LARGA, COLSUM, PAS_AUR, MM_TC, MM_TC128, MM_TC_PF, MM_TC_V4, MM_TC_V5, MM_TC128_V4, MM_TC_64K, ATTN_TILE, ATTN_TILE2, ATTN_TILE3, ATTN_TILE4, ATTN_TC, ATTN_TC_V4, ATTN_TC0, ATTN_TC0_V4, ATTN_TC_64K, RAM_VRAM, ADAM_RAM, TRAIN_MINI, TRAIN_MIXED, TRAIN_MIXED_FAST, TRAIN_VRAM, TRAIN_FUSED, TRAIN_513, TRAIN_513_TILES3, TRAIN_513_MM_TILE, TRAIN_513_FUSED_TILE, TRAIN_513_COLSUM, TRAIN_513_MIXED, TRAIN_513_MIXED_FAST, TRAIN_513_VIA_RAM, TRAIN_513_SLOTS, TRAIN_513_ATTN_TC16, TRAIN_513_ATTN_V4, TRAIN_513_ATTN_DS, TRAIN_513_MM_CHOICE, TRAIN_513_SM_STEP, TRAIN_513_MM_MODEL, TRAIN_513_RACE, TRAIN_513_PROBE_64K8, TRAIN_513_OCCUPANCY, TRAIN_RAM];

/// P6-96: `test-placa [--i9 …]` of the automatic list -> the I9.
pub fn board_command(command: &str) -> Option<Result<String, &'static str>> {
    const FORM: &str = "forma corecta: `test-placa` sau `test-placa --i9 c135` (c125, c133, c135, os302), plus optional `--flr-proba`, `--tablou` sau `--tablou-viteza`, `--lipit`, `--simplu`, `--ln-nou`, `--ln-vechi`, `--lipita-ln`, `--sferturi` sau `--ln-32`, apoi `--k651`, apoi `--argmax-placa` sau `--argmax-fara-copie`, apoi `--dieta2`, apoi `--lot`, apoi `--token32`, apoi `--p8` (urmat optional de `--p8-s8`), apoi `--bariera`, apoi `--bariera-larga`, apoi `--colsum`, apoi `--pas-aur`, apoi `--mm-tc` (urmat optional de `--mm-tc128`), apoi `--atentie-placi`, apoi `--atentie2`, apoi `--atentie3`, apoi `--atentie4`, apoi `--atentie-tc`, apoi `--atentie-tc16-v4`, apoi `--atentie-tc0`, apoi `--atentie-tc0-v4`, apoi `--atentie-tc-64k`, apoi `--ram-vram`, apoi `--adam-ram`, apoi `--antrenare-mini` la sfarsit, optional urmat de `--mixt` sau `--vram` (iar dupa `--vram` optional `--pas-lipit`, `--antrenare-513` sau `--lot-ram`; dupa `--antrenare-513` optional `--placi3`, dupa el optional `--mm-placi`, dupa el optional `--lipit-placi` (ultimul) sau `--colsum-impartit`, la sfarsit optional `--mixt-513` (ultimul), `--mm-alegere` (ultimul, urmat optional de `--treapta-pas`) sau `--cursa-mm` dupa `--mm-placi` sau `--colsum-impartit`, `--cursa-mm` urmat optional de `--proba-64k8` sau `--ocupare`) (in aceasta ordine; `--mixt` sau `--vram` doar imediat dupa `--antrenare-mini`, nu amandoua; `--pas-lipit`, `--antrenare-513` sau `--lot-ram` doar imediat dupa `--vram`; `--adam-ram` doar imediat dupa `--ram-vram`; `--atentie2` doar dupa `--atentie-placi`, `--atentie3` doar dupa `--atentie2`, `--atentie4` doar dupa `--atentie3`, `--atentie-tc` doar dupa `--atentie3` sau `--atentie4`, `--atentie-tc16-v4` doar imediat dupa `--atentie-tc`, `--atentie-tc0` doar imediat dupa `--atentie-tc` sau `--atentie-tc16-v4`, `--atentie-tc0-v4` doar imediat dupa `--atentie-tc0`, `--atentie-tc-64k` doar imediat dupa `--atentie-tc`, `--atentie-tc16-v4`, `--atentie-tc0` sau `--atentie-tc0-v4`; `--atentie-v4` doar imediat dupa `--atentie-tc16`, ultimul; `--atentie-ds` doar imediat dupa `--atentie-tc16` sau `--atentie-v4`, ultimul; doar una din `--lipit`, `--simplu`, `--ln-nou`, `--ln-vechi`, `--lipita-ln`, `--sferturi`, `--ln-32`; `--k651` nu merge cu `--lipit` sau `--simplu`; `--argmax-fara-copie` nu merge cu `--lipit`, `--ln-nou`, `--sferturi` sau `--ln-32`; `--lot` doar cu `--argmax-fara-copie` si lista implicita lipita-ln-32, fara alta alegere de lista)";
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some(("test-placa", rest)) = words.split_first().map(|(a, b)| (*a, b)) else { return None };
    let (i9, flags) = match rest {
        ["--i9", x, flags @ ..] if i9_ok(x) => (*x, flags),
        ["--i9", ..] => return Some(Err(FORM)),
        flags => (I9_DEFAULT, flags),
    };
    // P6-101 (C-152 point 1): `--flr-proba` = the usual test, then the chain a second time in the same power-on.
    // P6-105 (C-152 point 5): `--tablou` = the board's dashboard at the end of the test's chain.
    // P6-142 (C-157 point 5): `--tablou-viteza` = in its place, the dashboard before the max-speed request, kept on.
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
    // P6-145 (C-157 point 7): `--lot` = after the texts, the same generation in batches (the token stays on the board,
    // `embed_dev`); only with `--argmax-fara-copie` and the default list (`lipita-ln-32`, no list choice).
    // P6-117 (C-161 point 2): `--p8` = after #513's texts, P8 (the first tensor-core computation).
    // P6-134 (C-161 point 4): `--colsum` = after everything, the first backward program of the training.
    // P6-136 (C-161 point 4): `--pas-aur` = after everything (after `--colsum`), the small golden step on the board.
    // P6-137 (C-161 point 4): `--mm-tc` = after everything (after `--pas-aur`), the `mm_tc` probe (training matmul).
    // P6-141 (C-161 point 4): `--atentie-placi` = after everything (after `--mm-tc`), the tiled training attention probe.
    // P6-149 (cyber-gpu note C-157_P2m): `--atentie2` = right after `--atentie-placi`, the new programs beside the old.
    // P6-160 (cyber-gpu note C-157_P2n): `--atentie3` = right after `--atentie2`, the third generation after the second.
    // P6-168 (cyber-gpu note C-157_P2q): `--atentie4` = right after `--atentie3`, the fourth generation after the third.
    // P6-227 (cyber-gpu note C-157_P2zm): `--atentie-tc` = right after `--atentie3` or `--atentie4`, dQ / dK / dV on the
    // tensor cores (`attn_back_tile_tc`, `attn_back_tile_tc16`) after the newest chain.
    // P6-161 (C-157, cyber-gpu `9359f21`): `--ram-vram` = after everything, the RAM ↔ VRAM probe with the copy engine.
    // P6-163 (C-157, cyber-gpu `ab9b537`): `--adam-ram` = right after `--ram-vram`, the golden step with m, v in RAM.
    // P6-164 (C-161, center's note 23 Nov 18:16): `--antrenare-mini` = last, the whole step of `antrenare mini` on the board.
    // P6-167 (center's note 23 Nov 23:36): `--mixt` = right after `--antrenare-mini`, the same step with every matmul on `mm_tc`.
    // Each at most once, in this order (the I9 text carries them as suffixes, read back in the same order).
    // P6-208 (centre note 25 Nov 14:40, point 2): `--lot-513 N` (1..=16) = the last two words, #513's batch of N
    // sequences of 512 per step (default 2); only with `--antrenare-513` in the list. Carried as a suffix with its number.
    let (flags, batch_513) = match flags {
        [rest @ .., "--lot-513", n] => match n.parse::<usize>() {
            Ok(n) if (1..=TRAIN_513_BATCH_MAX).contains(&n) => (rest, Some(n)),
            _ => return Some(Err(FORM)),
        },
        f => (f, None),
    };
    let mut out = String::from(i9);
    let mut next = 0;
    // P6-128: `--ln-32` = `lipita-ln`, then `lipita-ln-32` (every product on quarters), compared; one more list choice.
    // P6-135 (note C-157_P2i, point 1): `--lipita-ln` = the default until now (`lipita-ln` alone); without any list
    // choice the cycle now runs `lipita-ln-32` alone. The checks below go by name, not by position.
    let all = BOARD_OPTIONS;
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
        // P6-145: the batch needs the token chosen on the board and the default list (`lipita-ln-32` alone).
        if all[at] == LOT && (!out.split(' ').any(|w| w == ARGMAX_TOP.trim()) || all[lists.clone()].iter().any(|x| out.split(' ').any(|w| w == x.trim()))) {
            return Some(Err(FORM));
        }
        // P6-200: `mma16_s8` only right after P8 (the same path, the same page).
        if all[at] == P8_S8 && !out.ends_with(P8) {
            return Some(Err(FORM));
        }
        // P6-197: `mm_tc128` only beside `mm_tc`, in its probe.
        if all[at] == MM_TC128 && !out.ends_with(MM_TC) {
            return Some(Err(FORM));
        }
        // P6-207: `mm_tc_pf` only right after `mm_tc` (not with `mm_tc128`: one companion per probe).
        if all[at] == MM_TC_PF && !out.ends_with(MM_TC) {
            return Some(Err(FORM));
        }
        // P6-210: `mm_tc_v4` only right after `mm_tc` or `mm_tc_pf` (not with `mm_tc128`).
        if all[at] == MM_TC_V4 && !(out.ends_with(MM_TC) || out.ends_with(MM_TC_PF)) {
            return Some(Err(FORM));
        }
        // P6-242: `mm_tc_v5` only right after `mm_tc_v4` (the note compares it with v4, set by set).
        if all[at] == MM_TC_V5 && !out.ends_with(MM_TC_V4) {
            return Some(Err(FORM));
        }
        // P6-212: `mm_tc128_v4` only right after `mm_tc`, `mm_tc_pf`, `mm_tc_v4` or `mm_tc_v5` (not with `mm_tc128`).
        if all[at] == MM_TC128_V4 && !(out.ends_with(MM_TC) || out.ends_with(MM_TC_PF) || out.ends_with(MM_TC_V4) || out.ends_with(MM_TC_V5)) {
            return Some(Err(FORM));
        }
        // P6-214: the 64 KiB step only right after `mm_tc` or one of its companions (last of them).
        if all[at] == MM_TC_64K && ![MM_TC, MM_TC128, MM_TC_PF, MM_TC_V4, MM_TC_V5, MM_TC128_V4].iter().any(|x| out.ends_with(x)) {
            return Some(Err(FORM));
        }
        // P6-149: the new chain only beside the old one.
        if all[at] == ATTN_TILE2 && !out.ends_with(ATTN_TILE) {
            return Some(Err(FORM));
        }
        // P6-160: the third only right after the second.
        if all[at] == ATTN_TILE3 && !out.ends_with(ATTN_TILE2) {
            return Some(Err(FORM));
        }
        // P6-168: the fourth only right after the third.
        if all[at] == ATTN_TILE4 && !out.ends_with(ATTN_TILE3) {
            return Some(Err(FORM));
        }
        // P6-227: the tensor-core programs only right after the third or the fourth (they replace `attn_back_tile_3`).
        if all[at] == ATTN_TC && !(out.ends_with(ATTN_TILE3) || out.ends_with(ATTN_TILE4)) {
            return Some(Err(FORM));
        }
        // P6-233: v4 / v4pf only right after the tensor-core programs (they are compared with tc16).
        if all[at] == ATTN_TC_V4 && !out.ends_with(ATTN_TC) {
            return Some(Err(FORM));
        }
        // P6-234: dS on the tensor cores only right after the dQ / dK / dV ones (tc or the v4 pair).
        if all[at] == ATTN_TC0 && !(out.ends_with(ATTN_TC) || out.ends_with(ATTN_TC_V4)) {
            return Some(Err(FORM));
        }
        // P6-235: v4 of the dS on the tensor cores only right after `--atentie-tc0`.
        if all[at] == ATTN_TC0_V4 && !out.ends_with(ATTN_TC0) {
            return Some(Err(FORM));
        }
        // P6-238: the 64 KiB step of the tensor-core programs only right after them (or their companions).
        if all[at] == ATTN_TC_64K && ![ATTN_TC, ATTN_TC_V4, ATTN_TC0, ATTN_TC0_V4].iter().any(|o| out.ends_with(o)) {
            return Some(Err(FORM));
        }
        // P6-163: m, v in RAM only right after the RAM ↔ VRAM probe (it must pass first, in the same power-on).
        if all[at] == ADAM_RAM && !out.ends_with(RAM_VRAM) {
            return Some(Err(FORM));
        }
        // P6-167: the mixed step only right after the whole step (it changes how `--antrenare-mini` runs).
        if all[at] == TRAIN_MIXED && !out.ends_with(TRAIN_MINI) {
            return Some(Err(FORM));
        }
        // P6-215: the fast `mm_tc` programs in the mixed step only right after `--mixt`, last.
        if all[at] == TRAIN_MIXED_FAST && !out.ends_with(TRAIN_MIXED) {
            return Some(Err(FORM));
        }
        // P6-169: the resident arena only right after the whole step (not with `--mixt`: one new thing per test).
        if all[at] == TRAIN_VRAM && !out.ends_with(TRAIN_MINI) {
            return Some(Err(FORM));
        }
        // P6-175: the glued step only right after the resident arena (it compares two resident runs).
        if all[at] == TRAIN_FUSED && !out.ends_with(TRAIN_VRAM) {
            return Some(Err(FORM));
        }
        // P6-172: #513's step only right after the resident arena (the small model's resident run must pass first).
        if all[at] == TRAIN_513 && !out.ends_with(TRAIN_VRAM) {
            return Some(Err(FORM));
        }
        // P6-185: the tiled attention of #513's step only right after `--antrenare-513`.
        if all[at] == TRAIN_513_TILES3 && !out.ends_with(TRAIN_513) {
            return Some(Err(FORM));
        }
        // P6-188: #513's products on `mm_train_tile` only right after `--placi3`.
        if all[at] == TRAIN_513_MM_TILE && !out.ends_with(TRAIN_513_TILES3) {
            return Some(Err(FORM));
        }
        // P6-193: #513's glued step on the 64x64 tiles only right after `--mm-placi`, last (nothing may follow it:
        // `--colsum-impartit` and `--cursa-mm` check what comes right before them).
        if all[at] == TRAIN_513_FUSED_TILE && !out.ends_with(TRAIN_513_MM_TILE) {
            return Some(Err(FORM));
        }
        // P6-189: #513's `colsum` split over rows only right after `--mm-placi`.
        if all[at] == TRAIN_513_COLSUM && !out.ends_with(TRAIN_513_MM_TILE) {
            return Some(Err(FORM));
        }
        // P6-198: #513's mixed step only right after `--mm-placi` or `--colsum-impartit`; last (every later option checks
        // what comes right before it).
        if all[at] == TRAIN_513_MIXED && !out.ends_with(TRAIN_513_MM_TILE) && !out.ends_with(TRAIN_513_COLSUM) {
            return Some(Err(FORM));
        }
        // P6-215: the fast `mm_tc` programs in #513's mixed step only right after `--mixt-513`, last.
        if all[at] == TRAIN_513_MIXED_FAST && !out.ends_with(TRAIN_513_MIXED) {
            return Some(Err(FORM));
        }
        // P6-216: #513's batch and results through pinned RAM only right after `--mixt-513-rapid`, last.
        if all[at] == TRAIN_513_VIA_RAM && !out.ends_with(TRAIN_513_MIXED_FAST) {
            return Some(Err(FORM));
        }
        // P6-217: two batch slots only right after `--lot-prin-ram`, last.
        if all[at] == TRAIN_513_SLOTS && !out.ends_with(TRAIN_513_VIA_RAM) {
            return Some(Err(FORM));
        }
        // P6-231: dQ / dK / dV on `attn_back_tile_tc16` in #513's fast mixed step only right after `--mixt-513-rapid`,
        // `--lot-prin-ram` or `--doua-locuri`, last.
        if all[at] == TRAIN_513_ATTN_TC16
            && !out.ends_with(TRAIN_513_MIXED_FAST)
            && !out.ends_with(TRAIN_513_VIA_RAM)
            && !out.ends_with(TRAIN_513_SLOTS)
        {
            return Some(Err(FORM));
        }
        // P6-237: the v4 winner of P2zq in #513's fast mixed step only right after `--atentie-tc16`, last.
        if all[at] == TRAIN_513_ATTN_V4 && !out.ends_with(TRAIN_513_ATTN_TC16) {
            return Some(Err(FORM));
        }
        // P6-241: dS on `attn_back_tile0_tc` / `_v4` in #513's fast mixed step only right after `--atentie-tc16` or `--atentie-v4`.
        if all[at] == TRAIN_513_ATTN_DS && !out.ends_with(TRAIN_513_ATTN_TC16) && !out.ends_with(TRAIN_513_ATTN_V4) {
            return Some(Err(FORM));
        }
        // P6-196: #513's products on the variant the os-416 race picked only right after `--mm-placi` or
        // `--colsum-impartit`, last (`--cursa-mm` checks what comes right before it).
        if all[at] == TRAIN_513_MM_CHOICE && !out.ends_with(TRAIN_513_MM_TILE) && !out.ends_with(TRAIN_513_COLSUM) {
            return Some(Err(FORM));
        }
        // P6-205: one SM shared-memory step on all of #513's step only right after `--mm-alegere`, last.
        if all[at] == TRAIN_513_SM_STEP && !out.ends_with(TRAIN_513_MM_CHOICE) {
            return Some(Err(FORM));
        }
        // P6-213: the waves model's choice only right after `--treapta-pas` (it judges the shapes at 64 KiB), last.
        if all[at] == TRAIN_513_MM_MODEL && !out.ends_with(TRAIN_513_SM_STEP) {
            return Some(Err(FORM));
        }
        // P6-190: the night race only right after `--mm-placi` or `--colsum-impartit`.
        if all[at] == TRAIN_513_RACE && !out.ends_with(TRAIN_513_MM_TILE) && !out.ends_with(TRAIN_513_COLSUM) {
            return Some(Err(FORM));
        }
        // P6-195: the lone `mm_tv_64x64k8` probe only right after `--cursa-mm` (it replaces the big race).
        if all[at] == TRAIN_513_PROBE_64K8 && !out.ends_with(TRAIN_513_RACE) {
            return Some(Err(FORM));
        }
        // P6-201: the occupancy probe only right after `--cursa-mm` (it replaces the big race; not with `--proba-64k8`).
        if all[at] == TRAIN_513_OCCUPANCY && !out.ends_with(TRAIN_513_RACE) {
            return Some(Err(FORM));
        }
        // P6-178: the batch through RAM only right after the resident arena (it compares two resident runs).
        if all[at] == TRAIN_RAM && !out.ends_with(TRAIN_VRAM) {
            return Some(Err(FORM));
        }
        out.push_str(all[at]);
        // The list choices exclude each other (each picks the lists #513 runs with); likewise `--argmax-placa` and
        // `--argmax-fara-copie`.
        next = if lists.contains(&at) {
            pos(K651_OPT)
        } else if all[at] == ARGMAX_PLACA {
            pos(DIETA2)
        } else if all[at] == TABLOU {
            // P6-142: `--tablou` or `--tablou-viteza`, not both.
            pos(LIPIT)
        } else {
            at + 1
        };
    }
    if let Some(n) = batch_513 {
        if !out.split(' ').any(|w| w == TRAIN_513.trim()) {
            return Some(Err(FORM));
        }
        out.push_str(&format!("{} {}", TRAIN_513_BATCH, n));
    }
    Some(Ok(out))
}

/// P6-236 (centre note 26 Nov 17:50, after os-462 / os-465): why a refused `test-placa` line was refused, in one
/// sentence for the report: the first word the line cannot take where it stands (the line up to it is refused while
/// the line before it is not), or the `--lot-513 N` ending. `None` when the line is not `test-placa` or is accepted.
pub fn board_refusal_hint(command: &str) -> Option<String> {
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.first() != Some(&"test-placa") || !matches!(board_command(command), Some(Err(_))) {
        return None;
    }
    let refused = |n: usize| matches!(board_command(&words[..n].join(" ")), Some(Err(_)));
    let mut start = 1;
    if words.get(1) == Some(&"--i9") {
        if words.len() < 3 || refused(3) {
            return Some(String::from("dupa `--i9` vine una din c125, c133, c135, os302"));
        }
        start = 3;
    }
    let lot = words.len() >= start + 2 && words[words.len() - 2] == "--lot-513";
    let end = if lot { words.len() - 2 } else { words.len() };
    let rank = |w: &str| BOARD_OPTIONS.iter().position(|x| x.trim() == w);
    for k in start..end {
        if !refused(k + 1) {
            continue;
        }
        let w = words[k];
        if w == "--lot-513" {
            return Some(String::from("`--lot-513 N` sta doar la sfarsitul randului (ultimele doua cuvinte), cu N intre 1 si 16"));
        }
        let Some(at) = rank(w) else {
            return Some(format!("`{}` nu e o optiune cunoscuta a lui `test-placa` (cuvantul {} din rand)", w, k + 1));
        };
        let prev = words[k - 1];
        if words[start..k].contains(&w) {
            return Some(format!("`{}` apare de doua ori", w));
        }
        // An earlier word that comes later in the one order: the usual mistake (os-462 / os-465).
        if let Some(later) = words[start..k].iter().find(|x| rank(x).is_some_and(|r| r > at)) {
            return Some(format!(
                "`{}` e pus prea tarziu: in ordinea optiunilor vine inaintea lui `{}` (muta `{}` inaintea lui `{}`)",
                w, later, w, later
            ));
        }
        return Some(format!("`{}` nu poate sta dupa `{}` (vezi forma corecta: ce optiune o cere imediat inainte)", w, prev));
    }
    if lot {
        return Some(String::from("`--lot-513 N` cere `--antrenare-513` in rand si N intre 1 si 16"));
    }
    Some(String::from("randul intreg nu se potriveste cu forma corecta"))
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
/// P6-142 (C-157 point 5): in place of `--tablou`: the dashboard started before the max-speed request, read before it,
/// 1 s after it and at the end, every read through the thermal watch.
pub const TABLOU_VITEZA: &str = " --tablou-viteza";

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

/// P6-145 (C-157 point 7): suffix of the I9 text when the batch is asked (`test-placa … --argmax-fara-copie [--dieta2]
/// --lot`, after `--dieta2`, before `--p8`).
pub const LOT: &str = " --lot";

/// P6-240 (cyber-gpu `ce8324a`, note `C-157_P2zz_token32-pe-placa.md`): suffix of the I9 text when the texts run once
/// more with `token32` (`test-placa … --token32`, after `--lot`, before `--p8`).
pub const TOKEN32: &str = " --token32";

/// P6-117 (C-161 point 2): suffix of the I9 text when P8 is asked (`test-placa … --p8`, always last).
pub const P8: &str = " --p8";

/// P6-200 (pista B1, cyber-gpu `ebbb3af`): suffix of the I9 text when the int8 tensor-core probe `mma16_s8` runs after
/// P8 (`test-placa … --p8 --p8-s8`, only right after `--p8`).
pub const P8_S8: &str = " --p8-s8";

/// P6-134 (C-161 point 4): suffix of the I9 text when the `colsum` probe is asked (`test-placa … --colsum`, always last).
pub const COLSUM: &str = " --colsum";

/// P6-218 (cyber-gpu `adc783c`, note `C-157_P2zh_proba-barierei.md`): suffix of the I9 text when the grid-barrier probe
/// runs (`test-placa … --bariera`, after `--p8` / `--p8-s8`, before `--colsum`).
pub const BARIERA: &str = " --bariera";

/// P6-232 (cyber-gpu `6e69f1a`, note `C-157_P2zr_bariera-larga.md`): suffix of the I9 text when the wide grid-barrier
/// probe runs (`test-placa … --bariera-larga`, G = SMs × 8 then × 16; right after `--bariera`, before `--colsum`).
pub const BARIERA_LARGA: &str = " --bariera-larga";

/// P6-136 (C-161 point 4): suffix of the I9 text when the small golden step on the board is asked (`test-placa …
/// --pas-aur`, always last).
pub const PAS_AUR: &str = " --pas-aur";

/// P6-137 (C-161 point 4): suffix of the I9 text when the `mm_tc` probe is asked (`test-placa … --mm-tc`, always last).
pub const MM_TC: &str = " --mm-tc";

/// P6-197 (C-165): suffix of the I9 text when `mm_tc128` runs beside `mm_tc` in its probe (`test-placa … --mm-tc
/// --mm-tc128`, only right after `--mm-tc`).
pub const MM_TC128: &str = " --mm-tc128";

/// P6-207 (C-157 p.2, cyber-gpu `835c6c9`): suffix of the I9 text when `mm_tc_pf` (`mm_tc` with the reads in advance) runs
/// beside `mm_tc` in its probe (`test-placa … --mm-tc --mm-tc-pf`, only right after `--mm-tc`).
pub const MM_TC_PF: &str = " --mm-tc-pf";

/// P6-210 (C-157 p.2, cyber-gpu `273a42b`): suffix of the I9 text when `mm_tc_v4` (`mm_tc_pf` with 128-bit reads) runs
/// beside `mm_tc` in its probe, after `mm_tc_pf` if both (`test-placa … --mm-tc [--mm-tc-pf] --mm-tc-v4`).
pub const MM_TC_V4: &str = " --mm-tc-v4";

/// P6-242 (C-157 p.2, cyber-gpu `2b24cc7`, note P2zzb): suffix of the I9 text when `mm_tc_v5` (`mm_tc_v4` with a 12 KiB
/// block: 4 blocks per SM at the 64 KiB step) runs beside `mm_tc` in its probe, right after `mm_tc_v4`
/// (`test-placa … --mm-tc [--mm-tc-pf] --mm-tc-v4 --mm-tc-v5 [--mm-tc128-v4] [--mm-tc-64k]`).
pub const MM_TC_V5: &str = " --mm-tc-v5";

/// P6-212 (C-157 p.2, cyber-gpu `abb4f73`): suffix of the I9 text when `mm_tc128_v4` (`mm_tc128` with 128-bit reads in
/// advance) runs beside `mm_tc` in its probe, last (`test-placa … --mm-tc [--mm-tc-pf] [--mm-tc-v4] --mm-tc128-v4`).
pub const MM_TC128_V4: &str = " --mm-tc128-v4";

/// P6-214 (C-157 p.2, cyber-gpu note `C-157_P2ze_os436.md` point 1): suffix of the I9 text when `mm_tc` and its companions
/// run once more at the 64 KiB shared-memory step in its probe, last of the `mm_tc` options
/// (`test-placa … --mm-tc [companions] --mm-tc-64k`).
pub const MM_TC_64K: &str = " --mm-tc-64k";

/// P6-141 (C-161 point 4): suffix of the I9 text when the tiled attention probe is asked (`test-placa … --atentie-placi`,
/// always last).
pub const ATTN_TILE: &str = " --atentie-placi";

/// P6-149 (cyber-gpu note C-157_P2m): suffix of the I9 text when the second generation of the tiled attention runs beside
/// the first (`test-placa … --atentie-placi --atentie2`, always last).
pub const ATTN_TILE2: &str = " --atentie2";

/// P6-160 (cyber-gpu note C-157_P2n, `d885efd`): suffix of the I9 text when the third generation (`attn_*_3`) runs after
/// the second (`test-placa … --atentie-placi --atentie2 --atentie3`, always last).
pub const ATTN_TILE3: &str = " --atentie3";

/// P6-168 (cyber-gpu note C-157_P2q, commit `c5645e9`): suffix of the I9 text when the fourth generation
/// (`attn_fwd_tile3` → `attn_back_tile0_3` → `attn_back_tile_4`) runs after the third (`test-placa … --atentie-placi
/// --atentie2 --atentie3 --atentie4`, only right after `--atentie3`).
pub const ATTN_TILE4: &str = " --atentie4";

/// P6-227 (cyber-gpu note C-157_P2zm, commit `3c6fa3b`): suffix of the I9 text when `attn_back_tile_tc` and
/// `attn_back_tile_tc16` run after the newest tiled chain (`test-placa … --atentie-placi --atentie2 --atentie3
/// [--atentie4] --atentie-tc`, only right after `--atentie3` or `--atentie4`).
pub const ATTN_TC: &str = " --atentie-tc";

/// P6-233 (cyber-gpu note C-157_P2zq, commit `4293b5c`): suffix of the I9 text when `attn_back_tile_tc16_v4` and
/// `_v4pf` run after tc16 in the same probe (`test-placa … --atentie-tc --atentie-tc16-v4`, only right after `--atentie-tc`).
pub const ATTN_TC_V4: &str = " --atentie-tc16-v4";

/// P6-234 (cyber-gpu note C-157_P2zs, commit `f6ce737`): suffix of the I9 text when `attn_back_tile0_tc` (dS with dP on
/// the tensor cores) runs after the tensor-core dQ / dK / dV programs in the same probe (`test-placa … --atentie-tc
/// [--atentie-tc16-v4] --atentie-tc0`).
pub const ATTN_TC0: &str = " --atentie-tc0";

/// P6-235 (cyber-gpu note C-157_P2zt, commit `62197c0`): suffix of the I9 text when `attn_back_tile0_tc_v4` runs after
/// `attn_back_tile0_tc` in the same probe (`test-placa … --atentie-tc0 --atentie-tc0-v4`).
pub const ATTN_TC0_V4: &str = " --atentie-tc0-v4";

/// P6-238 (cyber-gpu note C-157_P2zw, point 3): the tensor-core dQ / dK / dV programs of the probe once more at the
/// 64 KiB shared-memory step (the step of the mixed #513 step), word for word against today's run, with the times
/// (`test-placa … --atentie-tc [--atentie-tc16-v4] [--atentie-tc0 [--atentie-tc0-v4]] --atentie-tc-64k`).
pub const ATTN_TC_64K: &str = " --atentie-tc-64k";

/// P6-161 (C-157, cyber-gpu `9359f21`): suffix of the I9 text when the RAM ↔ VRAM probe with the copy engine is asked
/// (`test-placa … --ram-vram`, always last).
pub const RAM_VRAM: &str = " --ram-vram";

/// P6-163 (C-157, cyber-gpu `ab9b537`): suffix of the I9 text when the small golden step with AdamW's m, v in host RAM
/// is asked (`test-placa … --ram-vram --adam-ram`, always last, only right after `--ram-vram`).
pub const ADAM_RAM: &str = " --adam-ram";

/// P6-164 (C-161, command-center note of 23 Nov 18:16): suffix of the I9 text when the whole training step of `antrenare
/// mini` on the board is asked (`test-placa … --antrenare-mini`, always last).
pub const TRAIN_MINI: &str = " --antrenare-mini";

/// P6-167 (command-center note of 23 Nov 23:36; cyber-gpu note C-160_P7f): suffix of the I9 text when `--antrenare-mini`
/// runs as the mixed step (every matmul on `mm_tc`, f16 inputs, f32 sums, loss scale S = 1024): `test-placa …
/// --antrenare-mini --mixt`, always last, only right after `--antrenare-mini`.
pub const TRAIN_MIXED: &str = " --mixt";

/// P6-215 (C-157 p.2, cyber-gpu note `C-157_P2zf_pas-mixt-rapid.md`): suffix of the I9 text when the mixed step of
/// `antrenare mini` puts its products on the fast `mm_tc` programs (`mm_tc_v4` / `mm_tc_pf`, `pack_mixed_fast`):
/// `test-placa … --antrenare-mini --mixt --mixt-rapid`, last, only right after `--mixt`.
pub const TRAIN_MIXED_FAST: &str = " --mixt-rapid";

/// P6-169 (command-center note of 23 Nov 23:36 point 2; cyber-gpu note C-160_P8): suffix of the I9 text when
/// `--antrenare-mini` runs with the resident arena (made once in VRAM): `test-placa … --antrenare-mini --vram`, always
/// last, only right after `--antrenare-mini` (not with `--mixt`).
pub const TRAIN_VRAM: &str = " --vram";

/// P6-175 (cyber-gpu `d3d1ddd` + `f170042`): suffix of the I9 text when the resident run is done twice — without gluing,
/// then with `ln_back_res` + `mm_train_gelu` glued into the step — and compared bit for bit: `test-placa …
/// --antrenare-mini --vram --pas-lipit`, always last, only right after `--vram` (not `--lipit`: that one picks #513's
/// lists).
pub const TRAIN_FUSED: &str = " --pas-lipit";

/// P6-172 (command-center note of 23 Nov 23:36, point 3): suffix of the I9 text when, after the resident run of `antrenare
/// mini` passed, the training step of Kronos #513 runs with its arena in VRAM: `test-placa … --antrenare-mini --vram
/// --antrenare-513`, always last, only right after `--vram`.
pub const TRAIN_513: &str = " --antrenare-513";

/// P6-185 (cyber-gpu `9a71427`, note C-157_P2v): suffix of the I9 text when #513's step is built on the third-generation
/// tiled attention (`Plan::new_tiles3`): `test-placa … --antrenare-mini --vram --antrenare-513 --placi3`, only right
/// after `--antrenare-513`.
pub const TRAIN_513_TILES3: &str = " --placi3";

/// P6-188 (cyber-gpu `4042ce9`, note C-157_P2x step 2): suffix of the I9 text when #513's step (on the tiled attention)
/// also runs its `mm_train` launches on `mm_train_tile` (`Plan::with_mm_tile`):
/// `test-placa … --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi`, only right after `--placi3`.
pub const TRAIN_513_MM_TILE: &str = " --mm-placi";

/// P6-189 (cyber-gpu `ffdb679`, note C-157_P2y): suffix of the I9 text when #513's step (tiled attention, `mm_train_tile`)
/// also splits its `colsum` launches over rows (`Plan::with_colsum_split`):
/// `test-placa … --antrenare-513 --placi3 --mm-placi --colsum-impartit`, only right after `--mm-placi`.
pub const TRAIN_513_COLSUM: &str = " --colsum-impartit";

/// P6-193 (cyber-gpu `2289b2e`, note C-165_mm-tile-gelu): suffix of the I9 text when #513's step (tiled attention,
/// `mm_train_tile`) is also glued (`ln_back_res`, GELU' in "proj-dX") with the glued products on `mm_train_tile_gelu`
/// (`Plan::with_fused_back().with_fused_gelu().with_mm_tile_gelu()`):
/// `test-placa … --antrenare-513 --placi3 --mm-placi --lipit-placi`, only right after `--mm-placi`, last.
pub const TRAIN_513_FUSED_TILE: &str = " --lipit-placi";

/// P6-196 (cyber-gpu `214be67` + `0c74b3c`, note C-165_cursa-mm steps 2-3): suffix of the I9 text when #513's
/// `mm_train` shapes run on the variant the os-416 race picked (`parse_race` -> `MmChoice` -> `Plan::with_mm_choice`;
/// `mm_tv_128x128` on all 15 shapes): `test-placa … --mm-placi [--colsum-impartit] --mm-alegere`, last.
pub const TRAIN_513_MM_CHOICE: &str = " --mm-alegere";

/// P6-205 (cyber-gpu `8b6a72d`, note C-165_treapta-pe-pas): suffix of the I9 text when #513's step (on the race's pick)
/// is packed with one SM shared-memory step for all its launches (`step_best_sm_step` -> `pack_with_sm_step`; 64 KiB on
/// the board): `test-placa … --mm-placi [--colsum-impartit] --mm-alegere --treapta-pas`, last.
pub const TRAIN_513_SM_STEP: &str = " --treapta-pas";

/// P6-213 (cyber-gpu `8fe7637`, note C-165_valuri-alegerea-64k, way 2): suffix of the I9 text when #513's shapes take
/// the waves model's choice at 64 KiB (`wave_model::predicted_choice` -> `Plan::with_mm_choice`) instead of the os-416
/// race: `test-placa … --mm-alegere --treapta-pas --mm-model`, last.
pub const TRAIN_513_MM_MODEL: &str = " --mm-model";

/// P6-208 (centre note 25 Nov 14:40, point 2: the big batch): suffix of the I9 text, followed by a number, when #513's
/// step runs with a batch of N sequences of 512 (default 2): `test-placa … --antrenare-513 … --lot-513 N`, always last.
pub const TRAIN_513_BATCH: &str = " --lot-513";
/// P6-208: the largest batch `--lot-513` accepts (the arena is checked against the free VRAM anyway).
pub const TRAIN_513_BATCH_MAX: usize = 16;

/// P6-208: splits ` --lot-513 N` off the end of an I9 text -> (the rest, N).
pub fn split_batch_513(i9: &str) -> (&str, Option<usize>) {
    match i9.rsplit_once(TRAIN_513_BATCH) {
        Some((rest, tail)) => match tail.strip_prefix(' ').filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).map(str::parse::<usize>) {
            Some(Ok(n)) if (1..=TRAIN_513_BATCH_MAX).contains(&n) => (rest, Some(n)),
            _ => (i9, None),
        },
        _ => (i9, None),
    }
}

/// P6-198 (cyber-gpu `1617228`, note C-165_pas-mixt-513): suffix of the I9 text when #513's step is the mixed one (every
/// product of the model on `mm_tc`, S = `LOSS_SCALE_513`): `test-placa … --mm-placi [--colsum-impartit] --mixt-513`, last.
pub const TRAIN_513_MIXED: &str = " --mixt-513";

/// P6-215 (C-157 p.2, cyber-gpu note `C-157_P2zf_pas-mixt-rapid.md`): suffix of the I9 text when #513's mixed step puts its
/// products on the fast `mm_tc` programs and every launch at the 64 KiB step (`pack_mixed_fast(…, Some(64 KiB), …)`):
/// `test-placa … --mixt-513 --mixt-513-rapid`, last, only right after `--mixt-513`.
pub const TRAIN_513_MIXED_FAST: &str = " --mixt-513-rapid";

/// P6-216 (C-157, cyber-gpu `1ddf092`, note `C-157_P2zg_lot-prin-ram-mixt.md`, step 1): suffix of the I9 text when #513's
/// fast mixed step takes its batch from pinned RAM and leaves its losses + norm there (the copy program in the step's own
/// submission, `batch_ram::pack_mixed_fast_via_ram`) instead of the VRAM window:
/// `test-placa … --mixt-513 --mixt-513-rapid --lot-prin-ram`, last, only right after `--mixt-513-rapid`.
pub const TRAIN_513_VIA_RAM: &str = " --lot-prin-ram";

/// P6-217 (C-157, cyber-gpu `1ddf092`, note `C-157_P2zg_lot-prin-ram-mixt.md`, step 2): suffix of the I9 text when #513's
/// steps through pinned RAM use two batch slots, the next step prepared while the board runs the current one:
/// `test-placa … --mixt-513-rapid --lot-prin-ram --doua-locuri`, last, only right after `--lot-prin-ram`.
pub const TRAIN_513_SLOTS: &str = " --doua-locuri";

/// P6-231 (C-157, cyber-gpu `3c6fa3b` / `250c9b8`, notes C-157_P2zm step 2 and C-157_P2zp): suffix of the I9 text when
/// #513's fast mixed step runs dQ, dK, dV of the attention on `attn_back_tile_tc16` (f16 tensor cores) instead of
/// `attn_back_tile_3`: `test-placa … --mixt-513-rapid [--lot-prin-ram [--doua-locuri]] --atentie-tc16`, last.
pub const TRAIN_513_ATTN_TC16: &str = " --atentie-tc16";

/// P6-237 (C-157, cyber-gpu `6e8b29a`, note C-157_P2zv, after os-465: v4 = tc16 on 4 / 4 aligned sets, 0,52 ms against
/// 0,87 ms): suffix of the I9 text when, in #513's fast mixed step with `--atentie-tc16`, every aligned dQ / dK / dV
/// launch goes to `attn_back_tile_tc16_v4` (the winner of P2zq; tc16 for the rest) — `pack_mixed_fast_attn4`:
/// `test-placa … --atentie-tc16 --atentie-v4`, last, only right after `--atentie-tc16`.
pub const TRAIN_513_ATTN_V4: &str = " --atentie-v4";

/// P6-241 (C-157, cyber-gpu `9b11018` / `5c83a80`, note C-157_P2zu, only after `attn_back_tile0_tc` passes on the board):
/// suffix of the I9 text when, in #513's fast mixed step with `--atentie-tc16`, the 12 dS launches (`attn_back_tile0_3`)
/// go to `attn_back_tile0_tc_v4` where the shape is aligned and to `attn_back_tile0_tc` for the rest
/// (`attn0_tc_list` + `pack_mixed_fast_attn4`): `test-placa … --atentie-tc16 [--atentie-v4] --atentie-ds`, last.
pub const TRAIN_513_ATTN_DS: &str = " --atentie-ds";

/// P6-190 (C-165, cyber-gpu `a98298e`, note C-165_cursa-mm): suffix of the I9 text when, after #513's steps, the night
/// race of `mm_train_tile`'s variants runs on every `mm_train` shape of the step:
/// `test-placa … --antrenare-513 --placi3 --mm-placi [--colsum-impartit] --cursa-mm`, last.
pub const TRAIN_513_RACE: &str = " --cursa-mm";

/// P6-195 (cyber-gpu `edc3ae3`, note C-165_os413_64x64k8, proposal 3): suffix of the I9 text when, instead of the big
/// race, only `mm_tv_64x64k8` runs beside the witness on the qkv shape, mapped with the tail page (did os-413's hang come
/// from the instruction prefetch past the end of the code?):
/// `test-placa … --mm-placi [--colsum-impartit] --cursa-mm --proba-64k8`, last.
pub const TRAIN_513_PROBE_64K8: &str = " --proba-64k8";

/// P6-201 (cyber-gpu `54f5ab8`, note C-165_proba-ocuparii): suffix of the I9 text when, instead of the big race, the
/// occupancy probe runs on the qkv shape (the QMD's SM shared-memory step 8/16/32/64 KiB, `occupancy::PROBE`):
/// `test-placa … --mm-placi [--colsum-impartit] --cursa-mm --ocupare`, last.
pub const TRAIN_513_OCCUPANCY: &str = " --ocupare";

/// P6-178 (cyber-gpu `d173a59`, note C-157_P2s): suffix of the I9 text when the resident run is done twice — the batch
/// and the results through the VRAM window, then through pinned RAM (7 copies in the step's list) — and compared bit for
/// bit: `test-placa … --antrenare-mini --vram --lot-ram`, always last, only right after `--vram`.
pub const TRAIN_RAM: &str = " --lot-ram";

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
    let (i9, batch_513) = split_batch_513(i9);
    let (i9, train_ram) = match i9.strip_suffix(TRAIN_RAM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_occupancy) = match i9.strip_suffix(TRAIN_513_OCCUPANCY) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_probe64) = match i9.strip_suffix(TRAIN_513_PROBE_64K8) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_race) = match i9.strip_suffix(TRAIN_513_RACE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_mm_model) = match i9.strip_suffix(TRAIN_513_MM_MODEL) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_sm_step) = match i9.strip_suffix(TRAIN_513_SM_STEP) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_mm_choice) = match i9.strip_suffix(TRAIN_513_MM_CHOICE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_attn_ds) = match i9.strip_suffix(TRAIN_513_ATTN_DS) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_attn_v4) = match i9.strip_suffix(TRAIN_513_ATTN_V4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_attn_tc16) = match i9.strip_suffix(TRAIN_513_ATTN_TC16) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_slots) = match i9.strip_suffix(TRAIN_513_SLOTS) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_via_ram) = match i9.strip_suffix(TRAIN_513_VIA_RAM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_mixed_fast) = match i9.strip_suffix(TRAIN_513_MIXED_FAST) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_mixed) = match i9.strip_suffix(TRAIN_513_MIXED) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_colsum) = match i9.strip_suffix(TRAIN_513_COLSUM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_fused_tile) = match i9.strip_suffix(TRAIN_513_FUSED_TILE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_mm_tile) = match i9.strip_suffix(TRAIN_513_MM_TILE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513_tiles3) = match i9.strip_suffix(TRAIN_513_TILES3) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_513) = match i9.strip_suffix(TRAIN_513) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_fused) = match i9.strip_suffix(TRAIN_FUSED) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_vram) = match i9.strip_suffix(TRAIN_VRAM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_mixed_fast) = match i9.strip_suffix(TRAIN_MIXED_FAST) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_mixed) = match i9.strip_suffix(TRAIN_MIXED) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, train_mini) = match i9.strip_suffix(TRAIN_MINI) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, adam_ram) = match i9.strip_suffix(ADAM_RAM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, ram_vram) = match i9.strip_suffix(RAM_VRAM) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tc_64k) = match i9.strip_suffix(ATTN_TC_64K) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tc0_v4) = match i9.strip_suffix(ATTN_TC0_V4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tc0) = match i9.strip_suffix(ATTN_TC0) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tc_v4) = match i9.strip_suffix(ATTN_TC_V4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tc) = match i9.strip_suffix(ATTN_TC) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tile4) = match i9.strip_suffix(ATTN_TILE4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tile3) = match i9.strip_suffix(ATTN_TILE3) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tile2) = match i9.strip_suffix(ATTN_TILE2) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, attn_tile) = match i9.strip_suffix(ATTN_TILE) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc_64k) = match i9.strip_suffix(MM_TC_64K) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc128_v4) = match i9.strip_suffix(MM_TC128_V4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc_v5) = match i9.strip_suffix(MM_TC_V5) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc_v4) = match i9.strip_suffix(MM_TC_V4) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc_pf) = match i9.strip_suffix(MM_TC_PF) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, mm_tc128) = match i9.strip_suffix(MM_TC128) {
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
    let (i9, barrier_wide) = match i9.strip_suffix(BARIERA_LARGA) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, barrier) = match i9.strip_suffix(BARIERA) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, p8_s8) = match i9.strip_suffix(P8_S8) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, p8) = match i9.strip_suffix(P8) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, token32) = match i9.strip_suffix(TOKEN32) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, batch) = match i9.strip_suffix(LOT) {
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
    let (i9, dashboard_early) = match i9.strip_suffix(TABLOU_VITEZA) {
        Some(x) => (x, true),
        None => (i9, false),
    };
    let (i9, dashboard) = match i9.strip_suffix(TABLOU) {
        Some(x) => (x, true),
        None => (i9, dashboard_early),
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
    // P6-142: the dashboard before the max-speed request, kept on (the thermal watch).
    crate::drivers::gpu_tur::RUSD_EARLY.store(dashboard_early, core::sync::atomic::Ordering::SeqCst);
    if dashboard_early {
        text.push_str("\n(tabloul de bord al placii: cerut de lista, pornit INAINTE de cererea de viteza maxima, citit inainte / dupa 1 s / la final, cu ceasul termic 80/85/70 grade C)\n");
    } else if dashboard {
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
    // P6-200: `mma16_s8` after it only when the list asked for it.
    crate::drivers::gpu_tur::P8_S8_IN_CYCLE.store(p8 && p8_s8, core::sync::atomic::Ordering::SeqCst);
    // P6-218: the grid-barrier probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::BARRIER_IN_CYCLE.store(barrier, core::sync::atomic::Ordering::SeqCst);
    // P6-232: the wide barrier probe (G = SMs × 8 / × 16) only when the list asked for it.
    crate::drivers::gpu_tur::BARRIER_WIDE_IN_CYCLE.store(barrier_wide, core::sync::atomic::Ordering::SeqCst);
    // P6-134 (C-161 point 4): the `colsum` probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(colsum, core::sync::atomic::Ordering::SeqCst);
    // P6-136 (C-161 point 4): the small golden step only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(golden, core::sync::atomic::Ordering::SeqCst);
    // P6-137 (C-161 point 4): the `mm_tc` probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(mm_tc, core::sync::atomic::Ordering::SeqCst);
    // P6-197: `mm_tc128` beside it only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC128_IN_CYCLE.store(mm_tc && mm_tc128, core::sync::atomic::Ordering::SeqCst);
    // P6-207: `mm_tc_pf` beside it only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC_PF_IN_CYCLE.store(mm_tc && mm_tc_pf, core::sync::atomic::Ordering::SeqCst);
    // P6-210: `mm_tc_v4` too, only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC_V4_IN_CYCLE.store(mm_tc && mm_tc_v4, core::sync::atomic::Ordering::SeqCst);
    // P6-242: `mm_tc_v5` too, only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC_V5_IN_CYCLE.store(mm_tc && mm_tc_v5, core::sync::atomic::Ordering::SeqCst);
    // P6-212: `mm_tc128_v4` too, only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC128_V4_IN_CYCLE.store(mm_tc && mm_tc128_v4, core::sync::atomic::Ordering::SeqCst);
    // P6-214: the 64 KiB step too, only when the list asked for it.
    crate::drivers::gpu_tur::MM_TC_64K_IN_CYCLE.store(mm_tc && mm_tc_64k, core::sync::atomic::Ordering::SeqCst);
    // P6-141 (C-161 point 4): the tiled attention probe only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(attn_tile, core::sync::atomic::Ordering::SeqCst);
    // P6-149: the second generation beside the first only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TILE2_IN_CYCLE.store(attn_tile && attn_tile2, core::sync::atomic::Ordering::SeqCst);
    // P6-160: the third generation after the second only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TILE3_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3, core::sync::atomic::Ordering::SeqCst);
    // P6-168: the fourth generation after the third only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TILE4_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tile4, core::sync::atomic::Ordering::SeqCst);
    // P6-227: the tensor-core programs after the newest chain only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TC_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tc, core::sync::atomic::Ordering::SeqCst);
    // P6-233: v4 / v4pf after tc16 only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TC_V4_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tc && attn_tc_v4, core::sync::atomic::Ordering::SeqCst);
    // P6-234: dS on the tensor cores after them only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TC0_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tc && attn_tc0, core::sync::atomic::Ordering::SeqCst);
    // P6-235: v4 of it only when the list asked for it.
    crate::drivers::gpu_tur::ATTN_TC0_V4_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tc && attn_tc0 && attn_tc0_v4, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC_64K_IN_CYCLE.store(attn_tile && attn_tile2 && attn_tile3 && attn_tc && attn_tc_64k, core::sync::atomic::Ordering::SeqCst);
    // P6-161 (C-157): the copy object (right after I9) and the RAM ↔ VRAM probe only when the list asked for it.
    crate::drivers::gpu_tur::RAM_VRAM_IN_CYCLE.store(ram_vram, core::sync::atomic::Ordering::SeqCst);
    // P6-163 (C-157): the golden step with m, v in RAM only when the list asked for it (and only after `--ram-vram`).
    crate::drivers::gpu_tur::ADAM_RAM_IN_CYCLE.store(ram_vram && adam_ram, core::sync::atomic::Ordering::SeqCst);
    // P6-164: the whole step of `antrenare mini` on the board only when the list asked for it, last.
    crate::drivers::gpu_tur::TRAIN_MINI_IN_CYCLE.store(train_mini, core::sync::atomic::Ordering::SeqCst);
    // P6-167: as the mixed step (every matmul on `mm_tc`) only when the list asked for it, right after it.
    crate::drivers::gpu_tur::TRAIN_MINI_MIXED_IN_CYCLE.store(train_mini && train_mixed, core::sync::atomic::Ordering::SeqCst);
    // P6-169: with the resident arena (in VRAM) only when the list asked for it, right after it.
    crate::drivers::gpu_tur::TRAIN_MINI_VRAM_IN_CYCLE.store(train_mini && train_vram, core::sync::atomic::Ordering::SeqCst);
    // P6-175: the resident run twice (plain, then glued) only when the list asked for it, right after `--vram`.
    crate::drivers::gpu_tur::TRAIN_MINI_FUSED_IN_CYCLE.store(train_mini && train_vram && train_fused, core::sync::atomic::Ordering::SeqCst);
    // P6-172: #513's step after the resident run only when the list asked for it, right after `--vram`.
    crate::drivers::gpu_tur::TRAIN_513_IN_CYCLE.store(train_mini && train_vram && train_513, core::sync::atomic::Ordering::SeqCst);
    // P6-208: #513's batch only when the list asked for it (0 = the default 2).
    crate::drivers::gpu_tur::TRAIN_513_BATCH.store(if train_mini && train_vram && train_513 { batch_513.unwrap_or(0) } else { 0 }, core::sync::atomic::Ordering::SeqCst);
    // P6-185: #513's step on the third-generation tiled attention only when the list asked for it, right after it.
    crate::drivers::gpu_tur::TRAIN_513_TILES3_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3, core::sync::atomic::Ordering::SeqCst);
    // P6-188: #513's products on `mm_train_tile` only when the list asked for it, right after `--placi3`.
    crate::drivers::gpu_tur::TRAIN_513_MM_TILE_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile, core::sync::atomic::Ordering::SeqCst);
    // P6-193: #513's glued step on the 64x64 tiles only when the list asked for it, right after `--mm-placi`.
    crate::drivers::gpu_tur::TRAIN_513_FUSED_TILE_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_fused_tile, core::sync::atomic::Ordering::SeqCst);
    // P6-189: #513's `colsum` split over rows only when the list asked for it, right after `--mm-placi`.
    crate::drivers::gpu_tur::TRAIN_513_COLSUM_SPLIT_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_colsum, core::sync::atomic::Ordering::SeqCst);
    // P6-196: #513's products on the os-416 race's pick only when the list asked for it, last.
    crate::drivers::gpu_tur::TRAIN_513_MM_CHOICE_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice, core::sync::atomic::Ordering::SeqCst);
    // P6-205: one SM shared-memory step on all of #513's step only when the list asked for it, after `--mm-alegere`.
    crate::drivers::gpu_tur::TRAIN_513_SM_STEP_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice && train_513_sm_step, core::sync::atomic::Ordering::SeqCst);
    // P6-213: the waves model's choice only when the list asked for it, after `--treapta-pas`.
    crate::drivers::gpu_tur::TRAIN_513_MM_MODEL_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice && train_513_sm_step && train_513_mm_model, core::sync::atomic::Ordering::SeqCst);
    // P6-198: #513's mixed step only when the list asked for it, last.
    crate::drivers::gpu_tur::TRAIN_513_MIXED_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed, core::sync::atomic::Ordering::SeqCst);
    // P6-215: the fast `mm_tc` programs in a mixed step only when the list asked for it, right after its `--mixt*`.
    crate::drivers::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.store((train_mini && train_mixed && train_mixed_fast) || (train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast), core::sync::atomic::Ordering::SeqCst);
    // P6-216: #513's batch through pinned RAM only when the list asked for it, right after `--mixt-513-rapid`.
    crate::drivers::gpu_tur::TRAIN_513_VIA_RAM_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_via_ram, core::sync::atomic::Ordering::SeqCst);
    // P6-217: two batch slots only when the list asked for it, right after `--lot-prin-ram`.
    crate::drivers::gpu_tur::TRAIN_513_SLOTS_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_via_ram && train_513_slots, core::sync::atomic::Ordering::SeqCst);
    // P6-231: dQ / dK / dV on `attn_back_tile_tc16` only when the list asked for it, after `--mixt-513-rapid`.
    crate::drivers::gpu_tur::TRAIN_513_ATTN_TC16_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16, core::sync::atomic::Ordering::SeqCst);
    // P6-237: the aligned dQ / dK / dV launches on `attn_back_tile_tc16_v4` only when the list asked for it, after `--atentie-tc16`.
    crate::drivers::gpu_tur::TRAIN_513_ATTN_V4_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16 && train_513_attn_v4, core::sync::atomic::Ordering::SeqCst);
    // P6-241: the dS launches on `attn_back_tile0_tc` / `_v4` only when the list asked for it, after `--atentie-tc16` / `--atentie-v4`.
    crate::drivers::gpu_tur::TRAIN_513_ATTN_DS_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16 && train_513_attn_ds, core::sync::atomic::Ordering::SeqCst);
    // P6-190: the night race only when the list asked for it, last, after `--mm-placi`.
    crate::drivers::gpu_tur::TRAIN_513_RACE_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race, core::sync::atomic::Ordering::SeqCst);
    // P6-195: the lone `mm_tv_64x64k8` probe only when the list asked for it, right after `--cursa-mm`.
    crate::drivers::gpu_tur::TRAIN_513_PROBE_64K8_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race && train_513_probe64, core::sync::atomic::Ordering::SeqCst);
    // P6-201: the occupancy probe only when the list asked for it, right after `--cursa-mm`.
    crate::drivers::gpu_tur::TRAIN_513_OCCUPANCY_IN_CYCLE.store(train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race && train_513_occupancy, core::sync::atomic::Ordering::SeqCst);
    // P6-178: the resident run twice (window, then RAM) only when the list asked for it, right after `--vram`.
    crate::drivers::gpu_tur::TRAIN_MINI_RAM_IN_CYCLE.store(train_mini && train_vram && train_ram, core::sync::atomic::Ordering::SeqCst);
    // P6-119 (C-157 point 3): the second diet only when the list asked for it, only for this chain's #513.
    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(diet2, core::sync::atomic::Ordering::SeqCst);
    // P6-123 (C-155 point 3): the argmax on the board only when the list asked for it, only for this chain's model.
    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(argmax, core::sync::atomic::Ordering::SeqCst);
    // P6-126 (C-155 point 4): the argmax without the copy likewise only when the list asked for it.
    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(argmax_top, core::sync::atomic::Ordering::SeqCst);
    // P6-145 (C-157 point 7): the batch only when the list asked for it (the parser lets it only with the argmax without
    // the copy and the default list).
    let batch = batch && argmax_top;
    crate::drivers::gpu_tur::BATCH_IN_CYCLE.store(batch, core::sync::atomic::Ordering::SeqCst);
    // P6-240: `token32` only when the list asked for it, only after this chain's texts.
    crate::drivers::gpu_tur::TOKEN32_IN_CYCLE.store(token32, core::sync::atomic::Ordering::SeqCst);
    if token32 {
        text.push_str("\n(token32: cerut de lista (--token32); dupa textele lui #513, aceleasi texte inca o data cu programul nou `token32` (92 registre, 2176 B partajati, blocuri de 32): tot simbolul intr-o singura lansare, G grupuri care se intalnesc la o bariera pe toata placa intre operatii; intai G = SM-uri x 8 (384, treapta 32 KiB), apoi doar daca a trecut G = SM-uri x 16 (768, treapta 64 KiB); limita 100 ms pe lansare; trece = aceleasi simboluri si aceeasi amprenta a logit-urilor ca rularea obisnuita, contorul barierei (n - 1) x G la fiecare simbol; o lansare care nu se termina = oprire, nimic altceva trimis cu token32; prezis Cyber-GPU: aceleasi simboluri, la 384 ~1,26 ms pe simbol, la 768 ~1,0 ms)\n");
    }
    if batch {
        text.push_str("\n(lotul, simbolul ales ramane pe placa: cerut de lista (--lot); dupa textele cu argmax fara copie, aceeasi generare inca o data pe loturi: programul nou `embed_dev` (19 registre, blocuri de 256) citeste simbolul din rezultatul argmax al pozitiei de dinainte, deci mai multe pozitii (88 de lansari fiecare, invalidare inaintea fiecarei lansari) intra intr-o singura trimitere si o singura sonerie; intai loturi de 4, apoi tot textul (sau cat incape in zona de comenzi); trece = aceleasi simboluri si aceleasi 4 cuvinte argmax pe fiecare pozitie ca rularea cu o pozitie pe trimitere; Cyber-GPU estimeaza ~710-800 simb/s)\n");
    }
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
    if p8 && p8_s8 {
        text.push_str("\n(proba int8 pe circuitele speciale: ceruta de lista (--p8-s8); dupa P8, programul mma16_s8 (nou pentru placa: 32 de registre, un warp, fara memorie partajata) de 11 ori cu 11 seturi fixe (8 aleatoare, 3 margini), judecat bit cu bit pe procesor; trece = 11 / 11 seturi cu aceiasi biti ca oracolul)\n");
    }
    if barrier {
        text.push_str("\n(proba barierei pe toata placa: ceruta de lista (--bariera); dupa textele lui #513, programul grid_barrier (nou pentru placa: 19 registre, blocuri de 32, adunari atomice pe un contor in VRAM) cu G = SM-uri x 1, R = 1 apoi 1001, limita 100 ms pe lansare; doar daca trece, la fel cu G = SM-uri x 2; trece = contorul R x G si niciun cuvant vechi; o lansare care nu se termina = BLOCATA, nimic altceva trimis)\n");
    }
    if barrier_wide {
        text.push_str("\n(proba barierei larga: ceruta de lista (--bariera-larga); acelasi program grid_barrier, aceleasi bufere si limita 100 ms pe lansare ca --bariera, doar G = SM-uri x 8 (384), R = 1 apoi 1001; doar daca trece, G = SM-uri x 16 (768); trece = contorul R x G si niciun cuvant vechi; o lansare care nu se termina = BLOCATA, nimic altceva trimis; prezis Cyber-GPU: 384 TRECE, o bariera 1,2-2,5 us; 768 TRECE, 1,5-4,0 us)\n");
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
    if mm_tc && mm_tc_pf {
        text.push_str("\n(mm_tc_pf langa mm_tc: cerut de lista (--mm-tc-pf); in proba mm_tc, dupa fiecare set, acelasi set cu programul mm_tc_pf (aceeasi treaba ca mm_tc, cu citirile cerute in avans; nou pentru placa: 248 de registre, blocuri de 128, 24 KiB de memorie partajata + o bariera), iesirea comparata octet cu octet cu a lui mm_tc; la qkv-513 tot 5 lansari si timpul median al amandurora; trece = aceiasi biti ca mm_tc pe toate seturile; predictia Cyber-GPU: qkv-513 ~3-5 TFLOP/s (os-423: 1,03); sub 2 = frana e altundeva)\n");
    }
    if mm_tc && mm_tc_v4 {
        text.push_str("\n(mm_tc_v4 langa mm_tc: cerut de lista (--mm-tc-v4); in proba mm_tc, dupa fiecare set (si dupa mm_tc_pf, daca e cerut), acelasi set cu programul mm_tc_v4 (aceeasi treaba ca mm_tc_pf, cu citiri de 128 de biti = 4 numere pe citire; nou pentru placa: 128 de registre, blocuri de 128, 24 KiB de memorie partajata + o bariera), doar pe seturile aliniate (celelalte: nealiniat, sarit), iesirea comparata octet cu octet cu a lui mm_tc; la qkv-513 tot 5 lansari si timpul median; trece = aceiasi biti ca mm_tc pe toate seturile rulate; predictia Cyber-GPU: qkv-513 ~5-9 TFLOP/s (os-423: 1,03); sub x1,5 fata de mm_tc_pf = frana e altundeva)\n");
    }
    if mm_tc && mm_tc_v5 {
        text.push_str("\n(mm_tc_v5 langa mm_tc: cerut de lista (--mm-tc-v5); in proba mm_tc, dupa fiecare set (si dupa mm_tc_v4), acelasi set cu programul mm_tc_v5 (aceeasi treaba si aceeasi bucla ca mm_tc_v4, dar iesirea scrisa prin cate 1 KiB pe grup de fire, deci blocul cere 12 KiB de memorie partajata in loc de 24: la treapta 64 KiB intra 4 blocuri pe unitate in loc de 2; nou pentru placa: programul mm_tc_v5, 128 de registre, blocuri de 128, 12 KiB + o bariera), doar pe seturile aliniate (celelalte: nealiniat, sarit), iesirea comparata octet cu octet cu a lui mm_tc; la qkv-513 tot 5 lansari si timpul median; trece = aceiasi biti ca mm_tc pe toate seturile rulate; predictia Cyber-GPU (cu --mm-tc-64k): mm_tc_v5@64k qkv-513 1,59-1,94 ms = 11,2-13,7 TFLOP/s (mm_tc_v4@64k 2,62 ms); peste 2,19 ms ocuparea nu e frana si ramane mm_tc_v4)\n");
    }
    if mm_tc && mm_tc128_v4 {
        text.push_str("\n(mm_tc128_v4 langa mm_tc: cerut de lista (--mm-tc128-v4); in proba mm_tc, dupa fiecare set (si dupa mm_tc_pf / mm_tc_v4, daca sunt cerute), acelasi set cu programul mm_tc128_v4 (aceeasi treaba ca mm_tc128, pe placi 128 x 128, cu citiri in avans de 128 de biti = 4 numere pe citire; nou pentru placa: 168 de registre, blocuri de 256, 24 KiB de memorie partajata), doar pe seturile aliniate (celelalte: nealiniat, sarit), iesirea comparata octet cu octet cu a lui mm_tc; la qkv-513 tot 5 lansari si timpul median; trece = aceiasi biti ca mm_tc pe toate seturile rulate; predictia Cyber-GPU: qkv-513 ~4-7 TFLOP/s, cam cat mm_tc_v4; daca nu e macar x1,2 fata de mm_tc_v4, ramane mm_tc_v4)\n");
    }
    if mm_tc && mm_tc_64k {
        text.push_str("\n(mm_tc* la treapta 64 KiB: cerut de lista (--mm-tc-64k); in proba mm_tc, dupa fiecare set, acelasi set inca o data cu mm_tc si cu fiecare program cerut langa el, lansate cu treapta memoriei comune 64 KiB (aceleasi campuri ca --treapta-pas din os-431: 2 blocuri pe unitate in loc de 1), iesirea comparata octet cu octet cu a lui mm_tc la treapta de azi; randurile lor au numele cu @64k; la qkv-513 tot 5 lansari si timpul median; nou pentru placa: nimic (aceleasi programe, aceleasi campuri); predictia Cyber-GPU: aceiasi biti, mm_tc_pf qkv-513 x1,6-1,9 -> ~4,4-5,2 TFLOP/s)\n");
    }
    if mm_tc && mm_tc128 {
        text.push_str("\n(mm_tc128 langa mm_tc: cerut de lista (--mm-tc128); in proba mm_tc, dupa fiecare set, acelasi set cu programul mm_tc128 (aceeasi treaba pe placi de 128 x 128; nou pentru placa: 128 de registre, blocuri de 256, 24 KiB de memorie partajata + o bariera), iesirea comparata octet cu octet cu a lui mm_tc; la qkv-513 tot 5 lansari si timpul median al amandurora; trece = aceiasi biti ca mm_tc pe toate seturile; estimarea Cyber-GPU: x1,3-2 mai rapid pe qkv-513)\n");
    }
    if attn_tile {
        text.push_str("\n(proba atentiei pe placi: ceruta de lista (--atentie-placi); dupa textele lui #513, lantul atentiei de antrenare pe placi: attn_fwd_tile (P si att) -> attn_back_tile0 (randurile lui dS) -> attn_back_tile (dQ, dK, dV); 3 programe noi pentru placa (55 / 50 / 73 de registre, 37 / 32 / 32 KiB de memorie partajata + o bariera, blocuri de 256), pe 5 seturi mici (fiecare program judecat singur pe procesor: inainte in marginea MUFU, cele doua inapoi pe P si dS citite de pe placa, prezise bit cu bit), apoi setul atentie-513-b4 (4 texte x 512 pozitii, 12 capete; ~151 MB in VRAM, intrarile urcate si recitite inainte de orice lansare) de 5 ori fiecare program, pentru timp; trece = toate seturile in margine, garzile neatinse; Cyber-GPU estimeaza ~0,53 / ~0,49 / ~0,42-0,68 ms pe lansare)\n");
    }
    if ram_vram {
        text.push_str("\n(proba RAM <-> VRAM cu motorul de copiere: ceruta de lista (--ram-vram); imediat dupa canalul de lucru un mesaj RM aloca motorul de copiere (AMPERE_DMA_COPY_B) pe canal; dupa textele lui #513, o trimitere: copii RAM -> VRAM si VRAM -> RAM la M/64, M/16, M/4, M (M = 64 MiB: 192 MiB RAM fixata + 128 MiB VRAM), apoi 4 x M/4 in paralel, fiecare de 3 ori, cu timpul placii; B si A verificate octet cu octet; nou pentru placa: doar motorul de copiere (metodele pe subcanalul 4), niciun registru, niciun program; Cyber-GPU prezice sus 11-13 GB/s, jos 11-13,5 GB/s, in paralel +-5 %)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race && train_513_occupancy {
        text.push_str("\n(proba ocuparii, ceruta de lista (--ocupare; C-165, studiul centrului 25 nov 07:00, PREDARE Cyber-GPU 54f5ab8): in locul cursei mari, dupa pasii #513, pe forma qkv, 10 rulari din occupancy::PROBE: mm_train_tile la treapta memoriei comune 8, 16, 32, 64 KiB, mm_tv_64x64k8 la 8, 16, 32 KiB, mm_tv_128x128 la 8, 16, 32 KiB (programe trecute pe placa, SHA-256 verificate, pagina de paza); treapta = campurile MIN = TARGET_SM_CONFIG_SHARED_MEM_SIZE din QMD (scrise deja la fiecare lansare; MAX ca azi); 1 lansare de incalzire + 7 cronometrate pe rulare; C comparat bit cu bit cu prima rulare (lansarea de azi); dupa fiecare rulare puterea din tablou; H1 = mm_train_tile la 64 KiB <= 0,85 din timpul de la 8 KiB, H2 = toate treptele in +-3 %; nicio scriere noua in registre; nu schimba verdictul lui #513)\n");
    } else if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race && train_513_probe64 {
        text.push_str("\n(proba mm_tv_64x64k8, ceruta de lista (--proba-64k8; propunerea 3 Cyber-GPU dupa os-413): in locul cursei mari, dupa pasii #513, doar martorul mm_train_tile si mm_tv_64x64k8 pe forma qkv (1024 x 2304 x 768), 64x64k8 mapata ultima, ca la os-413, dar cu o pagina de paza dupa cod (qmd::program_map_bytes); 1 lansare de incalzire + 7 cronometrate; C comparat bit cu bit cu martorul; termina cu aceiasi biti = blocajul de la os-413 venea din citirea in avans dupa capatul codului; nu termina = cauza e in program (placa si-a revenit singura la os-413); nicio scriere noua in registre; nu schimba verdictul lui #513)\n");
    } else if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_race {
        text.push_str("\n(cursa de noapte a inmultirilor, ceruta de lista (--cursa-mm; C-165, decizia H3): dupa pasii #513, pe fiecare forma a lui mm_train din pas (147 de lansari), cele 5 variante din mm_train_variants::race_list_full() (martorul mm_train_tile, trecut pe placa la os-410, + mm_tv_128x64, mm_tv_64x128, mm_tv_128x128, mm_tv_64x64k8; aceasta din urma intoarsa in cursa dupa os-417, unde a terminat cu pagina de paza, RACE_CLEARED_OS417, P6-199; aceiasi biti in SIMULARE, T217; SHA-256 verificate; fiecare program cu o pagina de paza dupa cod, P6-194), cu A, B din arena si C pe o bucata separata cu paza; 1 lansare de incalzire + 7 cronometrate pe varianta si forma; C comparat bit cu bit cu martorul; niciun registru nou, nicio comanda noua; nu schimba verdictul lui #513; raportul: varianta | forma | us median | TFLOP/s | biti, si cea mai rapida cu aceiasi biti pe forma)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_fused_tile {
        text.push_str("\n(pasul #513 lipit pe placile de 64 x 64, cerut de lista (--lipit-placi): planul facut cu Plan::new_tiles3(...).with_fused_back().with_fused_gelu().with_mm_tile_gelu() — aceeasi arena; cele doua adunari reziduale lipite in LayerNorm-ul inapoi (ln_back_res) si GELU' scris direct de inmultirea proj-dX (fara gelu_tr inapoi): 385 -> 349 de lansari; cele 12 inmultiri proj-dX+gelu pe programul nou mm_train_tile_gelu (34 registre, blocuri 256 pe 64 x 64, 8 KiB partajati, acelasi cbuf ca mm_train_gelu; SHA-256 verificat; inca neprobat pe placa), restul inmultirilor pe mm_train_tile; aceiasi biti ca lipit fara placi (SIMULARE); Cyber-GPU asteapta pierderile = oracolul procesorului in pasii 1-9 si pasul cu ~2-4 ms mai scurt decat --mm-placi nelipit)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16 {
        text.push_str("\n(dQ, dK, dV ai atentiei pe circuitele f16 in pasul mixt rapid #513, cerut de lista (--atentie-tc16; C-157, PREDARE Cyber-GPU 3c6fa3b / 250c9b8, notele C-157_P2zm pasul 2 si C-157_P2zp): cele 12 lansari attn_back_tile_3 ale pasului mutate pe attn_back_tile_tc16 (train_step::attn_tc_list, pack_mixed_fast_attn; cu --lot-prin-ram pack_mixed_fast_via_ram_attn), SHA-256 verificat, pagina de paza; programul a rulat pe placa in os-457 (--atentie-tc: 5 / 5 = modelul); Cyber-GPU prezice ~2,2-4,0 ms mai putin pe pas la lotul 2 (DEDUS); verdictul ramane oracolul pasului mixt cu tolerantele lui (1e-3 / 2e-3), dar nu se asteapta aceleasi numere bit cu bit: oracolul are atentia in F32)");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16 && train_513_attn_ds {
        text.push_str("\n(dS al atentiei pe circuitele f16 in pasul mixt rapid #513, cerut de lista (--atentie-ds; C-157, PREDARE Cyber-GPU 9b11018, nota C-157_P2zu; programele reparate 5c83a80, nota C-157_P2zx): cele 12 lansari attn_back_tile0_3 ale pasului mutate pe attn_back_tile0_tc_v4 unde forma e aliniata, attn_back_tile0_tc pe rest (train_step::attn0_tc_list + pack_mixed_fast_attn4 cu Attn0Programs; cu --lot-prin-ram batch_ram::pack_mixed_fast_via_ram_attn4), SHA-256 verificat, pagina de paza; nou pentru placa: nimic fata de proba atentie-tc0 / atentie-tc0-v4 (aceleasi programe); Cyber-GPU asteapta 12 pe v4 / 0 pe tc si programul dS de x1,2-1,8 mai rapid (DEDUS); verdictul ramane oracolul pasului mixt cu tolerantele lui (oracolul are dS in F32))");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_attn_tc16 && train_513_attn_v4 {
        text.push_str("\n(dQ, dK, dV pe attn_back_tile_tc16_v4 in pasul mixt rapid #513, cerut de lista (--atentie-v4; C-157, PREDARE Cyber-GPU 6e8b29a, nota C-157_P2zv): castigatoarea probei P2zq din os-465 (v4 = tc16 pe 4 / 4 seturi aliniate, 0,52 ms fata de 0,87 ms la atentie-513-b4) aleasa pe fiecare lansare aliniata, tc16 pe rest (train_step::pack_mixed_fast_attn4; cu --lot-prin-ram batch_ram::pack_mixed_fast_via_ram_attn4), SHA-256 verificat, pagina de paza; nou pentru placa: nimic fata de os-465 (acelasi program); Cyber-GPU asteapta 12 pe v4 / 0 pe tc16 si pierderile + gradientul identice bit cu bit cu pasul pe tc16 (aceeasi aritmetica); verdictul ramane oracolul pasului mixt)");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_via_ram && train_513_slots {
        text.push_str("\n(doua locuri de lot pentru pasul #513 prin RAM fixata, cerut de lista (--doua-locuri; C-157, PREDARE Cyber-GPU 1ddf092, nota C-157_P2zg, pasul 2): Plan::with_batch_slots — al doilea loc de lot la capatul arenei, fiecare loc cu zona lui de tranzit, zona lui de lansari si inregistrarile lui; procesorul pregateste pasul N+1 cat placa lucreaza pasul N, iar partile pasului N+1 se trimit pe masura ce se termina partile pasului N (in bufferul de comenzi niciodata mai mult de un pas); nou pentru placa: niciun program, niciun registru, doar doua trimiteri in zbor in loc de una; trece = aceleasi verificari ca --lot-prin-ram; Cyber-GPU prezice pasul intreg ~ placa + < 0,5 ms (DEDUS))");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast && train_513_via_ram {
        text.push_str("\n(lotul si rezultatele pasului mixt rapid #513 prin RAM fixata, cerut de lista (--lot-prin-ram; C-157, PREDARE Cyber-GPU 1ddf092, nota C-157_P2zg, pasul 1): lotul scris in RAM fixata si adus in arena de programul de copiere (5 copieri la inceputul trimiterii), pierderile + norma duse inapoi in RAM (2 copieri la sfarsit), in loc de fereastra VRAM; nou pentru placa: niciun program (copy_b128 din os-397), niciun registru, doar 7 copieri mici in pas, la treapta 64 KiB ca restul; trece = aceleasi verificari ca --mixt-513-rapid; randul nou `in afara placii` (pasul intreg - placa, pe bucati); Cyber-GPU prezice in afara placii -4-5 ms la lot 2 (os-431: 10,95 ms) si -11-15 ms la lot 6 (os-437: 18,5 ms), placa +20-40 us (DEDUS))");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed && train_513_mixed_fast {
        text.push_str("\n(pasul mixt #513 cu programele mm_tc rapide, cerut de lista (--mixt-513-rapid; C-157 p.2, PREDARE Cyber-GPU a4c9db0, nota C-157_P2zf): aceeasi lista mixta, dar inmultirile pe mm_tc_v4 / mm_tc_pf prin pack_mixed_fast si toate lansarile la treapta memoriei comune 64 KiB (campurile lui --treapta-pas din os-431); Cyber-GPU prezice toate cele 147 de inmultiri pe mm_tc_v4 si ~115-136 ms pe inmultiri (DEDUS); nimic nou pentru placa in afara amestecului)");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mixed {
        text.push_str("\n(pasul mixt pe #513, cerut de lista (--mixt-513; C-165, PREDARE Cyber-GPU 1617228, nota C-165_pas-mixt-513): toate cele 147 de inmultiri ale modelului din pas pe mm_tc (intrari f16, sume f32 pe nucleele tensoriale; SHA-256 verificat, mapat cu pagina de paza), scara erorii S = 16384 (LOSS_SCALE_513), 1/S scris o data in arena; m, v si greutatile raman f32; aceeasi arena; norma citita = res[0] / S; verdictul fata de oracolul procesorului cu tolerantele pasului mixt (pierderea 1e-3, norma 2e-3); nou pentru placa: mm_tc in pasul #513 (singur doar in proba os-352); DEDUS Cyber-GPU: daca mm_tc atinge ~5 TFLOP/s, inmultirile pasului scad de la ~300 la ~60-120 ms)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice {
        text.push_str("\n(inmultirile pasului #513 pe varianta aleasa de cursa os-416, cerut de lista (--mm-alegere; C-165, PREDARE Cyber-GPU with_mm_choice + parse_race): randurile cursei os-416 (60, toate cu aceiasi biti ca martorul) citite cu mm_train_variants::parse_race -> MmChoice (cea mai rapida cu aceiasi biti, doar la >= 3 % castig) -> Plan::with_mm_choice; la os-416 mm_tv_128x128 a castigat pe toate cele 15 forme, deci cele 147 de inmultiri ale pasului pe mm_tv_128x128 (90 registre, blocuri 256 pe 128 x 128; SHA-256 verificat; trecuta in cursa cu aceiasi biti); aceeasi arena, aceleasi lansari; nicio comanda noua; Cyber-GPU / centrul asteapta pierderile = oracolul in pasii 1-9 si pasul ~474 -> ~333 ms (DEDUS din cursa))\n");
    }
    if let (true, Some(n)) = (train_mini && train_vram && train_513, batch_513) {
        text.push_str(&format!("\n(lotul pasului #513 = {} secvente de 512 in loc de 2, cerut de lista (--lot-513 {}; C-158, nota centrului 25 nov 14:40 punctul 2, lotul mare; arena fara plafon fix, DECIZII 25 nov 11:52/11:55): inainte de orice trimitere, arena lotului {} (aceleasi optiuni ale planului) e verificata fata de cea mai lunga bucata libera de VRAM, cu rezerva de 256 MiB; daca nu incape, pasul ruleaza cu lotul vechi 2 si raportul spune de ce; acelasi drum, aceleasi programe, nimic nou pentru placa in afara de marimea arenei si a grilelor; fara oracol pe placa la alt lot decat 2 (verdictul: pierderea la pasul 1 ~ ln 16384, toate finite, ultima sub prima); asteptat: simb/s mai mare decat la lotul 2 (aceeasi lucrare fixa a pasului impartita pe mai multe simboluri))\n", n, n, n));
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice && train_513_sm_step {
        text.push_str("\n(o singura treapta a memoriei comune pe tot pasul #513, cerut de lista (--treapta-pas; C-165, PREDARE Cyber-GPU 8b6a72d, dupa proba ocuparii os-423 = H1): treapta aleasa cu train_step::step_best_sm_step (rezerva 1 KiB; pe placa 64 KiB, cat cere atentia _3) si lista impachetata cu pack_with_sm_step: MIN = TARGET = treapta, MAX = 64 KiB la fiecare lansare a pasului; aceleasi programe, adrese, cbuf-uri si comenzi, nimic altceva nou pentru placa; Cyber-GPU / centrul asteapta pierderile = oracolul in pasii 1-9 (aceiasi biti ca os-418) si pasul ~338 -> ~200-250 ms (DEDUS din proba ocuparii))\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_mm_choice && train_513_sm_step && train_513_mm_model {
        text.push_str("\n(alegerea inmultirilor din modelul valurilor, cerut de lista (--mm-model; C-165, PREDARE Cyber-GPU 8fe7637, nota C-165_valuri-alegerea-64k, drumul 2): in locul cursei os-416, wave_model::predicted_choice la 64 KiB (timpii pe val din os-423) alege pe fiecare forma mm_train_tile, mm_tv_64x64k8 sau mm_tv_128x128 -> Plan::with_mm_choice; aceleasi trei programe (toate cu aceiasi biti ca martorul in os-416/417/423), aceeasi treapta 64 KiB ca --treapta-pas; nou pentru placa doar amestecul lor in acelasi pas; Cyber-GPU / centrul asteapta pierderile = oracolul in pasii 1-9 si pasul ~267 -> ~204 ms (195-215; ~5 000 simb/s; DEDUS din modelul valurilor))\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile && train_513_colsum {
        text.push_str("\n(colsum-ul pasului #513 impartit pe randuri, cerut de lista (--colsum-impartit): planul facut cu Plan::new_tiles3(...).with_mm_tile().with_colsum_split() — fiecare colsum devine colsum_part (bucati de randuri in sume partiale) + colsum peste sumele partiale (inca unul la LayerNorm): 385 -> 483 de lansari, tot 2 trimiteri; arena + o zona de lucru de 768 KiB (scrisa si citita doar de placa); programul nou colsum_part (20 registre, bloc 256, fara memorie partajata; SHA-256 verificat; inca neprobat pe placa); nu aceiasi biti ca fara optiune (alta ordine a sumelor), judecat tot fata de oracolul procesorului in pasii 1-9; Cyber-GPU asteapta colsum 24,3 -> ~2,5-5 ms pe pas, pasul cu ~19-22 ms mai scurt)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 && train_513_mm_tile {
        text.push_str("\n(inmultirile pasului #513 pe mm_train_tile, cerute de lista (--mm-placi): planul pasului #513 facut cu Plan::new_tiles3(...).with_mm_tile() — aceeasi arena (2 560 MiB), aceleasi 385 de lansari in 2 trimiteri; cele 147 de lansari mm_train folosesc programul nou mm_train_tile (blocuri de 256 fire pe 64 x 64, 8 KiB partajati, 34 registre; SHA-256 verificat; inca neprobat pe placa) cu acelasi cbuf; Cyber-GPU asteapta inmultirile de 3-6 ori mai repede (~718 ms -> ~120-240 ms pe pas) si pierderile = oracolul procesorului in pasii 1-9)\n");
    }
    if train_mini && train_vram && train_513 && train_513_tiles3 {
        text.push_str("\n(pasul #513 pe atentia pe placi gen 3, cerut de lista (--placi3): planul pasului #513 facut cu Plan::new_tiles3 — aceeasi arena (2 560 MiB), aceleasi 385 de lansari in 2 trimiteri; doar cele 36 de lansari de atentie (12 inainte, 12 dS, 12 dQKV) folosesc attn_fwd_tile3 / attn_back_tile0_3 / attn_back_tile_3 (trecute pe placa la os-375) in locul lui attn_fwd / attn_back; niciun program nou, niciun registru nou; Cyber-GPU asteapta atentia ~20-40 ms in loc de 371 si pasul ~770-800 ms in loc de 1 123, pierderile = oracolul procesorului in pasii 1-9)\n");
    }
    if train_mini && train_vram && train_513 {
        text.push_str("\n(pasul #513, cerut de lista (--antrenare-513): dupa ce antrenarea mini cu arena in VRAM a trecut, pasul de antrenare al lui Kronos #513 (12 straturi, E 768, 12 capete, vocabular 16 384; lot 2 x 512, AdamW, 10 pasi) facut de lista placii cu arena in VRAM (2 560 MiB, sub limita de 6 GiB): urcata o data in 40 de bucati de 64 MiB cu programul de copiere, pasul (385 de lansari) trimis in 2 parti (255 + 130); nou pentru placa: doar forma mare (arena de GiB, partile), niciun program nou, niciun registru nou; fara oracol pe procesor (prea lent pentru #513): trece = pierderea la pasul 1 ~ ln 16 384 = 9,70 (la 2 %), scade pana la ultimul pas, toate numerele finite; cifra de comparat: simbolurile/s ale placii fata de Windows / PyTorch ~39 300)\n");
    }
    if train_mini && train_vram && train_ram {
        text.push_str("\n(lotul prin RAM, cerut de lista (--lot-ram): pasul cu arena in VRAM facut de doua ori din aceeasi stare — intai ca in os-390 (lotul scris si pierderile citite prin fereastra VRAM), apoi lotul scris in RAM fixata si adus de programul de copiere in lista pasului (5 copieri la inceput, 2 la sfarsit pentru pierderi + norma); nou pentru placa: niciun program nou (copy_b128 a mers in os-390), doar 7 copieri mici in lista pasului; trece = ambele rulari = oracolul si prin RAM = prin fereastra bit cu bit (pierderea si norma la fiecare pas, toate greutatile la sfarsit); Cyber-GPU prezice pasul intreg ~3,8 ms (os-390: 4,15 ms) ≈ 16 700 simboluri/s)\n");
    }
    if train_mini && train_vram && train_fused {
        text.push_str("\n(pasul lipit, cerut de lista (--pas-lipit): pasul cu arena in VRAM facut de doua ori din aceeasi stare — intai nelipit (75 de lansari), apoi cu cele doua programe lipite ale lui Cyber-GPU (ln_back_res: LayerNorm inapoi + adunarea reziduala; mm_train_gelu: GELU inapoi in produsul proj-dX; 69 de lansari); nou pentru placa: doar cele doua programe lipite (SHA-256 verificate), nicio scriere noua in registre; trece = ambele rulari = oracolul si lipit = nelipit bit cu bit (pierderea si norma la fiecare pas, toate greutatile la sfarsit); in SIMULARE: aceiasi biti)\n");
    }
    if train_mini && train_vram {
        text.push_str("\n(arena in VRAM, ceruta de lista (--vram): acelasi pas F32 al lui `antrenare mini` (cele 13 programe ale pasului de aur), dar arena (~26 MiB: greutati, gradient, m, v) facuta o data in VRAM: urcata o data RAM -> VRAM cu programul de copiere, apoi pe pas doar lotul (~1,3 KiB) sus si pierderile + norma (260 B) jos, iar la sfarsit o copie VRAM -> RAM pentru greutati; nou pentru placa: arena in VRAM si cele doua copii mari, niciun program nou, niciun registru nou; trece = pierderea si norma la fiecare pas si greutatile la sfarsit la cel mult 1e-3 fata de oracol; Cyber-GPU prezice placa 1-5 ms pe pas (os-380: 25,6 ms) si pasul intreg cativa ms (os-380: 286 ms))\n");
    }
    if train_mini && train_mixed && train_mixed_fast {
        text.push_str("\n(pasul mixt cu programele mm_tc rapide, cerut de lista (--mixt-rapid; C-157 p.2, PREDARE Cyber-GPU a4c9db0, nota C-157_P2zf): acelasi pas mixt, dar fiecare inmultire pe mm_tc_v4 (citiri de 128 de biti) cand produsul e aliniat, altfel pe mm_tc_pf (citiri in avans), prin pack_mixed_fast; nou pentru placa: niciun program nou (mm_tc_pf din os-437, mm_tc_v4 din os-438), doar amestecul lor in pas; trece = aceleasi verificari fata de oracol ca --mixt; randul `alegerile mm_tc` spune cate inmultiri pe fiecare program)");
    }
    if train_mini && train_mixed {
        text.push_str("\n(pasul mixt, cerut de lista (--mixt): acelasi pas al lui `antrenare mini`, dar toate cele 27 de inmultiri ale pasului pe circuitele speciale (programul mm_tc: intrari f16, sume f32, scara erorii S = 1024, impartita inapoi inainte de AdamW); nou pentru placa: mm_tc in pasul de antrenare (proba lui singura: os-352); trece = pierderea la cel mult 1e-3 si norma gradientului la cel mult 2e-3 fata de oracolul F32 (greutatile doar afisate: f16 + AdamW le misca ~1e-3); in SIMULARE abaterile au fost 5,1e-6 si 1,6e-4)\n");
    }
    if train_mini {
        text.push_str("\n(antrenare mini pe placa, ceruta de lista (--antrenare-mini): dupa toate celelalte probe, pasul intreg de antrenare al lui `antrenare mini` (modelul mic pe vocabularul lui Kronos: 16 384 simboluri, 2 straturi, E 64, 1 150 720 greutati; lot 2 x 32; AdamW lr 1e-3, taiere 1,0) facut de lista placii (cele 13 programe ale pasului de aur, os-351), 10 pasi, cate o trimitere pe pas; oracolul Cyber-GPU pe procesor alaturi, pe aceleasi loturi; arena ~26 MiB in RAM fixata; datele: antrenare/date de pe disc sau stick (doar citite), altfel loturi sintetice; nou pentru placa: doar forma mare a pasului (stratul de iesire cu 16 384 simboluri), niciun program nou; trece = aceleasi numere ca oracolul (toleranta 1e-3); in raport: timpul placii pe pas si simbolurile pe secunda la antrenare)\n");
    }
    if adam_ram {
        text.push_str("\n(AdamW cu m, v in RAM, cerut de lista (--adam-ram): dupa proba RAM <-> VRAM si doar daca ea a trecut, pasul de aur mic pe placi (2 straturi, C 64, lot 2 x 32), adamw_513 x 10 pasi de doua ori pe aceeasi arena: planul de azi (m, v in arena) si m, v in RAM fixata, urcate pe placa bucata cu bucata (tabelele, fiecare strat, LayerNorm-ul final) de motorul de copiere; nou pentru placa: motorul de copiere in pasul de antrenare si asteptarea canalului pe semafor (SEM_EXECUTE ACQ_STRICT_GEQ); niciun program nou; judecat fata de PyTorch si comparat bit cu bit cu planul de azi; trece = AdamW cu m, v in RAM: TRECE)\n");
    }
    if attn_tc {
        text.push_str("\n(atentie-tc, vechi langa nou: ceruta de lista (--atentie-tc); in aceeasi proba, dupa ultimul lant nou (_3 sau _4) pe fiecare set, pe aceleasi P / dS, attn_back_tile_tc si apoi attn_back_tile_tc16 (dQ, dK, dV pe circuitele speciale, intrarile in f16), fiecare cu dqkv umplut cu NaN inainte; 2 programe noi pentru placa (48 / 48 de registre, 32 768 / 16 384 octeti partajati, argumentele la 0x700 intr-un cbuf de 0x800); judecate cu gpu_compute::attn_tc::judge (marginea mm_tc pe cap), tc16 comparat cuvant cu cuvant cu tc, cronometrate langa attn_back_tile_3; Cyber-GPU asteapta 5 / 5 seturi in margine, tc16 = tc peste tot, abaterea fata de F32 ~3e-4, la atentie-513-b4 tc ~0,2-0,37 ms pe lansare (2,4-4,5 ms pe 12), tc16 cel mult cat tc, fata de ~0,77 la _3)\n");
    }
    if attn_tc && attn_tc0 {
        text.push_str("\n(atentie-tc0, vechi langa nou: ceruta de lista (--atentie-tc0); in proba atentie-tc, dupa programele dQ / dK / dV pe circuitele speciale, pe aceleasi P / dO / V, programul nou attn_back_tile0_tc (dS cu dP = dO*V^T pe circuitele speciale, restul F32; 56 de registre, 33 792 octeti partajati, argumentele la 0x700) cu dS umplut cu NaN inainte; fata de dS-ul F32 al lantului nou: abaterea, NaN ramasi sub diagonala, zerourile de deasupra, garda; cuvintele exacte fata de gpu_compute::attn_tc::back0_model; timpii langa attn_back_tile0_3; prezis Cyber-GPU: abaterea ~3e-4 (sub 1e-3), garda 0, la atentie-513-b4 o treime - jumatate din timpul lui attn_back_tile0_3)\n");
    }
    if attn_tc && attn_tc0 && attn_tc0_v4 {
        text.push_str("\n(atentie-tc0-v4, vechi langa nou: ceruta de lista (--atentie-tc0-v4); in proba atentie-tc0, dupa attn_back_tile0_tc, pe aceleasi intrari, programul nou attn_back_tile0_tc_v4 (acelasi dS cu citiri si scrieri de 128 de biti; 56 de registre, 33 792 octeti partajati) doar pe seturile aliniate (fits; celelalte sarite), dS umplut cu NaN inainte; trece = aceleasi cuvinte ca attn_back_tile0_tc pe toate seturile aliniate si pe randurile atentie-513-b4, garzi neatinse; prezis Cyber-GPU: aceiasi biti, la atentie-513-b4 x1,2-1,8 mai rapid decat attn_back_tile0_tc (sub x1,15 nu se leaga in pas))\n");
    }
    if attn_tc && attn_tc_64k {
        text.push_str("\n(atentie-tc@64k: ceruta de lista (--atentie-tc-64k; C-157 P2zw punctul 3, nota Cyber-GPU 8125211); in proba atentie-tc, dupa fiecare program pe circuitele speciale (tc, tc16 si, daca sunt cerute, v4 / v4pf), acelasi program inca o data cu treapta memoriei comune 64 KiB (campurile lui --treapta-pas, cum ruleaza in pasul mixt #513: 3-4 blocuri pe unitate pentru tc16 in loc de 1), dqkv umplut iar cu NaN, comparat cuvant cu cuvant cu rularea de azi a aceluiasi program; la atentie-513-b4 tot 5 lansari si mediana; nou pentru placa: nimic (aceleasi programe, aceleasi campuri); trece = aceiasi biti peste tot, garzi neatinse; Cyber-GPU: in pas tc16 a castigat 6,7 ms in loc de 2,2-4,0 prezis, din cauza treptei)");
    }
    if attn_tc && attn_tc_v4 {
        text.push_str("\n(atentie-tc16-v4, vechi langa nou: ceruta de lista (--atentie-tc16-v4); in proba atentie-tc, dupa tc16, pe aceleasi P / dS, programele noi attn_back_tile_tc16_v4 (72 de registre) si attn_back_tile_tc16_v4pf (104), citiri de 128 de biti, doar pe seturile aliniate (fits; celelalte sarite); trece = aceleasi cuvinte ca tc16 pe toate seturile aliniate si pe benzile atentie-513-b4, garzi neatinse; prezis Cyber-GPU: aceiasi biti, 0,35-0,55 ms pe lansare la atentie-513-b4 fata de 0,87 ms tc16)\n");
    }
    if attn_tile4 {
        text.push_str("\n(atentie4, vechi langa nou: ceruta de lista (--atentie4); in aceeasi proba, dupa lantul attn_*_3 pe fiecare set, pe acelasi buffer reumplut, lantul attn_fwd_tile3 -> attn_back_tile0_3 -> attn_back_tile_4 (primele doua = fisierele _3; doar ultimul e nou: placa intreaga a diagonalei dK / dV pe bucla fara garda, aceleasi operatii in aceeasi ordine); un singur program nou pentru placa (attn_back_tile_4: 80 de registre, 33 024 octeti partajati); judecat fata de prima generatie, cronometrat langa _3; Cyber-GPU asteapta 6 / 6 seturi in margine, 6 / 6 identice cu vechii (~85 %), back1 ~1,2-1,35 ms fata de ~1,60)\n");
    }
    if attn_tile3 {
        text.push_str("\n(atentie3, vechi langa nou: ceruta de lista (--atentie3); in aceeasi proba, dupa lantul attn_*_2 pe fiecare set, pe acelasi buffer, lantul attn_fwd_tile3 -> attn_back_tile0_3 -> attn_back_tile_3 (aceleasi operatii in aceeasi ordine, citirile din memorie fara ramura); 3 programe noi pentru placa (120 / 68 / 80 de registre, 37 888 / 33 024 / 33 024 octeti partajati); judecat cu aceleasi margini, comparat cuvant cu cuvant cu lantul vechi si cronometrat langa attn_*_2; Cyber-GPU asteapta 6 / 6 seturi in margine, 6 / 6 identice cu vechii, la atentie-513-b4 ~0,7-1,1 / ~0,5-0,9 / ~1,6-2,0 ms pe lansare)\n");
    }
    if attn_tile2 {
        text.push_str("\n(atentie2, vechi langa nou: ceruta de lista (--atentie2); in aceeasi proba, dupa lantul vechi pe fiecare set, pe acelasi buffer, lantul nou attn_fwd_tile2 -> attn_back_tile0_2 -> attn_back_tile_2 (aceleasi operatii in aceeasi ordine, bucle mai scurte); 3 programe noi pentru placa (117 / 52 / 79 de registre, 37 888 / 33 024 / 33 024 octeti partajati); judecat cu aceleasi margini si comparat cuvant cu cuvant cu cel vechi; Cyber-GPU asteapta 6 / 6 seturi in margine, 6 / 6 identice cu vechii, la atentie-513-b4 ~0,6-1,2 / ~0,7-1,1 / ~1,2-2,0 ms pe lansare)\n");
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
    crate::drivers::gpu_tur::RUSD_EARLY.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::FUSED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::FUSED_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN_NEW_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::QUARTERS_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN32_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::LN32_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::P8_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::P8_S8_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::BARRIER_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::BARRIER_WIDE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC128_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_PF_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_V4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_V5_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC128_V4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::MM_TC_64K_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TILE2_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TILE3_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TILE4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC_V4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC0_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC0_V4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ATTN_TC_64K_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::RAM_VRAM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ADAM_RAM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MINI_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MINI_MIXED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MINI_VRAM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MINI_FUSED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_BATCH.store(0, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_TILES3_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_MM_TILE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_COLSUM_SPLIT_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_MM_CHOICE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_SM_STEP_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_MIXED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_VIA_RAM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_SLOTS_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_ATTN_TC16_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_ATTN_V4_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_ATTN_DS_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_MM_MODEL_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_FUSED_TILE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_RACE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_PROBE_64K8_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_513_OCCUPANCY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TRAIN_MINI_RAM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::BATCH_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    crate::drivers::gpu_tur::TOKEN32_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
    // P6-240: the `token32` verdict too.
    let result = if token32 {
        let verdict = text.lines().rev().find(|l| l.contains("token32 (o lansare pe simbol)")).map_or("token32: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-126: the verdict of the argmax without the copy next to the chain's in the answer.
    let result = if argmax_top {
        let verdict = text.lines().rev().find(|l| l.contains("argmax fara copie (simbolul ales pe placa)")).map_or("argmax fara copie: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-145: the batch's verdict too.
    let result = if batch {
        let verdict = text.lines().rev().find(|l| l.contains("lot (simbolul ramane pe placa)")).map_or("lot: niciun rezultat (vezi raportul)", |l| l.trim());
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
    // P6-200: `mma16_s8`'s verdict too.
    let result = if p8 && p8_s8 {
        let verdict = text.lines().rev().find(|l| l.contains("proba mma16_s8: ")).map_or("proba mma16_s8: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-218: the grid-barrier probe's verdict too.
    let result = if barrier {
        let verdict = text.lines().rev().find(|l| l.contains("proba barierei: ")).map_or("proba barierei: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
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
    // P6-197: `mm_tc128`'s verdict too.
    let result = if mm_tc && mm_tc128 {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc128: ")).map_or("proba mm_tc128: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-207: `mm_tc_pf`'s verdict too.
    let result = if mm_tc && mm_tc_pf {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc_pf: ")).map_or("proba mm_tc_pf: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-210: `mm_tc_v4`'s verdict too.
    let result = if mm_tc && mm_tc_v4 {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc_v4: ")).map_or("proba mm_tc_v4: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-242: `mm_tc_v5`'s verdict too.
    let result = if mm_tc && mm_tc_v5 {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc_v5: ")).map_or("proba mm_tc_v5: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-212: `mm_tc128_v4`'s verdict too.
    let result = if mm_tc && mm_tc128_v4 {
        let verdict = text.lines().rev().find(|l| l.contains("proba mm_tc128_v4: ")).map_or("proba mm_tc128_v4: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-214: every verdict at the 64 KiB step too (`proba <program>@64k: …`).
    let result = if mm_tc && mm_tc_64k {
        let v: Vec<&str> = text.lines().filter(|l| l.contains("@64k: ") && l.contains("proba mm_tc")).map(|l| l.trim()).collect();
        if v.is_empty() {
            format!("{}; proba mm_tc@64k: niciun rezultat (vezi raportul)", result)
        } else {
            format!("{}; {}", result, v.join("; "))
        }
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
    // P6-149: the new chain's verdict too.
    let result = if attn_tile2 {
        let verdict = text.lines().rev().find(|l| l.contains("atentie2 (vechi langa nou): ")).map_or("atentie2: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-160: and the third's.
    let result = if attn_tile3 {
        let verdict = text.lines().rev().find(|l| l.contains("atentie3 (vechi langa nou): ")).map_or("atentie3: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-168: and the fourth's.
    let result = if attn_tile4 {
        let verdict = text.lines().rev().find(|l| l.contains("atentie4 (vechi langa nou): ")).map_or("atentie4: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-227: and the tensor-core programs'.
    let result = if attn_tc {
        let verdict = text.lines().rev().find(|l| l.contains("atentie-tc (vechi langa nou): ")).map_or("atentie-tc: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-161: the RAM ↔ VRAM probe's verdict.
    let result = if ram_vram {
        let verdict = text.lines().rev().find(|l| l.contains("proba RAM <-> VRAM: ")).map_or("proba RAM <-> VRAM: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-163: the m, v in RAM step's verdict.
    let result = if adam_ram {
        let verdict = text.lines().rev().find(|l| l.contains("AdamW cu m, v in RAM: ")).map_or("AdamW cu m, v in RAM: niciun rezultat (vezi raportul)", |l| l.trim());
        format!("{}; {}", result, verdict)
    } else {
        result
    };
    // P6-164: the whole training step on the board, its verdict.
    let result = if train_mini {
        format!("{}; {}", result, train_verdicts(&text))
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

/// P6-181 (os-390 / os-392 / os-395 reports: the summary said "niciun rezultat" although the block said TRECE): every
/// verdict row of the training on the board (`antrenare mini pe placa …`, with or without `(arena in VRAM …)` /
/// `(mixt)`, and `antrenare #513 pe placa …`), in order, after `kronos --gpu: `; none = "niciun rezultat".
pub fn train_verdicts(text: &str) -> String {
    let rows: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let at = l.find("kronos --gpu: antrenare ")?;
            let r = l[at + "kronos --gpu: ".len()..].trim();
            let ends = [": TRECE (", ": NU TRECE (", ": NU A RULAT", ": NU A TERMINAT"];
            ends.iter().any(|e| r.contains(e)).then_some(r)
        })
        .collect();
    if rows.is_empty() {
        return String::from("antrenare mini pe placa: niciun rezultat (vezi raportul)");
    }
    rows.join("; ")
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
    // P6-181: the training verdicts in the summary (the rows of os-390 / os-395, with the cycle's prefix).
    let rep = "kronos: lant: gpu0: kronos --gpu: antrenare mini pe placa (arena in VRAM): pasul 1: pierderea 9.7404 (oracolul 9.7404)\nkronos: lant: gpu0: kronos --gpu: antrenare mini pe placa (arena in VRAM): TRECE (antrenarea cu arena in VRAM = oracolul)\nkronos: lant: gpu0: kronos --gpu: antrenare #513 pe placa (arena in VRAM): NU TRECE (vezi randurile de mai sus)\n";
    let v = train_verdicts(rep);
    assert!(v == "antrenare mini pe placa (arena in VRAM): TRECE (antrenarea cu arena in VRAM = oracolul); antrenare #513 pe placa (arena in VRAM): NU TRECE (vezi randurile de mai sus)", "test: verdictele antrenarii: {}", v);
    assert!(train_verdicts("kronos: lant: gpu0: kronos --gpu: antrenare mini pe placa (mixt): NU A RULAT: x; nimic trimis\n") == "antrenare mini pe placa (mixt): NU A RULAT: x; nimic trimis");
    assert!(train_verdicts("x antrenare mini pe placa: pasul 1\n").contains("niciun rezultat"));
    // P6-101: the FLR trial form.
    assert!(board_command("test-placa --flr-proba") == Some(Ok(String::from("c135 --flr-proba"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba") == Some(Ok(String::from("c133 --flr-proba"))));
    assert!(board_command("test-placa --flr-proba --i9 c133").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --tablou") == Some(Ok(String::from("c135 --tablou"))));
    assert!(board_command("test-placa --i9 c133 --flr-proba --tablou") == Some(Ok(String::from("c133 --flr-proba --tablou"))));
    assert!(board_command("test-placa --tablou --flr-proba").is_some_and(|r| r.is_err()));
    // P6-142: `--tablou-viteza` in place of `--tablou` (not both), before the list options.
    assert!(board_command("test-placa --tablou-viteza") == Some(Ok(String::from("c135 --tablou-viteza"))));
    assert!(board_command("test-placa --flr-proba --tablou-viteza --k651 --argmax-fara-copie --dieta2") == Some(Ok(String::from("c135 --flr-proba --tablou-viteza --k651 --argmax-fara-copie --dieta2"))));
    assert!(board_command("test-placa --tablou --tablou-viteza").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --tablou-viteza --tablou").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit --tablou-viteza").is_some_and(|r| r.is_err()));
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
    // P6-200: `--p8-s8` only right after `--p8`.
    assert!(board_command("test-placa --p8 --p8-s8") == Some(Ok(String::from("c135 --p8 --p8-s8"))));
    assert!(board_command("test-placa --tablou --p8 --p8-s8 --colsum") == Some(Ok(String::from("c135 --tablou --p8 --p8-s8 --colsum"))));
    assert!(board_command("test-placa --p8-s8").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --p8-s8 --p8").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --dieta2 --p8-s8").is_some_and(|r| r.is_err()));
    // P6-218: `--bariera` after `--p8` / `--p8-s8`, before `--colsum`.
    assert!(board_command("test-placa --bariera") == Some(Ok(String::from("c135 --bariera"))));
    assert!(board_command("test-placa --p8 --p8-s8 --bariera --colsum") == Some(Ok(String::from("c135 --p8 --p8-s8 --bariera --colsum"))));
    assert!(board_command("test-placa --colsum --bariera").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --bariera --p8").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --bariera --bariera").is_some_and(|r| r.is_err()));
    // P6-240: `--token32` after `--lot`, before `--p8`.
    assert!(board_command("test-placa --token32") == Some(Ok(String::from("c135 --token32"))));
    assert!(board_command("test-placa --argmax-fara-copie --lot --token32 --p8") == Some(Ok(String::from("c135 --argmax-fara-copie --lot --token32 --p8"))));
    assert!(board_command("test-placa --p8 --token32").is_some_and(|r| r.is_err()));
    // P6-232: the wide barrier probe, alone or right after `--bariera`, before `--colsum`.
    assert!(board_command("test-placa --bariera-larga") == Some(Ok(String::from("c135 --bariera-larga"))));
    assert!(board_command("test-placa --bariera --bariera-larga --colsum") == Some(Ok(String::from("c135 --bariera --bariera-larga --colsum"))));
    assert!(board_command("test-placa --bariera-larga --bariera").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --colsum --bariera-larga").is_some_and(|r| r.is_err()));
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
    // P6-197: `--mm-tc128` only right after `--mm-tc`.
    assert!(board_command("test-placa --mm-tc --mm-tc128") == Some(Ok(String::from("c135 --mm-tc --mm-tc128"))));
    assert!(board_command("test-placa --mm-tc --mm-tc128 --atentie-placi") == Some(Ok(String::from("c135 --mm-tc --mm-tc128 --atentie-placi"))));
    assert!(board_command("test-placa --mm-tc128").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --pas-aur --mm-tc128").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc128 --mm-tc").is_some_and(|r| r.is_err()));
    // P6-207: `--mm-tc-pf` only right after `--mm-tc`, not with `--mm-tc128`.
    assert!(board_command("test-placa --mm-tc --mm-tc-pf") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --atentie-placi") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --atentie-placi"))));
    assert!(board_command("test-placa --mm-tc-pf").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc128 --mm-tc-pf").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc-pf --mm-tc").is_some_and(|r| r.is_err()));
    // P6-210: `--mm-tc-v4` right after `--mm-tc` or `--mm-tc-pf`, not with `--mm-tc128`, not alone.
    assert!(board_command("test-placa --mm-tc --mm-tc-v4") == Some(Ok(String::from("c135 --mm-tc --mm-tc-v4"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v4") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --mm-tc-v4"))));
    assert!(board_command("test-placa --mm-tc-v4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc128 --mm-tc-v4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-v4 --mm-tc-pf").is_some_and(|r| r.is_err()));
    // P6-212: `mm_tc128_v4` last, right after `mm_tc`, `mm_tc_pf` or `mm_tc_v4`.
    assert!(board_command("test-placa --mm-tc --mm-tc128-v4") == Some(Ok(String::from("c135 --mm-tc --mm-tc128-v4"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc128-v4") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc128-v4"))));
    assert!(board_command("test-placa --mm-tc128-v4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc128 --mm-tc128-v4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc128-v4 --mm-tc-v4").is_some_and(|r| r.is_err()));
    // P6-242: `mm_tc_v5` only right after `mm_tc_v4`, before `mm_tc128_v4` / `--mm-tc-64k`.
    assert!(board_command("test-placa --mm-tc --mm-tc-v4 --mm-tc-v5 --mm-tc-64k") == Some(Ok(String::from("c135 --mm-tc --mm-tc-v4 --mm-tc-v5 --mm-tc-64k"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc-v5 --mm-tc128-v4 --mm-tc-64k") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc-v5 --mm-tc128-v4 --mm-tc-64k"))));
    assert!(board_command("test-placa --tablou-viteza --mm-tc --mm-tc-v4 --mm-tc-v5 --mm-tc-64k").is_some_and(|r| r.is_ok()));
    assert!(board_command("test-placa --mm-tc --mm-tc-v5").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v5").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-v5 --mm-tc-v4").is_some_and(|r| r.is_err()));
    // P6-214: `--mm-tc-64k` right after `mm_tc` or any of its companions.
    assert!(board_command("test-placa --mm-tc --mm-tc-64k") == Some(Ok(String::from("c135 --mm-tc --mm-tc-64k"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc-64k") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc-64k"))));
    assert!(board_command("test-placa --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc128-v4 --mm-tc-64k") == Some(Ok(String::from("c135 --mm-tc --mm-tc-pf --mm-tc-v4 --mm-tc128-v4 --mm-tc-64k"))));
    assert!(board_command("test-placa --mm-tc-64k").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-64k --mm-tc-pf").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-64k --atentie-placi") == Some(Ok(String::from("c135 --mm-tc --mm-tc-64k --atentie-placi"))));
    // P6-141: `--atentie-placi` last, alone or after any other option (`--mm-tc` included).
    assert!(board_command("test-placa --atentie-placi") == Some(Ok(String::from("c135 --atentie-placi"))));
    assert!(board_command("test-placa --mm-tc --atentie-placi") == Some(Ok(String::from("c135 --mm-tc --atentie-placi"))));
    assert!(board_command("test-placa --tablou --pas-aur --mm-tc --atentie-placi") == Some(Ok(String::from("c135 --tablou --pas-aur --mm-tc --atentie-placi"))));
    assert!(board_command("test-placa --atentie-placi --mm-tc").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie-placi").is_some_and(|r| r.is_err()));
    // P6-149: `--atentie2` only right after `--atentie-placi`.
    assert!(board_command("test-placa --atentie-placi --atentie2") == Some(Ok(String::from("c135 --atentie-placi --atentie2"))));
    assert!(board_command("test-placa --mm-tc --atentie-placi --atentie2") == Some(Ok(String::from("c135 --mm-tc --atentie-placi --atentie2"))));
    assert!(board_command("test-placa --atentie2").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --atentie2").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie2 --atentie-placi").is_some_and(|r| r.is_err()));
    // P6-160: `--atentie3` only right after `--atentie2`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3"))));
    assert!(board_command("test-placa --atentie-placi --atentie3").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie3").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie3 --atentie2").is_some_and(|r| r.is_err()));
    // P6-168: `--atentie4` only right after `--atentie3`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie4") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie4"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie4").is_some_and(|r| r.is_err()));
    // P6-227: `--atentie-tc` only right after `--atentie3` or `--atentie4`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie4 --atentie-tc") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie4 --atentie-tc"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie-tc").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-tc").is_some_and(|r| r.is_err()));
    // P6-233: v4 / v4pf only right after `--atentie-tc`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc16-v4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --ram-vram") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --ram-vram"))));
    // P6-234: dS on the tensor cores only right after `--atentie-tc` or `--atentie-tc16-v4`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc0").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0 --atentie-tc16-v4").is_some_and(|r| r.is_err()));
    // P6-235: v4 of the dS only right after `--atentie-tc0`.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0 --atentie-tc0-v4") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0 --atentie-tc0-v4"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0 --atentie-tc0-v4 --ram-vram") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0 --atentie-tc0-v4 --ram-vram"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc0-v4").is_some_and(|r| r.is_err()));
    // P6-238: the 64 KiB step only right after the tensor-core programs or their companions.
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc-64k") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc-64k"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc-64k") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc-64k"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0 --atentie-tc0-v4 --atentie-tc-64k --ram-vram") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc16-v4 --atentie-tc0 --atentie-tc0-v4 --atentie-tc-64k --ram-vram"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc-64k").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie-tc-64k --atentie-tc0").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-tc --mm-tc-64k --atentie-tc-64k").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --atentie4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie-tc --ram-vram") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie-tc --ram-vram"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --atentie4 --antrenare-mini --mixt") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --atentie4 --antrenare-mini --mixt"))));
    // P6-161: `--ram-vram` last, alone or after any other option.
    assert!(board_command("test-placa --ram-vram") == Some(Ok(String::from("c135 --ram-vram"))));
    assert!(board_command("test-placa --tablou --ram-vram") == Some(Ok(String::from("c135 --tablou --ram-vram"))));
    assert!(board_command("test-placa --atentie-placi --atentie2 --atentie3 --ram-vram") == Some(Ok(String::from("c135 --atentie-placi --atentie2 --atentie3 --ram-vram"))));
    assert!(board_command("test-placa --ram-vram --mm-tc").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ram-vram --ram-vram").is_some_and(|r| r.is_err()));
    // P6-163: `--adam-ram` only right after `--ram-vram`, last.
    assert!(board_command("test-placa --ram-vram --adam-ram") == Some(Ok(String::from("c135 --ram-vram --adam-ram"))));
    assert!(board_command("test-placa --tablou --ram-vram --adam-ram") == Some(Ok(String::from("c135 --tablou --ram-vram --adam-ram"))));
    assert!(board_command("test-placa --adam-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --pas-aur --adam-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --adam-ram --ram-vram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --ram-vram --adam-ram --adam-ram").is_some_and(|r| r.is_err()));
    // P6-164: `--antrenare-mini` last, alone or after anything.
    assert!(board_command("test-placa --antrenare-mini") == Some(Ok(String::from("c135 --antrenare-mini"))));
    assert!(board_command("test-placa --pas-aur --antrenare-mini") == Some(Ok(String::from("c135 --pas-aur --antrenare-mini"))));
    assert!(board_command("test-placa --ram-vram --adam-ram --antrenare-mini") == Some(Ok(String::from("c135 --ram-vram --adam-ram --antrenare-mini"))));
    assert!(board_command("test-placa --antrenare-mini --pas-aur").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --antrenare-mini").is_some_and(|r| r.is_err()));
    // P6-167: `--mixt` only right after `--antrenare-mini`.
    assert!(board_command("test-placa --antrenare-mini --mixt") == Some(Ok(String::from("c135 --antrenare-mini --mixt"))));
    assert!(board_command("test-placa --pas-aur --antrenare-mini --mixt") == Some(Ok(String::from("c135 --pas-aur --antrenare-mini --mixt"))));
    assert!(board_command("test-placa --mixt").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --pas-aur --mixt").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mixt --antrenare-mini").is_some_and(|r| r.is_err()));
    // P6-215: `--mixt-rapid` only right after `--mixt`; `--mixt-513-rapid` only right after `--mixt-513`.
    assert!(board_command("test-placa --antrenare-mini --mixt --mixt-rapid") == Some(Ok(String::from("c135 --antrenare-mini --mixt --mixt-rapid"))));
    assert!(board_command("test-placa --antrenare-mini --mixt-rapid").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt-rapid --mixt").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513-rapid").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt --mixt-513-rapid").is_some_and(|r| r.is_err()));
    // P6-216: `--lot-prin-ram` only right after `--mixt-513-rapid` (before `--lot-513 N`).
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --lot-513 6") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --lot-513 6"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --lot-prin-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --lot-prin-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt --mixt-rapid --lot-prin-ram").is_some_and(|r| r.is_err()));
    // P6-217: `--doua-locuri` only right after `--lot-prin-ram` (before `--lot-513 N`).
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri"))));
    // P6-231: `--atentie-tc16` only right after `--mixt-513-rapid`, `--lot-prin-ram` or `--doua-locuri`, last.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --atentie-tc16") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --atentie-tc16"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --atentie-tc16").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --lot-prin-ram").is_some_and(|r| r.is_err()));
    // P6-237: `--atentie-v4` only right after `--atentie-tc16`, last.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --atentie-tc16 --atentie-v4 --lot-513 6") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --atentie-tc16 --atentie-v4 --lot-513 6"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-v4").is_some_and(|r| r.is_err()));
    assert!(board_refusal_hint("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-v4 --atentie-tc16").is_some_and(|h| h.contains("`--atentie-v4` nu poate sta dupa `--mixt-513-rapid`")));
    // P6-241: `--atentie-ds` only right after `--atentie-tc16` or `--atentie-v4`, last.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --atentie-v4 --atentie-ds") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --atentie-v4 --atentie-ds"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --atentie-ds") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --atentie-ds"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-ds").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --atentie-ds --atentie-v4").is_some_and(|r| r.is_err()));
    // P6-236 (centre note 26 Nov 17:50): the refused line says which word is out of place (os-465's first line).
    let os465 = "test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --atentie-tc16 --lot-prin-ram --doua-locuri --lot-513 6";
    assert!(board_refusal_hint(os465).is_some_and(|h| h.contains("`--lot-prin-ram` e pus prea tarziu") && h.contains("inaintea lui `--atentie-tc16`")), "ciclu: indiciul os-465");
    assert!(board_refusal_hint("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --atentie-tc16 --lot-513 6").is_none());
    assert!(board_refusal_hint("gpu lant").is_none() && board_refusal_hint("test-placa --bariera").is_none());
    assert!(board_refusal_hint("test-placa --bariera --atentie-tc17").is_some_and(|h| h.contains("`--atentie-tc17` nu e o optiune cunoscuta")));
    assert!(board_refusal_hint("test-placa --bariera --bariera").is_some_and(|h| h.contains("de doua ori")));
    assert!(board_refusal_hint("test-placa --atentie2").is_some_and(|h| h.contains("`--atentie2` nu poate sta dupa `test-placa`")));
    assert!(board_refusal_hint("test-placa --i9 c999 --bariera").is_some_and(|h| h.contains("dupa `--i9`")));
    assert!(board_refusal_hint("test-placa --bariera --lot-513 6").is_some_and(|h| h.contains("cere `--antrenare-513`")));
    assert!(board_refusal_hint("test-placa --lot-513 6 --bariera").is_some_and(|h| h.contains("la sfarsitul randului")));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --lot-513 6") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --lot-prin-ram --doua-locuri --lot-513 6"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513 --mixt-513-rapid --doua-locuri").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --doua-locuri").is_some_and(|r| r.is_err()));
    // P6-169: `--vram` only right after `--antrenare-mini`, not with `--mixt`.
    assert!(board_command("test-placa --antrenare-mini --vram") == Some(Ok(String::from("c135 --antrenare-mini --vram"))));
    assert!(board_command("test-placa --pas-aur --antrenare-mini --vram") == Some(Ok(String::from("c135 --pas-aur --antrenare-mini --vram"))));
    assert!(board_command("test-placa --vram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt --vram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --mixt").is_some_and(|r| r.is_err()));
    // P6-175: `--pas-lipit` only right after `--vram`.
    assert!(board_command("test-placa --antrenare-mini --vram --pas-lipit") == Some(Ok(String::from("c135 --antrenare-mini --vram --pas-lipit"))));
    assert!(board_command("test-placa --antrenare-mini --pas-lipit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt --pas-lipit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --pas-lipit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --pas-lipit --vram").is_some_and(|r| r.is_err()));
    // P6-172: `--antrenare-513` only right after `--vram`.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513"))));
    assert!(board_command("test-placa --antrenare-mini --antrenare-513").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --pas-lipit --antrenare-513").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-513").is_some_and(|r| r.is_err()));
    // P6-185: `--placi3` only right after `--antrenare-513`.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3"))));
    assert!(board_command("test-placa --antrenare-mini --vram --placi3").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --lot-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --placi3").is_some_and(|r| r.is_err()));
    // P6-188: `--mm-placi` only right after `--placi3`.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --mm-placi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --mm-placi --placi3").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-placi").is_some_and(|r| r.is_err()));
    // P6-193: `--lipit-placi` only right after `--mm-placi`, last.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --lipit-placi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi --colsum-impartit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi --cursa-mm").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --lipit-placi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipit-placi").is_some_and(|r| r.is_err()));
    // P6-189: `--colsum-impartit` only right after `--mm-placi`.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --colsum-impartit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --colsum-impartit --mm-placi").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --colsum-impartit").is_some_and(|r| r.is_err()));
    // P6-196: `--mm-alegere` only right after `--mm-placi` or `--colsum-impartit`, last.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-alegere").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi --mm-alegere").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --cursa-mm").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm --mm-alegere").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --colsum-impartit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-alegere").is_some_and(|r| r.is_err()));
    // P6-198: `--mixt-513` only right after `--mm-placi` or `--colsum-impartit`, last.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mixt-513"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mixt-513") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mixt-513"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mixt-513").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mixt-513 --cursa-mm").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mixt-513 --mm-alegere").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --lipit-placi --mixt-513").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --mixt-513").is_some_and(|r| r.is_err()));
    // P6-201: `--ocupare` only right after `--cursa-mm`, not with `--proba-64k8`.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm --ocupare") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm --ocupare"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --ocupare").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm --proba-64k8 --ocupare").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm --ocupare --proba-64k8").is_some_and(|r| r.is_err()));
    // P6-205: `--treapta-pas` only right after `--mm-alegere`, last.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --treapta-pas") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --treapta-pas"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --treapta-pas").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --treapta-pas").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --treapta-pas --mm-alegere").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --treapta-pas --cursa-mm").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --treapta-pas").is_some_and(|r| r.is_err()));
    // P6-213: `--mm-model` only right after `--treapta-pas`, last (before `--lot-513 N`).
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas --mm-model") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas --mm-model"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --treapta-pas --mm-model --lot-513 4") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --treapta-pas --mm-model --lot-513 4"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --mm-model").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-model").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --mm-alegere --mm-model --treapta-pas").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --mm-model").is_some_and(|r| r.is_err()));
    // P6-208: `--lot-513 N` = the last two words, only with `--antrenare-513`; carried with its number and split back.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas --lot-513 6") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --mm-alegere --treapta-pas --lot-513 6"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-513 8") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --lot-513 8"))));
    assert!(board_command("test-placa --antrenare-mini --vram --lot-513 4").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-513 0").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-513 17").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-513").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-513 4 --placi3").is_some_and(|r| r.is_err()));
    assert!(split_batch_513("c135 --antrenare-mini --vram --antrenare-513 --lot-513 6") == ("c135 --antrenare-mini --vram --antrenare-513", Some(6)));
    assert!(split_batch_513("c135 --antrenare-mini --vram --antrenare-513") == ("c135 --antrenare-mini --vram --antrenare-513", None));
    assert!(split_batch_513("c135 --lot-513 x") == ("c135 --lot-513 x", None));
    // P6-195: `--proba-64k8` only right after `--cursa-mm`.
    assert!(board_command("test-placa --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm --proba-64k8") == Some(Ok(String::from("c135 --tablou-viteza --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm --proba-64k8"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --proba-64k8").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --proba-64k8 --cursa-mm").is_some_and(|r| r.is_err()));
    // P6-190: `--cursa-mm` only right after `--mm-placi` or `--colsum-impartit`.
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm") == Some(Ok(String::from("c135 --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --colsum-impartit --cursa-mm"))));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --cursa-mm").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --placi3 --mm-placi --cursa-mm --colsum-impartit").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --cursa-mm").is_some_and(|r| r.is_err()));
    // P6-178: `--lot-ram` only right after `--vram`, alone after it.
    assert!(board_command("test-placa --antrenare-mini --vram --lot-ram") == Some(Ok(String::from("c135 --antrenare-mini --vram --lot-ram"))));
    assert!(board_command("test-placa --antrenare-mini --lot-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --pas-lipit --lot-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --antrenare-mini --vram --antrenare-513 --lot-ram").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lot-ram").is_some_and(|r| r.is_err()));
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
    // P6-145: `--lot` after `--dieta2` (or after `--argmax-fara-copie`), before `--p8`; only with the argmax without the copy
    // and the default list.
    assert!(board_command("test-placa --argmax-fara-copie --dieta2 --lot") == Some(Ok(String::from("c135 --argmax-fara-copie --dieta2 --lot"))));
    assert!(board_command("test-placa --argmax-fara-copie --lot") == Some(Ok(String::from("c135 --argmax-fara-copie --lot"))));
    assert!(board_command("test-placa --tablou --argmax-fara-copie --dieta2 --lot --p8") == Some(Ok(String::from("c135 --tablou --argmax-fara-copie --dieta2 --lot --p8"))));
    assert!(board_command("test-placa --lot").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-placa --dieta2 --lot").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --lipita-ln --argmax-fara-copie --lot").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-fara-copie --lot --dieta2").is_some_and(|r| r.is_err()));
    assert!(board_command("test-placa --argmax-fara-copie --p8 --lot").is_some_and(|r| r.is_err()));
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
