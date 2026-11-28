//! P6-62 (command C-83): I14 of the window chain in the OS — whole tokens of a Kronos model on the work channel,
//! after `i13-strat` (Cyber-GPU's notes `C-83_i14.md`, `C-83_i14_vectori.md`).
//!
//! `i14-simbol` runs **Kronos's tiny control model first** (`mic_q8_0.kgr`, 840 KB, 2 layers, E 32, vocabulary 16 384,
//! RANDOM weights; hub `integrare/kronos/vectori/`, copied in `tests/kronos/`), as Cyber-GPU proposed: the answer is
//! known exactly (`vectori.json`, made by Kronos's own engine), so if it passes here and #513 does not later, the
//! problem is size (VRAM, pushbuffer, time), not computation. #513 (P6-60) runs through `kronos --gpu`, which picks it by default from the stick.
//!
//! The step: the KGR1 file read from the stick (`/usb/kronos/`, on the test disk `/disk/kronos/` in SIMULATION), its
//! SHA-256 checked against `vectori.json`; all weights + a NaN scratch + the logits packed into one blob, uploaded in
//! pieces of at most 8 MiB through the DMA arena of I11 (one `copy_b128` launch each); then for each of the three
//! control texts: every prompt position and 4 more greedy positions, each = `kronos_model::token_launches`
//! (`16·layers + 3` = 35 launches) + one copy of the logits down into the arena, ONE doorbell per token. Each token's
//! logits are judged against our CPU oracle (`gpu_compute::model::step_cpu`, same weights) with `kronos_model::judge`
//! (greedy token identical, every logit within 1e-3), the argmax of every prompt position and the 5 greedy tokens
//! against Kronos's vectors. The next token is the board's greedy one.
//!
//! No new register is written: only the VRAM window and the work-channel doorbell, as in I10–I13.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU with `gpu-sim`'s command processor.
//! The KGR1 reader here is ours, from the hub's specification (`integrare/kronos/format_greutati.md`).

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_compute::layer::Shape;
use gpu_compute::model::{self as cm, Caches};
use gpu_core::diag::{Event, Journal};
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::gsp::golden::Mapping;
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::kronos_layer::{self as kl, LayerVas, OwnedLayer, Prog, WeightVas, PROGRAMS};
use nvidia_ga104::kronos_batch as kb;
use nvidia_ga104::kronos_diag as kd;
use nvidia_ga104::kronos_fast as kf;
use nvidia_ga104::kronos_gen::{GenPlan, Generation, Stop, TokenCheck, TokenOutput, TokenRunner, TopOutput, TopRunner, VramBudget};
use nvidia_ga104::kronos_model::{self as km, ModelVas, OwnedModel};
use nvidia_ga104::kronos_perf as kp;
use nvidia_ga104::kronos_speed as ks;
use nvidia_ga104::mmu::SysMapping;
use nvidia_ga104::qmd::program_map_bytes;
use nvidia_ga104::push::PushBuf;
use nvidia_ga104::transfer;

use super::gpu_golden::GpuState;
use super::gpu_i10::{put_bytes, Launched, WAIT_US};
use super::gpu_i11::{arena_pages, f32_bytes, map_vram, ram_read, ram_read_at, ram_write, ram_write_at, run_records, submit_records, wait_records, wait_records_within, Bell, Sent, I11_BOARD_BYTES, PAGE};

/// Where I14 looks for the control model on the board's stick, and on the test disk in SIMULATION.
pub(super) const MIC_BOARD: &str = "/usb/kronos/mic_q8_0.kgr";
#[cfg(feature = "test-gpu-sim")]
pub(super) const MIC_SIM: &str = "/disk/kronos/mic_q8_0.kgr";
/// `vectori.json` → `fisiere.q8_0.sha256`.
const MIC_SHA256: &str = "b08f4c2976b95d9e1cc3058766b41e13d1ab1a8466b5eb06fa59ff665dd692a0";

/// One control text of `vectori.json` (part `q8_0`): its token ids, the argmax on every position, the 5 greedy tokens.
struct Control {
    text: &'static str,
    prompt: &'static [usize],
    argmax: &'static [usize],
    greedy: [usize; 5],
}

const MIC_CONTROLS: [Control; 3] = [
    Control {
        text: "Ana are mere.",
        prompt: &[65, 1988, 1585, 2717, 46],
        argmax: &[10337, 606, 3286, 3604, 4978],
        greedy: [4978, 3604, 4649, 13843, 3604],
    },
    Control {
        text: "Capitala Romaniei este",
        prompt: &[7132, 1373, 2710, 5734, 1744],
        argmax: &[3604, 3604, 3286, 3604, 3604],
        greedy: [3604, 3604, 3604, 13843, 3604],
    },
    Control {
        text: "def suma(a, b):",
        prompt: &[2082, 15746, 40, 97, 44, 1376, 1754],
        argmax: &[6961, 6961, 10450, 3604, 3604, 10793, 8117],
        greedy: [8117, 8117, 3604, 3604, 3604],
    },
];

/// VRAM kept free after a model's upload (C-87 step 4: the screen and the steps after I14).
const VRAM_RESERVE: u64 = 4 << 20;

/// P6-95 (C-141): the `kronos --gpu` result rows in the gpu journal (they go up to the hub with it); the speed rows are
/// cyber-gpu's `kv …` (`kronos_speed::rows`), plus the CPU engine's own two.
const KG_MODEL: &str = "kg kronos --gpu model";
const KG_ASKED: &str = "kg simboluri cerute";
const KG_MADE: &str = "kg simboluri generate pe placa (identice cu procesorul)";
const KG_FIRST_DIFF: &str = "kg primul simbol diferit de procesor (pozitia; ffffffff = niciunul)";
const KG_WORST: &str = "kg abaterea maxima a logit-urilor fata de procesor (x1e6)";
const KV_CPU_MED: &str = "kv motorul procesorului median pe pozitie (us, step_cpu)";
const KV_CPU_X100: &str = "kv simboluri pe secunda x100 (motorul procesorului)";
/// P6-99 (C-148 point 2): `kronos --gpu` checked AFTER the board by `/bin/kronos`'s fast engine (other names than the
/// two rows above, so a journal never mixes the two engines).
const KV_OS_MED: &str = "kv verificarea dupa placa median pe pozitie (us, motorul rapid /bin/kronos)";
const KV_OS_X100: &str = "kv pozitii pe secunda x100 (verificarea dupa placa)";
const KV_WITH_CHECK_X100: &str = "kv simboluri pe secunda x100 cu verificare (placa + driver + verificarea dupa placa)";

/// P6-99: how long the chain waits for `/bin/kronos`'s logits of one position before it gives up (a clear line, no
/// verdict = no token accepted). The fast engine needs ~14 ms per position for #513 on the workstation.
const OS_CHECK_WAIT_US: u64 = 60_000_000;

/// P6-97 (C-146): whether the board rewrote any word of the launch slots (read back after the text; 0 = the shadow held).
const KT_SHADOW: &str = "kt umbra lansarilor: cuvinte schimbate de placa in sloturi (0 = umbra corecta)";
/// P6-107 (C-154, cyber-gpu C-147 point 7): the driver's diet and what it saves (`kronos --gpu` only).
const KT_DIET: &str = "kt dieta (0/1)";
/// P6-163: the m, v in RAM step against today's tiled plan: 1 = the same bits (losses, norms, weights, m, v).
const KT_ADAM_RAM_SAME: &str = "kt adam-ram identice cu planul de azi (1/0)";
/// P6-163: the median board time of one step (us), today's tiled plan / m, v in RAM.
const KT_ADAM_RAM_US_TILES: &str = "kt adam-ram us pe pas planul de azi";
const KT_ADAM_RAM_US_RAM: &str = "kt adam-ram us pe pas m v in RAM";
/// P6-163: 1 = both plans as PyTorch and the same bits.
const KT_ADAM_RAM_PASS: &str = "kt adam-ram verdict (1/0)";
/// P6-164: `test-placa --antrenare-mini` (the whole training step of `antrenare mini` on the board, beside the oracle).
const KT_MINI_STEPS: &str = "kt antrenare-mini pasi terminati pe placa";
const KT_MINI_US: &str = "kt antrenare-mini us pe pas placa (median, din inregistrari)";
const KT_MINI_HOST_MS: &str = "kt antrenare-mini ms pe pas intreg (urcari + placa + citiri, fara oracol, median)";
const KT_MINI_TOK_S: &str = "kt antrenare-mini simboluri pe secunda (placa, median)";
const KT_MINI_LOSS_E9: &str = "kt antrenare-mini abaterea maxima a pierderii fata de oracol (x1e9)";
const KT_MINI_W_E9: &str = "kt antrenare-mini abaterea maxima a greutatilor fata de oracol (x1e9)";
const KT_MINI_PASS: &str = "kt antrenare-mini verdict (1/0)";
/// P6-167: 1 = the mixed list (`--mixt`, every matmul on `mm_tc`), 0 = the F32 list.
const KT_MINI_MIXED: &str = "kt antrenare-mini pasul mixt (1/0)";
/// P6-167: the mixed step: the largest relative gap of the gradient norm (divided by S) against the oracle's (x1e9).
const KT_MINI_NORM_E9: &str = "kt antrenare-mini abaterea maxima a normei fata de oracol (x1e9)";
/// P6-167: the mixed step: `mm_tc` launches in one step.
const KT_MINI_TC: &str = "kt antrenare-mini lansari mm_tc pe pas";
/// P6-169: the resident arena (in VRAM) was used (1/0).
const KT_MINI_VRAM: &str = "kt antrenare-mini arena in VRAM (1/0)";
/// P6-169: the board time of the one RAM -> VRAM copy of the resident arena (us).
const KT_MINI_UP_US: &str = "kt antrenare-mini urcarea arenei o data (us, placa)";
/// P6-169: bytes written to the board per step after the first (the batch) / read per step (losses + norm).
const KT_MINI_STEP_UP: &str = "kt antrenare-mini octeti sus pe pas";
const KT_MINI_STEP_DOWN: &str = "kt antrenare-mini octeti jos pe pas";
/// P6-169: the whole step in us (the resident step is a few ms; the ms row rounds it away).
const KT_MINI_HOST_US: &str = "kt antrenare-mini us pe pas intreg (median, fara oracol)";
/// P6-174 (cyber-gpu notes C-157_P2_os-386 + C-157_P2_taieturi): the stick writes of the live journal during the steps
/// (the routine rows are held in memory, as in the generation; expected 0), the journal's time in the steps, and the
/// record words read over all steps (only the first + the last on most steps).
const KT_MINI_STICK: &str = "kt antrenare-mini scrieri pe stick in pasi";
const KT_MINI_JOURNAL_US: &str = "kt antrenare-mini us de jurnal in pasi (total)";
const KT_MINI_REC_WORDS: &str = "kt antrenare-mini cuvinte de inregistrari citite (total)";
/// P6-175 (cyber-gpu `d3d1ddd` + `f170042`, P6-170 on the simulated board): the glued step on the real board — the
/// resident run with `ln_back_res` + `mm_train_gelu` (1/0 per run), and the steps whose loss, norm and (at the end)
/// weights came out bit for bit the same as the run without gluing.
const KT_MINI_FUSED: &str = "kt antrenare-mini pas lipit (1/0)";
const KT_MINI_FUSED_SAME: &str = "kt antrenare-mini lipit = nelipit bit cu bit (pasi)";
/// P6-178 (cyber-gpu `d173a59`, note C-157_P2s): the batch and the step's results through pinned RAM, moved by the copy
/// program inside the step's own list (1/0 per run), and the steps bit for bit the same as the run through the window.
const KT_MINI_RAM: &str = "kt antrenare-mini lot prin RAM (1/0)";
const KT_MINI_RAM_SAME: &str = "kt antrenare-mini lot prin RAM = prin fereastra bit cu bit (pasi)";
/// P6-179: how many fully read steps the per-launch profile of the step was made from.
const KT_MINI_PROFILE: &str = "kt antrenare-mini profilul pasului (pasi cititi intregi)";
/// P6-173 (command-center note of 24 Nov 03:12): where the resident step's host time goes, piece by piece (median us).
const KT_MINI_PARTS: [&str; 5] = [
    "kt antrenare-mini bucata planul lotului (rebatch, us median)",
    "kt antrenare-mini bucata lotul sus (us median)",
    "kt antrenare-mini bucata lista + zona de control + inregistrari (us median)",
    "kt antrenare-mini bucata trimitere + asteptare (us median)",
    "kt antrenare-mini bucata pierderile + norma jos (us median)",
];
/// P6-173: the pieces of one submission (gaps between `gpu_i10::KICK_MARKS`), journal rows and names.
const KT_MINI_KICKS: [&str; 6] = [
    "kt antrenare-mini trimitere: pushbuffer scris (us median)",
    "kt antrenare-mini trimitere: golirea scrierilor in VRAM (us median)",
    "kt antrenare-mini trimitere: TLB (us median)",
    "kt antrenare-mini trimitere: soneria (us median)",
    "kt antrenare-mini trimitere: asteptarea ultimei inregistrari (us median)",
    "kt antrenare-mini trimitere: inregistrarile citite (us median)",
];
const KICK_NAMES: [&str; 6] = ["pushbuffer", "golirea in VRAM", "TLB", "soneria", "asteptarea", "inregistrarile citite"];
/// P6-173: the names of the pieces, in the order of [`KT_MINI_PARTS`].
const MINI_PART_NAMES: [&str; 5] = ["planul lotului", "lotul sus", "lista + zona de control", "trimitere + asteptare", "pierderile + norma jos"];
/// P6-112 (C-157 point 2b): the note written (with every held row) right before the texts, outside position 0's clock.
const GEN_STARTS: &str = "kt generarea incepe (randurile tinute sunt pe stick; pana la final randurile de rutina raman in memorie)";
/// P6-112: routine rows dropped from the memory ring during the generation (0 = all of them reached the stick).
const KT_DROPPED: &str = "kt randuri de rutina aruncate";
/// P6-112 (center's note 13:00, point 1): P3's comparison also as journal rows (before only in the output).
const KT_FUSED_DIFFER: &str = "kt lipit fata de simplu: logit-uri diferite (0 = bit cu bit)";
const KT_FUSED_SAME_TOKENS: &str = "kt lipit fata de simplu: simboluri identice (1/0)";
/// P6-112: which list `kronos --gpu` sent (0 = simpla, 1 = lipita, 2 = amandoua, comparate).
/// P6-116 (C-157 point 2, cyber-gpu note `C-157_P2c_pozitia-0.md`): position 0 of every list sent once before the clock
/// (the warm-up), so the launch and pushbuffer shadows hold that list's slots before the timed position 0.
const KT_WARM: &str = "kt incalzirea: pozitia 0 trimisa o data inainte de ceas (numarul listelor)";
const KT_WARM_US: &str = "kt incalzirea: durata totala (us, in afara ceasului)";
const KT_WARM_WORDS: &str = "kt incalzirea: cuvinte VRAM scrise (prima lista)";
/// P6-120: 4 = the glued list with the new LayerNorm alone (the cycle's default from os-336 on).
pub const KT_LIST: &str = "kt lista (0 simpla, 1 lipita, 2 comparate, 3 ln nou comparat, 4 lipita-ln)";
// P6-128: `KT_LIST` = 6 for `lipita-ln` then `lipita-ln-32`, compared (the row's name stays: the center reads it).
// P6-135: `KT_LIST` = 7 for `lipita-ln-32` alone (the cycle's default from this Release on).
/// P6-124 (C-155 point 4): `KT_LIST` = 5 for the glued list then the glued list on quarters (`Fused32`), compared (the
/// row's name stays as it is: the command center reads it). Threshold: the same tokens and every logit within 1e-4; then
/// the kinds of launch: one passes on quarters only if it is faster by at least `QUARTERS_MIN_GAIN` per mille.
const KT_Q_WORST_BITS: &str = "kt sferturi fata de lipit: max |d| (f32, biti)";
const KT_Q_SAME_TOKENS: &str = "kt sferturi fata de lipit: simboluri identice (1/0)";
const KT_Q_PASS: &str = "kt sferturi fata de lipit: trece pragul 1e-4 si aceleasi simboluri (1/0)";
const KT_Q_KINDS_WON: &str = "kt sferturi: feluri mai rapide cu cel putin 50 la mie";
const KT_Q_GAIN: &str = "kt sferturi: castigul pe simbol (la mie, toate felurile)";
const QUARTERS_MAX_DIFF: f32 = 1e-4;
const QUARTERS_MIN_GAIN: i64 = 50;
/// P6-128 (C-157 point 2, cyber-gpu `feb0819`, note `C-157_P2g_sferturi.md`): which two lists a race on quarters compares
/// and the words of its rows. `Q_FUSED32` = P6-124's (`--sferturi`); `Q_LN32` = the chosen list `lipita-ln-32` (the new
/// LayerNorm and every product on quarters) against `lipita-ln`, the cycle's default (`--ln-32`, `KT_LIST` = 6).
#[derive(Clone, Copy)]
struct QuarterRace {
    base: kd::TokenList,
    new: kd::TokenList,
    name: &'static str,
    base_name: &'static str,
    row: &'static str,
}
const Q_FUSED32: QuarterRace =
    QuarterRace { base: kd::TokenList::Fused, new: kd::TokenList::Fused32, name: "lipita-32", base_name: "lista lipita", row: "kt sferturi fata de lipit" };
const Q_LN32: QuarterRace =
    QuarterRace { base: kd::TokenList::FusedLn, new: kd::TokenList::FusedLn32, name: "lipita-ln-32", base_name: "lipita-ln", row: "kt ln-32 fata de lipita-ln" };
/// P6-114 (C-155 point 2): the new LayerNorm against the old one on the glued list (threshold: same tokens, max |d| < 1e-4).
const KT_LN_WORST_BITS: &str = "kt ln nou fata de vechi: max |d| (f32, biti)";
const KT_LN_SAME_TOKENS: &str = "kt ln nou fata de vechi: simboluri identice (1/0)";
const KT_LN_PASS: &str = "kt ln nou fata de vechi: trece pragul 1e-4 si aceleasi simboluri (1/0)";
/// The note's threshold for the new LayerNorm against the old one (SIMULARE: 3.1e-7).
const LN_NEW_MAX_DIFF: f32 = 1e-4;
/// P6-123 (C-155 point 3, cyber-gpu note `C-155_argmax-pe-placa.md`): the argmax on the board, checked against the
/// copied logits (threshold 0: the 4 words equal `argmax_result`).
const KT_AM_ON: &str = "kt argmax placa (0/1)";
const KT_AM_AGREE: &str = "kt argmax placa = procesor: pozitii la care cele 4 cuvinte = argmax_result al logit-urilor copiate";
const KT_AM_TOTAL: &str = "kt argmax placa: pozitii verificate";
const KT_AM_MEDIAN_NS: &str = "kt median argmax placa (ns, lansarea, din inregistrari)";
/// P6-126 (C-155 point 4, cyber-gpu note C-157_P2f): the argmax without the copy: the token chosen on the board, every
/// position's token checked after the board against the processor's.
pub const KT_TOP_ON: &str = "kt argmax fara copie (0/1)";
const KT_TOP_AGREE: &str = "kt argmax fara copie: pozitii la care simbolul placii = simbolul procesorului";
const KT_TOP_TOTAL: &str = "kt argmax fara copie: pozitii verificate";
const KT_TOP_READ: &str = "kt median citirea rezultatului (us)";
const KT_VRAM_WORDS: &str = "kt cuvinte VRAM pe simbol (median)";
const KT_READ: &str = "kt median citirea logit-urilor (us)";
const KT_ARGMAX: &str = "kt median argmax (us)";
/// P6-145 (C-157 point 7): the batch's journal rows.
const KT_LOT_K: &str = "kt lot pozitii K";
const KT_LOT_BELLS: &str = "kt lot sonerii";
const KT_LOT_AGREE: &str = "kt lot pozitii identice";
const KT_LOT_TOTAL: &str = "kt lot pozitii judecate";
const KT_LOT_BOARD_NS: &str = "kt lot median placa pe pozitie (ns)";
const KT_LOT_RATE_X1000: &str = "kt lot simb/s x1000";
const KT_LOT_PASS: &str = "kt lot verdict (1 aceleasi simboluri)";
/// P6-107: the logits compared bit for bit between two runs (os-322 had neither row: its worst deviation was printed,
/// 1.6212463e-5 = 0x37880000, so that one is compared).
const KT_WORST_BITS: &str = "kt abaterea maxima a logit-urilor (f32, biti)";
const KT_PRINT: &str = "kt amprenta logit-urilor (FNV-1a, toate pozitiile)";
/// P6-97 (C-145): the notes the OS repeats with the routine writes of a generation (the permit's reason, the doorbell's
/// note) — exactly the texts of `gpu_tur.rs`.
const GEN_NOTES: [&str; 2] = ["C-55 gpu lant: fereastra cu Adrian (P6-42)", "soneria canalului de lucru (I10): scriere fara recitire"];

const UP_FIRST: u32 = 0xc000;
const TOKEN_FIRST: u32 = 0x1_0000;
/// Payload room per doorbell (a token has 36 launches; #513 will have 196).
const STRIDE: u32 = 0x100;

fn bad(why: &'static str) -> GpuError {
    GpuError::InvalidArgument(why)
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// SHA-256 of a whole file, as lowercase hex.
fn sha256_hex(b: &[u8]) -> String {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut h = Sha256::new();
    h.update(b);
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    hex(&out)
}

// ---------------------------------------------------------------------------------------------------------------
// The KGR1 file (hub `integrare/kronos/format_greutati.md`): header, key-values, tensor list, zeros to 32, data zone.
// ---------------------------------------------------------------------------------------------------------------

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> GpuResult<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(bad("KGR1: fisier trunchiat"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> GpuResult<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> GpuResult<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }
    fn text(&mut self) -> GpuResult<&'a [u8]> {
        let n = usize::try_from(self.u64()?).map_err(|_| bad("KGR1: text prea lung"))?;
        self.take(n)
    }
}

struct Tensor<'a> {
    name: &'a [u8],
    dims: Vec<usize>,
    kind: u32,
    off: usize,
}

/// P6-177 (C-157 point 8): the model card read from the start of a KGR1 file (`ModelCard::from_kgr1`) and the compute
/// pack chosen for it (`packs::select`). Returns the console rows and `Ok(true)` = the form was proved on the board,
/// `Ok(false)` = "PROBA NECESARA" (the CPU check runs first), `Err` = refused (a broken card or no pack): nothing may
/// run on the board.
pub(crate) fn card_choice(f: &[u8]) -> (Result<bool, &'static str>, Vec<String>) {
    let card = match gpu_compute::card::ModelCard::from_kgr1(f) {
        Ok(c) => c,
        Err(e) => {
            return (
                Err("fisa modelului nu se poate citi: nimic nu ruleaza pe placa"),
                alloc::vec![format!("REFUZ: fisa modelului: {}; nimic nu ruleaza pe placa", e.line())],
            )
        }
    };
    let (l, h, e, c, v) = card.form();
    let head = format!("fisa modelului: {} straturi, {} capete, n_emb {}, context {}, vocabular {}", l, h, e, c, v);
    match nvidia_ga104::packs::select(&card) {
        Ok(choice) => {
            let proved = matches!(choice.proof, nvidia_ga104::packs::Proof::Board(_));
            let mut rows = alloc::vec![head, choice.line()];
            // P6-180 (C-157 point 8 step 2): the work channel's limits from the card alone, before any tensor is read
            // (`run_model` checks the same numbers again from the tensors' shapes, with the lists that actually run).
            match choice.limits_with(I11_BOARD_BYTES, &[Prog::Argmax]) {
                Ok(lim) => rows.extend(lim.lines().into_iter().map(|x| format!("limitele din fisa (cu argmax pe placa): {}", x))),
                Err(e) => {
                    rows.push(format!("REFUZ: limitele din fisa: {}; nimic nu ruleaza pe placa", e));
                    return (Err("limitele fisei modelului nu se pot calcula: nimic nu ruleaza pe placa"), rows);
                }
            }
            (Ok(proved), rows)
        }
        Err(r) => {
            let mut out = alloc::vec![head];
            out.extend(r.lines());
            (Err("niciun pachet de calcul nu se potriveste fisei modelului: nimic nu ruleaza pe placa"), out)
        }
    }
}

/// P6-180 (C-157 point 8 step 2): the card read from the start of a KGR1 file must describe the tensors read from the
/// same file (layers, heads, n_emb, context, vocabulary): the pack was chosen on the card. `None` = they agree.
pub(crate) fn card_vs_tensors(f: &[u8], m: &OwnedModel) -> Option<String> {
    let card = match gpu_compute::card::ModelCard::from_kgr1(f) {
        Ok(c) => c,
        Err(e) => return Some(format!("REFUZ: fisa modelului: {}; nimic nu ruleaza pe placa", e.line())),
    };
    let t = (m.layers.len(), m.shape.heads, m.shape.e, m.context, m.vocab);
    let c = card.form();
    if c == t {
        return None;
    }
    Some(format!(
        "REFUZ: fisa modelului nu se potriveste cu tensorii (fisa: {} straturi, {} capete, n_emb {}, context {}, vocabular {}; tensorii: {} straturi, {} capete, n_emb {}, context {}, vocabular {}); nimic nu ruleaza pe placa",
        c.0, c.1, c.2, c.3, c.4, t.0, t.1, t.2, t.3, t.4
    ))
}

/// [`card_choice`] with the rows prefixed for the `kronos --gpu` report.
fn model_pack(p: &str, f: &[u8], lines: &mut Vec<String>) -> Result<bool, &'static str> {
    let (r, rows) = card_choice(f);
    for x in rows {
        lines.push(format!("{}: kronos --gpu: {}", p, x));
    }
    r
}

/// A KGR1 Q8_0 `kronos_gpt` file → the model Cyber-GPU's launches and CPU oracle take (Q8_0 blocks = f32 scale +
/// 32 × i8, the same 36 bytes in both).
pub(crate) fn read_kgr1(f: &[u8]) -> GpuResult<OwnedModel> {
    let mut c = Cursor { b: f, at: 0 };
    if c.take(4)? != b"KGR1" || c.u32()? != 1 {
        return Err(bad("KGR1: antet necunoscut"));
    }
    let (nkv, nt) = (c.u64()?, c.u64()?);
    if nkv > 4096 || nt > 4096 {
        return Err(bad("KGR1: prea multe chei sau tensori"));
    }
    let mut ints: Vec<(&[u8], i64)> = Vec::new();
    let (mut eps, mut arch_ok, mut q8_ok) = (None, false, false);
    for _ in 0..nkv {
        let k = c.text()?;
        match c.u32()? {
            1 => ints.push((k, c.u64()? as i64)),
            2 => {
                let v = f64::from_bits(c.u64()?);
                if k == b"ln_eps" {
                    eps = Some(v);
                }
            }
            3 => {
                let v = c.text()?;
                if k == b"arhitectura" {
                    arch_ok = v == b"kronos_gpt";
                }
                if k == b"fel" {
                    q8_ok = v == b"q8_0";
                }
            }
            _ => return Err(bad("KGR1: fel de valoare necunoscut")),
        }
    }
    if !arch_ok || !q8_ok {
        return Err(bad("KGR1: nu e kronos_gpt Q8_0"));
    }
    let int = |k: &[u8]| -> GpuResult<usize> {
        ints.iter()
            .find(|(n, _)| *n == k)
            .and_then(|(_, v)| usize::try_from(*v).ok())
            .ok_or(bad("KGR1: lipseste o cheie (n_straturi, n_capete, n_emb, context, vocabular, cap_legat)"))
    };
    let mut ts: Vec<Tensor<'_>> = Vec::new();
    for _ in 0..nt {
        let name = c.text()?;
        let nd = c.u32()? as usize;
        if nd == 0 || nd > 4 {
            return Err(bad("KGR1: numar de dimensiuni nesustinut"));
        }
        let mut dims = Vec::with_capacity(nd);
        for _ in 0..nd {
            dims.push(usize::try_from(c.u64()?).map_err(|_| bad("KGR1: dimensiune prea mare"))?);
        }
        let kind = c.u32()?;
        let off = usize::try_from(c.u64()?).map_err(|_| bad("KGR1: loc prea mare"))?;
        ts.push(Tensor { name, dims, kind, off });
    }
    let data = c.at.next_multiple_of(32);
    let bytes = |name: &str, kind: u32, want: &[usize]| -> GpuResult<&[u8]> {
        let t = ts.iter().find(|t| t.name == name.as_bytes()).ok_or(bad("KGR1: lipseste un tensor"))?;
        if t.kind != kind || t.dims != want {
            return Err(bad("KGR1: tensor cu fel sau forma neasteptata"));
        }
        let n = match kind {
            0 => 4 * t.dims.iter().product::<usize>(),
            _ => {
                if t.dims.len() != 2 || t.dims[1] % 32 != 0 {
                    return Err(bad("KGR1: Q8_0 cere 2 dimensiuni, coloane multiplu de 32"));
                }
                t.dims[0] * t.dims[1] / 32 * 36
            }
        };
        let start = data.checked_add(t.off).ok_or(bad("KGR1: loc gresit"))?;
        f.get(start..start + n).ok_or(bad("KGR1: tensor in afara fisierului"))
    };
    let f32s = |name: &str, n: usize| -> GpuResult<Vec<f32>> {
        Ok(bytes(name, 0, &[n])?.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
    };
    let q8 = |name: &str, r: usize, cols: usize| -> GpuResult<Vec<u8>> { Ok(bytes(name, 1, &[r, cols])?.to_vec()) };
    if int(b"cap_legat")? != 1 {
        return Err(bad("KGR1: capul trebuie sa fie legat de emb_token"));
    }
    let (layers, heads, e, context, vocab) =
        (int(b"n_straturi")?, int(b"n_capete")?, int(b"n_emb")?, int(b"context")?, int(b"vocabular")?);
    if layers == 0 || layers > 64 || heads == 0 || e == 0 || e % heads != 0 || e % 32 != 0 {
        return Err(bad("KGR1: forma modelului nesustinuta"));
    }
    let shape = Shape { e, heads, eps: eps.ok_or(bad("KGR1: lipseste ln_eps"))? as f32 };
    let mut owned = Vec::with_capacity(layers);
    for i in 0..layers {
        let p = |s: &str| format!("straturi.{}.{}", i, s);
        owned.push(OwnedLayer {
            ln1_g: f32s(&p("ln1.weight"), e)?,
            ln1_b: f32s(&p("ln1.bias"), e)?,
            wqkv: q8(&p("atentie.in_proj_weight"), 3 * e, e)?,
            bqkv: f32s(&p("atentie.in_proj_bias"), 3 * e)?,
            wout: q8(&p("atentie.out_proj.weight"), e, e)?,
            bout: f32s(&p("atentie.out_proj.bias"), e)?,
            ln2_g: f32s(&p("ln2.weight"), e)?,
            ln2_b: f32s(&p("ln2.bias"), e)?,
            wfc: q8(&p("mlp.0.weight"), 4 * e, e)?,
            bfc: f32s(&p("mlp.0.bias"), 4 * e)?,
            wproj: q8(&p("mlp.2.weight"), e, 4 * e)?,
            bproj: f32s(&p("mlp.2.bias"), e)?,
        });
    }
    let m = OwnedModel {
        shape,
        vocab,
        context,
        emb_tok: q8("emb_token.weight", vocab, e)?,
        emb_pos: q8("emb_pozitie.weight", context, e)?,
        layers: owned,
        ln_g: f32s("ln.weight", e)?,
        ln_b: f32s("ln.bias", e)?,
    };
    m.weights().check()?;
    Ok(m)
}

// ---------------------------------------------------------------------------------------------------------------
// The step.
// ---------------------------------------------------------------------------------------------------------------

/// The blob in VRAM: offsets of every part (all 16-aligned).
struct Blob {
    bytes: Vec<u8>,
    emb_tok: u64,
    emb_pos: u64,
    ln_g: u64,
    ln_b: u64,
    layers: Vec<[u64; 12]>,
    scratch: u64,
    fin: u64,
}

fn pack(m: &OwnedModel) -> Blob {
    let mut b: Vec<u8> = Vec::new();
    let put = |b: &mut Vec<u8>, part: &[u8]| {
        let o = b.len() as u64;
        b.extend_from_slice(part);
        b.resize(b.len().next_multiple_of(16), 0);
        o
    };
    let emb_tok = put(&mut b, &m.emb_tok);
    let emb_pos = put(&mut b, &m.emb_pos);
    let ln_g = put(&mut b, &f32_bytes(&m.ln_g));
    let ln_b = put(&mut b, &f32_bytes(&m.ln_b));
    let layers = m
        .layers
        .iter()
        .map(|l| {
            [
                put(&mut b, &f32_bytes(&l.ln1_g)),
                put(&mut b, &f32_bytes(&l.ln1_b)),
                put(&mut b, &l.wqkv),
                put(&mut b, &f32_bytes(&l.bqkv)),
                put(&mut b, &l.wout),
                put(&mut b, &f32_bytes(&l.bout)),
                put(&mut b, &f32_bytes(&l.ln2_g)),
                put(&mut b, &f32_bytes(&l.ln2_b)),
                put(&mut b, &l.wfc),
                put(&mut b, &f32_bytes(&l.bfc)),
                put(&mut b, &l.wproj),
                put(&mut b, &f32_bytes(&l.bproj)),
            ]
        })
        .collect();
    // The scratch and the final LayerNorm + logits start as NaN: anything not computed cannot pass.
    let scratch = b.len() as u64;
    b.resize((scratch + kl::scratch_bytes(m.shape.e)) as usize, 0xff);
    let fin = b.len() as u64;
    b.resize((fin + 4 * (m.shape.e + m.vocab) as u64).next_multiple_of(16) as usize, 0xff);
    Blob { bytes: b, emb_tok, emb_pos, ln_g, ln_b, layers, scratch, fin }
}

/// P6-62 step 3: a `kronos --gpu` request. `/bin/kronos` gives the model file, the prompt's token ids and how many
/// tokens to generate; the chain's `i14-simbol` step runs it on the board right after the control model passed (the
/// board is set up only inside the chain, behind Adrian's `DA`). Every token is judged against the CPU oracle like the
/// control texts; any error ends the request and `/bin/kronos` continues on the processor.
pub struct KronosJob {
    pub path: String,
    pub prompt: Vec<usize>,
    pub n: usize,
    /// P6-99 (C-148 point 2): every position is judged after the board by `/bin/kronos`'s fast engine (through
    /// [`OS_CHECK`]) instead of the kernel's reference engine between tokens.
    pub os_check: bool,
}

/// P6-99 (C-148 point 2): the meeting point of the chain's check (`OsCheck`, on the chain thread) and `/bin/kronos`
/// (on the thread of its `gpu_kronos` call): the chain asks for the CPU logits of one (token, position), the program
/// gets the question as the answer of its call, computes them with its fast engine and gives them back
/// (`gpu_kronos_logits`). Only the thread that made the request may answer.
pub struct OsCheckLink {
    /// The thread of the `gpu_kronos` call (0 = none).
    pub owner: u64,
    /// The (token, position) the chain waits for.
    pub want: Option<(usize, usize)>,
    /// `want` was already given to the program.
    pub handed: bool,
    /// The program's answer: the logits, or why it could not compute them.
    pub answer: Option<Result<Vec<f32>, String>>,
}

pub static OS_CHECK: crate::sched::Mutex<OsCheckLink> =
    crate::sched::Mutex::new(OsCheckLink { owner: 0, want: None, handed: false, answer: None });

/// P6-99: waits until `ready()` gives a value or the OS clock passes `deadline` (us since boot): yields first (an answer
/// usually comes within milliseconds), then sleeps one tick at a time.
pub fn wait_for<T>(deadline: u64, mut ready: impl FnMut() -> Option<T>) -> Option<T> {
    let t0 = crate::time::since_boot_us().unwrap_or(0);
    loop {
        if let Some(v) = ready() {
            return Some(v);
        }
        let now = crate::time::since_boot_us().unwrap_or(0);
        if now >= deadline || crate::sched::killed() {
            return None;
        }
        if now.saturating_sub(t0) < 200_000 {
            crate::sched::yield_now();
        } else {
            crate::sched::sleep_ms(1);
        }
    }
}

/// The request waiting for the next chain, and its answer (the greedy tokens of the board, or why not).
pub static KRONOS_JOB: crate::sched::Mutex<Option<KronosJob>> = crate::sched::Mutex::new(None);
pub static KRONOS_RESULT: crate::sched::Mutex<Option<Result<Vec<usize>, String>>> = crate::sched::Mutex::new(None);

/// One text run on the board: its prompt, how many greedy tokens, and (control texts only) Kronos's expected answer.
#[derive(Clone)]
struct Run<'a> {
    name: String,
    prompt: Vec<usize>,
    steps: usize,
    argmax: Option<&'a [usize]>,
    greedy: Option<&'a [usize]>,
    /// P6-99: checked after the board by `/bin/kronos` (`kronos --gpu` only); otherwise the reference engine between tokens.
    os_check: bool,
    /// P6-110 (C-155 point 1): the launch list of every token (`Plain` everywhere except P3's second run in the cycle).
    list: kd::TokenList,
}

/// What a run left behind: the greedy tokens of every text, the worst logit deviation, the median token time, and
/// whether the logits had to be read from VRAM because only the copy into RAM was wrong (P6-94, C-139).
struct RunOut {
    all: Vec<Vec<usize>>,
    worst: f32,
    median: u64,
    /// P6-120 (cyber-gpu note C-157_P2d, point 2): the median token time of every text, in the order they ran.
    run_medians: Vec<u64>,
    from_vram: bool,
    /// P6-95 (C-141/C-142): the upload and every position (OS clock without the CPU check), and the CPU engine's time
    /// per position (`step_cpu`, the same weights) for the comparison.
    speed: ks::Speed,
    cpu_us: Vec<u64>,
    /// P6-99: `cpu_us` are the times of the check after the board by `/bin/kronos` (not the kernel's `step_cpu`).
    os_check: bool,
    /// P6-97 (C-146): every position of the last text, split (board / journal / screen / the rest = driver / CPU check).
    split: Vec<kf::TokenSplit>,
    /// P6-97: words of the launch slots the board changed (read back after the texts); `None` = no shadow (control model).
    shadow_changed: Option<usize>,
    /// P6-98 (C-147 stage 0 (a), `kronos --gpu` only): the launch times split by kind, and why a token was left out.
    prof: kp::KindProfile,
    prof_err: Option<String>,
    /// P6-124: with two lists compared, the first run's kinds of launch (the race on quarters compares them).
    prof_first: Option<kp::KindProfile>,
    /// P6-98 (C-147 stage 0 (d), `kronos --gpu` only): what the VRAM → VRAM copy needs after the texts.
    copy: Option<CopyCtx>,
    /// P6-107 (C-154): the diet's rows (`kronos --gpu` only).
    diet: Option<DietOut>,
}

/// P6-107 (C-154): per position of the last text: the VRAM words the driver wrote, the logits read and the argmax
/// (us, OS clock); whether the diet was on; the fingerprint of every logit of the text.
struct DietOut {
    /// P6-119: the second diet's per-position figures (only when it was on).
    diet2: Option<nvidia_ga104::kronos_diet2::Diet2Stats>,
    on: bool,
    words: Vec<u64>,
    read_us: Vec<u64>,
    argmax_us: Vec<u64>,
    print: u32,
    /// P6-126: the argmax without the copy: no logits read (no print, no deviation).
    top: bool,
}

/// P6-107: FNV-1a (32 bits) over the logits' bits, in order (the fingerprint compared between two runs).
fn fnv1a(mut h: u32, logits: &[f32]) -> u32 {
    for v in logits {
        for b in v.to_bits().to_le_bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}
const FNV_START: u32 = 0x811c_9dc5;

/// The median of `v` (`None` when empty).
fn median_of(v: &[u64]) -> Option<u64> {
    let mut s = v.to_vec();
    s.sort_unstable();
    s.get(s.len() / 2).copied()
}

/// P6-98 (C-147 stage 0 (d)): the pieces of a `kronos --gpu` run the VRAM → VRAM copy reuses (the weights as source,
/// the first launch slot, the records, the `copy_b128` program), kept until the `kg`/`kv`/`kt`/`kp` rows are on the stick.
struct CopyCtx {
    src_va: u64,
    /// The host's copy of the bytes at `src_va` (the weights part of the uploaded blob — the board never writes it), for
    /// the three words read back.
    src: Vec<u8>,
    launches: Mapping,
    rec: Mapping,
    maps: Vec<Mapping>,
    sys: SysMapping,
    copy_va: u64,
}

/// The board's pieces a run needs (P6-62 step 3: shared by the control model and a `kronos --gpu` request).
///
/// P6-94 (C-139, cyber-gpu `kronos_diag`): when `way` is `None` (the control model), the ladder runs first on the first
/// token of the first text (the os-305 way as a control, then the repair, then the fallbacks), writes its `i14d` rows
/// in the journal and picks how the texts are sent; `kronos --gpu` reuses that way. A position that does not pass is
/// explained in the journal (`kd::explain`, `kd::locate`).
#[allow(clippy::too_many_arguments)]
fn run_model<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    m: &OwnedModel,
    runs: &[Run<'_>],
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    tokens_done: &mut usize,
    subs: &mut u32,
    way: &mut Option<kd::Way>,
) -> GpuResult<RunOut> {
    let s = m.shape;
    let e = s.e;
    let n_tok = km::launches_per_token(m.layers.len());
    // P6-123 (C-155 point 3): with the argmax on the board (only for `kronos --gpu`), one launch more after the copy.
    let argmax_board = runs.iter().all(|r| r.argmax.is_none()) && super::gpu_tur::ARGMAX_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-126 (C-155 point 4): the argmax WITHOUT the copy (only for `kronos --gpu`, not with the trial): the argmax
    // launch takes the copy's place, the token is chosen on the board.
    let argmax_top = !argmax_board
        && runs.iter().all(|r| r.argmax.is_none())
        && super::gpu_tur::ARGMAX_TOP_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let am_prog = argmax_board || argmax_top;
    let n = n_tok + 1 + usize::from(argmax_board); // + the copy of the logits down or the argmax (+ the argmax)
    // 0. C-87 step 4 + P6-95 (C-142 `kronos_speed::ModelLimits`): the VRAM, the pushbuffer area (two tokens in the way that
    //    needs the most words) and the records' stride, from the model's shapes, checked before anything is uploaded.
    let budget = VramBudget::of_model(m)?;
    // P6-122 (C-158 point 3, cyber-gpu note C-157_P4 `839704f`): the pushbuffer area and the records' stride from the
    // lists that actually run (the ladder, when it runs, sends the plain list: then it is counted too). With the plain
    // list alone, #651 (24 layers, 388 launches) would be refused even when only the glued list runs (172 launches).
    let mut run_lists: Vec<kd::TokenList> = Vec::new();
    if way.is_none() {
        run_lists.push(kd::TokenList::Plain);
    }
    for r in runs {
        if !run_lists.contains(&r.list) {
            run_lists.push(r.list);
        }
    }
    // P6-145 (C-157 point 7): the batch only after the texts with the argmax without the copy, only with `lipita-ln-32`
    // alone (`kronos_batch::TokenList::FusedLn32`); otherwise a row says why and the texts run as before.
    let batch_asked = argmax_top && super::gpu_tur::BATCH_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let batch = batch_asked && runs.iter().all(|r| r.list == kd::TokenList::FusedLn32);
    if batch_asked && !batch {
        lines.push(format!("{}: kronos --gpu: lot: sarit (doar cu lista lipita-ln-32 singura)", p));
    }
    let extra_progs: &[Prog] = match (am_prog, batch) {
        (true, true) => &[Prog::Argmax, Prog::EmbedDev],
        (true, false) => &[Prog::Argmax],
        _ => &[],
    };
    let lim = ks::ModelLimits::of_lists(&s, m.layers.len(), m.vocab, m.context, I11_BOARD_BYTES, &run_lists, extra_progs)?;
    let (free, largest) = st.mem.vram_free();
    lines.push(format!("{}: i14: {}; VRAM liber {} MiB (cea mai lunga bucata {} MiB)", p, budget.line(m.layers.len()), free >> 20, largest >> 20));
    let vram_line = budget.line(m.layers.len());
    for x in lim.lines().into_iter().filter(|x| *x != vram_line) {
        let have = if x.contains("zona de comenzi: cel putin") { format!(" (are {} KiB)", l.push.size >> 10) } else { String::new() };
        lines.push(format!("{}: i14: {}{}", p, x, have));
    }
    let refused = lim.check(l.push.size, STRIDE, free, largest, VRAM_RESERVE);
    if !refused.is_empty() {
        for r in &refused {
            lines.push(format!("{}: I14: {}", p, r.line()));
        }
        lines.push(format!("{}: I14: modelul nu incape (motivul e mai sus; pastrez {} MiB rezerva de VRAM): nimic urcat", p, VRAM_RESERVE >> 20));
        return Err(bad("I14: modelul nu incape (zona de comenzi, pasul inregistrarilor sau VRAM-ul): nimic urcat"));
    }
    // 1. The blob, uploaded through the arena in pieces of at most 8 MiB.
    let blob = pack(m);
    let total = blob.bytes.len() as u64;
    let vblob = map_vram(st, total)?;
    let piece = total.min(I11_BOARD_BYTES);
    let host = st.mem.alloc_sys_mapped(&arena_pages(0, piece.div_ceil(PAGE))?)?;
    let logits_bytes = (4 * m.vocab as u64).next_multiple_of(16);
    // P6-123: the argmax's 16-byte result right after the logits (`logits_bytes` is a multiple of 16), same arena.
    // P6-145: with the batch, one 16-byte slot per position of the context (#513: 8 KiB).
    let am_bytes = if batch {
        kb::SLOT * m.context as u64
    } else if am_prog {
        nvidia_ga104::argmax::RESULT_BYTES
    } else {
        0
    };
    let host_logits = st.mem.alloc_sys_mapped(&arena_pages(I11_BOARD_BYTES / PAGE, (logits_bytes + am_bytes).div_ceil(PAGE))?)?;
    let mut progs: Vec<(Prog, Mapping)> = Vec::new();
    // P6-110: P3's glued list needs its own program (`q8_fused`) too; uploaded only when a run uses that list.
    // P6-112: the glued list may now run alone (the cycle's default); compared only when both lists run.
    // P6-114 (C-155 point 2): likewise `layernorm256` only when a run uses the new LayerNorm (`FusedLn`).
    let mut lists: Vec<kd::TokenList> = Vec::new();
    for r in runs {
        if !lists.contains(&r.list) {
            lists.push(r.list);
        }
    }
    let compare = lists.len() >= 2;
    // P6-126: two runs compared need the logits of both; without the copy there are none.
    if argmax_top && compare {
        lines.push(format!("{}: I14: argmax fara copie nu merge cu doua rulari comparate (logit-urile nu se citesc): nimic urcat", p));
        return Err(bad("I14: argmax fara copie cu doua rulari comparate"));
    }
    let mut all_progs: Vec<Prog> = PROGRAMS.to_vec();
    if am_prog {
        all_progs.push(Prog::Argmax);
    }
    if batch {
        all_progs.push(Prog::EmbedDev);
    }
    for list in &lists {
        for q in list.programs() {
            if !all_progs.contains(q) {
                all_progs.push(*q);
            }
        }
    }
    for prog in all_progs {
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    let prog_va = |want: Prog| progs.iter().find(|(q, _)| *q == want).map(|(_, pm)| pm.va);
    let copy_va = prog_va(Prog::Copy).ok_or(bad("I14: lipseste programul de copiere"))?;
    let mut caches = Vec::with_capacity(m.layers.len());
    for _ in 0..m.layers.len() {
        caches.push((map_vram(st, kl::cache_bytes(e))?, map_vram(st, kl::cache_bytes(e))?));
    }
    // P6-145: the batch's positions per doorbell: as many as the pushbuffer area holds with `EachLaunch` (#513: 88
    // launches per position, 48 bytes each); the launch slots and the records grow with them.
    let lpp = kb::launches_per_position(m.layers.len());
    let batch_max = if batch { (1..=m.context).take_while(|&q| 4 * kd::words(kd::SeqMode::EachLaunch, q * lpp) as u64 <= l.push.size).last().unwrap_or(0) } else { 0 };
    let slots_n = n.max(batch_max * lpp);
    let launches = map_vram(st, 512 * slots_n as u64)?;
    let (cb_up, q_up, rec_up) = (map_vram(st, PAGE)?, map_vram(st, PAGE)?, map_vram(st, PAGE)?);
    let rec = map_vram(st, (16 * (slots_n as u64 + 1)).next_multiple_of(PAGE))?;
    let mut maps: Vec<Mapping> = alloc::vec![vblob, launches, cb_up, q_up, rec_up];
    maps.extend(caches.iter().flat_map(|(k, v)| [*k, *v]));
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    let mut up_ns = 0u64;
    let mut k = 0u32;
    let source = if permit.is_some() { ks::Source::Placa } else { ks::Source::Simulare };
    let mut speed = ks::Speed::new(source);
    // P6-186 (C-157 p.2 after the x16 test): the host's time per piece in three parts — the copy into the RAM arena,
    // the copy's commands, the submission + wait — to see where ~19 ms per 8 MiB piece go. Clock reads only.
    let mut parts = ks::UploadHostParts::new();
    for (i, chunk) in blob.bytes.chunks(piece as usize).enumerate() {
        let host_t0 = crate::time::since_boot_us();
        let len = (chunk.len() as u64).next_multiple_of(16);
        ram_write(&host, chunk);
        let host_t1 = crate::time::since_boot_us();
        let w = st.mem.writer_mut();
        put_bytes(w, cb_up.pa, &transfer::copy_cbuf(host.va, vblob.va + i as u64 * piece, len)?)?;
        put_bytes(w, q_up.pa, &transfer::copy_qmd(copy_va, cb_up.va, len)?.to_bytes()?)?;
        put_bytes(w, rec_up.pa, &[0; 32])?;
        let first = UP_FIRST + STRIDE * k;
        k += 1;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[q_up.va], rec_up.va, first)?;
        let nw = pb.len();
        let sys = [host.clone()];
        let host_t2 = crate::time::since_boot_us();
        match run_records(vbar, clock, journal, st, l, permit, &words[..nw], rec_up, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => {
                let host_t3 = crate::time::since_boot_us();
                let ns = transfer::analyze_sequence(&b, 1, first)?[0];
                up_ns += ns;
                let host = host_t0.zip(host_t3).map(|(a, b)| b.saturating_sub(a));
                speed.add_piece(chunk.len() as u64, ns, host);
                if let (Some(a), Some(b1), Some(c), Some(d)) = (host_t0, host_t1, host_t2, host_t3) {
                    // A clock going back only skips this piece; the measurement never stops the upload.
                    let _ = parts.add_piece(chunk.len() as u64, a, b1, c, d);
                }
            }
            Err(last) => {
                lines.push(format!("{}: I14: urcarea bucatii {} nu s-a terminat (inregistrarea 1 = {:#x})", p, i, last));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        }
    }
    lines.push(format!(
        "{}: i14: {} octeti (greutati + spatiu de lucru + logit-uri) urcati in {} bucati in {} ns ({} MB/s)",
        p, total, k, up_ns, transfer::CopySpeed::mb_per_s(total, up_ns)
    ));
    lines.push(format!("{}: kronos --gpu: {}", p, parts.line(source)));
    parts.journal(journal);
    // 2. The VAs. The copy of the logits down is now the last launch of a token's list (`kl::copy_launch`, C-139).
    let at = |o: u64| vblob.va + o;
    let mut lvas = Vec::with_capacity(m.layers.len());
    for (o, (kc, vc)) in blob.layers.iter().zip(&caches) {
        let wv = WeightVas {
            ln1_g: at(o[0]),
            ln1_b: at(o[1]),
            wqkv: at(o[2]),
            bqkv: at(o[3]),
            wout: at(o[4]),
            bout: at(o[5]),
            ln2_g: at(o[6]),
            ln2_b: at(o[7]),
            wfc: at(o[8]),
            bfc: at(o[9]),
            wproj: at(o[10]),
            bproj: at(o[11]),
        };
        lvas.push(LayerVas::with_scratch(e, at(blob.scratch), kc.va, vc.va, wv)?);
    }
    let vas = ModelVas {
        emb_tok: at(blob.emb_tok),
        emb_pos: at(blob.emb_pos),
        ln_g: at(blob.ln_g),
        ln_b: at(blob.ln_b),
        layers: lvas,
        h_final: at(blob.fin),
        logits: at(blob.fin) + 4 * e as u64,
    };
    let mut tmaps = maps.clone();
    tmaps.push(rec);
    let weights = m.weights();
    let mut worst = 0.0f32;
    let mut medians = Vec::new();
    let mut cpu_us: Vec<u64> = Vec::new();
    let mut all = Vec::with_capacity(runs.len());
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let fast = runs.iter().all(|r| r.argmax.is_none());
    let mut split_all: Vec<kf::TokenSplit> = Vec::new();
    // P6-107 (C-154): the diet only in the cycle's ordinary test (set by `ciclu::board_test`), only for `kronos --gpu`.
    let diet = fast && super::gpu_tur::DIET_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let push_shadow = if diet { Some(kf::WriteShadow::new(l.push.pa, l.push.size)?) } else { None };
    // P6-119 (C-157 point 3): the second diet only when the cycle's list asks for it, only for `kronos --gpu`.
    // P6-126 kept it off with the argmax without the copy; P6-138 (cyber-gpu note C-157_P2j, point 2): only parts C
    // (print after the clock) and D (argmax on bits) touch the logits and live in `run_token`; parts A (two records) and
    // B (the planned wait) are in `submit` and work in `run_top` too.
    let diet2 = fast && super::gpu_tur::DIET2_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let mut runner = BoardRunner {
        p,
        vbar: &mut *vbar,
        clock: &mut *clock,
        journal: &mut *journal,
        st: &mut *st,
        l: &mut *l,
        permit,
        lines: &mut *lines,
        doorbell: &mut *doorbell,
        tokens_done: &mut *tokens_done,
        subs: &mut *subs,
        m,
        vas: &vas,
        launches,
        rec,
        host_logits: host_logits.clone(),
        logits_bytes,
        progs: &progs,
        tmaps: &tmaps,
        n: slots_n,
        name: String::new(),
        way: way.unwrap_or(kd::Way::REPAIR),
        from_vram: false,
        pos: 0,
        timing: None,
        last_logits: Vec::new(),
        kept: None,
        argmaxes: Vec::new(),
        speed,
        // P6-97 (C-145/C-146): only for `kronos --gpu` (no control answers): the control model and the ladder keep
        // writing every slot whole, as in os-306/os-307.
        shadow: if fast { Some(kf::WriteShadow::new(launches.pa, launches.size)?) } else { None },
        shadow_ranges: alloc::collections::BTreeMap::new(),
        fast,
        split: Vec::new(),
        launch_ns: Vec::new(),
        prof: kp::KindProfile::new(),
        prof_err: None,
        diet,
        push_shadow,
        list: kd::TokenList::Plain,
        rec_zeroed: false,
        diet_words: Vec::new(),
        read_us: Vec::new(),
        argmax_us: Vec::new(),
        print: FNV_START,
        diet2,
        wait_plan: nvidia_ga104::kronos_diet2::WaitPlan::new(),
        d2_words: Vec::new(),
        d2_polls: Vec::new(),
        argmax_board,
        am_agree: 0,
        am_total: 0,
        am_first_bad: None,
        am_ns: Vec::new(),
        argmax_top,
        top_agree: 0,
        top_total: 0,
        top_first_bad: None,
        top_words: Vec::new(),
        keep_records: false,
        records: Vec::new(),
        write_us: 0,
        t32: None,
    };
    // 3. C-139: the ladder, once, on the first token of the first control text, before the texts.
    if way.is_none() {
        let first = runs.first().ok_or(bad("I14: niciun text"))?;
        let tok0 = *first.prompt.first().ok_or(bad("I14: text fara simboluri"))?;
        let cpu0 = cm::step_cpu(&weights, &mut Caches::new(m.layers.len(), e), tok0, 0)?;
        let prev = Caches::new(m.layers.len(), e);
        let ctx = kd::ProbeCtx {
            w: &weights,
            vas: &vas,
            prev: &prev,
            token: tok0,
            pos: 0,
            cpu_logits: &cpu0,
            want: first.argmax.and_then(|a| a.first().copied()),
        };
        runner.name = String::from("scara");
        let ladder = kd::ladder(&mut runner, &ctx); // writes its own rows in the journal
        runner.lines.extend(ladder.lines().into_iter().map(|x| format!("{}: {}", p, x)));
        let Some(chosen) = ladder.chosen else {
            return Err(bad("I14: niciun fel de a trimite simbolul nu trece (diagnosticul e in jurnal)"));
        };
        *way = Some(chosen);
        runner.way = chosen;
    }
    // 4. The texts: cyber-gpu's continuous generation (C-87 step 1, `kronos_gen::Generation`), every position judged
    //    against the CPU engine first; stop at the first error, explained in the journal (C-139).
    // P6-110: the logits of every position, per list (only when P3 is compared).
    let mut by_list: Vec<(kd::TokenList, Vec<Vec<f32>>)> = Vec::new();
    // P6-112 (C-157 point 2b, proposals 1-2): for `kronos --gpu`, every row held so far goes to the stick now, outside
    // the clock of position 0 (one row that must be on the stick), and from here to the end of the texts the routine
    // rows stay in memory (`gpu_i6::GenHold`): nothing is written on the stick in the middle of a generation.
    let _hold = if fast {
        runner.journal.flush_live();
        runner.journal.push(Event::Note(GEN_STARTS));
        Some(super::gpu_i6::GenHold::start())
    } else {
        None
    };
    // P6-116: the warm-ups (lists warmed, total us outside the clock, VRAM words of the first one).
    let mut warm: (u32, u64, u64) = (0, 0, 0);
    // P6-145: the batch's verdict over all texts (None = not run).
    let mut batch_pass: Option<bool> = None;
    let mut first_prof: Option<kp::KindProfile> = None;
    // P6-240 (cyber-gpu `ce8324a`): `token32` after the texts (only `kronos --gpu`, only when the cycle's list asks for
    // it); each text's own run is its reference.
    let token32 = fast && super::gpu_tur::TOKEN32_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let mut t32_refs: Vec<T32Ref> = Vec::new();
    for (ri, run) in runs.iter().enumerate() {
        runner.name = run.name.clone();
        runner.list = run.list;
        if compare && ri > 0 {
            // The launch kinds (`kp`) of the last run only: the glued list has other kinds than the plain one.
            // P6-124: the first run's profile is kept for the race on quarters (the same kinds, other programs).
            if ri == 1 {
                first_prof = Some(core::mem::take(&mut runner.prof));
            }
            runner.prof = kp::KindProfile::new();
            runner.prof_err = None;
        }
        // P6-116 (C-157 point 2, cyber-gpu `538e3b4`): the warm-up. Outside the clock, position 0 of this run is sent
        // once exactly as `run_token` sends it (the same list, token and logits copy), so the launch and pushbuffer
        // shadows hold this list's slots: the timed position 0 then writes only the records' payloads (os-329: the
        // only slow position, 11 264 + 1 064 words). Nothing new for the board; nothing of it is kept: the speed
        // counters and the token count are put back, the per-position rows and the kinds' profile start empty, and
        // the K/V rows of position 0 are written again with the same bits. An error here is the error position 0
        // would have given: the chain stops as before.
        if fast {
            if let Some(&t0) = run.prompt.first() {
                let (speed0, done0, words0) = (runner.speed.clone(), *runner.tokens_done, runner.st.vram_words);
                let am0 = (runner.am_agree, runner.am_total, runner.am_first_bad.clone(), runner.am_ns.len());
                let w0 = crate::time::since_boot_us();
                // P6-126: without the copy, the warm-up is the same argmax-only token.
                if runner.argmax_top {
                    runner.run_top(t0, 0)?;
                } else {
                    runner.run_token(t0, 0)?;
                }
                let w_us = w0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a)).unwrap_or(0);
                let w_words = runner.st.vram_words.saturating_sub(words0);
                runner.speed = speed0;
                *runner.tokens_done = done0;
                runner.prof = kp::KindProfile::new();
                runner.prof_err = None;
                runner.split.clear();
                runner.launch_ns.clear();
                // P6-123: the warm-up's argmax is not one of the checked positions.
                (runner.am_agree, runner.am_total, runner.am_first_bad) = (am0.0, am0.1, am0.2);
                runner.am_ns.truncate(am0.3);
                warm.0 += 1;
                warm.1 = warm.1.saturating_add(w_us);
                if warm.0 == 1 {
                    warm.2 = w_words;
                }
            }
        }
        let (generation, positions) = loop {
            let plan = GenPlan::new(&run.prompt, run.steps, m.vocab, m.context)?;
            let positions = plan.positions();
            let mut generation = Generation::new(plan);
            runner.argmaxes.clear();
            runner.top_words.clear();
            runner.split.clear();
            runner.diet_words.clear();
            runner.read_us.clear();
            runner.argmax_us.clear();
            runner.print = FNV_START;
            runner.d2_words.clear();
            runner.d2_polls.clear();
            runner.kept = (run.os_check || compare).then(Vec::new);
            let mut check = CpuCheck { w: &weights, caches: Caches::new(m.layers.len(), e), us: Vec::new() };
            // P6-99 (C-148 point 2): `kronos --gpu` from `/bin/kronos`: the board first (`run_check_after`), then every
            // position judged by the same rule (`kronos_model::judge`, in the kernel) against the fast engine's logits.
            let mut os = OsCheck { us: Vec::new(), why: None };
            let stop = if runner.argmax_top {
                // P6-126 (C-155 point 4): the token chosen on the board (its 16 bytes), no logits read.
                generation.run_top(&mut runner).clone()
            } else if run.os_check {
                generation.run_check_after(&mut runner, &mut os).clone()
            } else {
                generation.run(&mut runner, Some(&mut check)).clone()
            };
            // P6-126: the check after the board: the processor (the same engine as the check of `kronos --gpu`, in
            // order) computes every position's token from the board's sequence; it must be the board's token.
            if runner.argmax_top && stop.is_clean_end() {
                let seq = generation.seq.clone();
                let board = runner.argmaxes.clone();
                let chk: &mut dyn TokenCheck = if run.os_check { &mut os } else { &mut check };
                for (q, &got) in board.iter().enumerate() {
                    let cpu = match chk.cpu_logits(seq[q], q) {
                        Ok(c) => c,
                        Err(e) => {
                            runner.lines.push(format!("{}: kronos --gpu: argmax fara copie: verificarea dupa placa nu a putut judeca pozitia {}", p, q));
                            return Err(e);
                        }
                    };
                    let want = cm::argmax(&cpu);
                    runner.top_total += 1;
                    if want == Some(got) {
                        runner.top_agree += 1;
                    } else if runner.top_first_bad.is_none() {
                        runner.top_first_bad = Some(format!(
                            "\"{}\" pozitia {}: placa {}, procesorul {}",
                            run.name,
                            q,
                            got,
                            want.map_or(String::from("-"), |w| format!("{}", w))
                        ));
                    }
                }
            }
            let mut us = if run.os_check { core::mem::take(&mut os.us) } else { core::mem::take(&mut check.us) };
            // P6-97: the CPU check of each position next to its token (Generation checks every position once, in order).
            let mut split = core::mem::take(&mut runner.split);
            if split.len() == us.len() {
                for (t, &c) in split.iter_mut().zip(&us) {
                    t.check_us = c;
                }
            }
            split_all = split;
            cpu_us.append(&mut us);
            if stop.is_clean_end() {
                break (generation, positions);
            }
            runner.lines.push(format!("{}: I14: \"{}\": {}", p, run.name, stop.line()));
            // P6-126: without the copy there are no logits to explain: the stop is the error.
            if runner.argmax_top {
                return Err(match stop {
                    Stop::Eroare { err, .. } => err,
                    _ => bad("I14: argmax fara copie: generarea s-a oprit (randul de mai sus)"),
                });
            }
            if let Some(why) = os.why.take() {
                runner.lines.push(format!("{}: kronos --gpu: verificarea dupa placa: {}", p, why));
            }
            let pos = match stop {
                Stop::NuTrece { pos, .. } | Stop::LogitiNefiniti { pos } => pos,
                Stop::Eroare { err, .. } => return Err(err),
                _ => return Err(bad("I14: generarea s-a oprit (vezi randul de mai sus)")),
            };
            // P6-95 (C-141): for `kronos --gpu` the first position that differs from the processor, in the journal.
            if run.argmax.is_none() {
                let made = pos.saturating_sub(run.prompt.len());
                runner.journal.push(Event::Value { what: KG_FIRST_DIFF, value: pos as u32 });
                runner.journal.push(Event::Value { what: KG_MADE, value: made as u32 });
                runner.lines.push(format!(
                    "{}: kronos --gpu: primul simbol diferit de procesor la pozitia {} (dupa {} simboluri generate identice)",
                    p, pos, made
                ));
            }
            // What that token left in memory (nothing sent). The CPU engine's caches hold the earlier rows.
            let tok = generation.seq[pos];
            // P6-99: after the board, the reference engine had not run: it runs now up to the refused position (its
            // caches as the check between tokens would have left them), and the refused position's own board logits
            // are the ones explained. If the reference engine passes them, the board is not the one that differs.
            if run.os_check {
                for q in 0..=pos {
                    check.cpu_logits(generation.seq[q], q)?;
                }
                let got = runner.kept.as_ref().and_then(|k| k.get(pos)).cloned().unwrap_or_default();
                let reference = check.cpu_logits(tok, pos)?;
                if matches!(km::judge(&got, &reference), km::TokenVerdict::Trece { .. }) {
                    runner.lines.push(format!(
                        "{}: kronos --gpu: pozitia {}: motorul de referinta al nucleului (step_cpu) TRECE logit-urile placii; motorul rapid al OS-ului nu: diferenta e intre cele doua motoare, nu de la placa",
                        p, pos
                    ));
                    return Err(bad("I14: verificarea dupa placa (motorul rapid al OS-ului) a refuzat o pozitie pe care motorul de referinta o trece"));
                }
                runner.last_logits = got;
            }
            let cpu = check.cpu_logits(tok, pos)?;
            let ctx = kd::ProbeCtx {
                w: &weights,
                vas: &vas,
                prev: &check.caches,
                token: tok,
                pos,
                cpu_logits: &cpu,
                want: run.argmax.and_then(|a| a.get(pos).copied()),
            };
            let read = core::mem::take(&mut runner.last_logits);
            let ex = kd::explain(&mut runner, &ctx, &read);
            runner.lines.extend(ex.probe.lines().into_iter().map(|x| format!("{}: {}", p, x)));
            if ex.vram_pass && !runner.from_vram {
                runner.lines.push(format!(
                    "{}: I14: \"{}\": doar copierea logit-urilor in RAM e gresita; textul o ia de la capat cu logit-urile citite din VRAM",
                    p, run.name
                ));
                runner.from_vram = true;
                continue;
            }
            if matches!(ex.probe.verdict, kd::Verdict::Mostenit(_)) {
                let way_now = runner.way;
                for pr in kd::locate(&mut runner, &ctx, way_now.embed) {
                    runner.lines.extend(pr.lines().into_iter().map(|x| format!("{}: {}", p, x)));
                }
            }
            return Err(bad("I14: simbolul nu trece fata de procesor (diagnosticul i14d e in jurnal)"));
        };
        if compare {
            by_list.push((run.list, runner.kept.clone().unwrap_or_default()));
        }
        let stats = generation.stats();
        worst = worst.max(generation.worst_abs);
        // C-139: the argmax against Kronos's vectors only after the text ended clean against the CPU engine.
        if let Some(want) = run.argmax {
            for (pos, (&g, &w)) in runner.argmaxes.iter().zip(want).enumerate() {
                if g != w {
                    runner.lines.push(format!("{}: I14: \"{}\" pozitia {}: argmax {} pe placa, vectori.json {}", p, run.name, pos, g, w));
                    return Err(bad("I14: argmax diferit de vectorii lui Kronos"));
                }
            }
        }
        let greedy = generation.generated().to_vec();
        if let Some(want) = run.greedy {
            if greedy[..] != want[..] {
                runner.lines.push(format!("{}: I14: \"{}\": 5 simboluri greedy {:?}, vectori.json {:?}", p, run.name, greedy, want));
                return Err(bad("I14: simbolurile greedy difera de vectorii lui Kronos"));
            }
            runner.lines.push(format!(
                "{}: i14: \"{}\": TRECE - {} pozitii, argmax ca vectori.json pe fiecare pozitie, 5 simboluri greedy {:?} identice",
                p, run.name, positions, greedy
            ));
        } else {
            runner.lines.push(format!("{}: {}: {} ({})", p, run.name, stats.line(label), generation.stop.as_ref().map(|s| s.line()).unwrap_or_default()));
        }
        medians.push(stats.median_ns);
        all.push(greedy);
        if token32 {
            t32_refs.push(T32Ref {
                ri,
                seq: generation.seq.clone(),
                print: (!runner.argmax_top).then_some(runner.print),
                host_us: median_of(&split_all.iter().map(|t| t.host_us).collect::<Vec<u64>>()),
                board_us: median_of(&split_all.iter().map(|t| t.board_us).collect::<Vec<u64>>()),
            });
        }
        if batch {
            let want_seq = generation.seq.clone();
            let want_words = runner.top_words.clone();
            batch_runs(&mut runner, run, positions, lpp, batch_max, &want_seq, &want_words, &mut batch_pass)?;
        }
    }
    if token32 {
        let t32_ok = match token32_runs(&mut runner, runs, &t32_refs, T32Kind::Plain) {
            Ok(_) => true,
            Err(e) => {
                runner.lines.push(format!("{}: kronos --gpu: token32 (o lansare pe simbol): oprit: {}", p, e));
                false
            }
        };
        runner.t32 = None;
        // P6-243 (cyber-gpu `16a0cf5`, P2zzd): `token32_v2` after `token32` (the same warmed board), when the list asks;
        // not after a `token32` that stopped (a launch that did not finish leaves the board unsure).
        let v2 = super::gpu_tur::TOKEN32_V2_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
        if v2 && !t32_ok {
            runner.lines.push(format!("{}: kronos --gpu: token32_v2 (o lansare pe simbol): NETRIMIS (token32 s-a oprit)", p));
        }
        let mut v2_pass = false;
        if v2 && t32_ok {
            match token32_runs(&mut runner, runs, &t32_refs, T32Kind::V2) {
                Ok(all) => v2_pass = all,
                Err(e) => runner.lines.push(format!("{}: kronos --gpu: token32_v2 (o lansare pe simbol): oprit: {}", p, e)),
            }
        }
        runner.t32 = None;
        // P6-255 (cyber-gpu `e117394`, P2zzq): `token32_v2c` (v2 + the SM clock) only after a `token32_v2` that passed
        // at every G (the same texts, a measuring program).
        let v2c = super::gpu_tur::TOKEN32_V2C_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
        if v2c && !(v2 && v2_pass) {
            runner.lines.push(format!("{}: kronos --gpu: token32_v2c (o lansare pe simbol): NETRIMIS (token32_v2 nu a trecut)", p));
        }
        if v2c && v2 && v2_pass {
            if let Err(e) = token32_runs(&mut runner, runs, &t32_refs, T32Kind::V2c) {
                runner.lines.push(format!("{}: kronos --gpu: token32_v2c (o lansare pe simbol): oprit: {}", p, e));
            }
        }
        runner.t32 = None;
        // P6-257 (cyber-gpu `b93e876`, P2zzt): `token32_v3` (v2 with the 25 layernorms folded into the product that
        // reads them) only after a `token32_v2` that passed at every G (passes = the same bits as v2).
        let v3 = super::gpu_tur::TOKEN32_V3_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
        if v3 && !(v2 && v2_pass) {
            runner.lines.push(format!("{}: kronos --gpu: token32_v3 (o lansare pe simbol): NETRIMIS (token32_v2 nu a trecut)", p));
        }
        if v3 && v2 && v2_pass {
            if let Err(e) = token32_runs(&mut runner, runs, &t32_refs, T32Kind::V3) {
                runner.lines.push(format!("{}: kronos --gpu: token32_v3 (o lansare pe simbol): oprit: {}", p, e));
            }
        }
        runner.t32 = None;
    }
    if batch {
        let verdict = if batch_pass == Some(true) { "TRECE" } else { "NU TRECE" };
        runner.journal.push(Event::Value { what: KT_LOT_PASS, value: u32::from(batch_pass == Some(true)) });
        runner.lines.push(format!(
            "{}: kronos --gpu: lot (simbolul ramane pe placa): {} (embed_dev, mai multe pozitii pe o sonerie, fata de rularea cu o pozitie pe trimitere)",
            p, verdict
        ));
    }
    if warm.0 > 0 {
        runner.journal.push(Event::Value { what: KT_WARM, value: warm.0 });
        runner.journal.push(Event::Value { what: KT_WARM_US, value: warm.1.min(u64::from(u32::MAX)) as u32 });
        runner.journal.push(Event::Value { what: KT_WARM_WORDS, value: warm.2.min(u64::from(u32::MAX)) as u32 });
        runner.lines.push(format!(
            "{}: kronos --gpu: kt incalzirea: pozitia 0 trimisa o data inainte de ceas pe {} liste ({} us in afara ceasului, {} cuvinte VRAM la prima)",
            p, warm.0, warm.1, warm.2
        ));
    }
    if argmax_board {
        // P6-123 (C-155 point 3): the trial's verdict (threshold 0) and the argmax launch's median time.
        let (agree, total) = (runner.am_agree, runner.am_total);
        let mut ns = runner.am_ns.clone();
        ns.sort_unstable();
        let med = ns.get(ns.len() / 2).copied();
        runner.journal.push(Event::Value { what: KT_AM_ON, value: 1 });
        runner.journal.push(Event::Value { what: KT_AM_AGREE, value: agree });
        runner.journal.push(Event::Value { what: KT_AM_TOTAL, value: total });
        if let Some(t) = med {
            runner.journal.push(Event::Value { what: KT_AM_MEDIAN_NS, value: t.min(u64::from(u32::MAX)) as u32 });
        }
        runner.lines.push(format!(
            "{}: kronos --gpu: kt argmax placa = procesor: {}/{} (cele 4 cuvinte ale programului argmax = argmax_result al logit-urilor copiate, prag 0)",
            p, agree, total
        ));
        runner.lines.push(match med {
            Some(t) => format!("{}: kronos --gpu: kt median argmax placa (us): {}.{:03} (lansarea argmax, din inregistrari, {} pozitii)", p, t / 1000, t % 1000, ns.len()),
            None => format!("{}: kronos --gpu: kt median argmax placa (us): - (nicio pozitie cu toate inregistrarile citite)", p),
        });
        if let Some(why) = runner.am_first_bad.take() {
            runner.lines.push(format!("{}: kronos --gpu: kt argmax placa: prima pozitie diferita: {}", p, why));
        }
        let verdict = if total > 0 && agree == total { "TRECE" } else { "NU TRECE" };
        runner.lines.push(format!(
            "{}: kronos --gpu: argmax pe placa (proba, copie + argmax in acelasi simbol): {} ({}/{} pozitii identice; simbolul ales tot pe procesor)",
            p, verdict, agree, total
        ));
    }
    if argmax_top {
        // P6-126 (C-155 point 4): the verdict (the board's token = the processor's at every position) and the reads.
        let (agree, total) = (runner.top_agree, runner.top_total);
        let mut ns = runner.am_ns.clone();
        ns.sort_unstable();
        let med = ns.get(ns.len() / 2).copied();
        let read = median_of(&runner.read_us);
        runner.journal.push(Event::Value { what: KT_TOP_ON, value: 1 });
        runner.journal.push(Event::Value { what: KT_TOP_AGREE, value: agree });
        runner.journal.push(Event::Value { what: KT_TOP_TOTAL, value: total });
        runner.journal.push(Event::Value { what: KT_TOP_READ, value: read.map_or(u32::MAX, |r| r.min(u64::from(u32::MAX - 1)) as u32) });
        if let Some(t) = med {
            runner.journal.push(Event::Value { what: KT_AM_MEDIAN_NS, value: t.min(u64::from(u32::MAX)) as u32 });
        }
        runner.lines.push(format!(
            "{}: kronos --gpu: kt argmax fara copie: simbolul placii = procesorului: {}/{} (logit-urile raman pe placa; procesorul verifica dupa placa)",
            p, agree, total
        ));
        runner.lines.push(match read {
            Some(r) => format!("{}: kronos --gpu: kt median citirea rezultatului (us): {} (cei 16 octeti ai programului argmax)", p, r),
            None => format!("{}: kronos --gpu: kt median citirea rezultatului (us): -", p),
        });
        runner.lines.push(match med {
            Some(t) => format!("{}: kronos --gpu: kt median argmax placa (us): {}.{:03} (lansarea argmax, din inregistrari, {} pozitii)", p, t / 1000, t % 1000, ns.len()),
            None => format!("{}: kronos --gpu: kt median argmax placa (us): - (nicio pozitie cu toate inregistrarile citite)", p),
        });
        if let Some(why) = runner.top_first_bad.take() {
            runner.lines.push(format!("{}: kronos --gpu: kt argmax fara copie: prima pozitie diferita: {}", p, why));
        }
        let pass = total > 0 && agree == total;
        runner.lines.push(format!(
            "{}: kronos --gpu: argmax fara copie (simbolul ales pe placa): {} ({}/{} pozitii identice cu procesorul)",
            p,
            if pass { "TRECE" } else { "NU TRECE" },
            agree,
            total
        ));
        if !pass {
            return Err(bad("I14: argmax fara copie: simbolul placii difera de procesor (randul de mai sus)"));
        }
    }
    if compare {
        let rows = if lists.contains(&kd::TokenList::FusedLn32) {
            // P6-128 (C-157 point 2, note C-157_P2g): `lipita-ln-32` against `lipita-ln` (the cycle's default).
            quarters_vs_fused(p, m.layers.len(), &by_list, &all, &mut *runner.journal, Q_LN32)
        } else if lists.contains(&kd::TokenList::Fused32) {
            quarters_vs_fused(p, m.layers.len(), &by_list, &all, &mut *runner.journal, Q_FUSED32)
        } else if lists.contains(&kd::TokenList::FusedLn) {
            ln_new_vs_old(p, m.layers.len(), &by_list, &all, &mut *runner.journal)
        } else {
            fused_vs_plain(p, m.layers.len(), &by_list, &all, &mut *runner.journal)
        };
        runner.lines.extend(rows);
    }
    let from_vram = runner.from_vram;
    let speed = core::mem::replace(&mut runner.speed, ks::Speed::new(ks::Source::Simulare));
    let shadow_changed = runner.shadow_check()?;
    let prof = core::mem::take(&mut runner.prof);
    let prof_err = runner.prof_err.take();
    let diet_out = fast.then(|| DietOut {
        diet2: runner.diet2.then(|| nvidia_ga104::kronos_diet2::Diet2Stats {
            record_words: core::mem::take(&mut runner.d2_words),
            polls: core::mem::take(&mut runner.d2_polls),
        }),
        on: runner.diet,
        words: core::mem::take(&mut runner.diet_words),
        read_us: core::mem::take(&mut runner.read_us),
        argmax_us: core::mem::take(&mut runner.argmax_us),
        print: runner.print,
        top: runner.argmax_top,
    });
    drop(runner);
    let run_medians = medians.clone();
    medians.sort_unstable();
    let median = medians.get(medians.len() / 2).copied().unwrap_or(0);
    // P6-98 (C-147 stage 0 (d)): `kronos --gpu` keeps what the VRAM → VRAM copy needs (it runs after the result rows).
    // Only the weights (the bytes before the scratch): the scratch, the final vector and the logits are written by the board.
    let mut src = blob.bytes;
    src.truncate(blob.scratch as usize);
    let copy = fast.then(|| CopyCtx { src_va: vblob.va, src, launches, rec, maps: tmaps, sys: host_logits, copy_va });
    let os_check = runs.iter().any(|r| r.os_check);
    Ok(RunOut { all, worst, median, run_medians, from_vram, speed, cpu_us, os_check, split: split_all, shadow_changed, prof, prof_err, prof_first: first_prof, copy, diet: diet_out })
}

/// P6-240: one text's own run, the reference of its `token32` runs.
struct T32Ref {
    ri: usize,
    seq: Vec<usize>,
    /// The logits' print (`None` with the argmax without the copy: no logits read).
    print: Option<u32>,
    /// Medians per position of the text's own run: with the driver (OS clock) and on the board (us).
    host_us: Option<u64>,
    board_us: Option<u64>,
}

/// P6-240 (cyber-gpu `ce8324a`, note `C-157_P2zz_token32-pe-placa.md`): every text once more with `token32` — the whole
/// token in one launch, G workgroups meeting at a grid barrier between its operations. G = SMs × 8 (384), then × 16
/// (768) only if 384 passed. Passes = for every text the same tokens (and the same logits' print) as its own run, and
/// the barrier counter (n − 1) · G after every launch. A launch that does not finish stops `token32` (nothing more is
/// sent with it). Every counter of the ordinary run (speed, profile, splits, prints) is put back afterwards.
/// The program `token32_runs` sends: `token32`, `token32_v2` (P6-243), `token32_v2c` (P6-255, v2 + the SM clock) or
/// `token32_v3` (P6-257, v2 with the layernorms folded into the next product: `token_ln::work_list_ln`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum T32Kind {
    Plain,
    V2,
    V2c,
    V3,
}

/// Returns whether every G passed.
fn token32_runs<V: Mmio + ?Sized, K: Clock + ?Sized>(runner: &mut BoardRunner<'_, '_, V, K>, runs: &[Run<'_>], refs: &[T32Ref], kind: T32Kind) -> GpuResult<bool> {
    use nvidia_ga104::kronos_token as kt;
    use nvidia_ga104::token_clock as tc;
    // P6-243 (cyber-gpu `16a0cf5`, P2zzd): `token32_v2` = the same code with the reads through L1, and only the
    // changed words of the list written before every token. P6-255 (cyber-gpu `e117394`, P2zzq): `token32_v2c` = v2's
    // arithmetic plus the SM clock around every operation, written to a clock area; G = SMs x 16 only.
    use nvidia_ga104::token_ln as tl;
    let (pn, code, want_sha, regs) = match kind {
        T32Kind::V3 => ("token32_v3", tl::TOKEN32_V3, TOKEN32_V3_SHA256, tl::TOKEN32_V3_REGISTERS),
        T32Kind::V2c => ("token32_v2c", tc::TOKEN32_V2C, TOKEN32_V2C_SHA256, tc::TOKEN32_V2C_REGISTERS),
        T32Kind::V2 => ("token32_v2", kt::TOKEN32_V2, TOKEN32_V2_SHA256, kt::TOKEN32_V2_REGISTERS),
        T32Kind::Plain => ("token32", kt::TOKEN32, TOKEN32_SHA256, kt::TOKEN32_REGISTERS),
    };
    let v2 = kind != T32Kind::Plain;
    if kind == T32Kind::V2c && tc::TOKEN32_V2C_SHA256 != TOKEN32_V2C_SHA256 {
        runner.lines.push(format!("{}: kronos --gpu: {pn}: SHA-256 din dependenta nu e cel din nota: nimic trimis", runner.p));
        return Err(bad("token32_v2c: SHA-256 gresit"));
    }
    if kind == T32Kind::V3 && tl::TOKEN32_V3_SHA256 != TOKEN32_V3_SHA256 {
        runner.lines.push(format!("{}: kronos --gpu: {pn}: SHA-256 din dependenta nu e cel din nota: nimic trimis", runner.p));
        return Err(bad("token32_v3: SHA-256 gresit"));
    }
    let p = runner.p;
    let label = if runner.permit.is_some() { "placa" } else { "SIMULARE" };
    let sha = sha256_hex(code);
    if sha != want_sha {
        runner.lines.push(format!("{}: kronos --gpu: {pn}: SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, sha, want_sha));
        return Err(bad("token32: SHA-256 gresit"));
    }
    let counts = runner.st.gr_counts.ok_or_else(|| bad("token32: numarul de TPC lipseste (I9 nu l-a raportat)"))?;
    let sms = counts.sms;
    runner.lines.push(format!(
        "{}: kronos --gpu: {pn} ({}): programul {pn} ({} octeti, SHA-256 {}..., {} registre, blocuri de {}), {} SM-uri active, G = SM-uri x {}, limita {} ms pe lansare, {} texte",
        p,
        label,
        code.len(),
        &sha[..16],
        regs,
        kt::TOKEN_BLOCK,
        sms,
        if kind == T32Kind::V2c { "16 (doar)" } else { "8 apoi x 16" },
        BARRIER_LIMIT_US / 1000,
        refs.len()
    ));
    if sms != EXPECTED_SMS {
        runner.lines.push(format!("{}: kronos --gpu: {pn}: GSP-RM raporteaza {} SM, asteptat {} (RTX 3070 Ti); oprit, nimic trimis", p, sms, EXPECTED_SMS));
        return Err(bad("token32: numarul de SM nu e cel asteptat"));
    }
    let prog = map_vram(runner.st, program_map_bytes(code.len() as u64))?;
    put_bytes(runner.st.mem.writer_mut(), prog.pa, code)?;
    let list = map_vram(runner.st, ((kt::ENTRY * runner.n) as u64).next_multiple_of(PAGE))?;
    let zone = map_vram(runner.st, PAGE)?;
    // What the ordinary run left (put back at the end; `token32` only measures).
    let speed0 = runner.speed.clone();
    let done0 = *runner.tokens_done;
    let prof0 = core::mem::replace(&mut runner.prof, kp::KindProfile::new());
    let prof_err0 = runner.prof_err.take();
    let split0 = core::mem::take(&mut runner.split);
    let words0 = core::mem::take(&mut runner.diet_words);
    let read0 = core::mem::take(&mut runner.read_us);
    let am_us0 = core::mem::take(&mut runner.argmax_us);
    let d2w0 = core::mem::take(&mut runner.d2_words);
    let d2p0 = core::mem::take(&mut runner.d2_polls);
    let print0 = runner.print;
    let am0 = (runner.am_agree, runner.am_total, runner.am_first_bad.clone(), runner.am_ns.len());
    let top_words0 = core::mem::take(&mut runner.top_words);
    let argmaxes0 = core::mem::take(&mut runner.argmaxes);
    let kept0 = runner.kept.take();
    let (name0, list0) = (runner.name.clone(), runner.list);
    let mut verdicts: Vec<String> = Vec::new();
    let mut result: GpuResult<()> = Ok(());
    let mut all_pass = true;
    let per_sms: &[u32] = if kind == T32Kind::V2c { &[16] } else { &[8, 16] };
    'g: for &per_sm in per_sms {
        let g = kt::token32_groups_for_sms(sms, per_sm)?;
        let step = kt::token32_step_for(g, sms)?;
        // P6-255: the clock area (n·G + n + 1 + G words) for the most launches a token can have.
        let clock = if kind == T32Kind::V2c { Some(map_vram(runner.st, (4 * tc::clock_words(runner.n, g)? as u64).next_multiple_of(PAGE))?) } else { None };
        runner.t32 = Some(T32 { prog, list, zone, groups: g, sms, counted: 0, counter_ok: 0, counter_bad: None, hung: None, prev: None, diff: v2, words: 0, tokens: 0, per_token: Vec::new(), clock, last: None, ln: kind == T32Kind::V3, ops: None });
        let mut kc: Option<Vec<String>> = None;
        let mut pass = true;
        for r in refs {
            let run = runs.get(r.ri).ok_or(bad("token32: text necunoscut"))?;
            runner.name = run.name.clone();
            runner.list = run.list;
            runner.split.clear();
            runner.print = FNV_START;
            if let Some(t) = runner.t32.as_mut() {
                (t.counted, t.counter_ok, t.counter_bad) = (0, 0, None);
            }
            let plan = GenPlan::new(&run.prompt, run.steps, runner.m.vocab, runner.m.context)?;
            let mut generation = Generation::new(plan);
            let stop = if runner.argmax_top { generation.run_top(&mut *runner).clone() } else { generation.run(&mut *runner, None).clone() };
            let t = runner.t32.as_ref().ok_or(bad("token32: nepregatit"))?;
            if let Some(why) = t.hung.clone() {
                runner.lines.push(format!("{}: kronos --gpu: {pn} {}; oprit, nimic altceva trimis cu {pn}", p, why));
                runner.lines.push(format!("{}: kronos --gpu: kt {pn} G={} NU A TERMINAT", p, g));
                verdicts.push(format!("G={} NU A TERMINAT", g));
                result = Err(GpuError::Timeout { waited_us: BARRIER_LIMIT_US });
                break 'g;
            }
            if !stop.is_clean_end() {
                runner.lines.push(format!("{}: kronos --gpu: {pn} G={} \"{}\": oprit: {}", p, g, run.name, stop.line()));
                verdicts.push(format!("G={} OPRIT", g));
                pass = false;
                break;
            }
            let same_seq = generation.seq == r.seq;
            let print = (!runner.argmax_top).then_some(runner.print);
            let same_print = print == r.print;
            let counters = t.counted > 0 && t.counter_ok == t.counted;
            let host = median_of(&runner.split.iter().map(|x| x.host_us).collect::<Vec<u64>>());
            let board = median_of(&runner.split.iter().map(|x| x.board_us).collect::<Vec<u64>>());
            let us = |v: Option<u64>| v.map_or(String::from("-"), |x| format!("{}", x));
            let ok = same_seq && same_print && counters;
            pass &= ok;
            runner.lines.push(format!(
                "{}: kronos --gpu: {pn} G={} \"{}\" ({}, treapta {} KiB): {} - {} simboluri {}, amprenta {} ({}), contorul barierei corect la {} / {} simboluri; placa pe simbol median {} us (lista obisnuita {} us), cu driverul {} us (lista {} us)",
                p,
                g,
                run.name,
                label,
                step / 1024,
                if ok { "TRECE" } else { "NU TRECE" },
                generation.generated().len(),
                if same_seq { "identice" } else { "DIFERITE" },
                print.map_or(String::from("-"), |x| format!("{:#010x}", x)),
                match (r.print, same_print) {
                    (None, _) => String::from("fara logit-uri: argmax fara copie"),
                    (Some(w), true) => format!("aceeasi ca lista obisnuita {:#010x}", w),
                    (Some(w), false) => format!("DIFERITA de lista obisnuita {:#010x}", w),
                },
                t.counter_ok,
                t.counted,
                us(board),
                us(r.board_us),
                us(host),
                us(r.host_us)
            ));
            if let Some(why) = &t.counter_bad {
                runner.lines.push(format!("{}: kronos --gpu: {pn} G={}: primul contor gresit: {}", p, g, why));
            }
            if !same_seq {
                runner.lines.push(format!("{}: kronos --gpu: {pn} G={} \"{}\": simbolurile {:?}, lista obisnuita {:?}", p, g, run.name, generation.generated(), &r.seq[run.prompt.len().min(r.seq.len())..]));
            }
            runner.lines.push(format!("{}: kronos --gpu: kt {pn} G={} placa {} us, cu driverul {} us", p, g, us(board), us(host)));
            // P6-255: the clock area holds the text's last token (every launch rewrites every word); read once here, so
            // the tokens' own driver time is not touched.
            if t.clock.is_some() {
                kc = Some(runner.clock_rows(g));
            }
        }
        if let Some(rows) = kc.take() {
            runner.lines.extend(rows);
        }
        if let Some(t) = runner.t32.as_ref() {
            runner.lines.push(format!(
                "{}: kronos --gpu: kt {pn} G={} cuvinte scrise in lista pe simbol {} ({} simboluri, {})",
                p,
                g,
                if t.tokens > 0 { t.words / t.tokens } else { 0 },
                t.tokens,
                if t.diff { "doar cuvintele schimbate, work_list_changes" } else { "lista intreaga" }
            ));
            if let Some((first, rest)) = t.per_token.split_first() {
                let us = |v: Option<u64>| v.map_or(String::from("-"), |x| format!("{}", x));
                runner.lines.push(format!(
                    "{}: kronos --gpu: kt {pn} G={} cuvinte scrise in lista: primul simbol {}, mediana celorlalte {} (cel mai mic {}, cel mai mare {}, {} simboluri)",
                    p,
                    g,
                    first,
                    us(median_of(rest)),
                    us(rest.iter().min().copied()),
                    us(rest.iter().max().copied()),
                    rest.len()
                ));
            }
        }
        // P6-257: with `token32_v3` the operations of the last token (layernorms folded) against the ordinary list.
        if let Some((n, launches)) = runner.t32.as_ref().and_then(|t| t.ops) {
            runner.lines.push(format!(
                "{}: kronos --gpu: kt {pn} G={} operatii n={} pe simbol (lista obisnuita {}, {} LayerNorm lipite in inmultirea urmatoare, token_ln::work_list_ln)",
                p,
                g,
                n,
                launches,
                launches - n
            ));
        }
        runner.lines.push(format!("{}: kronos --gpu: kt {pn} G={} {}", p, g, if pass { "TRECE" } else { "NU TRECE" }));
        verdicts.push(format!("G={} {}", g, if pass { "TRECE" } else { "NU TRECE" }));
        all_pass &= pass;
        if !pass {
            break; // G = 768 only after 384 passed
        }
    }
    runner.lines.push(format!("{}: kronos --gpu: {pn} (o lansare pe simbol): {}", p, verdicts.join(", ")));
    runner.t32 = None;
    runner.speed = speed0;
    *runner.tokens_done = done0;
    runner.prof = prof0;
    runner.prof_err = prof_err0;
    runner.split = split0;
    runner.diet_words = words0;
    runner.read_us = read0;
    runner.argmax_us = am_us0;
    runner.d2_words = d2w0;
    runner.d2_polls = d2p0;
    runner.print = print0;
    (runner.am_agree, runner.am_total, runner.am_first_bad) = (am0.0, am0.1, am0.2);
    runner.am_ns.truncate(am0.3);
    runner.top_words = top_words0;
    runner.argmaxes = argmaxes0;
    runner.kept = kept0;
    runner.name = name0;
    runner.list = list0;
    result.map(|_| all_pass)
}

/// P6-145 (C-157 point 7): the batch runs of one text (batches of 4, then the whole text or as many positions as the
/// pushbuffer area holds), their rows and journal values; `pass` = every run so far the same as one position per doorbell.
#[allow(clippy::too_many_arguments)]
fn batch_runs<V: Mmio + ?Sized, K: Clock + ?Sized>(
    runner: &mut BoardRunner<'_, '_, V, K>,
    run: &Run<'_>,
    positions: usize,
    lpp: usize,
    batch_max: usize,
    want_seq: &[usize],
    want_words: &[[u32; 4]],
    pass: &mut Option<bool>,
) -> GpuResult<()> {
    let p = runner.p;
    if batch_max == 0 {
        runner.lines.push(format!("{}: kronos --gpu: lot: nicio pozitie nu incape in zona de comenzi ({} lansari pe pozitie)", p, lpp));
        *pass = Some(false);
        return Ok(());
    }
    let whole = positions.min(batch_max);
    if positions > batch_max {
        runner.lines.push(format!(
            "{}: kronos --gpu: lot: tot textul ({} pozitii) nu incape intr-o trimitere: zona de comenzi tine {} pozitii ({} lansari pe pozitie)",
            p, positions, batch_max, lpp
        ));
    }
    let mut ks: Vec<usize> = alloc::vec![4.min(whole)];
    if whole > 4 {
        ks.push(whole);
    }
    for k in ks {
        let o = runner.run_batch(&run.prompt, run.steps, k, want_seq, want_words)?;
        let mut b = o.board_ns.clone();
        b.sort_unstable();
        let med_board = b.get(b.len() / 2).copied();
        let host_per_pos = o.host_us.checked_div(o.positions as u64);
        let rate = (o.host_us > 0).then(|| o.positions as u64 * 1_000_000_000 / o.host_us);
        let board_rate = (o.span_ns > 0).then(|| o.positions as u64 * 1_000_000_000_000 / o.span_ns);
        let ok = o.clean && o.same_seq && o.total > 0 && o.agree == o.total;
        *pass = Some(pass.unwrap_or(true) && ok);
        runner.journal.push(Event::Value { what: KT_LOT_K, value: k as u32 });
        runner.journal.push(Event::Value { what: KT_LOT_BELLS, value: o.bells });
        runner.journal.push(Event::Value { what: KT_LOT_AGREE, value: o.agree });
        runner.journal.push(Event::Value { what: KT_LOT_TOTAL, value: o.total });
        if let Some(t) = med_board {
            runner.journal.push(Event::Value { what: KT_LOT_BOARD_NS, value: t.min(u64::from(u32::MAX)) as u32 });
        }
        if let Some(r) = rate {
            runner.journal.push(Event::Value { what: KT_LOT_RATE_X1000, value: r.min(u64::from(u32::MAX)) as u32 });
        }
        let name = &run.name;
        runner.lines.push(format!(
            "{}: kronos --gpu: kt lot pozitii K {}: \"{}\": {} pozitii in {} trimiteri ({} lansari pe pozitie, invalidare inaintea fiecarei lansari)",
            p, k, name, o.positions, o.bells, lpp
        ));
        runner.lines.push(format!("{}: kronos --gpu: kt lot sonerii (K {}): {}", p, k, o.bells));
        runner.lines.push(match med_board {
            Some(t) => format!("{}: kronos --gpu: kt lot median placa pe pozitie (us, K {}): {}.{:03} (din inregistrari)", p, k, t / 1000, t % 1000),
            None => format!("{}: kronos --gpu: kt lot median placa pe pozitie (us, K {}): -", p, k),
        });
        runner.lines.push(format!(
            "{}: kronos --gpu: kt lot driver pe lot (us, K {}, median): construirea listei {}, scrierea lansarilor {}, trimiterea + asteptarea {}, citirea rezultatelor {}",
            p,
            k,
            median_of(&o.build_us).unwrap_or(0),
            median_of(&o.write_us).unwrap_or(0),
            median_of(&o.send_us).unwrap_or(0),
            median_of(&o.read_us).unwrap_or(0)
        ));
        runner.lines.push(format!(
            "{}: kronos --gpu: kt lot simb/s (K {}): {} (ceasul OS-ului, {} us pe pozitie); doar placa: {}",
            p,
            k,
            rate.map_or(String::from("-"), |r| format!("{}.{:03}", r / 1000, r % 1000)),
            host_per_pos.map_or(String::from("-"), |u| format!("{}", u)),
            board_rate.map_or(String::from("-"), |r| format!("{}.{:03}", r / 1000, r % 1000))
        ));
        runner.lines.push(format!(
            "{}: kronos --gpu: kt lot K {}: {} ({}/{} pozitii cu aceleasi 4 cuvinte argmax ca o pozitie pe trimitere; simbolurile {}; {})",
            p,
            k,
            if ok { "TRECE" } else { "NU TRECE" },
            o.agree,
            o.total,
            if o.same_seq { "identice" } else { "DIFERITE" },
            o.stop
        ));
        if let Some(why) = o.first_bad {
            runner.lines.push(format!("{}: kronos --gpu: kt lot K {}: prima diferenta: {}", p, k, why));
        }
        if !ok {
            // The note: at the first difference, the report, no other attempt.
            break;
        }
    }
    Ok(())
}

/// P6-110 (C-155 point 1): P3's proof — the glued list's logits against the plain list's on the same text, bit for bit
/// (the note's threshold is 0), and the same greedy tokens. Rows for the output (and so the cycle's report).
fn fused_vs_plain(p: &str, layers: usize, by_list: &[(kd::TokenList, Vec<Vec<f32>>)], all: &[Vec<usize>], journal: &mut Journal) -> Vec<String> {
    let pick = |want: kd::TokenList| by_list.iter().find(|(l, _)| *l == want).map(|(_, v)| v);
    let mut out = alloc::vec![format!(
        "{}: kronos --gpu: kt lista: lipita ({} lansari pe simbol fata de {} cu lista simpla, fiecare cu copierea logit-urilor; programul q8_fused)",
        p,
        kd::TokenList::Fused.launches_per_token(layers) + 1,
        kd::TokenList::Plain.launches_per_token(layers) + 1
    )];
    let (Some(plain), Some(fused)) = (pick(kd::TokenList::Plain), pick(kd::TokenList::Fused)) else {
        out.push(format!("{}: kronos --gpu: kt lipit fata de simplu: necomparat (o lista nu a terminat textul)", p));
        return out;
    };
    let (mut differ, mut total, mut worst) = (0usize, 0usize, 0.0f32);
    for (a, b) in plain.iter().zip(fused) {
        total += a.len().max(b.len());
        differ += a.len().abs_diff(b.len());
        for (x, y) in a.iter().zip(b) {
            if x.to_bits() != y.to_bits() {
                differ += 1;
                worst = worst.max((x - y).abs());
            }
        }
    }
    let positions = plain.len().min(fused.len());
    out.push(if differ == 0 && plain.len() == fused.len() {
        format!("{}: kronos --gpu: kt lipit fata de simplu: identice ({} pozitii, {} logit-uri, bit cu bit)", p, positions, total)
    } else {
        format!(
            "{}: kronos --gpu: kt lipit fata de simplu: {} diferite din {} logit-uri ({} / {} pozitii; max |d| = {:e})",
            p, differ, total, plain.len(), fused.len(), worst
        )
    });
    let same = all.len() >= 2 && all[all.len() - 2] == all[all.len() - 1];
    // P6-112 (center's note 13:00, point 1): the comparison in the gpu journal too (it goes up to the hub by itself).
    let differ_row = if plain.len() == fused.len() { differ } else { differ.max(1) };
    journal.push(Event::Value { what: KT_FUSED_DIFFER, value: differ_row.min(u32::MAX as usize) as u32 });
    journal.push(Event::Value { what: KT_FUSED_SAME_TOKENS, value: u32::from(same) });
    out.push(format!(
        "{}: kronos --gpu: kt lipit fata de simplu: simbolurile generate {}",
        p,
        if same { String::from("identice") } else { format!("DIFERITE: simplu {:?}, lipit {:?}", all.get(all.len().wrapping_sub(2)), all.last()) }
    ));
    out
}

/// P6-124 (C-155 point 4, note `C-155_cursa-pe-sferturi.md`): the race on quarters, correctness half — the glued list
/// on quarters (`Fused32`) against the glued list on the same text. Not bit for bit (the sums run in another order): the
/// note's threshold is the same greedy tokens and every logit within 1e-4. Both texts were already judged against the
/// processor. The speed half (kinds of launch, 50 per mille) is `quarters_kinds`.
fn quarters_vs_fused(p: &str, layers: usize, by_list: &[(kd::TokenList, Vec<Vec<f32>>)], all: &[Vec<usize>], journal: &mut Journal, q: QuarterRace) -> Vec<String> {
    let pick = |want: kd::TokenList| by_list.iter().find(|(l, _)| *l == want).map(|(_, v)| v);
    let mut out = alloc::vec![format!(
        "{}: kronos --gpu: kt lista: {} (aceleasi {} lansari pe simbol ca {}, cu copierea logit-urilor, aceleasi cbuf-uri; inmultirile lipite pe q8_fused32, logit-urile pe q8_matvec32)",
        p,
        q.name,
        // P6-135 (note C-157_P2i, point 5): counted with the copy of the logits, like every other `kt lista` row (88).
        q.new.launches_per_token(layers) + 1,
        q.base_name
    )];
    let (Some(old), Some(new)) = (pick(q.base), pick(q.new)) else {
        out.push(format!("{}: kronos --gpu: {}: necomparat (o lista nu a terminat textul)", p, q.row));
        journal.push(Event::Value { what: KT_Q_PASS, value: 0 });
        return out;
    };
    let (mut differ, mut total, mut worst, mut finite) = (0usize, 0usize, 0.0f32, true);
    for (a, b) in old.iter().zip(new) {
        total += a.len().max(b.len());
        differ += a.len().abs_diff(b.len());
        for (x, y) in a.iter().zip(b) {
            if x.to_bits() != y.to_bits() {
                differ += 1;
                let d = (x - y).abs();
                if d.is_finite() {
                    worst = worst.max(d);
                } else {
                    finite = false;
                }
            }
        }
    }
    let same_len = old.len() == new.len();
    let same = all.len() >= 2 && all[all.len() - 2] == all[all.len() - 1];
    let pass = same && same_len && finite && worst < QUARTERS_MAX_DIFF;
    out.push(format!(
        "{}: kronos --gpu: {}: max |d| = {:e} ({} diferite din {} logit-uri, {} / {} pozitii), simboluri identice: {}",
        p,
        q.row,
        worst,
        differ,
        total,
        old.len(),
        new.len(),
        if same { "da" } else { "nu" }
    ));
    out.push(format!(
        "{}: kronos --gpu: {}: {}",
        p,
        q.row,
        if pass {
            String::from("CORECT (aceleasi simboluri, abaterea sub 1e-4)")
        } else if !same {
            format!("NU TRECE: simbolurile DIFERITE: {} {:?}, {} {:?}", q.base_name, all.get(all.len().wrapping_sub(2)), q.name, all.last())
        } else {
            String::from("NU TRECE: abaterea peste 1e-4 (sau logit-uri nefinite / pozitii lipsa)")
        }
    ));
    journal.push(Event::Value { what: KT_Q_WORST_BITS, value: if finite { worst.to_bits() } else { u32::MAX } });
    journal.push(Event::Value { what: KT_Q_SAME_TOKENS, value: u32::from(same) });
    journal.push(Event::Value { what: KT_Q_PASS, value: u32::from(pass) });
    out
}

/// P6-124 (C-155 point 4): the race on quarters, speed half — the median per token of every kind of launch, glued list
/// (`first`) against quarters (`last`), and the rule written before the run: a kind passes on quarters only if the
/// texts were correct (`correct`) and it is faster by at least 50 per mille; otherwise it keeps today's program. Only the
/// products change program (fc, proj, qkv, out, logit); the other kinds are shown for reference.
fn quarters_kinds(first: &kp::KindProfile, last: &kp::KindProfile, correct: bool) -> (Vec<String>, u32, i64) {
    let a = first.summary();
    let b = last.summary();
    let (mut rows, mut won) = (Vec::new(), 0u32);
    let (mut sum_a, mut sum_b) = (0u64, 0u64);
    let mut kinds: Vec<(kp::Kind, u64, u64)> = Vec::new();
    for ka in a.iter() {
        let Some(kind) = ka.kind else { continue };
        let Some(kb) = b.iter().find(|x| x.kind == Some(kind)) else { continue };
        sum_a += ka.token_median_ns;
        sum_b += kb.token_median_ns;
        kinds.push((kind, ka.token_median_ns, kb.token_median_ns));
    }
    kinds.sort_by(|x, y| y.1.cmp(&x.1));
    let per_mille = |old: u64, new: u64| if old == 0 { 0 } else { (old as i64 - new as i64) * 1000 / old as i64 };
    for (kind, ta, tb) in kinds {
        let g = per_mille(ta, tb);
        let product = matches!(kind, kp::Kind::Qkv | kp::Kind::Out | kp::Kind::Fc | kp::Kind::Proj | kp::Kind::Logits);
        let verdict = if !product {
            "acelasi program"
        } else if correct && g >= QUARTERS_MIN_GAIN {
            won += 1;
            "TRECE pe sferturi"
        } else if !correct {
            "ramane (textele nu au fost corecte)"
        } else {
            "ramane programul de azi"
        };
        rows.push(format!("  {:<12} lipit {:>6} us, sferturi {:>6} us pe simbol: {:+} la mie -> {}", kind.name(), ta / 1000, tb / 1000, g, verdict));
    }
    (rows, won, per_mille(sum_a, sum_b))
}

/// P6-114 (C-155 point 2): the new LayerNorm's proof — the glued list with `layernorm256` against the glued list with the
/// old LayerNorm on the same text. Not bit for bit (the sums run in another order): the note's threshold is the same greedy
/// tokens and every logit within 1e-4 (SIMULARE: 3.1e-7). Both texts were already judged against the processor.
fn ln_new_vs_old(p: &str, layers: usize, by_list: &[(kd::TokenList, Vec<Vec<f32>>)], all: &[Vec<usize>], journal: &mut Journal) -> Vec<String> {
    let pick = |want: kd::TokenList| by_list.iter().find(|(l, _)| *l == want).map(|(_, v)| v);
    let mut out = alloc::vec![format!(
        "{}: kronos --gpu: kt lista: lipita-ln (aceleasi {} lansari pe simbol ca lista lipita, cu copierea logit-urilor; LayerNorm pe programul layernorm256, 256 de fire)",
        p,
        kd::TokenList::FusedLn.launches_per_token(layers) + 1
    )];
    let (Some(old), Some(new)) = (pick(kd::TokenList::Fused), pick(kd::TokenList::FusedLn)) else {
        out.push(format!("{}: kronos --gpu: kt ln nou fata de vechi: necomparat (o lista nu a terminat textul)", p));
        journal.push(Event::Value { what: KT_LN_PASS, value: 0 });
        return out;
    };
    let (mut differ, mut total, mut worst, mut finite) = (0usize, 0usize, 0.0f32, true);
    for (a, b) in old.iter().zip(new) {
        total += a.len().max(b.len());
        differ += a.len().abs_diff(b.len());
        for (x, y) in a.iter().zip(b) {
            if x.to_bits() != y.to_bits() {
                differ += 1;
                let d = (x - y).abs();
                if d.is_finite() {
                    worst = worst.max(d);
                } else {
                    finite = false;
                }
            }
        }
    }
    let same_len = old.len() == new.len();
    let same = all.len() >= 2 && all[all.len() - 2] == all[all.len() - 1];
    let pass = same && same_len && finite && worst < LN_NEW_MAX_DIFF;
    out.push(format!(
        "{}: kronos --gpu: kt ln nou fata de vechi: max |d| = {:e} ({} diferite din {} logit-uri, {} / {} pozitii), simboluri identice: {}",
        p,
        worst,
        differ,
        total,
        old.len(),
        new.len(),
        if same { "da" } else { "nu" }
    ));
    out.push(format!(
        "{}: kronos --gpu: kt ln nou fata de vechi: {}",
        p,
        if pass {
            String::from("TRECE (aceleasi simboluri, abaterea sub 1e-4)")
        } else if !same {
            format!("NU TRECE: simbolurile DIFERITE: vechi {:?}, nou {:?}", all.get(all.len().wrapping_sub(2)), all.last())
        } else {
            String::from("NU TRECE: abaterea peste 1e-4 (sau logit-uri nefinite / pozitii lipsa)")
        }
    ));
    journal.push(Event::Value { what: KT_LN_WORST_BITS, value: if finite { worst.to_bits() } else { u32::MAX } });
    journal.push(Event::Value { what: KT_LN_SAME_TOKENS, value: u32::from(same) });
    journal.push(Event::Value { what: KT_LN_PASS, value: u32::from(pass) });
    out
}

/// One position on the board for `kronos_gen::Generation`, and the 4 operations of cyber-gpu's diagnosis
/// (`kronos_diag::TokenIo`, C-139): the launches written, the submissions of the chosen way, the records and the
/// logits read back.
struct BoardRunner<'r, 'b, V: Mmio + ?Sized, K: Clock + ?Sized> {
    p: &'r str,
    vbar: &'r mut V,
    clock: &'r mut K,
    journal: &'r mut Journal,
    st: &'r mut GpuState,
    l: &'r mut Launched,
    permit: Option<&'r HardwareWritePermit>,
    lines: &'r mut Vec<String>,
    doorbell: &'r mut Bell<'b, V>,
    tokens_done: &'r mut usize,
    /// Submissions made in this step (the records' first payload grows per submission: a token may have several).
    subs: &'r mut u32,
    m: &'r OwnedModel,
    vas: &'r ModelVas,
    launches: Mapping,
    rec: Mapping,
    host_logits: SysMapping,
    logits_bytes: u64,
    progs: &'r [(Prog, Mapping)],
    tmaps: &'r [Mapping],
    /// Launch slots: the launches of a token + the copy of the logits down.
    n: usize,
    name: String,
    /// How tokens are sent (chosen by the ladder).
    way: kd::Way,
    /// The logits are read from VRAM (only the copy into RAM was wrong); the copy launch is not sent.
    from_vram: bool,
    /// The position being sent (for the error lines).
    pos: usize,
    /// Board clock of the current token: start of its first record, end of its last, busy time summed.
    timing: Option<(u64, u64, u64)>,
    /// The logits the last token returned (for `kd::explain`).
    last_logits: Vec<f32>,
    /// P6-99: with the check after the board, the logits of every position of the current text (the refused one is
    /// explained with its own logits, not the last token's).
    kept: Option<Vec<Vec<f32>>>,
    /// The board's argmax at every position of the current text.
    argmaxes: Vec<usize>,
    /// P6-95 (C-142): the speed counters (upload + every position).
    speed: ks::Speed,
    /// P6-97 (C-145 point 2): what was last written in the launch slots (`kronos --gpu` only): a token writes (and reads
    /// back) only the words that changed. Forgotten after any error and after any other write through `TokenIo::write`.
    shadow: Option<kf::WriteShadow>,
    /// The slot pieces written under the shadow (VRAM address → bytes), read back once after the texts.
    shadow_ranges: alloc::collections::BTreeMap<u64, usize>,
    /// `kronos --gpu`: the routine journal rows go to the stick once per token (C-145 point 1).
    fast: bool,
    /// P6-97 (C-146): every position of the current text, split (the CPU check is added after).
    split: Vec<kf::TokenSplit>,
    /// P6-98 (C-147 stage 0 (a), `kronos --gpu` only): every launch time of the current token, in order (all its
    /// submissions), then added to `prof` by kind.
    launch_ns: Vec<u64>,
    prof: kp::KindProfile,
    prof_err: Option<String>,
    /// P6-107 (C-154, cyber-gpu C-147 point 7): the driver's diet (only in the cycle's ordinary test): the pushbuffer
    /// written through `push_shadow` and the records not zeroed (except at the first submission of the run).
    diet: bool,
    /// P6-110: the launch list of the run in progress.
    list: kd::TokenList,
    /// The pushbuffer's shadow; forgotten after any error and after any other write through `TokenIo::write`.
    push_shadow: Option<kf::WriteShadow>,
    rec_zeroed: bool,
    /// Per position of the current text: VRAM words written by the driver, the logits read, the argmax (us).
    diet_words: Vec<u64>,
    read_us: Vec<u64>,
    argmax_us: Vec<u64>,
    /// FNV-1a of every logit of the current text.
    print: u32,
    /// P6-119 (C-157 point 3, cyber-gpu `367edf4`): the second diet (only in the cycle with `test-placa … --dieta2`):
    /// two records read on most positions, the planned wait, the fingerprint after the token's clock, the argmax on bits.
    diet2: bool,
    wait_plan: nvidia_ga104::kronos_diet2::WaitPlan,
    /// Per position of the current text: record words read and reads of the wait (all its submissions).
    d2_words: Vec<u32>,
    d2_polls: Vec<u32>,
    /// P6-123 (C-155 point 3): the argmax on the board after the copy (the trial: the token is still chosen on the CPU).
    argmax_board: bool,
    /// P6-123: positions whose 4 words equal `argmax_result` of the copied logits / positions checked (timed runs only).
    am_agree: u32,
    am_total: u32,
    /// P6-123: the first position that did not agree, explained (one line).
    am_first_bad: Option<String>,
    /// P6-123: the argmax launch's time of every position that read all records (ns, board clock).
    am_ns: Vec<u64>,
    /// P6-126 (C-155 point 4): the argmax without the copy (`TopRunner`): the token chosen on the board.
    argmax_top: bool,
    /// P6-126: positions whose board token = the processor's token / positions checked after the board.
    top_agree: u32,
    top_total: u32,
    top_first_bad: Option<String>,
    /// P6-145 (C-157 point 7): the 4 argmax words of every position of the current text (one position per doorbell),
    /// the reference of the batch run.
    top_words: Vec<[u32; 4]>,
    /// P6-145: `submit` keeps the records it read (the batch splits them per position).
    keep_records: bool,
    records: Vec<u8>,
    /// P6-145: host time of the last `submit` spent writing the launch slots and zeroing the records (us).
    write_us: u64,
    /// P6-240 (cyber-gpu `ce8324a`): while set, every `submit` sends its list as ONE `token32` launch.
    t32: Option<T32>,
}

/// P6-240 (cyber-gpu `ce8324a`, note `C-157_P2zz_token32-pe-placa.md`): the pieces of the `token32` runs — the program,
/// the work list (256 B per operation), a page with its constant buffer (+0), QMD (+0x100) and barrier counter (+0x200);
/// G and the SMs it was computed from; what the launches gave (barrier counters checked, the first wrong one, a launch
/// that did not finish).
struct T32 {
    prog: Mapping,
    list: Mapping,
    zone: Mapping,
    groups: u32,
    sms: u32,
    counted: u32,
    counter_ok: u32,
    counter_bad: Option<String>,
    hung: Option<String>,
    /// P6-243 (cyber-gpu `16a0cf5`, P2zzd): with `token32_v2` the list of the token before (only the changed words are
    /// written, `kronos_token::work_list_changes`); `None` = the whole list is written (the first token, or `token32`).
    prev: Option<Vec<u8>>,
    /// P6-243: write only the changed words (set for `token32_v2`).
    diff: bool,
    /// P6-243: words written in the list (all tokens) and tokens sent, for the average words per token.
    words: u64,
    tokens: u64,
    /// P6-252 (cyber-gpu `de8bfbf`, P2zzn): words written for each token in order (the first one = the whole list), for
    /// the row with the first token apart and the median of the others.
    per_token: Vec<u64>,
    /// P6-255 (cyber-gpu `e117394`, P2zzq): with `token32_v2c` the clock area in VRAM (`4 · clock_words(n, G)` bytes).
    clock: Option<Mapping>,
    /// P6-255: the last token sent — its operations, its parts (`token_clock::parts_of`, or why they were refused) and
    /// its board time (ns, from the launch's records), for the `kc` rows of the clock area.
    last: Option<(usize, Result<Vec<nvidia_ga104::token_clock::Part>, String>, u64)>,
    /// P6-257 (cyber-gpu `b93e876`, P2zzt): `token32_v3` — the list is `token_ln::work_list_ln` (every layernorm folded
    /// into the product that reads it; its n, not the launches, goes in the constant buffer and the counter's target)
    /// and the QMD `token32_v3_qmd_wide` (97 registers).
    ln: bool,
    /// P6-257: the last token's operations with `token32_v3` and the launches of the ordinary list.
    ops: Option<(usize, usize)>,
}

const T32_CBUF: u64 = 0;
const T32_QMD: u64 = 0x100;
const T32_SYNC: u64 = 0x200;
/// P6-240: `kronos_token::TOKEN32`'s SHA-256 as the note writes it (checked before anything is sent).
const TOKEN32_SHA256: &str = "40d9749cc8074f639ca0eb3fafae9686b2ca4d2491e868b3b2544ec7faa1ad5f";
/// P6-243: `kronos_token::TOKEN32_V2`'s SHA-256 as note P2zzd writes it (checked before anything is sent).
const TOKEN32_V2_SHA256: &str = "5f4d22a0d01f35d2bd3b88f0e43e492acede1a1a8d912b80fc0712d012f11171";
/// P6-255: `token_clock::TOKEN32_V2C`'s SHA-256 as note P2zzq writes it (checked before anything is sent).
const TOKEN32_V2C_SHA256: &str = "e899fa7cce1d7e31e511f0304bb6d82961c0e13bcd19d86827d58db4b6c18206";
/// P6-257: `token_ln::TOKEN32_V3`'s SHA-256 as note P2zzt writes it (checked before anything is sent).
const TOKEN32_V3_SHA256: &str = "8ecf4fed9ec987af4de86aeba024556f00ce1a7310842e46fa0e10b6e5c5b719";

impl<V: Mmio + ?Sized, K: Clock + ?Sized> BoardRunner<'_, '_, V, K> {
    /// The VRAM address of `bytes` at GPU VA `va` (one of the step's own mappings).
    fn pa_of(&self, va: u64, bytes: u64) -> GpuResult<u64> {
        self.tmaps
            .iter()
            .find(|m| va >= m.va && va.checked_add(bytes).is_some_and(|end| end <= m.va + m.size))
            .map(|m| m.pa + (va - m.va))
            .ok_or(bad("I14: adresa GPU in afara zonelor pasului"))
    }

    /// P6-97: after the texts, the slot pieces written under the shadow are read back once and compared with it: a word
    /// the board changed would mean the shadow is wrong (C-145 §6). Returns how many words differ (`None`: no shadow).
    /// The shadow is dropped either way (the next run starts whole).
    /// P6-145 (C-157 point 7, cyber-gpu note C-157_P7 `0d22357`): the text once more, in batches of at most `k` positions
    /// on ONE doorbell each (`kronos_batch`): a position whose token comes from the argmax before it starts with
    /// `embed_dev`, every position ends with its argmax into its own 16-byte slot right after the logits' buffer. Always
    /// `EachLaunch` (the note: with `Once` the batch gives other tokens without an error, T180f). Judged against the run
    /// with one position per doorbell (`want_seq`, `want_words`): the same tokens and the same 4 argmax words on every
    /// position. A board error (no record, timeout) is the error; a difference is a verdict.
    fn run_batch(&mut self, prompt: &[usize], steps: usize, k: usize, want_seq: &[usize], want_words: &[[u32; 4]]) -> GpuResult<BatchOut> {
        let m = self.m;
        let lpp = kb::launches_per_position(m.layers.len());
        let plan = GenPlan::new(prompt, steps, m.vocab, m.context)?;
        let positions = plan.positions();
        let mut generation = Generation::new(plan);
        let slots = (kb::SLOT as usize) * positions;
        let results_va = self.host_logits.va + self.logits_bytes;
        // Every slot filled with 0xEE first: a slot not written by its argmax is seen (`ArgmaxResult::token`).
        ram_write_at(&self.host_logits, self.logits_bytes, &alloc::vec![0xee; slots]);
        let mut out = BatchOut { k, positions, ..BatchOut::default() };
        let mut words: Vec<[u32; 4]> = Vec::with_capacity(positions);
        let diet2 = core::mem::replace(&mut self.diet2, false); // the planned wait is per position: not for a batch
        self.keep_records = true;
        let r = (|| -> GpuResult<()> {
            while let Some((p0, inputs)) = generation.next_batch(k) {
                let host_t0 = crate::time::since_boot_us();
                let list = kb::batch_launches(kb::TokenList::FusedLn32, &m.shape, m.vocab, m.context, self.vas, results_va, p0, &inputs)?;
                let built = crate::time::since_boot_us();
                let nl = list.len();
                self.pos = p0;
                self.timing = None;
                self.launch_ns.clear();
                kd::TokenIo::submit(self, &list, 0, kd::SeqMode::EachLaunch)?;
                // The next submission's first payload must be above this one's last (`first` grows by STRIDE).
                *self.subs += nl as u32 / STRIDE;
                let sent = crate::time::since_boot_us();
                let bytes = ram_read_at(&self.host_logits, self.logits_bytes, (kb::SLOT as usize * (p0 + inputs.len())) as u64);
                let res = kb::read_results(&bytes, p0, inputs.len())?;
                let host_t1 = crate::time::since_boot_us();
                let recs = core::mem::take(&mut self.records);
                let time = |j: usize| recs.get(16 * j + 8..16 * j + 16).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])));
                let mut outs = Vec::with_capacity(res.len());
                for (j, r) in res.into_iter().enumerate() {
                    let (a, b) = (time(j * lpp), time((j + 1) * lpp));
                    let busy = self.launch_ns.get(j * lpp..(j + 1) * lpp).map_or(b.saturating_sub(a), |t| t.iter().sum());
                    out.board_ns.push(b.saturating_sub(a));
                    words.push(r.words);
                    outs.push(TopOutput { result: r, start_ns: a, end_ns: b, busy_ns: busy });
                }
                let us = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| b.saturating_sub(a)).unwrap_or(0);
                out.bells += 1;
                out.build_us.push(us(host_t0, built));
                out.write_us.push(self.write_us);
                out.send_us.push(us(built, sent).saturating_sub(self.write_us));
                out.read_us.push(us(sent, host_t1));
                out.host_us += us(host_t0, host_t1);
                out.span_ns += time(nl).saturating_sub(time(0));
                if generation.accept_batch(Ok(outs)).is_some() {
                    break;
                }
            }
            Ok(())
        })();
        self.keep_records = false;
        self.records.clear();
        self.diet2 = diet2;
        r?;
        out.stop = generation.stop.as_ref().map(|s| s.line()).unwrap_or_default();
        out.clean = generation.stop.as_ref().is_some_and(|s| s.is_clean_end());
        // The judgement: position by position, the first difference named (no other attempt).
        let seq = &generation.seq;
        let n = want_words.len().min(positions);
        for q in 0..n {
            out.total += 1;
            let tok_ok = seq.get(q + 1).zip(want_seq.get(q + 1)).is_none_or(|(a, b)| a == b);
            if words.get(q) == want_words.get(q) && tok_ok {
                out.agree += 1;
            } else if out.first_bad.is_none() {
                out.first_bad = Some(match words.get(q) {
                    Some(w) => format!("pozitia {}: lot {:08x?}, o pozitie pe trimitere {:08x?}", q, w, want_words[q]),
                    None => format!("pozitia {}: lotul s-a oprit inainte ({})", q, out.stop),
                });
            }
        }
        out.same_seq = seq[..] == want_seq[..];
        Ok(out)
    }

    fn shadow_check(&mut self) -> GpuResult<Option<usize>> {
        let Some(sh) = self.shadow.take() else { return Ok(None) };
        let ranges = core::mem::take(&mut self.shadow_ranges);
        let GpuState { zone, .. } = &*self.st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, zone.vram_size, &zone.protected)?;
        let mut changed = 0usize;
        let mut r: GpuResult<()> = Ok(());
        'all: for (&pa, &len) in &ranges {
            let mut got = Vec::with_capacity(len);
            for i in 0..(len / 4) as u64 {
                match vw.read_u32(pa + 4 * i) {
                    Ok(v) => got.extend_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        r = Err(e);
                        break 'all;
                    }
                }
            }
            match sh.changes(pa, &got) {
                Ok(c) => changed += c.iter().map(|(_, b)| b.len() / 4).sum::<usize>(),
                Err(e) => {
                    r = Err(e);
                    break;
                }
            }
        }
        let back = vw.restore();
        r?;
        back?;
        Ok(Some(changed))
    }
}

/// P6-97 (C-145 point 2): writes `bytes` at `pa` through the shadow: only the words that changed since the last token
/// (each written and read back as before), then remembered. Any error forgets the shadow. `bytes` is padded to whole
/// words with zeros, exactly as `put_bytes` writes it.
fn shadow_put<W: nvidia_ga104::gsp::work_queue::ChannelMemory + ?Sized>(
    w: &mut W,
    sh: &mut kf::WriteShadow,
    ranges: &mut alloc::collections::BTreeMap<u64, usize>,
    pa: u64,
    bytes: &[u8],
) -> GpuResult<()> {
    let mut b = bytes.to_vec();
    b.resize(bytes.len().next_multiple_of(4), 0);
    let r = (|| {
        for (at, range) in sh.changes(pa, &b)? {
            put_bytes(w, at, &b[range])?;
        }
        sh.remember(pa, &b)
    })();
    if r.is_err() {
        sh.forget();
    } else {
        let e = ranges.entry(pa).or_insert(0);
        *e = (*e).max(b.len());
    }
    r
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> BoardRunner<'_, '_, V, K> {
    /// P6-240 (cyber-gpu `ce8324a`, note `C-157_P2zz_token32-pe-placa.md`): `list` as ONE `token32` launch —
    /// `work_list32` written in VRAM, the constant buffer with G at 0x0c (`token_cbuf_for`), the QMD at the SM step for G
    /// (`token32_qmd_wide`), the barrier counter cleared first; a 100 ms limit on the board (as the barrier probe). The
    /// counter is read back after the launch: it must be (n − 1) · G. The token's clock comes from the launch's two records.
    fn submit32(&mut self, list: &[kl::Launch]) -> GpuResult<()> {
        use nvidia_ga104::kronos_token as kt;
        let t = self.t32.as_ref().ok_or(bad("token32: nepregatit"))?;
        let (prog, lm, zone, g, sms, clock, ln) = (t.prog, t.list, t.zone, t.groups, t.sms, t.clock, t.ln);
        // P6-257: with `token32_v3` the folded list and ITS number of operations (63 at #513 instead of 88).
        let (bytes, n) = if ln { nvidia_ga104::token_ln::work_list_ln(list)? } else { (kt::work_list32(list)?, list.len()) };
        if bytes.len() as u64 > lm.size {
            return Err(bad("token32: lista de lucru nu incape in zona ei"));
        }
        // P6-243: with `token32_v2` only the words that differ from the token before (the first token, or another
        // length: the whole list).
        let changes = match (&t.prev, t.diff) {
            (Some(prev), true) if prev.len() == bytes.len() => Some(kt::work_list_changes(prev, &bytes)?),
            _ => None,
        };
        // P6-255: `token32_v2c` = the clock area's address in the constant buffer and ITS QMD (99 registers; the
        // `token32` QMD writes 92 and would break the registers).
        let (cb, q) = match clock {
            Some(c) => {
                if 4 * nvidia_ga104::token_clock::clock_words(n, g)? as u64 > c.size {
                    return Err(bad("token32_v2c: zona ceasului nu incape"));
                }
                (nvidia_ga104::token_clock::token_cbuf_clock(lm.va, zone.va + T32_SYNC, n, g, c.va)?, nvidia_ga104::token_clock::token32_v2c_qmd_wide(prog.va, zone.va + T32_CBUF, g, sms)?)
            }
            // P6-257: `token32_v3` = the same constant buffer, ITS QMD (97 registers).
            None if ln => (kt::token_cbuf_for(lm.va, zone.va + T32_SYNC, n, g)?, nvidia_ga104::token_ln::token32_v3_qmd_wide(prog.va, zone.va + T32_CBUF, g, sms)?),
            None => (kt::token_cbuf_for(lm.va, zone.va + T32_SYNC, n, g)?, kt::token32_qmd_wide(prog.va, zone.va + T32_CBUF, g, sms)?),
        };
        let written = changes.as_ref().map_or(bytes.len() as u64 / 4, |c| c.len() as u64);
        {
            let w = self.st.mem.writer_mut();
            match &changes {
                Some(c) => {
                    for &(off, word) in c {
                        put_bytes(w, lm.pa + off as u64, &word.to_le_bytes())?;
                    }
                }
                None => put_bytes(w, lm.pa, &bytes)?,
            }
            put_bytes(w, zone.pa + T32_CBUF, &cb)?;
            put_bytes(w, zone.pa + T32_QMD, &q)?;
            put_bytes(w, zone.pa + T32_SYNC, &[0u8; 16])?;
            put_bytes(w, self.rec.pa, &[0u8; 32])?;
        }
        if let Some(t) = self.t32.as_mut() {
            t.words += written;
            t.tokens += 1;
            t.per_token.push(written);
            if t.ln {
                t.ops = Some((n, list.len()));
            }
            if t.diff {
                t.prev = Some(bytes);
            }
        }
        // The pushbuffer is written here without its shadow: the shadow no longer knows it.
        if let Some(sh) = self.push_shadow.as_mut() {
            sh.forget();
        }
        let first = TOKEN_FIRST + STRIDE * *self.subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[zone.va + T32_QMD], self.rec.va, first)?;
        let nw = pb.len();
        let mut maps = self.tmaps.to_vec();
        maps.extend([prog, lm, zone]);
        maps.extend(clock);
        let sys = [self.host_logits.clone()];
        let limit = if self.permit.is_some() { BARRIER_LIMIT_US } else { WAIT_US };
        let sent = submit_records(self.vbar, self.clock, self.journal, self.st, self.l, self.permit, &words[..nw], self.rec, 2, first + 1, &maps, &sys, self.doorbell)?;
        let b = match wait_records_within(self.vbar, self.clock, self.journal, self.st, self.l, self.permit, &sent, limit)? {
            Ok(b) => b,
            Err(got) => {
                let why = format!(
                    "G={} \"{}\" pozitia {}: NU A TERMINAT in {} ms (ultima inregistrare = {:#x}, asteptat {:#x})",
                    g,
                    self.name,
                    self.pos,
                    limit / 1000,
                    got,
                    first + 1
                );
                if let Some(t) = self.t32.as_mut() {
                    t.hung = Some(why);
                }
                return Err(GpuError::Timeout { waited_us: limit });
            }
        };
        *self.subs += 1;
        let ns = transfer::analyze_sequence(&b, 1, first)?[0];
        let counter = {
            let GpuState { zone: z, .. } = &*self.st;
            let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, z.vram_size, &z.protected)?;
            let r = vw.read_u32(zone.pa + T32_SYNC);
            let back = vw.restore();
            r.and_then(|v| back.map(|_| v))?
        };
        let want = (n as u64 - 1) * u64::from(g);
        let (pos, name) = (self.pos, self.name.clone());
        // P6-255: the list of the ordinary chain ends with the logits' copy (no argmax on the board): its parts are those
        // of the list without it, the copy is summed apart (`clock_rows`).
        let parts = clock.map(|_| {
            let body = match list.split_last() {
                Some((l, rest)) if l.prog == kl::Prog::Copy => rest,
                _ => list,
            };
            nvidia_ga104::token_clock::parts_of(body).map_err(|e| format!("{} ({} operatii: {:?})", e, n, list.iter().map(|l| l.prog).collect::<Vec<_>>()))
        });
        if let Some(t) = self.t32.as_mut() {
            if let Some(parts) = parts {
                t.last = Some((n, parts, ns));
            }
            t.counted += 1;
            if u64::from(counter) == want {
                t.counter_ok += 1;
            } else if t.counter_bad.is_none() {
                t.counter_bad = Some(format!("\"{}\" pozitia {}: contorul {} in loc de {} ({} operatii)", name, pos, counter, want, n));
            }
        }
        let time = |k: usize| u64::from_le_bytes(b[16 * k + 8..16 * k + 16].try_into().unwrap_or([0; 8]));
        self.timing = Some(match self.timing {
            None => (time(0), time(1), ns),
            Some((s, _, sum)) => (s, time(1), sum + ns),
        });
        Ok(())
    }
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> BoardRunner<'_, '_, V, K> {
    /// P6-255 (cyber-gpu `e117394`, P2zzq): the `kc` rows of the last token sent with `token32_v2c` — its clock area read
    /// back, split per operation (`read_clock`), summed per part of the list (`by_part`), cycles turned into ns with the
    /// same token's board time (`rows`). A refused area is one error row (not an error of the board).
    fn clock_rows(&mut self, g: u32) -> Vec<String> {
        use nvidia_ga104::token_clock as tc;
        let p = self.p;
        let Some((cm, Some((n, parts, ns)))) = self.t32.as_ref().map(|t| (t.clock, t.last.clone())) else {
            return alloc::vec![format!("{}: kronos --gpu: kc token32_v2c G={}: niciun simbol trimis", p, g)];
        };
        let Some(cm) = cm else { return Vec::new() };
        let got = (|| -> GpuResult<Vec<u32>> {
            let words = tc::clock_words(n, g)?;
            let GpuState { zone, .. } = &*self.st;
            let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, zone.vram_size, &zone.protected)?;
            let r = (0..words as u64).map(|i| vw.read_u32(cm.pa + 4 * i)).collect::<GpuResult<Vec<u32>>>();
            let back = vw.restore();
            r.and_then(|v| back.map(|_| v))
        })();
        let rows = got.and_then(|w| tc::read_clock(&w, n, g)).map_err(|e| format!("{}", e)).and_then(|mut r| {
            let parts = parts.map_err(|e| format!("felurile listei refuzate: {}", e))?;
            // The logits' copy at the end (when the list has one more operation than its parts): taken out of the
            // report, and the board time shortened by the copy's share of the windows, so the deduced clock and every
            // ns row stay those of the whole token.
            let mut copy_ns = None;
            let mut board = ns;
            if r.ops.len() == parts.len() + 1 {
                let all: u64 = r.ops.iter().map(|o| u64::from(o.window)).sum();
                let last = r.ops.pop().map_or(0, |o| u64::from(o.window));
                if all > 0 {
                    let c = ((u128::from(last) * u128::from(ns)) / u128::from(all)) as u64;
                    board = ns - c.min(ns);
                    copy_ns = Some(c);
                }
            }
            tc::by_part(&r, &parts).map(|s| (s.rows(board), copy_ns)).map_err(|e| format!("{}", e))
        });
        match rows {
            Ok((rows, copy_ns)) => {
                let mut v = alloc::vec![format!("{}: kronos --gpu: kc token32_v2c G={}: zona ceasului a ultimului simbol ({} operatii, placa {} ns{})", p, g, n, ns, if copy_ns.is_some() { ", ultima = copia logit-urilor, numarata separat" } else { "" })];
                v.extend(rows.iter().map(|(name, value)| format!("{}: kronos --gpu: {} {}", p, name, value)));
                if let Some(c) = copy_ns {
                    v.push(format!("{}: kronos --gpu: kc copie logit fereastra (ns) {}", p, c));
                }
                v
            }
            Err(e) => alloc::vec![format!("{}: kronos --gpu: kc token32_v2c G={}: zona ceasului refuzata: {} (nu e o eroare a placii)", p, g, e)],
        }
    }
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> kd::TokenIo for BoardRunner<'_, '_, V, K> {
    fn submit(&mut self, list: &[kl::Launch], first_slot: usize, mode: kd::SeqMode) -> GpuResult<()> {
        let n = list.len();
        if n == 0 {
            return Ok(());
        }
        if self.t32.is_some() {
            return self.submit32(list);
        }
        if first_slot + n > self.n {
            return Err(bad("I14: mai multe lansari decat locurile lor"));
        }
        let progs = self.progs;
        let prog_va = |want: Prog| progs.iter().find(|(q, _)| *q == want).map(|(_, pm)| pm.va);
        let write_t0 = crate::time::since_boot_us();
        let w = self.st.mem.writer_mut();
        let mut qmds = Vec::with_capacity(n);
        for (i, la) in list.iter().enumerate() {
            let j = (first_slot + i) as u64;
            let (cb_va, cb_pa) = (self.launches.va + 512 * j, self.launches.pa + 512 * j);
            let pv = prog_va(la.prog).ok_or(bad("I14: program necunoscut"))?;
            if let Some(sh) = self.shadow.as_mut() {
                shadow_put(w, sh, &mut self.shadow_ranges, cb_pa, &la.cbuf)?;
                shadow_put(w, sh, &mut self.shadow_ranges, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
            } else {
                put_bytes(w, cb_pa, &la.cbuf)?;
                put_bytes(w, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
            }
            qmds.push(cb_va + 256);
        }
        // P6-107 (C-154): with the diet the records are zeroed only once per run (a guard against what an earlier chain
        // of this power-on left there); after that every submission's payloads are new (`first` grows by STRIDE), the
        // wait asks for the new last payload and `analyze_sequence` checks every record (cyber-gpu T141b).
        if !self.diet || !self.rec_zeroed {
            put_bytes(w, self.rec.pa, &alloc::vec![0u8; 16 * (n + 1)])?;
        }
        self.write_us = write_t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a)).unwrap_or(0);
        let first = TOKEN_FIRST + STRIDE * *self.subs;
        let mut words = alloc::vec![0u32; kd::words(mode, n)];
        let mut pb = PushBuf::new(&mut words);
        kd::encode(mode, &mut pb, &qmds, self.rec.va, first)?;
        let nw = pb.len();
        let last = first + n as u32;
        let sys = [self.host_logits.clone()];
        // P6-119 (C-157 point 3 A+B): with the second diet, the planned wait and, except on the positions that feed the
        // `kp` profile, only the first and the last record.
        let all = !self.diet2 || nvidia_ga104::kronos_diet2::RecordPlan::DIET.reads_all(self.pos);
        let mut polls = 0u32;
        let d2 = self.diet2.then_some(super::gpu_i11::Diet2Wait { plan: &self.wait_plan, all, polls: &mut polls });
        let b = match super::gpu_i11::run_records_d2(
            self.vbar,
            self.clock,
            self.journal,
            self.st,
            self.l,
            self.permit,
            &words[..nw],
            self.rec,
            n as u64 + 1,
            last,
            self.tmaps,
            &sys,
            self.doorbell,
            self.push_shadow.as_mut(),
            d2,
        ) {
            Ok(Ok(b)) => b,
            Err(e) => {
                self.wait_plan.forget();
                return Err(e);
            }
            Ok(Err(got)) => {
                self.wait_plan.forget();
                self.lines.push(format!(
                    "{}: I14: \"{}\" pozitia {}: ultima inregistrare = {:#x} (asteptat {:#x}): placa nu a terminat",
                    self.p, self.name, self.pos, got, last
                ));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        *self.subs += 1;
        if self.keep_records {
            self.records = b.clone();
        }
        // The records were zeroed by this submission (with or without the diet).
        self.rec_zeroed = true;
        let busy: u64 = if all {
            let t = transfer::analyze_sequence(&b, n, first)?;
            if self.fast {
                self.launch_ns.extend_from_slice(&t);
            }
            t.iter().sum()
        } else {
            // P6-119 (C-157 point 3 A): the span from the two ends (the same number, cyber-gpu T147); no launch times.
            nvidia_ga104::kronos_diet2::ends_span(&b[..16], &b[16 * n..16 * (n + 1)], n, first)?
        };
        if self.diet2 {
            self.wait_plan.observe(busy / 1000);
            let words = if all { 4 * (n as u32 + 1) } else { 8 };
            if let (Some(w), Some(p)) = (self.d2_words.last_mut(), self.d2_polls.last_mut()) {
                *w += words;
                *p += polls;
            }
        }
        let time = |k: usize| u64::from_le_bytes(b[16 * k + 8..16 * k + 16].try_into().unwrap_or([0; 8]));
        self.timing = Some(match self.timing {
            None => (time(0), time(n), busy),
            Some((s, _, sum)) => (s, time(n), sum + busy),
        });
        Ok(())
    }

    fn read(&mut self, ranges: &[(u64, usize)]) -> GpuResult<Vec<Vec<u32>>> {
        let pas = ranges.iter().map(|&(va, n)| self.pa_of(va, 4 * n as u64)).collect::<GpuResult<Vec<u64>>>()?;
        let GpuState { zone, .. } = &*self.st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, zone.vram_size, &zone.protected)?;
        let r = ranges
            .iter()
            .zip(&pas)
            .map(|(&(_, n), &pa)| (0..n as u64).map(|i| vw.read_u32(pa + 4 * i)).collect::<GpuResult<Vec<u32>>>())
            .collect::<GpuResult<Vec<Vec<u32>>>>();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))
    }

    fn write(&mut self, va: u64, words: &[u32]) -> GpuResult<()> {
        let pa = self.pa_of(va, 4 * words.len() as u64)?;
        // P6-97: a write from the diagnosis may land in the launch slots: the shadow no longer knows them.
        // P6-112 (C-157 point 2b, proposal 3): only the words written are forgotten (`forget_range`); the ladder writes
        // the activations, not the slots, so position 0 no longer rewrites all of them (~25 000 words, ~17 ms).
        if let Some(sh) = self.shadow.as_mut() {
            sh.forget_range(pa, 4 * words.len() as u64);
        }
        // P6-107: the pushbuffer's shadow too (P6-112: likewise only the words written).
        if let Some(sh) = self.push_shadow.as_mut() {
            sh.forget_range(pa, 4 * words.len() as u64);
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        put_bytes(self.st.mem.writer_mut(), pa, &bytes)
    }

    fn journal(&mut self) -> &mut Journal {
        &mut *self.journal
    }
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> TokenRunner for BoardRunner<'_, '_, V, K> {
    fn run_token(&mut self, tok: usize, pos: usize) -> GpuResult<TokenOutput> {
        let (m, vas, way) = (self.m, self.vas, self.way);
        let weights = m.weights();
        self.pos = pos;
        self.timing = None;
        self.launch_ns.clear();
        let extra = if self.from_vram {
            Vec::new()
        } else {
            let am = if self.argmax_board { nvidia_ga104::argmax::RESULT_BYTES } else { 0 };
            ram_write(&self.host_logits, &alloc::vec![0xee; (self.logits_bytes + am) as usize]);
            let mut x = alloc::vec![kl::copy_launch(vas.logits, self.host_logits.va, self.logits_bytes)?];
            // P6-123 (C-155 point 3): the argmax rides in the same token, right after the copy (the note's trial).
            if self.argmax_board {
                x.push(nvidia_ga104::argmax::launch(vas.logits, self.host_logits.va + self.logits_bytes, m.vocab)?);
            }
            x
        };
        let host_t0 = crate::time::since_boot_us();
        let journal_t0 = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed);
        let words0 = self.st.vram_words;
        if self.diet2 {
            self.d2_words.truncate(pos);
            self.d2_words.push(0);
            self.d2_polls.truncate(pos);
            self.d2_polls.push(0);
        }
        kd::send_token_list(self, self.list, &weights, vas, tok, pos, way, &extra)?;
        *self.tokens_done += 1;
        let words = self.st.vram_words.saturating_sub(words0);
        let read_t0 = crate::time::since_boot_us();
        let bits: Vec<u32> = if self.from_vram {
            let got = kd::TokenIo::read(self, &[(vas.logits, m.vocab)])?;
            got.into_iter().next().unwrap_or_default()
        } else {
            ram_read(&self.host_logits, 4 * m.vocab as u64).chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
        };
        let logits: Vec<f32> = bits.iter().map(|&b| f32::from_bits(b)).collect();
        let read_t1 = crate::time::since_boot_us();
        // P6-123 (C-155 point 3): the board's 16 bytes, read after the token's logits (outside the read's clock row).
        // P6-126 (cyber-gpu note C-157_P2f, proposal 1): only those 16 bytes (not the whole 64 KiB arena again), and
        // their check (`agrees`) after the argmax row's clock below: the trial's result is the same.
        let am_bytes = (self.argmax_board && !self.from_vram)
            .then(|| ram_read_at(&self.host_logits, self.logits_bytes, nvidia_ga104::argmax::RESULT_BYTES));
        self.argmaxes.truncate(pos);
        // P6-119 (C-157 point 3 D): with the second diet, the argmax on the bits (integer compares, the same choice).
        let am = if self.diet2 { gpu_compute::model::argmax_bits(&bits) } else { cm::argmax(&logits) };
        self.argmaxes.push(am.unwrap_or(usize::MAX));
        let argmax_t1 = crate::time::since_boot_us();
        if let Some(b) = am_bytes.as_ref().filter(|_| self.fast) {
            // Threshold 0: the 4 words exactly as the oracle computes them from the same copied logits, and the token
            // the same as the processor's argmax. Never a stop: the token is still the processor's (the trial).
            let r = nvidia_ga104::argmax::ArgmaxResult::from_bytes(b)?;
            let tok_ok = matches!(r.token(m.vocab), Ok(t) if t == am);
            let ok = r.agrees(&logits) && tok_ok;
            self.am_total += 1;
            if ok {
                self.am_agree += 1;
            } else if self.am_first_bad.is_none() {
                let want = gpu_compute::model::argmax_result(&logits);
                self.am_first_bad = Some(format!(
                    "\"{}\" pozitia {}: placa [{:#x}, {:#x}, {}, {}], procesorul [{:#x}, {:#x}, {}, {}]",
                    self.name, pos, r.words[0], r.words[1], r.words[2], r.words[3], want[0], want[1], want[2], want[3]
                ));
            }
        }
        if self.fast {
            // P6-107 (C-154 point 2): measurement only (nothing changed): the logits read from RAM, the argmax.
            let us = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| b.saturating_sub(a)).unwrap_or(0);
            self.diet_words.truncate(pos);
            self.diet_words.push(words);
            self.read_us.truncate(pos);
            self.read_us.push(us(read_t0, read_t1));
            self.argmax_us.truncate(pos);
            self.argmax_us.push(us(read_t1, argmax_t1));
            // P6-119 (C-157 point 3 C): with the second diet the fingerprint is taken after the token's clock stops.
            if !self.diet2 {
                self.print = fnv1a(self.print, &logits);
            }
        }
        self.last_logits = logits.clone();
        if let Some(k) = self.kept.as_mut() {
            k.truncate(pos);
            k.push(logits.clone());
        }
        let (start_ns, end_ns, busy_ns) = self.timing.unwrap_or((0, 0, 0));
        let out = TokenOutput { logits, start_ns, end_ns, busy_ns };
        // P6-97 (C-145 point 1): the routine rows of this token are handed to the stick (P6-98: written every 16 tokens
        // or every second, `gpu_i6::live_flush`), inside the token's time.
        if self.fast {
            self.journal.flush_live();
            // P6-98 (C-147 stage 0 (a)): this token's launch times by kind; a token that does not fit is left out (once
            // noted), never guessed.
            let (embed, down) = (way.embed == kd::EmbedBy::Program, !self.from_vram);
            let kinds = match self.list {
                kd::TokenList::Plain => kp::token_kinds(m.layers.len(), embed, down),
                // `Fused` and `FusedLn` (P6-114): the same 88 launches and kinds, only the LayerNorm program differs.
                _ => kp::token_kinds_fused(m.layers.len(), embed, down),
            };
            // P6-119: a position of the second diet that read only the two ends has no launch times: left out, not guessed.
            let skip = self.diet2 && self.launch_ns.is_empty();
            // P6-123: the argmax launch is the last one of the token; the kinds' profile has no kind for it (the note):
            // its time is kept on its own and the profile gets the same launches as without it.
            if self.argmax_board && !self.from_vram && !self.launch_ns.is_empty() {
                if let Some(t) = self.launch_ns.pop() {
                    self.am_ns.push(t);
                }
            }
            let added = if skip { Ok(()) } else { self.prof.add_token(&kinds, &self.launch_ns) };
            if let Err(e) = added {
                self.prof_err.get_or_insert_with(|| format!("pozitia {}: {} ({} lansari, {} timpi)", pos, e, kinds.len(), self.launch_ns.len()));
            }
        }
        // P6-95: the OS clock from the first launch written to the logits read (without the CPU check, C-142 point 3).
        let host = host_t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a));
        if self.fast && self.diet2 {
            self.print = fnv1a(self.print, &out.logits);
        }
        let _ = self.speed.add_token(pos, &out, host); // a board clock that went back is counted by Generation itself
        if self.fast {
            // P6-97 (C-146): the split. The board's span is on the board's clock: never more than the OS's own span
            // of the same token (the two clocks are not the same clock).
            let host_us = host.unwrap_or(0);
            let journal_us = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(journal_t0);
            self.split.push(kf::TokenSplit {
                pos,
                host_us,
                board_us: (end_ns.saturating_sub(start_ns) / 1000).min(host_us),
                journal_us: journal_us.min(host_us),
                screen_us: 0, // nothing is printed while a token runs (the text is shown at the end)
                check_us: 0,
            });
        }
        Ok(out)
    }
}

/// P6-126 (C-155 point 4, cyber-gpu notes `C-155_argmax-pe-placa.md` point 4 and `C-157_P2f`): one token whose last
/// launch is the `argmax` program alone (in the copy's place): the OS reads only its 16 bytes and the token is the
/// board's. The same bookkeeping as `run_token` (clock, speed, split, kinds' profile without the copy's kind); no
/// logits, so no print and no CPU check here (it runs after the board, in `run_model`).
impl<V: Mmio + ?Sized, K: Clock + ?Sized> TopRunner for BoardRunner<'_, '_, V, K> {
    fn run_top(&mut self, tok: usize, pos: usize) -> GpuResult<TopOutput> {
        let (m, vas, way) = (self.m, self.vas, self.way);
        let weights = m.weights();
        self.pos = pos;
        self.timing = None;
        self.launch_ns.clear();
        let am = nvidia_ga104::argmax::RESULT_BYTES;
        // The result's 16 bytes filled with 0xEE first: a launch that did not write is seen (`ArgmaxResult::token`).
        // P6-138 (note C-157_P2j, point 4): only those 16 bytes (the logits are never copied here).
        ram_write_at(&self.host_logits, self.logits_bytes, &[0xee; nvidia_ga104::argmax::RESULT_BYTES as usize]);
        let extra = alloc::vec![nvidia_ga104::argmax::launch(vas.logits, self.host_logits.va + self.logits_bytes, m.vocab)?];
        let host_t0 = crate::time::since_boot_us();
        let journal_t0 = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed);
        let words0 = self.st.vram_words;
        // P6-138 (note C-157_P2j, point 2): the second diet's counters of this position (else `submit` adds them to
        // the token before).
        if self.diet2 {
            self.d2_words.truncate(pos);
            self.d2_words.push(0);
            self.d2_polls.truncate(pos);
            self.d2_polls.push(0);
        }
        kd::send_token_list(self, self.list, &weights, vas, tok, pos, way, &extra)?;
        *self.tokens_done += 1;
        let words = self.st.vram_words.saturating_sub(words0);
        let read_t0 = crate::time::since_boot_us();
        let bytes = ram_read_at(&self.host_logits, self.logits_bytes, am);
        let read_t1 = crate::time::since_boot_us();
        let result = nvidia_ga104::argmax::ArgmaxResult::from_bytes(&bytes)?;
        let top = result.token(m.vocab).ok().flatten();
        self.argmaxes.truncate(pos);
        self.argmaxes.push(top.unwrap_or(usize::MAX));
        self.top_words.truncate(pos);
        self.top_words.push(result.words);
        let us = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| b.saturating_sub(a)).unwrap_or(0);
        self.diet_words.truncate(pos);
        self.diet_words.push(words);
        self.read_us.truncate(pos);
        self.read_us.push(us(read_t0, read_t1));
        self.argmax_us.truncate(pos);
        self.argmax_us.push(0); // no argmax on the processor: the board chose the token
        let (start_ns, end_ns, busy_ns) = self.timing.unwrap_or((0, 0, 0));
        self.journal.flush_live();
        // The kinds' profile: the token's launches without the copy's kind; the argmax launch (the last) on its own.
        let kinds = match self.list {
            kd::TokenList::Plain => kp::token_kinds(m.layers.len(), way.embed == kd::EmbedBy::Program, false),
            _ => kp::token_kinds_fused(m.layers.len(), way.embed == kd::EmbedBy::Program, false),
        };
        // P6-138 (note C-157_P2j, point 3): a second-diet position that read only the two ends has no launch times:
        // left out of the profile and of the argmax median (fewer samples, none guessed).
        let skip = self.diet2 && self.launch_ns.is_empty();
        if let Some(t) = self.launch_ns.pop() {
            self.am_ns.push(t);
        }
        let added = if skip { Ok(()) } else { self.prof.add_token(&kinds, &self.launch_ns) };
        if let Err(e) = added {
            self.prof_err.get_or_insert_with(|| format!("pozitia {}: {} ({} lansari, {} timpi)", pos, e, kinds.len(), self.launch_ns.len()));
        }
        let host = host_t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a));
        let out = TokenOutput { logits: Vec::new(), start_ns, end_ns, busy_ns };
        let _ = self.speed.add_token(pos, &out, host);
        let host_us = host.unwrap_or(0);
        let journal_us = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(journal_t0);
        self.split.push(kf::TokenSplit {
            pos,
            host_us,
            board_us: (end_ns.saturating_sub(start_ns) / 1000).min(host_us),
            journal_us: journal_us.min(host_us),
            screen_us: 0,
            check_us: 0,
        });
        Ok(TopOutput { result, start_ns, end_ns, busy_ns })
    }
}

/// P6-145: what one batch run gave (one batch size `k`).
#[derive(Default)]
struct BatchOut {
    k: usize,
    positions: usize,
    /// Doorbells rung (= batches sent).
    bells: u32,
    /// Board time of every position (from its first record to its last, ns).
    board_ns: Vec<u64>,
    /// Per batch (us, OS clock): building the launches, writing the slots, the rest of the submission (pushbuffer,
    /// doorbell, wait, records), reading the results.
    build_us: Vec<u64>,
    write_us: Vec<u64>,
    send_us: Vec<u64>,
    read_us: Vec<u64>,
    host_us: u64,
    /// Board span of all batches (first record to last, ns).
    span_ns: u64,
    agree: u32,
    total: u32,
    first_bad: Option<String>,
    same_seq: bool,
    clean: bool,
    stop: String,
}

/// The CPU engine on the same weights, for `Generation`'s judgement of every position.
struct CpuCheck<'a, 'w> {
    w: &'a cm::ModelWeights<'w>,
    caches: Caches,
    /// P6-95: the CPU engine's time of every position (us, OS clock).
    us: Vec<u64>,
}

impl TokenCheck for CpuCheck<'_, '_> {
    fn cpu_logits(&mut self, token: usize, pos: usize) -> GpuResult<Vec<f32>> {
        let t0 = crate::time::since_boot_us();
        let r = cm::step_cpu(self.w, &mut self.caches, token, pos);
        if let Some(d) = t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a)) {
            self.us.push(d);
        }
        r
    }
}

/// P6-99 (C-148 point 2): the CPU check after the board, by `/bin/kronos`'s fast engine: for every kept position, in
/// order, the question goes through [`OS_CHECK`] and the chain waits (at most [`OS_CHECK_WAIT_US`]) for the logits.
/// No answer, or a program that could not compute them, is an error of that position (`Stop::Eroare`), never a pass.
struct OsCheck {
    /// The wait of every position (us, OS clock): the program's engine + the hand-over.
    us: Vec<u64>,
    /// Why the check could not judge (the error's own text is a fixed one).
    why: Option<String>,
}

impl TokenCheck for OsCheck {
    fn cpu_logits(&mut self, token: usize, pos: usize) -> GpuResult<Vec<f32>> {
        let t0 = crate::time::since_boot_us().unwrap_or(0);
        {
            let mut c = OS_CHECK.lock();
            c.want = Some((token, pos));
            c.handed = false;
            c.answer = None;
        }
        let got = wait_for(t0 + OS_CHECK_WAIT_US, || OS_CHECK.lock().answer.take());
        OS_CHECK.lock().want = None;
        if let Some(d) = crate::time::since_boot_us().map(|b| b.saturating_sub(t0)) {
            self.us.push(d);
        }
        match got {
            Some(Ok(v)) => Ok(v),
            Some(Err(why)) => {
                self.why = Some(format!("pozitia {}: /bin/kronos nu a putut calcula logit-urile: {}", pos, why));
                Err(bad("verificarea dupa placa: /bin/kronos nu a putut calcula pozitia"))
            }
            None => {
                self.why = Some(format!("pozitia {}: niciun raspuns de la /bin/kronos in {} s", pos, OS_CHECK_WAIT_US / 1_000_000));
                Err(bad("verificarea dupa placa: /bin/kronos nu a raspuns la timp"))
            }
        }
    }
}

/// Step `i14-simbol` with the model file at `path` (the control model `mic_q8_0.kgr`); then a waiting `kronos --gpu`
/// request, if any (P6-62 step 3).
#[allow(clippy::too_many_arguments)]
pub(super) fn token_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    path: &str,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<()> {
    let mut tokens_done = 0usize;
    let mut subs = 0u32;
    let mut way: Option<kd::Way> = None;
    let r = (|| {
        let file = crate::fs::read_all(path).map_err(|_| {
            lines.push(format!("{}: I14: nu gasesc modelul de control {} (stick-ul de test il are in /kronos/)", p, path));
            bad("I14: lipseste modelul de control")
        })?;
        let sum = sha256_hex(&file);
        if sum != MIC_SHA256 {
            lines.push(format!("{}: I14: {}: SHA-256 {} difera de vectori.json ({})", p, path, sum, MIC_SHA256));
            return Err(bad("I14: modelul de control nu e cel din vectori.json"));
        }
        let m = read_kgr1(&file)?;
        drop(file);
        let n = km::launches_per_token(m.layers.len()) + 1;
        lines.push(format!(
            "{}: i14: modelul de control {} (SHA-256 ca vectori.json; greutati la intamplare, NU #513): {} straturi, E {}, {} capete, vocabular {}, context {}; {} lansari pe simbol + 1 copiere a logit-urilor",
            p, path, m.layers.len(), m.shape.e, m.shape.heads, m.vocab, m.context, n - 1
        ));
        let runs: Vec<Run<'_>> = MIC_CONTROLS
            .iter()
            .map(|c| Run {
                name: String::from(c.text),
                prompt: c.prompt.to_vec(),
                steps: c.greedy.len(),
                argmax: Some(c.argmax),
                greedy: Some(&c.greedy),
                os_check: false,
                list: kd::TokenList::Plain,
            })
            .collect();
        let out = run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, &mut tokens_done, &mut subs, &mut way)?;
        let chosen = way.ok_or(bad("I14: scara nu a ales un fel"))?;
        lines.push(format!(
            "{}: i14: {} simboluri pe placa in {} trimiteri ({} lansari pe simbol), trimise cu: {}; abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii)",
            p, tokens_done, subs, n, chosen.name(), out.worst, out.median
        ));
        // C-139, the honesty rule: a way of SENDING the same launches is a repair; going AROUND a launch is not.
        if chosen.embed == kd::EmbedBy::Driver {
            return Err(bad("I14: programul embed e gresit pe placa; cu embedding-ul scris de driver textele trec"));
        }
        if out.from_vram {
            return Err(bad("I14: copierea logit-urilor in RAM e gresita; cu logit-urile din VRAM textele trec"));
        }
        Ok(chosen)
    })();
    match &r {
        Ok(chosen) => lines.push(format!(
            "{}: pas I14 simbol: ok (modelul de control, {} simboluri comparate cu procesorul si cu vectorii lui Kronos, trimise cu: {}; #513 prin kronos --gpu)",
            p, tokens_done, chosen.name()
        )),
        Err(err) => {
            lines.push(format!("{}: pas I14 simbol: ESEC: {} (oprit la prima eroare, dupa {} simboluri)", p, err, tokens_done))
        }
    }
    // P6-62 step 3: the `kronos --gpu` request runs only after the control model passed. Its own failure does not fail
    // I14 (the chain still cleans up the same way); `/bin/kronos` then continues on the processor.
    let job = KRONOS_JOB.lock().take();
    if let Some(job) = job {
        let res = match &r {
            Ok(chosen) => kronos_job(p, vbar, clock, journal, st, l, &job, permit, lines, doorbell, &mut tokens_done, &mut subs, *chosen),
            Err(err) => Err(format!("modelul de control nu a trecut pe placa ({})", err)),
        };
        match &res {
            Ok(t) => lines.push(format!(
                "{}: kronos --gpu: {} simboluri generate pe placa (fiecare comparat cu procesorul{})",
                p,
                t.len(),
                if job.os_check { ", dupa placa" } else { "" }
            )),
            Err(why) => lines.push(format!("{}: kronos --gpu: ESEC: {} -> kronos continua pe procesor", p, why)),
        }
        *KRONOS_RESULT.lock() = Some(res);
    }
    r.map(|_| ())
}

/// The `kronos --gpu` request: the model from its path, the prompt + `n` greedy tokens on the board.
#[allow(clippy::too_many_arguments)]
fn kronos_job<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    job: &KronosJob,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    tokens_done: &mut usize,
    subs: &mut u32,
    way: kd::Way,
) -> Result<Vec<usize>, String> {
    let file = crate::fs::read_all(&job.path).map_err(|e| format!("nu pot citi {}: {}", job.path, e.message()))?;
    // P6-177 (C-157 point 8, cyber-gpu note `C-157_P8_fisa-modelului`): the model card picks the compute pack before
    // anything is uploaded; no pack → the refusal rows and nothing runs on the board. "PROBA NECESARA" needs nothing
    // more here: every `kronos --gpu` position is already judged against the processor (in the kernel or, with
    // `os_check`, by `/bin/kronos` after the board) — the proof the note asks for.
    if let Err(why) = model_pack(p, &file, lines) {
        return Err(format!("{}: {}", job.path, why));
    }
    let m = read_kgr1(&file).map_err(|e| format!("{}: {}", job.path, e))?;
    if let Some(why) = card_vs_tensors(&file, &m) {
        lines.push(format!("{}: kronos --gpu: {}", p, why));
        return Err(format!("{}: fisa modelului nu se potriveste cu tensorii: nimic nu ruleaza pe placa", job.path));
    }
    drop(file);
    if job.prompt.is_empty() || job.n == 0 {
        return Err(String::from("cererea nu are simboluri sau n = 0"));
    }
    if let Some(bad_id) = job.prompt.iter().find(|&&t| t >= m.vocab) {
        return Err(format!("simbolul {} e in afara vocabularului ({})", bad_id, m.vocab));
    }
    if job.prompt.len() >= m.context {
        return Err(format!("textul are {} simboluri, contextul modelului {}", job.prompt.len(), m.context));
    }
    lines.push(format!(
        "{}: kronos --gpu: {} ({} straturi, E {}, vocabular {}, context {}): {} simboluri date, {} de generat",
        p, job.path, m.layers.len(), m.shape.e, m.vocab, m.context, job.prompt.len(), job.n
    ));
    let name = job.path.rsplit('/').next().unwrap_or(&job.path);
    journal.push(Event::text(KG_MODEL, name.as_bytes()));
    journal.push(Event::Value { what: KG_ASKED, value: job.n as u32 });
    let mut runs = alloc::vec![Run {
        name: String::from("kronos --gpu"),
        prompt: job.prompt.clone(),
        steps: job.n,
        argmax: None,
        greedy: None,
        os_check: job.os_check,
        list: kd::TokenList::Plain,
    }];
    // P6-110 (C-155 point 1): with P3 asked (`test-placa … --lipit`), the same text again with the glued list.
    // P6-112 (center's note 21 Nov 13:00): otherwise, in the cycle, the glued list alone (it passed on the board, bit for
    // bit, in os-328); `test-placa … --simplu` keeps the plain list alone, as in os-324. By hand: the plain list.
    // P6-114 (C-155 point 2): `test-placa … --ln-nou` = the glued list with the old LayerNorm (as in os-329), then the
    // same text with the new one (`layernorm256`), compared against the note's threshold.
    // P6-120 (center's proposal 21 Nov 19:25): the cycle's default is now the new LayerNorm alone (`--ln-vechi` = the old).
    // P6-124 (C-155 point 4): `test-placa … --sferturi` = the glued list (old LayerNorm), then the same text on quarters.
    // P6-128 (C-157 point 2, note C-157_P2g): `test-placa … --ln-32` = `lipita-ln` (the default), then `lipita-ln-32`.
    let list_kind = if super::gpu_tur::LN32_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs[0].name = String::from("kronos --gpu (lista lipita, LayerNorm nou)");
        runs[0].list = kd::TokenList::FusedLn;
        runs.push(Run { name: String::from("kronos --gpu (lista lipita-ln-32, toate inmultirile pe sferturi)"), list: kd::TokenList::FusedLn32, ..runs[0].clone() });
        6
    } else if super::gpu_tur::QUARTERS_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs[0].name = String::from("kronos --gpu (lista lipita)");
        runs[0].list = kd::TokenList::Fused;
        runs.push(Run { name: String::from("kronos --gpu (lista lipita pe sferturi)"), list: kd::TokenList::Fused32, ..runs[0].clone() });
        5
    } else if super::gpu_tur::LN_NEW_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs[0].name = String::from("kronos --gpu (lista lipita)");
        runs[0].list = kd::TokenList::Fused;
        runs.push(Run { name: String::from("kronos --gpu (lista lipita, LayerNorm nou)"), list: kd::TokenList::FusedLn, ..runs[0].clone() });
        3
    } else if super::gpu_tur::FUSED_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs.push(Run { name: String::from("kronos --gpu (lista lipita)"), list: kd::TokenList::Fused, ..runs[0].clone() });
        2
    } else if super::gpu_tur::FUSED_ONLY_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs[0].name = String::from("kronos --gpu (lista lipita)");
        runs[0].list = kd::TokenList::Fused;
        1
    } else if super::gpu_tur::LN_ONLY_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        runs[0].name = String::from("kronos --gpu (lista lipita, LayerNorm nou)");
        runs[0].list = kd::TokenList::FusedLn;
        4
    } else if super::gpu_tur::LN32_ONLY_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        // P6-135 (note C-157_P2i, point 1): `lipita-ln-32` alone, the cycle's default from this Release on.
        runs[0].name = String::from("kronos --gpu (lista lipita-ln-32, toate inmultirile pe sferturi)");
        runs[0].list = kd::TokenList::FusedLn32;
        7
    } else {
        0
    };
    journal.push(Event::Value { what: KT_LIST, value: list_kind });
    let dropped0 = super::gpu_i6::DROPPED.load(core::sync::atomic::Ordering::Relaxed);
    let writes0 = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed);
    // P6-97 (C-145 point 1): the routine rows (window moves, TLB invalidates, doorbell and their notes) are held in
    // memory during the upload and the text and go to the stick once per token; every other row as before.
    journal.set_routine(Some(kf::gen_routine(&GEN_NOTES)));
    let run = run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, tokens_done, subs, &mut Some(way));
    let live = journal.live_stats();
    journal.set_routine(None);
    let mut out = run.map_err(|e| format!("{}", e))?;
    let writes = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(writes0);
    if out.from_vram {
        return Err(String::from("copierea logit-urilor in RAM e gresita (logit-urile citite din VRAM)"));
    }
    let (mut all, worst, median) = (out.all, out.worst, out.median);
    let many = runs.len() > 1;
    lines.push(format!(
        "{}: kronos --gpu: abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii{})",
        p,
        worst,
        median,
        if many { "; mediana cea mai mare dintre rulari, vezi kt rularile" } else { "" }
    ));
    // P6-120 (cyber-gpu note C-157_P2d, point 3): the list run alone, with the copy of the logits in the count (88 at #513).
    // P6-135: 7 = `lipita-ln-32` alone (reference print 0x6c8501f0, cycle 22 Nov 10:06).
    if list_kind == 1 || list_kind == 4 || list_kind == 7 {
        lines.push(format!(
            "{}: kronos --gpu: kt lista: {} singura ({} lansari pe simbol, {}; amprenta de referinta {})",
            p,
            match list_kind {
                7 => "lipita-ln-32 (LayerNorm nou + toate inmultirile pe sferturi)",
                4 => "lipita-ln (LayerNorm nou, layernorm256)",
                _ => "lipita (LayerNorm vechi)",
            },
            runs[0].list.launches_per_token(m.layers.len()) + 1,
            // P6-138 (note C-157_P2j, point 4): without the copy the last launch is the argmax program.
            if out.diet.as_ref().is_some_and(|d| d.top) {
                "ultima e programul argmax, fara copierea logit-urilor"
            } else {
                "cu copierea logit-urilor"
            },
            // P6-122: the reference prints are #513's (12 layers); a deeper model (#651, 24) has none yet.
            if m.layers.len() > 12 {
                "doar la #513; acest model nu are inca una"
            } else {
                match list_kind {
                    7 => "0x6c8501f0",
                    4 => "0x46a07914",
                    _ => "0xda9ae084",
                }
            }
        ));
    }
    // P6-120 (note C-157_P2d, point 2): with two runs on the same text, which run every row speaks of.
    if many {
        let each: Vec<String> = runs
            .iter()
            .enumerate()
            .map(|(i, r)| format!("rularea {} = {} ({} ns mediana pe simbol)", i + 1, r.name, out.run_medians.get(i).map_or(String::from("?"), |v| format!("{}", v))))
            .collect();
        lines.push(format!(
            "{}: kronos --gpu: kt rularile: {} rulari pe acelasi text: {}; randurile viteza (placa / cu driverul / procesorul) = toate rularile; randurile kt, kp, dieta si amprenta = rularea {} (ultima)",
            p,
            runs.len(),
            each.join("; "),
            runs.len()
        ));
    }
    let made = all.last().map_or(0, |g| g.len());
    // P6-95 (C-141): the result and the speed in the gpu journal (it goes up to the hub by itself) and on the screen.
    journal.push(Event::Value { what: KG_MADE, value: made as u32 });
    journal.push(Event::Value { what: KG_FIRST_DIFF, value: u32::MAX });
    journal.push(Event::Value { what: KG_WORST, value: (worst as f64 * 1e6).min(u32::MAX as f64) as u32 });
    out.speed.journal(journal, false);
    let (cpu_med, cpu_x100) = cpu_speed(&out.cpu_us);
    let row = |v: Option<u64>| v.map_or(u32::MAX, |v| v.min(u32::MAX as u64) as u32);
    // P6-99 (C-148 point 2): with the check after the board, its own rows (other names) + the speed with the check.
    let with_check = with_check_x100(&out.split);
    if out.os_check {
        journal.push(Event::Value { what: KV_OS_MED, value: row(cpu_med) });
        journal.push(Event::Value { what: KV_OS_X100, value: row(cpu_x100) });
        journal.push(Event::Value { what: KV_WITH_CHECK_X100, value: row(with_check) });
    } else {
        journal.push(Event::Value { what: KV_CPU_MED, value: row(cpu_med) });
        journal.push(Event::Value { what: KV_CPU_X100, value: row(cpu_x100) });
    }
    lines.extend(out.speed.lines().into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let x100 = |v: Option<u64>| v.map_or(String::from("?"), |x| format!("{}.{:02}", x / 100, x % 100));
    let check_part = if out.os_check {
        format!(
            "verificarea dupa placa (motorul rapid /bin/kronos) median {} us/pozitie = {} pozitii/s; cu verificare {} simboluri/s",
            cpu_med.map_or(String::from("?"), |v| format!("{}", v)),
            x100(cpu_x100),
            x100(with_check)
        )
    } else {
        format!(
            "motorul procesorului (nucleul, 1 nucleu) median {} us pe pozitie = {} simboluri/s",
            cpu_med.map_or(String::from("?"), |v| format!("{}", v)),
            x100(cpu_x100)
        )
    };
    lines.push(format!(
        "{}: kronos --gpu: viteza{}: placa {} simboluri/s (ceasul placii), cu driverul {} simboluri/s; {}",
        p,
        if many { format!(" (toate cele {} rulari)", runs.len()) } else { String::new() },
        x100(out.speed.summary().board_per_s_x100),
        x100(out.speed.summary().host_per_s_x100),
        check_part
    ));
    // P6-97 (C-146): the time per token, split (board / driver / journal / screen + the CPU check), in the gpu journal
    // (`kt …`) and in the output (so also in the cycle's report).
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let mut split = kf::TimeSplit::new();
    for t in &out.split {
        let _ = split.add(*t); // parts are clamped to the whole in `run_token`
    }
    split.journal(journal);
    // P6-110 (C-157 point 2b, cyber-gpu `46078f4`): the time of the slow positions (over 2 x the median), clock reads only.
    split.journal_slow(journal);
    // P6-112: the routine rows dropped from the memory ring during the texts (0 = the stick has all of them).
    let dropped = super::gpu_i6::DROPPED.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(dropped0);
    journal.push(Event::Value { what: KT_DROPPED, value: dropped.min(u32::MAX as usize) as u32 });
    lines.push(format!(
        "{}: kronos --gpu: jurnalul in generare: nicio scriere pe stick intre pozitii (randurile de rutina scrise la final); {} randuri de rutina aruncate",
        p, dropped
    ));
    lines.extend(split.slow_lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    if let Some(c) = out.shadow_changed {
        journal.push(Event::Value { what: KT_SHADOW, value: c.min(u32::MAX as usize) as u32 });
        lines.push(format!(
            "{}: kronos --gpu: umbra lansarilor: {}",
            p,
            if c == 0 { String::from("recitite la final, identice (placa nu rescrie sloturile)") } else { format!("{} cuvinte schimbate de placa in sloturi (umbra NU e sigura)", c) }
        ));
    }
    lines.extend(split.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    // P6-107 (C-154 points 2-3): the diet's rows (with or without it, so two runs compare).
    if let Some(d) = &out.diet {
        let words = median_of(&d.words);
        let (read, argmax) = (median_of(&d.read_us), median_of(&d.argmax_us));
        journal.push(Event::Value { what: KT_DIET, value: u32::from(d.on) });
        journal.push(Event::Value { what: KT_VRAM_WORDS, value: row(words) });
        journal.push(Event::Value { what: KT_READ, value: row(read) });
        journal.push(Event::Value { what: KT_ARGMAX, value: row(argmax) });
        // P6-126: without the copy there is no deviation and no print (the logits stayed on the board).
        if !d.top {
            journal.push(Event::Value { what: KT_WORST_BITS, value: worst.to_bits() });
            journal.push(Event::Value { what: KT_PRINT, value: d.print });
        }
        let n = |v: Option<u64>| v.map_or(String::from("?"), |v| format!("{}", v));
        lines.push(format!(
            "{}: kronos --gpu: dieta driverului: {}; cuvinte VRAM scrise pe simbol (median) {}; citirea logit-urilor {} us, argmax {} us (median)",
            p,
            if d.on { "PORNITA (pushbuffer prin umbra, inregistrari nezerorizate)" } else { "oprita" },
            n(words),
            n(read),
            n(argmax)
        ));
        if d.top {
            lines.push(format!(
                "{}: kronos --gpu: logit-urile: necitite (argmax fara copie, {} pozitii): fara amprenta si fara abatere in aceasta rulare",
                p,
                d.words.len()
            ));
        } else {
            lines.push(format!(
                "{}: kronos --gpu: logit-urile: amprenta {:#010x} ({} pozitii), abaterea maxima {:#010x} (biti f32)",
                p,
                d.print,
                d.words.len(),
                worst.to_bits()
            ));
        }
    }
    // P6-119 (C-157 point 3): the second diet's rows (only when it was on).
    if let Some(d2) = out.diet.as_ref().and_then(|d| d.diet2.as_ref()) {
        d2.journal(true, journal);
        let rows = d2.rows(true);
        // P6-138 (note C-157_P2j, point 4): with the argmax without the copy only parts A+B run.
        let top = out.diet.as_ref().is_some_and(|d| d.top);
        lines.push(format!(
            "{}: kronos --gpu: dieta 2: PORNITA (doua inregistrari citite pe {} din {} pozitii, asteptarea planificata, {}); cuvinte de inregistrari citite pe simbol (median) {}; citiri ale asteptarii pe simbol (median) {}",
            p,
            d2.record_words.iter().filter(|&&w| w == 8).count(),
            d2.record_words.len(),
            if top { "doar A+B: argmax pe placa, fara amprenta" } else { "amprenta dupa ceas, argmax pe biti" },
            rows[1].1,
            rows[2].1
        ));
    }
    // P6-98 (C-147 stage 0 (a)): the time per kind of launch (`kp …` rows + the table in the output / cycle report).
    out.prof.journal(journal);
    lines.extend(kind_lines(&out.prof, label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    // P6-124 (C-155 point 4): the race on quarters — every kind of launch, the glued list against quarters (50 per mille).
    // P6-128: the same race for `lipita-ln-32` against `lipita-ln` (6).
    if list_kind == 5 || list_kind == 6 {
        let q = if list_kind == 6 { Q_LN32 } else { Q_FUSED32 };
        match out.prof_first.as_ref().filter(|f| f.tokens() > 0 && out.prof.tokens() > 0) {
            Some(first) => {
                let correct = lines.iter().any(|l| l.contains(&format!("{}: CORECT", q.row)));
                let (rows, won, gain) = quarters_kinds(first, &out.prof, correct);
                lines.push(format!(
                    "{}: kronos --gpu: kt sferturi: cursa inmultirilor ({}; {} fata de {}; regula scrisa inainte: un fel trece pe sferturi doar daca e corect si mai rapid cu cel putin 50 la mie):",
                    p, label, q.name, q.base_name
                ));
                lines.extend(rows.into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
                lines.push(format!(
                    "{}: kronos --gpu: kt sferturi: {} feluri trec pe sferturi; toate felurile: {:+} la mie pe simbol ({})",
                    p,
                    won,
                    gain,
                    if !correct { "textele nu au fost corecte: nimic nu trece" } else if won == 0 { "niciun castig de 50 la mie: frana e altundeva, vezi nota Cyber-GPU" } else { "Cyber-GPU alege lista (mixta sau toata pe sferturi)" }
                ));
                journal.push(Event::Value { what: KT_Q_KINDS_WON, value: won });
                journal.push(Event::Value { what: KT_Q_GAIN, value: gain as i32 as u32 });
            }
            None => lines.push(format!("{}: kronos --gpu: kt sferturi: cursa nemasurata (timpul pe fel lipseste la o rulare)", p)),
        }
    }
    if let Some(why) = &out.prof_err {
        lines.push(format!("{}: kronos --gpu: timpul pe fel de lansare: un simbol lasat deoparte: {}", p, why));
    }
    lines.push(format!(
        "{}: kronos --gpu: jurnalul pe stick: {} randuri de rutina tinute in memorie (nu pe ecran), scrise pe stick in {} scrieri (in generare: doar dupa ultima pozitie, P6-112; in rest la {} simboluri sau la o secunda; {} goliri)",
        p, live.held, writes, super::gpu_i6::FLUSH_TOKENS, live.flushes
    ));
    lines.push(format!("{}: kronos --gpu: {} simboluri generate pe placa, toate identice cu procesorul (niciun simbol diferit)", p, made));
    // P6-98 (C-147 stage 0 (d)): the VRAM → VRAM copy, after the result rows are on the stick; its failure is a line,
    // not a failure of `kronos --gpu` (the text is already generated and checked).
    if let Some(ctx) = out.copy.take() {
        if let Err(e) = vram_copy(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
            lines.push(format!("{}: kronos --gpu: copierea VRAM -> VRAM: nemasurata: {}", p, e));
        }
        // P6-117 (C-161 point 2, cyber-gpu `3199fa2`): P8, the first tensor-core computation, only when the cycle's
        // list asks for it (`test-placa … --p8`), after everything of the generation (its rows are already written);
        // its failure is a line, not a failure of `kronos --gpu`.
        if super::gpu_tur::P8_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            let p8 = mma16_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx);
            if let Err(e) = &p8 {
                lines.push(format!("{}: kronos --gpu: P8 (mma16): oprita: {}", p, e));
            }
            // P6-200 (pista B1, cyber-gpu `ebbb3af`): the int8 probe right after P8, only when asked, only if P8 finished.
            if super::gpu_tur::P8_S8_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
                if p8.is_err() {
                    lines.push(format!("{}: kronos --gpu: proba mma16_s8: NU A RULAT: P8 nu a terminat", p));
                } else if let Err(e) = mma16_s8_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                    lines.push(format!("{}: kronos --gpu: proba mma16_s8: oprita: {}", p, e));
                }
            }
        }
        // P6-218 (cyber-gpu `adc783c`): the grid-barrier probe, only when the cycle's list asks for it
        // (`test-placa … --bariera`), after P8; its failure is a line.
        if super::gpu_tur::BARRIER_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = barrier_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx, false) {
                lines.push(format!("{}: kronos --gpu: proba barierei: oprita: {}", p, e));
            }
        }
        // P6-232 (cyber-gpu `6e69f1a`, note `C-157_P2zr_bariera-larga.md`): the wide barrier probe (G = SMs × 8, then
        // × 16), only when the cycle's list asks for it (`test-placa … --bariera-larga`); its failure is a line.
        if super::gpu_tur::BARRIER_WIDE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = barrier_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx, true) {
                lines.push(format!("{}: kronos --gpu: proba barierei larga: oprita: {}", p, e));
            }
        }
        // P6-134 (C-161 point 4, cyber-gpu `3b4ca77`): the first backward program of the training (`colsum`), only when
        // the cycle's list asks for it (`test-placa … --colsum`), after everything else; its failure is a line.
        if super::gpu_tur::COLSUM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = colsum_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: colsum (primul program inapoi): oprita: {}", p, e));
            }
        }
        // P6-136 (C-161 point 4, cyber-gpu `dd67483`): the small golden step, whole, on the board, only when the
        // cycle's list asks for it (`test-placa … --pas-aur`), after everything else; its failure is a line.
        if super::gpu_tur::GOLDEN_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = golden_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: pasul de aur mic pe placa: oprit: {}", p, e));
            }
        }
        // P6-137 (C-161 point 4, cyber-gpu `54563a5`): the `mm_tc` probe (the training matmul on the tensor cores), only
        // when the cycle's list asks for it (`test-placa … --mm-tc`), after everything else; its failure is a line.
        if super::gpu_tur::MM_TC_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = mm_tc_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: proba mm_tc: oprita: {}", p, e));
            }
        }
        // P6-141 (C-161 point 4, cyber-gpu `44a4d16`): the tiled training attention probe, only when the cycle's list asks
        // for it (`test-placa … --atentie-placi`), after everything else; its failure is a line.
        if super::gpu_tur::ATTN_TILE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = attn_tile_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: oprita: {}", p, e));
            }
            // P6-262 (C-168 p.4, cyber-gpu `b060477`, note P4a): `attn_fwd_tc` beside `attn_fwd_tile3`, only when the
            // cycle's list asks for it (`test-placa … --atentie3 … --atentie-fwd-tc`); its failure is a line.
            if super::gpu_tur::ATTN_FWD_TC_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
                if let Err(e) = attn_fwd_tc_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                    lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: oprita: {}", p, e));
                }
            }
        }
        // P6-161 (C-157, cyber-gpu `9359f21`): the RAM ↔ VRAM probe with the copy engine, only when the cycle's list asks
        // for it (`test-placa … --ram-vram`), after everything else; its failure is a line.
        let mut rv_ok = true;
        if super::gpu_tur::RAM_VRAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            match ram_vram_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                Ok(passed) => rv_ok = passed,
                Err(e) => {
                    rv_ok = false;
                    lines.push(format!("{}: kronos --gpu: proba RAM <-> VRAM: oprita: {}", p, e));
                }
            }
        }
        // P6-163 (C-157, cyber-gpu `ab9b537`): AdamW's m, v in host RAM, only when the cycle's list asks for it
        // (`test-placa … --ram-vram --adam-ram`) and only after the RAM ↔ VRAM probe passed (when it ran); its failure is
        // a line.
        if super::gpu_tur::ADAM_RAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if !rv_ok {
                lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: NU A RULAT: proba RAM <-> VRAM nu a trecut in aceasta pornire; nimic trimis", p));
            } else if let Err(e) = adam_ram_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: oprit: {}", p, e));
            }
        }
        // P6-164 (C-161, command-center note of 23 Nov 18:16): the whole training step of `antrenare mini` on the
        // board, only when the cycle's list asks for it (`test-placa … --antrenare-mini`), last; its failure is a line.
        if super::gpu_tur::TRAIN_MINI_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            if let Err(e) = train_mini_probe(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
                lines.push(format!("{}: kronos --gpu: antrenare mini pe placa: oprita: {}", p, e));
            }
        }
    }
    Ok(all.pop().unwrap_or_default())
}

/// P6-161: the host RAM of the RAM ↔ VRAM probe (A = `M` bytes, then B = `2M`): one contiguous run below 4 GiB, pinned
/// for the rest of the boot (the board wrote there) and reused by a later probe of the same boot when large enough.
static RV_RAM: crate::sched::Mutex<Option<(u64, u64)>> = crate::sched::Mutex::new(None);

/// P6-161: `pages` contiguous pinned host pages (base bus address), or `None` if the RAM below 4 GiB is too fragmented.
fn ram_vram_host(pages: u64) -> Option<u64> {
    let mut slot = RV_RAM.lock();
    if let Some((b, n)) = *slot {
        if n >= pages {
            return Some(b);
        }
    }
    let b = crate::mem::frame::alloc_contiguous_between(pages, 16 << 20, 1 << 32)?;
    crate::mem::frame::pin(b, pages, "gpu-ram-vram");
    *slot = Some((b, pages));
    Some(b)
}

/// P6-161: the bytes of the probe's host run (`len` bytes from bus address `base`).
fn ram_vram_bytes(base: u64, len: u64) -> &'static mut [u8] {
    // SAFETY: `base..base + len` lies inside the run `ram_vram_host` took from the frame allocator and pinned (nothing
    // else owns it); the HHDM maps physical memory linearly, so the run is one contiguous virtual range. Only the probe
    // touches it, and only while the board is not running (before the doorbell, after the last record).
    unsafe { core::slice::from_raw_parts_mut(crate::mem::map::phys_to_virt(base), len as usize) }
}

/// P6-161 (C-157, Adrian's point of 23 Nov a; cyber-gpu note `C-157_RAM_VRAM_proba-motor-copiere.md`, `9359f21`): the
/// speed of host RAM ↔ VRAM with the board's **copy engine**, outside the generation path. The copy object was allocated
/// on the work channel right after I9 (`gpu_golden::copy_object_step`). `M` = 64 MiB on the board (256 KiB in
/// SIMULARE), halved while the host RAM (3M, contiguous below 4 GiB) or the VRAM (2M) does not fit. A = `M` bytes RAM
/// with the pattern, B = `2M` RAM filled with `0xEE`, V1 + V2 = `2M` VRAM, the 31 records in VRAM; one submission of
/// `ram_vram::encode_probe` (up / down at M/64 … M, then 4 × M/4 in parallel, each 3 times). After the last record: the
/// group times from the records, B and A judged against the pattern; `kt ramvram …` rows. A record that never comes
/// stops the probe (no retry). New for the board: the copy engine (methods on subchannel 4); no register, no program.
#[allow(clippy::too_many_arguments)]
fn ram_vram_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<bool> {
    use nvidia_ga104::ram_vram as rv;
    const SEED: u32 = 0x157;
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let Some(obj) = st.copy_obj else {
        lines.push(format!("{}: kronos --gpu: proba RAM <-> VRAM: motorul de copiere nu e alocat pe canal: nimic trimis", p));
        return Err(bad("ram-vram: lipseste obiectul motorului de copiere"));
    };
    let mut m = if permit.is_some() { rv::DEFAULT_MAX_BYTES } else { rv::MAX_BYTES_UNIT };
    let (host, vram) = loop {
        rv::check_max(m)?;
        let got = ram_vram_host(3 * m / PAGE).and_then(|h| map_vram(st, 2 * m).ok().map(|v| (h, v)));
        match got {
            Some(x) => break x,
            None if m > rv::MAX_BYTES_UNIT => m /= 2,
            None => {
                lines.push(format!("{}: kronos --gpu: proba RAM <-> VRAM: nu am {} KiB RAM contigua sau {} KiB VRAM: nimic trimis", p, 3 * m >> 10, 2 * m >> 10));
                return Err(GpuError::OutOfMemory { requested: 3 * m });
            }
        }
    };
    let rec = map_vram(st, rv::RECORD_BYTES)?;
    let pages = |first: u64, n: u64| (first..first + n).map(|i| host + i * PAGE).collect::<Vec<u64>>();
    let a = st.mem.alloc_sys_mapped(&pages(0, m / PAGE))?;
    let b = st.mem.alloc_sys_mapped(&pages(m / PAGE, 2 * m / PAGE))?;
    if !a.va.is_multiple_of(PAGE) || !b.va.is_multiple_of(PAGE) || !vram.va.is_multiple_of(PAGE) {
        return Err(bad("ram-vram: bufferele nu sunt aliniate la 4 KiB"));
    }
    lines.push(format!(
        "{}: kronos --gpu: proba RAM <-> VRAM ({}): motorul de copiere (obiectul {:#x}, subcanalul {}), M = {} KiB: {} KiB RAM fixata + {} KiB VRAM, {} grupuri x {}, o trimitere",
        p,
        label,
        obj,
        rv::CE_SUBCH,
        m >> 10,
        3 * m >> 10,
        2 * m >> 10,
        rv::GROUPS,
        rv::REPEATS
    ));
    transfer::fill_pattern(ram_vram_bytes(host, m), SEED)?;
    ram_vram_bytes(host + m, 2 * m).fill(rv::FILL);
    put_bytes(st.mem.writer_mut(), rec.pa, &alloc::vec![0u8; rv::RECORD_BYTES as usize])?;
    let first = TOKEN_FIRST + STRIDE * *subs;
    let bufs = rv::Buffers { a: a.va, b: b.va, v1: vram.va, v2: vram.va + m, records: rec.va };
    let mut words = alloc::vec![0u32; rv::probe_words()];
    let mut pb = PushBuf::new(&mut words);
    rv::encode_probe(&mut pb, &bufs, m, first)?;
    let nw = pb.len();
    let n_rec = rv::RECORDS as u64;
    let last = first + (rv::RECORDS as u32 - 1);
    let mut maps = ctx.maps.clone();
    maps.push(vram);
    let sys = [ctx.sys.clone(), a.clone(), b.clone()];
    let recs = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], rec, n_rec, last, &maps, &sys, doorbell)? {
        Ok(x) => x,
        Err(got) => {
            lines.push(format!(
                "{}: kronos --gpu: proba RAM <-> VRAM: NU A TERMINAT: ultima inregistrare = {:#x} (asteptat {:#x}); proba oprita, fara reincercare",
                p, got, last
            ));
            return Err(GpuError::Timeout { waited_us: WAIT_US });
        }
    };
    *subs += 1;
    let times = rv::analyze(&recs, m, first)?;
    let bytes = rv::judge(ram_vram_bytes(host + m, 2 * m), ram_vram_bytes(host, m), m, SEED)?;
    let probe = rv::Probe { max_bytes: m, times, bytes };
    probe.journal(journal);
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let (lo_up, hi_up) = rv::PREDICTED_UP_MB_S;
    let (lo_dn, hi_dn) = rv::PREDICTED_DOWN_MB_S;
    lines.push(format!(
        "{}: kronos --gpu: proba RAM <-> VRAM: {}: sus {} MB/s (prezis {}-{}), jos {} MB/s (prezis {}-{}), 4 in paralel {} / {} promile din copia de M",
        p,
        if probe.passes() { "TRECE" } else { "NU TRECE" },
        probe.max_mb_s(rv::Dir::Up).unwrap_or(0),
        lo_up,
        hi_up,
        probe.max_mb_s(rv::Dir::Down).unwrap_or(0),
        lo_dn,
        hi_dn,
        probe.parallel_permille(rv::Dir::Up).unwrap_or(0),
        probe.parallel_permille(rv::Dir::Down).unwrap_or(0)
    ));
    Ok(probe.passes())
}

/// P6-163: what one run of the small golden step gave (bits compared between the two plans).
#[derive(Default)]
struct AdamRun {
    out: nvidia_ga104::train_step::Outcome,
    m: Vec<f32>,
    v: Vec<f32>,
}

/// P6-163: the first quantity on which two runs differ (bit for bit), or `None`.
fn adam_runs_differ(a: &AdamRun, b: &AdamRun) -> Option<&'static str> {
    fn same(x: &[f32], y: &[f32]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits())
    }
    fn same2(x: &[Vec<f32>], y: &[Vec<f32>]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
    }
    if !same(&a.out.losses, &b.out.losses) {
        Some("pierderile")
    } else if !same(&a.out.norms, &b.out.norms) {
        Some("normele")
    } else if !same2(&a.out.grad1, &b.out.grad1) {
        Some("gradientul dupa pasul 1")
    } else if !same2(&a.out.after1, &b.out.after1) {
        Some("greutatile dupa pasul 1")
    } else if !same2(&a.out.after_last, &b.out.after_last) {
        Some("greutatile dupa ultimul pas")
    } else if !same(&a.m, &b.m) {
        Some("m")
    } else if !same(&a.v, &b.v) {
        Some("v")
    } else {
        None
    }
}

/// P6-163: the median of the step times (us).
fn median_us(ns: &[u64]) -> u64 {
    if ns.is_empty() {
        return 0;
    }
    let mut v = ns.to_vec();
    v.sort_unstable();
    v[v.len() / 2] / 1000
}

/// P6-163 (C-157 step 2 of the RAM ↔ VRAM study; cyber-gpu note `C-157_ADAM_RAM_m-v-in-RAM.md`, commit `ab9b537`):
/// AdamW's m and v in host RAM, brought to the board piece by piece (the two tables, every layer, the final LayerNorm)
/// by the copy engine. The small golden step (2 layers, C 64, batch 2 × 32), `adamw_513`, 10 steps, twice on the same
/// host-RAM arena: (a) today's tiled plan (`Plan::new_tiles`, m, v in the arena), as `golden_probe` sends it; (b)
/// `Plan::new_tiles_mv_ram` + `adam_ram::RamMv`: m, v in pinned host RAM (after the control zone), the staging place in
/// VRAM, the copy engine's records right after the launches' records; one submission per step (`adam_ram::encode_step`:
/// the launches before the optimizer, then per piece m, v up, the channel WAITS on the copy's record
/// (`SEM_EXECUTE` `ACQ_STRICT_GEQ`), the piece's `opt_step`, m, v down; at the end a wait on the last copy down), the
/// last record waited for = the last copy down. Each judged against PyTorch (`train_step::judge`); (b) compared bit for
/// bit with (a); `adam_ram::check_sync` after every step of (b). A step the board does not finish stops the probe (no
/// retry). New for the board: the copy engine in the training step and the wait on a semaphore; no register, no program.
#[allow(clippy::too_many_arguments)]
fn adam_ram_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use gpu_compute::train::{self, AdamW, Recipe};
    use nvidia_ga104::adam_ram::{self as ar, RamMv};
    use nvidia_ga104::train_step::{self as ts, Plan};
    const STEPS: u32 = 10;
    let (bsz, tl) = (2usize, 32usize);
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    if st.copy_obj.is_none() {
        lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: motorul de copiere nu e alocat pe canal: nimic trimis", p));
        return Err(bad("adam-ram: lipseste obiectul motorului de copiere"));
    }
    let data = match crate::shell::antrenare::golden_data() {
        Ok(d) => d,
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: NU A RULAT: {}; nimic trimis", p, e));
            return Err(bad("adam-ram: datele lipsesc"));
        }
    };
    if ts::check_programs().is_err() {
        lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: SHA-256 al unui program nu e cel din nota: nimic trimis", p));
        return Err(bad("adam-ram: SHA-256 gresit"));
    }
    // Sizes first, at a provisional base: today's tiled plan has the larger arena (m, v inside).
    let base0 = nvidia_ga104::mmu::VA_BASE;
    let tiles0 = Plan::new_tiles(train::MIC, bsz, tl, &data.inputs, base0)?;
    let ram0 = Plan::new_tiles_mv_ram(train::MIC, bsz, tl, &data.inputs, base0)?;
    if tiles0.programs() != ram0.programs() {
        return Err(bad("adam-ram: cele doua planuri au programe diferite"));
    }
    let rm0 = RamMv::new(&ram0, 0, 0, 0)?;
    let pieces = rm0.chunks.len();
    let n_tiles = tiles0.launches_per_step(Recipe::AdamW513);
    let n_before = n_tiles - 1;
    let n_ram = n_before + pieces;
    let n_rec_ram = n_ram + 1 + 2 * pieces;
    let arena_bytes = tiles0.bytes().max(ram0.bytes()).next_multiple_of(PAGE);
    let zone_bytes = (n_tiles.max(n_ram) as u64 * ts::SLOT).next_multiple_of(PAGE);
    let mv_bytes = rm0.ram_bytes().next_multiple_of(PAGE);
    if 16 * n_rec_ram as u64 > ctx.rec.size || n_rec_ram as u32 > STRIDE || n_tiles as u32 + 1 > STRIDE {
        return Err(bad("adam-ram: prea putine inregistrari"));
    }
    if (arena_bytes + zone_bytes + mv_bytes) / PAGE > I11_BOARD_BYTES / PAGE {
        return Err(bad("adam-ram: arena nu incape in paginile de urcare"));
    }
    lines.push(format!(
        "{}: kronos --gpu: AdamW cu m, v in RAM ({}): pasul de aur mic pe placi, adamw_513 x {} pasi de doua ori pe aceeasi arena: planul de azi ({} lansari pe pas, m, v in arena) si m, v in RAM fixata ({} KiB; {} bucati: tabelele, {} straturi, LayerNorm-ul final; {} lansari pe pas), locul de trecere {} KiB VRAM, arena cu {} KiB mai mica; motorul de copiere + asteptarea pe semafor, cate o trimitere pe pas",
        p,
        label,
        STEPS,
        n_tiles,
        rm0.ram_bytes() >> 10,
        pieces,
        train::MIC.layers,
        n_ram,
        rm0.stage_bytes() >> 10,
        (tiles0.bytes() - ram0.bytes()) >> 10
    ));
    // The programs in VRAM (page-aligned).
    let mut progs: Vec<(ts::Prog, Mapping)> = Vec::with_capacity(tiles0.programs().len());
    for &prog in tiles0.programs().iter() {
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    let arena = st.mem.alloc_sys_mapped(&arena_pages(0, arena_bytes / PAGE)?)?;
    let zone = st.mem.alloc_sys_mapped(&arena_pages(arena_bytes / PAGE, zone_bytes / PAGE)?)?;
    let mv = st.mem.alloc_sys_mapped(&arena_pages((arena_bytes + zone_bytes) / PAGE, mv_bytes / PAGE)?)?;
    let stage = map_vram(st, rm0.stage_bytes())?;
    if !arena.va.is_multiple_of(PAGE) || !zone.va.is_multiple_of(PAGE) || !mv.va.is_multiple_of(PAGE) || !stage.va.is_multiple_of(PAGE) {
        return Err(bad("adam-ram: memoria nu e aliniata la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    maps.push(stage);
    let sys = [ctx.sys.clone(), arena.clone(), zone.clone(), mv.clone()];
    let prog_va = |want: ts::Prog| progs.iter().find(|(q, _)| *q == want).map_or(0, |(_, pm)| pm.va);
    let read_f32s = |va: u64, n: usize| ts::f32s(&ram_read_at(&arena, va - arena.va, 4 * n as u64));
    let upload = |plan: &Plan| -> GpuResult<()> {
        let mut image = alloc::vec![0u8; arena_bytes as usize];
        for (va, bytes) in plan.uploads(&data.initial, &data.inputs, &data.targets)? {
            let at = (va - arena.va) as usize;
            image[at..at + bytes.len()].copy_from_slice(&bytes);
        }
        ram_write(&arena, &image);
        Ok(())
    };
    let mut stopped: Option<String> = None;
    // (a) Today's tiled plan.
    let tiles = Plan::new_tiles(train::MIC, bsz, tl, &data.inputs, arena.va)?;
    upload(&tiles)?;
    let mut a = AdamRun::default();
    {
        let opt = AdamW::recipe_513(&data.initial);
        for step in 1..=STEPS {
            let list = tiles.step(Recipe::AdamW513, &opt, step)?;
            let (zb, qmds) = ts::pack(&list, prog_va, zone.va)?;
            ram_write(&zone, &zb);
            put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (list.len() + 1)])?;
            let first = TOKEN_FIRST + STRIDE * *subs;
            let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(list.len())];
            let mut pb = PushBuf::new(&mut words);
            transfer::encode_sequence_fresh(&mut pb, &qmds, ctx.rec.va, first)?;
            let nw = pb.len();
            let last = first + list.len() as u32;
            let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, list.len() as u64 + 1, last, &maps, &sys, doorbell)? {
                Ok(b) => b,
                Err(got) => {
                    stopped = Some(format!("planul de azi: placa nu a terminat la pasul {} (ultima inregistrare {:#x}, asteptat {:#x})", step, got, last));
                    break;
                }
            };
            *subs += 1;
            a.out.step_ns.push(transfer::analyze_sequence(&b, list.len(), first)?.iter().sum());
            a.out.losses.push(ts::Outcome::mean_loss(&read_f32s(tiles.loss_rows, bsz * tl)));
            let norm = read_f32s(tiles.res, 1)[0];
            a.out.norms.push(norm);
            if !norm.is_finite() {
                stopped = Some(format!("planul de azi: norma gradientului nu e finita la pasul {} ({:?})", step, norm));
                break;
            }
            if step == 1 {
                a.out.grad1 = tiles.unflat(&read_f32s(tiles.g, tiles.n_params))?;
                a.out.after1 = tiles.unflat(&read_f32s(tiles.w, tiles.n_params))?;
            }
        }
        if stopped.is_none() {
            a.out.after_last = tiles.unflat(&read_f32s(tiles.w, tiles.n_params))?;
            a.m = read_f32s(tiles.m, tiles.n_params);
            a.v = read_f32s(tiles.v, tiles.n_params);
        }
    }
    // (b) m, v in host RAM, piece by piece.
    let mut b_run = AdamRun::default();
    let ram_plan = Plan::new_tiles_mv_ram(train::MIC, bsz, tl, &data.inputs, arena.va)?;
    let sync_va = ctx.rec.va + 16 * (n_ram as u64 + 1);
    let rm = RamMv::new(&ram_plan, mv.va, stage.va, sync_va)?;
    if stopped.is_none() {
        upload(&ram_plan)?;
        ram_write(&mv, &alloc::vec![0u8; mv_bytes as usize]);
        let opt = AdamW::recipe_513(&data.initial);
        for step in 1..=STEPS {
            let (before, args) = ram_plan.step_until_opt(&opt, step)?;
            let opt_l = rm.opt_launches(&args)?;
            let n = before.len();
            let all: Vec<ts::Launch> = before.into_iter().chain(opt_l).collect();
            let (zb, qmds) = ts::pack(&all, prog_va, zone.va)?;
            ram_write(&zone, &zb);
            put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * n_rec_ram])?;
            let first = TOKEN_FIRST + STRIDE * *subs;
            let first_sync = first + all.len() as u32;
            let mut words = alloc::vec![0u32; ar::step_words(n, pieces)];
            let mut pb = PushBuf::new(&mut words);
            ar::encode_step(&mut pb, &rm, &qmds[..n], &qmds[n..], ctx.rec.va, first, first_sync)?;
            let nw = pb.len();
            let last = ar::sync_payload(first_sync, pieces - 1, true);
            let recs = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, n_rec_ram as u64, last, &maps, &sys, doorbell)? {
                Ok(r) => r,
                Err(got) => {
                    stopped = Some(format!(
                        "m, v in RAM: placa nu a terminat la pasul {} (ultima inregistrare {:#x}, asteptat {:#x}; daca lipseste, canalul a ramas la asteptarea pe semafor sau la o copiere)",
                        step, got, last
                    ));
                    break;
                }
            };
            *subs += 1;
            let split = 16 * (all.len() + 1);
            if let Err(e) = ar::check_sync(&recs[split..], pieces, first_sync) {
                stopped = Some(format!("m, v in RAM: pasul {}: {}", step, e));
                break;
            }
            b_run.out.step_ns.push(transfer::analyze_sequence(&recs[..split], all.len(), first)?.iter().sum());
            b_run.out.losses.push(ts::Outcome::mean_loss(&read_f32s(ram_plan.loss_rows, bsz * tl)));
            let norm = read_f32s(ram_plan.res, 1)[0];
            b_run.out.norms.push(norm);
            if !norm.is_finite() {
                stopped = Some(format!("m, v in RAM: norma gradientului nu e finita la pasul {} ({:?})", step, norm));
                break;
            }
            if step == 1 {
                b_run.out.grad1 = ram_plan.unflat(&read_f32s(ram_plan.g, ram_plan.n_params))?;
                b_run.out.after1 = ram_plan.unflat(&read_f32s(ram_plan.w, ram_plan.n_params))?;
            }
        }
        if stopped.is_none() {
            b_run.out.after_last = ram_plan.unflat(&read_f32s(ram_plan.w, ram_plan.n_params))?;
            let nb = 4 * ram_plan.n_params as u64;
            b_run.m = ts::f32s(&ram_read_at(&mv, 0, nb));
            b_run.v = ts::f32s(&ram_read_at(&mv, rm.v - rm.m, nb));
        }
    }
    if let Some(why) = &stopped {
        lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: {}; proba oprita, fara reincercare", p, why));
        lines.push(format!("{}: kronos --gpu: AdamW cu m, v in RAM: NU A TERMINAT (vezi randul de mai sus)", p));
        journal.push(Event::Value { what: KT_ADAM_RAM_PASS, value: 0 });
        return Err(GpuError::Timeout { waited_us: WAIT_US });
    }
    let refs = &data.adamw;
    let va = ts::judge(Recipe::AdamW513, n_tiles, &a.out, refs);
    let vb = ts::judge(Recipe::AdamW513, n_ram, &b_run.out, refs);
    lines.extend(va.lines(&format!("{}, planul de azi pe placi", label)).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    vb.journal(journal);
    lines.extend(vb.lines(&format!("{}, m v in RAM", label)).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let differ = adam_runs_differ(&a, &b_run);
    let (us_a, us_b) = (median_us(&a.out.step_ns), median_us(&b_run.out.step_ns));
    journal.push(Event::Value { what: KT_ADAM_RAM_SAME, value: u32::from(differ.is_none()) });
    journal.push(Event::Value { what: KT_ADAM_RAM_US_TILES, value: us_a as u32 });
    journal.push(Event::Value { what: KT_ADAM_RAM_US_RAM, value: us_b as u32 });
    lines.push(format!(
        "{}: kronos --gpu: AdamW cu m, v in RAM fata de planul de azi: {}",
        p,
        match differ {
            None => String::from("IDENTICE bit cu bit (pierderile, normele, gradientul si greutatile dupa pasul 1, greutatile dupa pasul 10, m, v)"),
            Some(what) => format!("DIFERA ({} nu sunt aceiasi biti)", what),
        }
    ));
    lines.push(format!(
        "{}: kronos --gpu: AdamW cu m, v in RAM: timpul placii pe pas (median): planul de azi {} us, m v in RAM {} us ({:+} us; {} KiB urcati si coborati pe pas)",
        p,
        us_a,
        us_b,
        us_b as i64 - us_a as i64,
        (16 * ram_plan.n_params as u64) >> 10
    ));
    let pass = va.passes() && vb.passes() && differ.is_none();
    journal.push(Event::Value { what: KT_ADAM_RAM_PASS, value: u32::from(pass) });
    lines.push(format!(
        "{}: kronos --gpu: AdamW cu m, v in RAM: {}",
        p,
        if pass {
            "TRECE (ambele planuri ca PyTorch, m v in RAM = planul de azi bit cu bit)"
        } else {
            "NU TRECE (vezi randurile de mai sus)"
        }
    ));
    Ok(())
}

/// P6-98 (C-147 stage 0 (a)): the table "time per kind of launch", longest first (median per token and per launch).
fn kind_lines(prof: &kp::KindProfile, label: &str) -> Vec<String> {
    if prof.tokens() == 0 {
        return Vec::new();
    }
    let mut kinds: Vec<kp::KindTime> = prof.summary().into_iter().filter(|k| k.kind.is_some()).collect();
    kinds.sort_by(|a, b| b.token_median_ns.cmp(&a.token_median_ns));
    let total: u64 = kinds.iter().map(|k| k.token_median_ns).sum();
    let mut v = alloc::vec![format!(
        "timpul pe fel de lansare ({}, median pe simbol, {} simboluri; toate ~{} us pe simbol):",
        label,
        prof.tokens(),
        total / 1000
    )];
    for k in kinds {
        let name = k.kind.map_or("?", |x| x.name());
        v.push(format!(
            "  {:<12} {:>7} us pe simbol ({} lansari; una {} ns)",
            name,
            k.token_median_ns / 1000,
            k.per_token,
            k.launch_median_ns
        ));
    }
    v
}

/// P6-98 (C-147 stage 0 (d)): one VRAM → VRAM copy of (at most) 64 MiB of the weights into a fresh scratch piece,
/// repeated `kp::VRAM_COPY_REPEATS` times in one submission with a timestamp record after each (`copy_b128`, already on
/// the board since I11), three words of the destination read back and compared with the host's copy; `kc …` rows.
#[allow(clippy::too_many_arguments)]
fn vram_copy<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    let bytes = kp::VramCopy::size_for(ctx.src.len() as u64);
    let scratch = map_vram(st, bytes)?;
    let c = kp::VramCopy::new(ctx.src_va, scratch.va, bytes, kp::VRAM_COPY_REPEATS)?;
    let (cb_va, cb_pa) = (ctx.launches.va, ctx.launches.pa);
    {
        let w = st.mem.writer_mut();
        put_bytes(w, cb_pa, &c.cbuf()?)?;
        put_bytes(w, cb_pa + 256, &c.qmd(ctx.copy_va, cb_va)?)?;
        put_bytes(w, ctx.rec.pa, &alloc::vec![0u8; c.record_bytes() as usize])?;
    }
    let first = TOKEN_FIRST + STRIDE * *subs;
    let mut words = alloc::vec![0u32; c.words()];
    let mut pb = PushBuf::new(&mut words);
    c.encode(&mut pb, cb_va + 256, ctx.rec.va, first)?;
    let nw = pb.len();
    let repeats = kp::VRAM_COPY_REPEATS;
    let last = first + repeats as u32;
    let mut maps = ctx.maps.clone();
    maps.push(scratch);
    let sys = [ctx.sys.clone()];
    let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, repeats as u64 + 1, last, &maps, &sys, doorbell)? {
        Ok(b) => b,
        Err(got) => {
            lines.push(format!("{}: kronos --gpu: copierea VRAM -> VRAM: ultima inregistrare = {:#x} (asteptat {:#x})", p, got, last));
            return Err(GpuError::Timeout { waited_us: WAIT_US });
        }
    };
    *subs += 1;
    let s = c.analyze(&b, first)?;
    let got = {
        let GpuState { zone, .. } = &*st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
        let spots = c.spots();
        let r = (|| Ok::<[u32; 3], GpuError>([vw.read_u32(scratch.pa + spots[0])?, vw.read_u32(scratch.pa + spots[1])?, vw.read_u32(scratch.pa + spots[2])?]))();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))?
    };
    let wrong = c.wrong_spots(&ctx.src, &got);
    c.journal(&s, Some(wrong), journal);
    lines.push(format!(
        "{}: kronos --gpu: copierea VRAM -> VRAM: {} KiB x {}, cuvinte gresite {} din 3; cea mai rapida {} us, mediana {} us = {} MB/s ({} MB/s trafic, {} promile din 608 GB/s)",
        p,
        bytes >> 10,
        repeats,
        wrong,
        s.min_ns / 1000,
        s.median_ns / 1000,
        s.copy_mb_s(),
        s.traffic_mb_s(),
        s.share_permille()
    ));
    Ok(())
}

/// P6-117 (C-161 point 2, C-160 point 1; cyber-gpu note `C-160_P8_primul-calcul-tensor.md`, commit `3199fa2`): P8 —
/// `D = A·B + C` (16×16, A and B binary16, C and D f32) on the tensor cores, with `mma16_f16` (Mesa NAK 26.3.1), once
/// for each of the 6 fixed sets of `mma16_probe::SETS`, one submission per set; D (1 KiB) read back from host RAM and
/// judged on the CPU against the written tolerance (`17 · 2⁻²³ · (|c| + Σ|a·b|)`). Outside the generation path: it
/// runs after `kronos --gpu`'s rows. New for the board: this program (its SHA-256 checked first), its 0x800-byte cbuf
/// (addresses at 0x700), 32 registers, one warp. A set the board does not finish stops the probe (no retry); the rows
/// written are those of the sets judged so far. The host page is the logits page (no longer used by then).
#[allow(clippy::too_many_arguments)]
fn mma16_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use nvidia_ga104::{mma16, mma16_probe as mp};
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let sha = sha256_hex(mma16::MMA16_F16);
    if sha != mma16::MMA16_F16_SHA256 {
        lines.push(format!("{}: kronos --gpu: P8 (mma16): SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, sha, mma16::MMA16_F16_SHA256));
        return Err(bad("P8: SHA-256 gresit pentru mma16_f16"));
    }
    if ctx.sys.size < mp::PAGE_BYTES as u64 || !ctx.sys.va.is_multiple_of(mp::PAGE_BYTES as u64) {
        return Err(bad("P8: pagina de RAM nu are 4 KiB aliniati"));
    }
    lines.push(format!(
        "{}: kronos --gpu: P8 (mma16, {}): programul mma16_f16 ({} octeti, SHA-256 {}...), {} seturi, cate o trimitere",
        p,
        label,
        mma16::MMA16_F16.len(),
        &sha[..16],
        mp::SETS.len()
    ));
    // The program, then one VRAM page: the cbuf at +0 (0x800 bytes), the QMD at +0x800 (aligned to 256).
    let prog = map_vram(st, program_map_bytes(mma16::MMA16_F16.len() as u64))?;
    let zone = map_vram(st, PAGE)?;
    put_bytes(st.mem.writer_mut(), prog.pa, mma16::MMA16_F16)?;
    let mut maps = ctx.maps.clone();
    maps.push(prog);
    maps.push(zone);
    let sys = [ctx.sys.clone()];
    let qmd_va = zone.va + mma16::CBUF_SIZE as u64;
    let mut probe = mp::Probe::default();
    let mut stopped = None;
    let mut fits: Vec<(u32, u32)> = Vec::new();
    for (i, set) in mp::SETS.iter().enumerate() {
        ram_write(&ctx.sys, &set.page()); // D starts as NaN
        let (cb, q) = mp::launch_bytes(prog.va, zone.va, ctx.sys.va)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, zone.pa, &cb)?;
            put_bytes(w, zone.pa + mma16::CBUF_SIZE as u64, &q)?;
            put_bytes(w, ctx.rec.pa, &[0; 32])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[qmd_va], ctx.rec.va, first)?;
        let nw = pb.len();
        let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => b,
            Err(got) => {
                lines.push(format!(
                    "{}: kronos --gpu: P8 (mma16): placa nu a terminat la setul {} ({}): ultima inregistrare = {:#x} (asteptat {:#x}); proba oprita, fara reincercare",
                    p, i, set.name, got, first + 1
                ));
                stopped = Some(i);
                break;
            }
        };
        *subs += 1;
        let ns = transfer::analyze_sequence(&b, 1, first)?[0];
        let page = ram_read(&ctx.sys, mp::D_OFF + mp::D_BYTES as u64);
        let d = &page[mp::D_OFF as usize..];
        probe.add(mp::judge_set(i, d, Some(ns))?);
        // P6-125 (cyber-gpu `cf9e911`, note C-157_P2e): D's FNV + whether it is bit for bit Cyber-GPU's adder model.
        mp::journal_fit(journal, i, d)?;
        let rows = mp::fit_rows(i, d)?;
        fits.push((rows[0].1, rows[1].1));
    }
    probe.journal(journal);
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let fit_ok = fits.iter().filter(|f| f.1 == 1).count();
    let fnvs: Vec<String> = fits.iter().map(|f| format!("{:#010x}", f.0)).collect();
    lines.push(format!(
        "{}: kronos --gpu: P8 model ga104 (sumatorul potrivit pe os-335): {}/{} seturi bit cu bit; iesirea pe seturi (fnv): {}",
        p,
        fit_ok,
        fits.len(),
        fnvs.join(" ")
    ));
    let verdict = if stopped.is_some() {
        "NU A TERMINAT"
    } else if probe.passes() {
        "TRECE"
    } else {
        "NU TRECE"
    };
    lines.push(format!(
        "{}: kronos --gpu: P8 (mma16): {}: {} / {} seturi in toleranta, identitate {}/256 exacte, abaterea maxima {} promile din toleranta, timp median {} ns",
        p,
        verdict,
        probe.in_tolerance(),
        mp::SETS.len(),
        probe.sets.first().map_or(0, |r| r.exact()),
        probe.worst_permille(),
        probe.median_ns().map_or(String::from("?"), |n| format!("{}", n))
    ));
    match stopped {
        Some(_) => Err(GpuError::Timeout { waited_us: WAIT_US }),
        None => Ok(()),
    }
}

/// P6-200 (pista B1): one set of the int8 probe — A (16×32 i8), B (32×16 i8), C (16×16 i32), row-major.
struct S8Set {
    name: &'static str,
    a: Vec<i8>,
    b: Vec<i8>,
    c: Vec<i32>,
}

/// P6-200: the 11 sets of the note `B1_mma16-s8.md` (the same as Cyber-GPU's T229): 8 random (the same LCG, seeds 1–8;
/// C in ±2²⁰), then the edges: all −128 × −128 with C = −3 (524 285 everywhere), −128 × 127 with C = 11, and
/// A = [I16 | 0] on the random set 99 (D = B's first 16 rows + C — catches a row/column mix-up). None leaves i32.
fn s8_sets() -> Vec<S8Set> {
    use gpu_compute::mma_s8 as s8;
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }
    let random = |name: &'static str, seed: u64| {
        let mut g = Lcg(seed);
        let a = (0..s8::M * s8::K).map(|_| g.next() as u8 as i8).collect();
        let b = (0..s8::K * s8::N).map(|_| g.next() as u8 as i8).collect();
        let c = (0..s8::M * s8::N).map(|_| (g.next() % (1 << 21)) as i32 - (1 << 20)).collect();
        S8Set { name, a, b, c }
    };
    let full = |name: &'static str, a: i8, b: i8, c: i32| S8Set { name, a: alloc::vec![a; s8::M * s8::K], b: alloc::vec![b; s8::K * s8::N], c: alloc::vec![c; s8::M * s8::N] };
    const NAMES: [&str; 8] = ["aleator-1", "aleator-2", "aleator-3", "aleator-4", "aleator-5", "aleator-6", "aleator-7", "aleator-8"];
    let mut v: Vec<S8Set> = NAMES.iter().enumerate().map(|(i, n)| random(n, i as u64 + 1)).collect();
    v.push(full("minus128-minus128", -128, -128, -3));
    v.push(full("minus128-127", -128, 127, 11));
    let mut id = random("identitate", 99);
    id.a = (0..s8::M * s8::K).map(|i| i8::from(i / s8::K == i % s8::K)).collect();
    v.push(id);
    v
}

/// P6-200 (pista B1, cyber-gpu `ebbb3af`, note `B1_mma16-s8.md`): the int8 tensor-core probe — `D = A·B + C` with
/// `mma16_s8` (Mesa NAK 26.3.1, two `imma.m16n8k32.i8.i8`), once for each of the 11 sets of [`s8_sets`], one submission
/// per set, on the same host page and the same launch path as P8 (`cbuf_s8` / `qmd_s8`, addresses at 0x700). D starts as
/// `0x7fff_fff7` and is judged **bit for bit** against `gpu_compute::mma_s8::mma16_s8_ref` (integers: nothing rounds).
/// New for the board: this program (its SHA-256 checked first), 32 registers, one warp, no shared memory, no barrier.
/// A set the board does not finish stops the probe (no retry).
#[allow(clippy::too_many_arguments)]
fn mma16_s8_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use gpu_compute::mma_s8 as s8;
    use nvidia_ga104::{mma16, mma16_probe as mp};
    const UNWRITTEN: i32 = 0x7fff_fff7;
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let sha = sha256_hex(mma16::MMA16_S8);
    if sha != mma16::MMA16_S8_SHA256 {
        lines.push(format!("{}: kronos --gpu: proba mma16_s8: SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, sha, mma16::MMA16_S8_SHA256));
        return Err(bad("mma16_s8: SHA-256 gresit"));
    }
    if ctx.sys.size < mp::PAGE_BYTES as u64 || !ctx.sys.va.is_multiple_of(mp::PAGE_BYTES as u64) {
        return Err(bad("mma16_s8: pagina de RAM nu are 4 KiB aliniati"));
    }
    let sets = s8_sets();
    lines.push(format!(
        "{}: kronos --gpu: proba mma16_s8 ({}): programul mma16_s8 ({} octeti, SHA-256 {}..., {} registre, un warp), {} seturi, cate o trimitere, judecat bit cu bit",
        p,
        label,
        mma16::MMA16_S8.len(),
        &sha[..16],
        mma16::MMA16_S8_REGISTERS,
        sets.len()
    ));
    let prog = map_vram(st, program_map_bytes(mma16::MMA16_S8.len() as u64))?;
    let zone = map_vram(st, PAGE)?;
    put_bytes(st.mem.writer_mut(), prog.pa, mma16::MMA16_S8)?;
    let mut maps = ctx.maps.clone();
    maps.push(prog);
    maps.push(zone);
    let sys = [ctx.sys.clone()];
    let qmd_va = zone.va + mma16::CBUF_SIZE as u64;
    let page_va = ctx.sys.va;
    let cb = mma16::cbuf_s8(page_va + mp::A_OFF, page_va + mp::B_OFF, page_va + mp::C_OFF, page_va + mp::D_OFF)?;
    let q = mma16::qmd_s8(prog.va, zone.va)?;
    let mut same = 0usize;
    let mut times: Vec<u64> = Vec::new();
    let mut stopped = false;
    for (i, set) in sets.iter().enumerate() {
        let want = s8::mma16_s8_ref(&set.a, &set.b, &set.c).ok_or_else(|| bad("mma16_s8: un set iese din i32 (nu se trimite)"))?;
        let mut page = alloc::vec![0u8; mp::PAGE_BYTES];
        for (k, v) in set.a.iter().enumerate() {
            page[mp::A_OFF as usize + k] = *v as u8;
        }
        for (k, v) in set.b.iter().enumerate() {
            page[mp::B_OFF as usize + k] = *v as u8;
        }
        for (k, v) in set.c.iter().enumerate() {
            page[mp::C_OFF as usize + 4 * k..mp::C_OFF as usize + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        for k in 0..s8::M * s8::N {
            page[mp::D_OFF as usize + 4 * k..mp::D_OFF as usize + 4 * k + 4].copy_from_slice(&UNWRITTEN.to_le_bytes());
        }
        ram_write(&ctx.sys, &page);
        {
            let w = st.mem.writer_mut();
            put_bytes(w, zone.pa, &cb)?;
            put_bytes(w, zone.pa + mma16::CBUF_SIZE as u64, &q)?;
            put_bytes(w, ctx.rec.pa, &[0; 32])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[qmd_va], ctx.rec.va, first)?;
        let nw = pb.len();
        let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => b,
            Err(got) => {
                lines.push(format!(
                    "{}: kronos --gpu: proba mma16_s8: placa nu a terminat la setul {} ({}): ultima inregistrare = {:#x} (asteptat {:#x}); proba oprita, fara reincercare",
                    p, i, set.name, got, first + 1
                ));
                stopped = true;
                break;
            }
        };
        *subs += 1;
        times.push(transfer::analyze_sequence(&b, 1, first)?[0]);
        let back = ram_read(&ctx.sys, mp::D_OFF + (4 * s8::M * s8::N) as u64);
        let d: Vec<i32> = back[mp::D_OFF as usize..].chunks(4).map(|w| i32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let diff: Vec<usize> = (0..d.len()).filter(|k| d[*k] != want[*k]).collect();
        match diff.first() {
            None => {
                same += 1;
                lines.push(format!("{}: kronos --gpu: mma16_s8 set {} {} ({}): ACEIASI BITI", p, i, set.name, label));
            }
            Some(k) => lines.push(format!(
                "{}: kronos --gpu: mma16_s8 set {} {} ({}): DIFERA la ({}, {}): placa {}, oracol {} ({} cuvinte diferite din 256{})",
                p,
                i,
                set.name,
                label,
                k / s8::N,
                k % s8::N,
                d[*k],
                want[*k],
                diff.len(),
                if d.contains(&UNWRITTEN) { "; unele nescrise" } else { "" }
            )),
        }
    }
    times.sort_unstable();
    let median = times.get(times.len() / 2).copied();
    let verdict = if stopped {
        "NU A TERMINAT"
    } else if same == sets.len() {
        "TRECE"
    } else {
        "NU TRECE"
    };
    let ns = median.map_or(String::from("?"), |n| format!("{}", n));
    lines.push(format!("{}: kronos --gpu: proba mma16_s8: {}: {} / {} seturi cu aceiasi biti ca oracolul, timp median {} ns", p, verdict, same, sets.len(), ns));
    lines.push(format!("{}: kronos --gpu: kt mma16_s8 {} {}/{} median {} ns", p, verdict, same, sets.len(), ns));
    if stopped {
        Err(GpuError::Timeout { waited_us: WAIT_US })
    } else {
        Ok(())
    }
}

/// P6-218: SHA-256 of `grid_barrier.sm86.sass` as the note `C-157_P2zh_proba-barierei.md` gives it.
const GRID_BARRIER_SHA256: &str = "535f353705579f16deea31c363755092e7b00a7ba1ac43764c32d6c72183b12d";

/// P6-218: the time limit of one barrier launch on the board (the note's proposal; predicted ≤ 3 ms).
const BARRIER_LIMIT_US: u64 = 100_000;
/// P6-228: the RTX 3070 Ti has 48 SMs (24 TPC x 2); any other count GSP-RM reports stops the barrier probe.
const EXPECTED_SMS: u32 = 48;

/// P6-218 (cyber-gpu `adc783c`, note `C-157_P2zh_proba-barierei.md`): the grid-barrier probe — `grid_barrier` with G
/// workgroups of 32 meeting R times at a barrier through a counter in VRAM. G = the SMs GSP-RM reported in I9 × 1, with
/// R = 1 then R = 1 001 (the counter cleared before each launch, a 100 ms limit each on the board), judged with
/// `grid_barrier::judge`; the cost of one barrier from the two board times. Only if G × 1 held: the same with G × 2. A
/// launch that does not finish in time stops the probe (`BLOCATA`): nothing else is sent. New for the board: this
/// program (its SHA-256 checked first), 19 registers, no shared memory, atomics on a VRAM word.
///
/// P6-232 (cyber-gpu `6e69f1a`, note `C-157_P2zr_bariera-larga.md`): `wide` = the same probe with G = SMs × 8 (384),
/// then × 16 (768) only if 384 held (`grid_barrier::groups_for_wide_sms`, never `groups_for_wide`); a larger zone
/// (3 pages) holds the 768 groups' data and results. Nothing else changes for the board.
#[allow(clippy::too_many_arguments)]
fn barrier_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    wide: bool,
) -> GpuResult<()> {
    use nvidia_ga104::grid_barrier as gb;
    // The page: counter +0, data +0x100 (8·G ≤ 768 B), results +0x400 (4·G ≤ 384 B); cbuf +0x800, QMD +0xa00.
    // P6-232, the wide run (G ≤ 768): data +0x100 (≤ 6 144 B), results +0x1900 (≤ 3 072 B); cbuf +0x2800, QMD +0x2a00.
    const SYNC: u64 = 0;
    const DATA: u64 = 0x100;
    #[allow(non_snake_case)]
    let (OUT, CBUF, QMD, ZONE): (u64, u64, u64, u64) = if wide { (0x1900, 0x2800, 0x2a00, 3 * PAGE) } else { (0x400, 0x800, 0xa00, PAGE) };
    let name = if wide { "proba barierei larga" } else { "proba barierei" };
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let limit = if permit.is_some() { BARRIER_LIMIT_US } else { WAIT_US };
    let sha = sha256_hex(gb::PROGRAM);
    if sha != GRID_BARRIER_SHA256 {
        lines.push(format!("{}: kronos --gpu: {}: SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, name, sha, GRID_BARRIER_SHA256));
        return Err(bad("bariera: SHA-256 gresit"));
    }
    let gr = st.gr.ok_or_else(|| bad("bariera: forma GR lipseste (I9 nu a raportat-o)"))?;
    // P6-228 (cyber-gpu `aa94bea`, after os-456): the LITTER_* product is the design maximum (84), not the board's SMs;
    // the group count comes from the enabled TPCs GSP-RM reports in the same I9 answer.
    let counts = st.gr_counts.ok_or_else(|| bad("bariera: numarul de TPC lipseste (I9 nu l-a raportat)"))?;
    let sms = counts.sms;
    lines.push(format!(
        "{}: kronos --gpu: {} ({}): programul grid_barrier ({} octeti, SHA-256 {}..., {} registre, blocuri de {}), {} SM-uri active ({} TPC x {} SM; maximele de proiect {} GPC x {} TPC x {} SM), R = {} apoi {}, limita {} ms pe lansare{}",
        p,
        name,
        label,
        gb::PROGRAM.len(),
        &sha[..16],
        gb::REGISTERS,
        gb::BLOCK,
        sms,
        counts.tpcs,
        gr.sm_per_tpc,
        gr.gpcs,
        gr.tpc_per_gpc,
        gr.sm_per_tpc,
        gb::ROUNDS_SHORT,
        gb::ROUNDS_LONG,
        limit / 1000,
        if wide { ", G = SM-uri x 8 apoi x 16" } else { "" }
    ));
    if sms != EXPECTED_SMS {
        // the note: if another count comes, stop and write the values (do not guess them)
        lines.push(format!(
            "{}: kronos --gpu: {}: GSP-RM raporteaza {} TPC -> {} SM, asteptat {} (RTX 3070 Ti); proba oprita, nimic trimis",
            p, name, counts.tpcs, sms, EXPECTED_SMS
        ));
        return Err(bad("bariera: numarul de SM nu e cel asteptat"));
    }
    let prog = map_vram(st, program_map_bytes(gb::PROGRAM.len() as u64))?;
    let zone = map_vram(st, ZONE)?;
    put_bytes(st.mem.writer_mut(), prog.pa, gb::PROGRAM)?;
    let mut maps = ctx.maps.clone();
    maps.push(prog);
    maps.push(zone);
    let sys = [ctx.sys.clone()];
    let mut verdicts: Vec<String> = Vec::new();
    let steps: &[u32] = if wide { &[8, gb::MAX_PER_SM_WIDE] } else { &[1, gb::MAX_PER_SM] };
    for &per_sm in steps {
        let g = if wide { gb::groups_for_wide_sms(sms, per_sm)? } else { gb::groups_for_sms(sms, per_sm)? };
        let (_, d_bytes, o_bytes) = gb::buffer_bytes(g);
        if DATA + d_bytes > OUT || OUT + o_bytes > CBUF {
            return Err(bad("bariera: G prea mare pentru zona probei"));
        }
        let mut times = [0u64; 2];
        let mut held = true;
        for (k, rounds) in [gb::ROUNDS_SHORT, gb::ROUNDS_LONG].into_iter().enumerate() {
            let cb = gb::cbuf(zone.va + SYNC, zone.va + DATA, zone.va + OUT, rounds, g)?;
            let q = gb::qmd(prog.va, gb::REGISTERS, zone.va + CBUF, g)?;
            {
                // the counter and the data cleared, the results filled with 0xff (the program writes every one)
                let w = st.mem.writer_mut();
                put_bytes(w, zone.pa + SYNC, &[0u8; 4])?;
                put_bytes(w, zone.pa + DATA, &alloc::vec![0u8; d_bytes as usize])?;
                put_bytes(w, zone.pa + OUT, &alloc::vec![0xffu8; o_bytes as usize])?;
                put_bytes(w, zone.pa + CBUF, &cb)?;
                put_bytes(w, zone.pa + QMD, &q)?;
                put_bytes(w, ctx.rec.pa, &[0; 32])?;
            }
            let first = TOKEN_FIRST + STRIDE * *subs;
            let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
            let mut pb = PushBuf::new(&mut words);
            transfer::encode_sequence(&mut pb, &[zone.va + QMD], ctx.rec.va, first)?;
            let nw = pb.len();
            let sent = submit_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)?;
            let b = match wait_records_within(vbar, clock, journal, st, l, permit, &sent, limit)? {
                Ok(b) => b,
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: bariera G={} R={}: BLOCATA (nu s-a terminat in {} ms; ultima inregistrare = {:#x}, asteptat {:#x}); proba oprita, nimic altceva trimis",
                        p, g, rounds, limit / 1000, got, first + 1
                    ));
                    lines.push(format!("{}: kronos --gpu: kt bariera G={} BLOCATA", p, g));
                    verdicts.push(format!("G={} BLOCATA", g));
                    lines.push(format!("{}: kronos --gpu: {}: {}", p, name, verdicts.join(", ")));
                    return Err(GpuError::Timeout { waited_us: limit });
                }
            };
            *subs += 1;
            times[k] = transfer::analyze_sequence(&b, 1, first)?[0];
            let (counter, out) = {
                let GpuState { zone: z, .. } = &*st;
                let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, z.vram_size, &z.protected)?;
                let r = (|| {
                    let c = vw.read_u32(zone.pa + SYNC)?;
                    let o = (0..g as u64).map(|i| vw.read_u32(zone.pa + OUT + 4 * i)).collect::<GpuResult<Vec<u32>>>()?;
                    Ok::<_, GpuError>((c, o))
                })();
                let back = vw.restore();
                r.and_then(|v| back.map(|_| v))?
            };
            let text = match gb::judge(g, rounds, counter, &out)? {
                gb::Verdict::Holds => String::from("TRECE"),
                gb::Verdict::BadCounter { got, want } => {
                    held = false;
                    format!("CONTOR GRESIT ({} in loc de {})", got, want)
                }
                gb::Verdict::StaleWords { groups, rounds: r } => {
                    held = false;
                    format!("CUVINTE VECHI ({} grupuri, {} runde gresite)", groups, r)
                }
            };
            lines.push(format!("{}: kronos --gpu: bariera G={} R={} ({}): {}, contorul {}, timp {} ns", p, g, rounds, label, text, counter, times[k]));
            if !held {
                break;
            }
        }
        if held {
            match gb::barrier_ns(times[0], times[1]) {
                Ok(ns) => {
                    lines.push(format!("{}: kronos --gpu: bariera G={}: o bariera = {} ns", p, g, ns));
                    lines.push(format!("{}: kronos --gpu: kt bariera G={} TRECE {} ns", p, g, ns));
                }
                Err(_) => lines.push(format!("{}: kronos --gpu: bariera G={}: costul nemasurat (rularea lunga nu e mai lunga: {} / {} ns)", p, g, times[0], times[1])),
            }
            verdicts.push(format!("G={} TRECE", g));
        } else {
            lines.push(format!("{}: kronos --gpu: kt bariera G={} NU TRECE", p, g));
            verdicts.push(format!("G={} NU TRECE", g));
            break; // G × 2 (× 16) only after G × 1 (× 8) held
        }
    }
    lines.push(format!("{}: kronos --gpu: {}: {}", p, name, verdicts.join(", ")));
    Ok(())
}

/// P6-134 (C-161 point 4, C-160 point 6; cyber-gpu note `C-160_P6_primul-program-inapoi.md`, commit `3b4ca77`): the
/// first backward program of the training on the board — `colsum` (column sums of a gradient: a bias's gradient, or
/// LayerNorm's dγ and dβ), once for each of the 5 fixed sets of `colsum_probe::SETS`, one submission per set. Each set's
/// 64 KiB buffer (inputs, then the two outputs with NaN fill and NaN guards) lives in host RAM: the arena's upload pages
/// (page 0 on), no longer used once the texts are done. The outputs are read back and judged on the CPU against the
/// written margin (and bit for bit against the model). Outside the generation path. New for the board: this program
/// (its SHA-256 checked first), 19 registers, blocks of 256, 1–3 blocks; cbuf 0x100 as today. A set the board does
/// not finish stops the probe (no retry).
#[allow(clippy::too_many_arguments)]
fn colsum_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use nvidia_ga104::{colsum as cs, colsum_probe as cp};
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let sha = sha256_hex(cs::COLSUM);
    if sha != cs::COLSUM_SHA256 {
        lines.push(format!("{}: kronos --gpu: colsum: SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, sha, cs::COLSUM_SHA256));
        return Err(bad("colsum: SHA-256 gresit"));
    }
    lines.push(format!(
        "{}: kronos --gpu: colsum (primul program inapoi, {}): programul colsum ({} octeti, SHA-256 {}...), {} seturi, cate o trimitere",
        p,
        label,
        cs::COLSUM.len(),
        &sha[..16],
        cp::SETS.len()
    ));
    // The program, then one VRAM page: the cbuf at +0 (0x100 bytes), the QMD at +0x800 (as P8); the buffer in host RAM.
    let prog = map_vram(st, program_map_bytes(cs::COLSUM.len() as u64))?;
    let zone = map_vram(st, PAGE)?;
    put_bytes(st.mem.writer_mut(), prog.pa, cs::COLSUM)?;
    let buf = st.mem.alloc_sys_mapped(&arena_pages(0, (cp::BUF_BYTES as u64).div_ceil(PAGE))?)?;
    if !buf.va.is_multiple_of(PAGE) {
        return Err(bad("colsum: bufferul nu e aliniat la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.push(prog);
    maps.push(zone);
    let sys = [ctx.sys.clone(), buf.clone()];
    let qmd_va = zone.va + 0x800;
    let mut probe = cp::Probe::default();
    let mut stopped = None;
    for (i, set) in cp::SETS.iter().enumerate() {
        ram_write(&buf, &set.buffer()); // the outputs and guards start as NaN
        let (cb, q) = cp::launch_bytes(i, prog.va, zone.va, buf.va)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, zone.pa, &cb)?;
            put_bytes(w, zone.pa + 0x800, &q)?;
            put_bytes(w, ctx.rec.pa, &[0; 32])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[qmd_va], ctx.rec.va, first)?;
        let nw = pb.len();
        let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => b,
            Err(got) => {
                lines.push(format!(
                    "{}: kronos --gpu: colsum: placa nu a terminat la setul {} ({}): ultima inregistrare = {:#x} (asteptat {:#x}); proba oprita, fara reincercare",
                    p, i, set.name, got, first + 1
                ));
                stopped = Some(i);
                break;
            }
        };
        *subs += 1;
        let ns = transfer::analyze_sequence(&b, 1, first)?[0];
        let (o, n) = set.out_range();
        let out = ram_read_at(&buf, o as u64, n as u64);
        probe.add(cp::judge_set(i, &out, Some(ns))?);
    }
    probe.journal(journal);
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let verdict = match stopped {
        Some(i) => format!("NU A TERMINAT (la setul {})", i),
        None if probe.passes() => String::from("TRECE"),
        None => String::from("NU TRECE"),
    };
    lines.push(format!(
        "{}: kronos --gpu: colsum (primul program inapoi): {}: {} / {} seturi in margine, {} / {} elemente exacte fata de model, abaterea maxima {} promile din margine, timp median {} ns",
        p,
        verdict,
        probe.in_margin(),
        cp::SETS.len(),
        probe.exact(),
        probe.elements(),
        probe.worst_permille(),
        probe.median_ns().map_or(String::from("?"), |n| format!("{}", n))
    ));
    match stopped {
        Some(_) => Err(GpuError::Timeout { waited_us: WAIT_US }),
        None => Ok(()),
    }
}

/// P6-137 (C-161 point 4, C-160 point 7 fourth part; cyber-gpu note `C-160_P7d_mm_tc-proba.md`, commit `54563a5`): the
/// `mm_tc` probe — the training matmul on the tensor cores (`C = op(A)·op(B) (+ bias)`, inputs rounded to f16, sums in
/// f32), alone on the board, outside the generation path. The 5 small fixed sets of `mm_tc_probe::SETS` judge the
/// correctness (one submission each; each buffer ≤ 256 KiB in host RAM, the arena's upload pages); the sixth, `qkv-513`
/// (6 144 × 2 304 × 768, ~82,6 MB), gives the first time of a fast training matmul: its buffer in VRAM (skipped if it does
/// not fit), the inputs uploaded through the arena with the copy program and read back byte for byte before any launch,
/// the three bands + the guard filled with NaN, then 5 submissions, the bands read back and judged on the CPU (after the
/// clock). In SIMULARE the 5 launches of `qkv-513` are skipped (21,7 GFLOP each on the emulated CPU); everything before
/// them runs. New for the board: this program (its SHA-256 checked first), 64 registers, blocks of 128, 24 KiB of shared
/// memory + one barrier, a grid with y > 1; cbuf 0x800 (arguments at 0x700) as P8. A set the board does not finish stops
/// the probe (no retry).
#[allow(clippy::too_many_arguments)]
fn mm_tc_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use nvidia_ga104::{mm_tc as tc, mm_tc_probe as tp};
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let sha = sha256_hex(tc::MM_TC);
    if sha != tc::MM_TC_SHA256 {
        lines.push(format!("{}: kronos --gpu: proba mm_tc: SHA-256 al programului {} nu e cel din nota ({}): nimic trimis", p, sha, tc::MM_TC_SHA256));
        return Err(bad("mm_tc: SHA-256 gresit"));
    }
    lines.push(format!(
        "{}: kronos --gpu: proba mm_tc ({}): programul mm_tc ({} octeti, SHA-256 {}..., {} registre, blocuri de {}, {} KiB partajati), {} seturi mici cate o trimitere, apoi qkv-513",
        p,
        label,
        tc::MM_TC.len(),
        &sha[..16],
        tc::REGISTERS,
        tc::BLOCK[0],
        tc::SHARED.bytes >> 10,
        tp::QKV_SET
    ));
    // P6-250 (C-157 P2zzl): the max-speed window guard, checked on os-485's own numbers before any time is taken.
    lines.push(format!(
        "{}: kronos --gpu: proba mm_tc: paza ferestrei de viteza maxima (P6-250, BoostPlan::covers_ms), verificata pe numerele os-485: {}",
        p,
        if super::gpu_golden::speed_window_self_test() { "MERGE (v5@64k dX la 677,6 s in fereastra, v6 dX la 722,9 s si dW in afara; 1 800 s ar acoperi tot lantul)" } else { "GRESIT" }
    ));
    // The program, then one VRAM page: the cbuf at +0 (0x800 bytes), the QMD at +0x800 (as P8).
    let prog = map_vram(st, program_map_bytes(tc::MM_TC.len() as u64))?;
    let zone = map_vram(st, PAGE)?;
    put_bytes(st.mem.writer_mut(), prog.pa, tc::MM_TC)?;
    // P6-197 (C-165, cyber-gpu `49efcbb`): `mm_tc128` beside `mm_tc` when the list asks for it — its own program (SHA-256
    // first, mapped with the guard page) and its own VRAM page for the same cbuf (+0) and its QMD (+0x800).
    // P6-207 (C-157 p.2, cyber-gpu `835c6c9`): or `mm_tc_pf` (the reads in advance), the same way; one companion per
    // probe (the cycle refuses both; `mm_tc128` wins if both are set by hand).
    // P6-210 (C-157 p.2, cyber-gpu `273a42b`): `mm_tc_v4` (128-bit reads) too, after `mm_tc_pf` when both are asked —
    // every companion runs on every set it fits, each compared with `mm_tc`.
    let ord = core::sync::atomic::Ordering::SeqCst;
    let comps: Vec<Comp> = if super::gpu_tur::MM_TC128_IN_CYCLE.load(ord) {
        alloc::vec![Comp::Tc128]
    } else {
        let mut v = Vec::new();
        if super::gpu_tur::MM_TC_PF_IN_CYCLE.load(ord) {
            v.push(Comp::Pf);
        }
        if super::gpu_tur::MM_TC_V4_IN_CYCLE.load(ord) {
            v.push(Comp::V4);
        }
        // P6-242 (C-157 p.2, cyber-gpu `2b24cc7`, note P2zzb): `mm_tc_v5` (12 KiB block) right after `mm_tc_v4`.
        if super::gpu_tur::MM_TC_V5_IN_CYCLE.load(ord) {
            v.push(Comp::V5);
        }
        // P6-246 (C-157 p.2, cyber-gpu `7f4eca5`, note P2zzg): `mm_tc_v6` (paired f16 writes) right after `mm_tc_v5`.
        if super::gpu_tur::MM_TC_V6_IN_CYCLE.load(ord) {
            v.push(Comp::V6);
        }
        // P6-248 (C-157 p.2, cyber-gpu `8b6c068`, note P2zzi): `mm_tc_v7` (padded shared rows) right after `mm_tc_v6`.
        if super::gpu_tur::MM_TC_V7_IN_CYCLE.load(ord) {
            v.push(Comp::V7);
        }
        // P6-258 (C-168 p.3, cyber-gpu `72ac8eb`, note P3a): the two v8 programs right after `mm_tc_v7`.
        if super::gpu_tur::MM_TC_V8_IN_CYCLE.load(ord) {
            v.push(Comp::V8);
            v.push(Comp::V8n);
        }
        // P6-263 (C-168 p.3 follow-up, cyber-gpu `35cdbd3`, note P3d): `mm_tc_v8r` (v8 with fewer register copies) right
        // after v8 / v8n, in the same cycle (same board, same heat).
        if super::gpu_tur::MM_TC_V8R_IN_CYCLE.load(ord) {
            v.push(Comp::V8r);
        }
        // P6-212 (C-157 p.2, cyber-gpu `abb4f73`): `mm_tc128_v4` last.
        if super::gpu_tur::MM_TC128_V4_IN_CYCLE.load(ord) {
            v.push(Comp::Tc128V4);
        }
        v
    };
    // P6-214 (C-157 p.2, cyber-gpu note `C-157_P2ze_os436.md` point 1): with `--mm-tc-64k`, `mm_tc` and every companion run
    // once more with the shared-memory step at 64 KiB (`to_bytes_with_sm_config`, as `--treapta-pas` in os-431) — two
    // blocks per unit instead of one — each compared bit for bit with `mm_tc` at today's step.
    let comps: Vec<(Comp, Option<u32>)> = if super::gpu_tur::MM_TC_64K_IN_CYCLE.load(ord) {
        let mut v = alloc::vec![(Comp::Tc, Some(STEP_64K))];
        for c in comps {
            v.push((c, None));
            v.push((c, Some(STEP_64K)));
        }
        v
    } else {
        comps.into_iter().map(|c| (c, None)).collect()
    };
    let mut t128: Vec<Tc128> = Vec::new();
    for (kind, step) in comps {
        {
            let n = Tc128::label(kind, step);
            let sha2 = sha256_hex(kind.program());
            if sha2 != kind.sha() {
                lines.push(format!("{}: kronos --gpu: proba {}: SHA-256 al programului {} nu e cel din nota ({}): {} nu se lanseaza", p, n, sha2, kind.sha(), n));
            } else {
                let prog2 = map_vram(st, program_map_bytes(kind.program().len() as u64))?;
                let zone2 = map_vram(st, PAGE)?;
                put_bytes(st.mem.writer_mut(), prog2.pa, kind.program())?;
                lines.push(format!(
                    "{}: kronos --gpu: proba {} ({}): programul {} ({} octeti, SHA-256 {}..., {} registre, blocuri de {}, {} KiB partajati{}) langa mm_tc pe fiecare set, comparat bit cu bit",
                    p,
                    n,
                    label,
                    kind.name(),
                    kind.program().len(),
                    &sha2[..16],
                    kind.registers(),
                    kind.block()[0],
                    kind.shared().bytes >> 10,
                    step.map_or(String::new(), |b| format!(", treapta memoriei comune {} KiB", b >> 10))
                ));
                t128.push(Tc128 { kind, step, prog: prog2, zone: zone2 });
            }
        }
    }
    let mut r128: Vec<Vec<Tc128Set>> = t128.iter().map(|_| Vec::new()).collect();
    let mut stopped128: Vec<Option<usize>> = t128.iter().map(|_| None).collect();
    // P6-210: the sets a companion cannot take (`mm_tc_v4`: rows not a multiple of 4 or A/B not 16-byte aligned).
    let mut unfit: Vec<Vec<usize>> = t128.iter().map(|_| Vec::new()).collect();
    let small = tp::SETS[..tp::QKV_SET].iter().map(|s| s.layout().bytes as u64).max().unwrap_or(PAGE);
    let buf = st.mem.alloc_sys_mapped(&arena_pages(0, small.div_ceil(PAGE))?)?;
    if !buf.va.is_multiple_of(PAGE) {
        return Err(bad("mm_tc: bufferul nu e aliniat la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.push(prog);
    maps.push(zone);
    for t in &t128 {
        maps.push(t.prog);
        maps.push(t.zone);
    }
    let qmd_va = zone.va + tc::CBUF_BYTES as u64;
    let mut probe = tp::Probe::default();
    let mut stopped: Option<usize> = None;
    for (i, set) in tp::SETS[..tp::QKV_SET].iter().enumerate() {
        let sys = [ctx.sys.clone(), buf.clone()];
        ram_write(&buf, &set.buffer()); // C and the guard start as NaN
        let (cb, q) = tp::launch_bytes(i, prog.va, zone.va, buf.va)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, zone.pa, &cb)?;
            put_bytes(w, zone.pa + tc::CBUF_BYTES as u64, &q)?;
        }
        match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, qmd_va)? {
            Ok(ns) => {
                let mut out = Vec::with_capacity(set.read_len());
                for (o, n) in set.read_ranges() {
                    out.extend_from_slice(&ram_read_at(&buf, o as u64, n as u64));
                }
                probe.add(tp::judge_set(i, &out, &[ns])?);
                // P6-197: the same set with `mm_tc128` (C and the guard NaN again, the same cbuf, its QMD).
                for (ci, t) in t128.iter().enumerate() {
                    if !t.kind.fits(&set.args(buf.va)) {
                        unfit[ci].push(i);
                        continue;
                    }
                    ram_write(&buf, &set.buffer());
                    let (cb2, q2) = tc128_launch_bytes(&set.args(buf.va), &cb, t)?;
                    {
                        let w = st.mem.writer_mut();
                        put_bytes(w, t.zone.pa, &cb2)?;
                        put_bytes(w, t.zone.pa + tc::CBUF_BYTES as u64, &q2)?;
                    }
                    match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, t.zone.va + tc::CBUF_BYTES as u64)? {
                        Ok(ns2) => {
                            let mut out2 = Vec::with_capacity(set.read_len());
                            for (o, n) in set.read_ranges() {
                                out2.extend_from_slice(&ram_read_at(&buf, o as u64, n as u64));
                            }
                            r128[ci].push(Tc128Set::new(i, &out, &out2, alloc::vec![ns], alloc::vec![ns2])?);
                        }
                        Err(got) => {
                            lines.push(format!(
                                "{}: kronos --gpu: proba {}: placa nu a terminat la setul {} ({}): ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                                p,
                                t.name(),
                                i,
                                set.name,
                                got
                            ));
                            stopped128[ci] = Some(i);
                            stopped = Some(i);
                            break;
                        }
                    }
                }
                if stopped.is_some() {
                    break;
                }
            }
            Err(got) => {
                lines.push(format!(
                    "{}: kronos --gpu: proba mm_tc: placa nu a terminat la setul {} ({}): ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                    p, i, set.name, got
                ));
                stopped = Some(i);
                break;
            }
        }
    }
    // P6-247 (C-157 p.2, cyber-gpu `1cb1846`, note P2zzh point 3.2): with `--mm-tc-dxdw`, after `qkv-513` the two backward
    // products of the same qkv (`qkv-513-dx`, `qkv-513-dw`), each the same way (its own ~83 MB buffer, 5 launches per program).
    let mut big_sets = alloc::vec![tp::QKV_SET];
    if super::gpu_tur::MM_TC_DXDW_IN_CYCLE.load(ord) {
        big_sets.push(tp::QKV_DX_SET);
        big_sets.push(tp::QKV_DW_SET);
    }
    for si in big_sets {
        if stopped.is_some() {
            break;
        }
        let first_line = lines.len();
        let mut q128: Vec<Qkv128> = t128.iter().map(|_| Qkv128::default()).collect();
        match mm_tc_qkv(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, &maps, prog.va, zone, &t128, &mut q128, si)? {
            QkvOutcome::Judged(r) => probe.add(r),
            QkvOutcome::Skipped => {}
            QkvOutcome::Stopped => stopped = Some(si),
        }
        // P6-247: the lines of `mm_tc_qkv` say `qkv-513`; for the backward sets, their own name.
        if si != tp::QKV_SET {
            let name = tp::set(si).map_or("?", |s| s.name);
            for x in &mut lines[first_line..] {
                *x = x.replace("qkv-513", name);
            }
        }
        for (ci, q) in q128.into_iter().enumerate() {
            if q.stopped {
                stopped128[ci] = Some(si);
            }
            if q.unfit {
                unfit[ci].push(si);
            }
            if let Some(x) = q.set {
                r128[ci].push(x);
            }
        }
    }
    probe.journal(journal);
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let verdict = match stopped {
        Some(i) => format!("NU A TERMINAT (la setul {})", i),
        None if probe.passes() => String::from("TRECE"),
        None => String::from("NU TRECE"),
    };
    let qkv = probe
        .sets
        .iter()
        .find(|r| r.set == tp::QKV_SET)
        .and_then(|r| r.gflops().zip(r.median_ns()))
        .map_or(String::from("qkv-513 omis"), |(g, ns)| format!("qkv-513 {} ns = {}.{:02} TFLOP/s", ns, g / 1000, (g % 1000) / 10));
    lines.push(format!(
        "{}: kronos --gpu: proba mm_tc: {}: {} / {} seturi in margine, {} / {} elemente exacte fata de model, abaterea maxima {} promile din margine, {}",
        p,
        verdict,
        probe.in_margin(),
        probe.sets.len(),
        probe.exact_model(),
        probe.elements(),
        probe.worst_permille(),
        qkv
    ));
    for (ci, t) in t128.iter().enumerate() {
        tc128_report(p, label, t, &r128[ci], stopped128[ci], &unfit[ci], journal, lines);
    }
    match stopped {
        Some(_) => Err(GpuError::Timeout { waited_us: WAIT_US }),
        None => Ok(()),
    }
}

/// P6-137: what happened to `qkv-513`.
enum QkvOutcome {
    Judged(nvidia_ga104::mm_tc_probe::SetResult),
    /// Not run (no VRAM, or SIMULARE): the `kt mm_tc qkv omis` row says so.
    Skipped,
    /// A copy or a launch the board did not finish (the line is written).
    Stopped,
}

/// P6-197 / P6-207: the program run beside `mm_tc` on every set (the same `Args`, the same cbuf; its own QMD).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Comp {
    /// P6-197: `mm_tc128`, the same job on 128×128 tiles.
    Tc128,
    /// P6-207: `mm_tc_pf`, `mm_tc` with the reads asked in advance (248 registers, the same block and grid).
    Pf,
    /// P6-210: `mm_tc_v4`, `mm_tc_pf` with 128-bit reads (128 registers, the same block and grid); only aligned products.
    V4,
    /// P6-242: `mm_tc_v5`, `mm_tc_v4` with a 12 KiB block (the same loop, block, grid and cbuf); only aligned products.
    V5,
    /// P6-246: `mm_tc_v6`, `mm_tc_v5` with the loop's f16 writes paired (the same block, grid, cbuf and 12 KiB); only
    /// aligned products.
    V6,
    /// P6-248: `mm_tc_v7`, `mm_tc_v6` with 16 bytes of padding per shared-memory row (same launch, grid, cbuf and bits;
    /// its own `SHARED`, 14 336 B); only aligned products.
    V7,
    /// P6-258 (C-168 p.3, P3a): `mm_tc_v8`, 128×128 tiles of C, 256 threads, 128 registers, 24 576 B (its own block, grid
    /// and `SHARED`; the same 48 B of arguments and `fits` as v7); only aligned products.
    V8,
    /// P6-258: `mm_tc_v8n`, 128 rows × 64 columns of C, 128 threads, 144 registers, 18 432 B (`BLOCK_N`, `grid_n`,
    /// `SHARED_N`, `REGISTERS_N`); only aligned products.
    V8n,
    /// P6-263 (C-168 p.3 follow-up, P3d): `mm_tc_v8r`, `mm_tc_v8` with B read first in each slice (same work, grid,
    /// `fits`, `SHARED` and bits as v8); only its binary and the QMD registers (120) differ; only aligned products.
    V8r,
    /// P6-212: `mm_tc128_v4`, `mm_tc128` with 128-bit reads in advance (168 registers, `mm_tc128`'s block and grid);
    /// only aligned products.
    Tc128V4,
    /// P6-214: `mm_tc` itself, run as a companion only at the 64 KiB step (`--mm-tc-64k`).
    Tc,
}

/// P6-214: the shared-memory step of `--mm-tc-64k` (two 24 KiB blocks per unit instead of one at 32 KiB).
const STEP_64K: u32 = 64 << 10;

impl Comp {
    /// P6-210: `mm_tc_v4` only on aligned products (a misaligned 128-bit read would be a memory fault on the board).
    fn fits(self, a: &nvidia_ga104::mm_tc::Args) -> bool {
        match self {
            Comp::V4 => nvidia_ga104::mm_tc_v4::fits(a),
            Comp::V5 => nvidia_ga104::mm_tc_v5::fits(a),
            Comp::V6 => nvidia_ga104::mm_tc_v6::fits(a),
            Comp::V7 => nvidia_ga104::mm_tc_v7::fits(a),
            Comp::V8 | Comp::V8n => nvidia_ga104::mm_tc_v8::fits(a),
            Comp::V8r => nvidia_ga104::mm_tc_v8r::fits(a),
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::fits(a),
            Comp::Tc128 | Comp::Pf | Comp::Tc => true,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Comp::Tc128 => "mm_tc128",
            Comp::Pf => "mm_tc_pf",
            Comp::V4 => "mm_tc_v4",
            Comp::V5 => "mm_tc_v5",
            Comp::V6 => "mm_tc_v6",
            Comp::V7 => "mm_tc_v7",
            Comp::V8 => "mm_tc_v8",
            Comp::V8n => "mm_tc_v8n",
            Comp::V8r => "mm_tc_v8r",
            Comp::Tc128V4 => "mm_tc128_v4",
            Comp::Tc => "mm_tc",
        }
    }
    fn program(self) -> &'static [u8] {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::MM_TC128,
            Comp::Pf => nvidia_ga104::mm_tc_pf::MM_TC_PF,
            Comp::V4 => nvidia_ga104::mm_tc_v4::MM_TC_V4,
            Comp::V5 => nvidia_ga104::mm_tc_v5::MM_TC_V5,
            Comp::V6 => nvidia_ga104::mm_tc_v6::MM_TC_V6,
            Comp::V7 => nvidia_ga104::mm_tc_v7::MM_TC_V7,
            Comp::V8 => nvidia_ga104::mm_tc_v8::MM_TC_V8,
            Comp::V8n => nvidia_ga104::mm_tc_v8::MM_TC_V8N,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::MM_TC_V8R,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::MM_TC128_V4,
            Comp::Tc => nvidia_ga104::mm_tc::MM_TC,
        }
    }
    fn sha(self) -> &'static str {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::MM_TC128_SHA256,
            Comp::Pf => nvidia_ga104::mm_tc_pf::MM_TC_PF_SHA256,
            Comp::V4 => nvidia_ga104::mm_tc_v4::MM_TC_V4_SHA256,
            Comp::V5 => nvidia_ga104::mm_tc_v5::MM_TC_V5_SHA256,
            Comp::V6 => nvidia_ga104::mm_tc_v6::MM_TC_V6_SHA256,
            Comp::V7 => nvidia_ga104::mm_tc_v7::MM_TC_V7_SHA256,
            Comp::V8 => nvidia_ga104::mm_tc_v8::MM_TC_V8_SHA256,
            Comp::V8n => nvidia_ga104::mm_tc_v8::MM_TC_V8N_SHA256,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::MM_TC_V8R_SHA256,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::MM_TC128_V4_SHA256,
            Comp::Tc => nvidia_ga104::mm_tc::MM_TC_SHA256,
        }
    }
    fn registers(self) -> u32 {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::REGISTERS,
            Comp::Pf => nvidia_ga104::mm_tc_pf::REGISTERS,
            Comp::V4 => nvidia_ga104::mm_tc_v4::REGISTERS,
            Comp::V5 => nvidia_ga104::mm_tc_v5::REGISTERS,
            Comp::V6 => nvidia_ga104::mm_tc_v6::REGISTERS,
            Comp::V7 => nvidia_ga104::mm_tc_v7::REGISTERS,
            Comp::V8 => nvidia_ga104::mm_tc_v8::REGISTERS,
            Comp::V8n => nvidia_ga104::mm_tc_v8::REGISTERS_N,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::REGISTERS,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::REGISTERS,
            Comp::Tc => nvidia_ga104::mm_tc::REGISTERS,
        }
    }
    fn block(self) -> [u32; 3] {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::BLOCK,
            Comp::Pf => nvidia_ga104::mm_tc_pf::BLOCK,
            Comp::V4 => nvidia_ga104::mm_tc_v4::BLOCK,
            Comp::V5 => nvidia_ga104::mm_tc_v5::BLOCK,
            Comp::V6 => nvidia_ga104::mm_tc_v6::BLOCK,
            Comp::V7 => nvidia_ga104::mm_tc_v7::BLOCK,
            Comp::V8 => nvidia_ga104::mm_tc_v8::BLOCK,
            Comp::V8n => nvidia_ga104::mm_tc_v8::BLOCK_N,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::BLOCK,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::BLOCK,
            Comp::Tc => nvidia_ga104::mm_tc::BLOCK,
        }
    }
    fn shared(self) -> nvidia_ga104::qmd::SharedMemory {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::SHARED,
            Comp::Pf => nvidia_ga104::mm_tc_pf::SHARED,
            Comp::V4 => nvidia_ga104::mm_tc_v4::SHARED,
            Comp::V5 => nvidia_ga104::mm_tc_v5::SHARED,
            Comp::V6 => nvidia_ga104::mm_tc_v6::SHARED,
            Comp::V7 => nvidia_ga104::mm_tc_v7::SHARED,
            Comp::V8 => nvidia_ga104::mm_tc_v8::SHARED,
            Comp::V8n => nvidia_ga104::mm_tc_v8::SHARED_N,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::SHARED,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::SHARED,
            Comp::Tc => nvidia_ga104::mm_tc::SHARED,
        }
    }
    fn grid(self, a: &nvidia_ga104::mm_tc::Args) -> [u32; 3] {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::grid(a),
            Comp::Pf => nvidia_ga104::mm_tc_pf::grid(a),
            Comp::V4 => nvidia_ga104::mm_tc_v4::grid(a),
            Comp::V5 => nvidia_ga104::mm_tc_v5::grid(a),
            Comp::V6 => nvidia_ga104::mm_tc_v6::grid(a),
            Comp::V7 => nvidia_ga104::mm_tc_v7::grid(a),
            Comp::V8 => nvidia_ga104::mm_tc_v8::grid(a),
            Comp::V8n => nvidia_ga104::mm_tc_v8::grid_n(a),
            Comp::V8r => nvidia_ga104::mm_tc_v8r::grid(a),
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::grid(a),
            Comp::Tc => a.grid(),
        }
    }
    fn cbuf_bytes(self) -> usize {
        match self {
            Comp::Tc128 => nvidia_ga104::mm_tc128::CBUF_BYTES,
            Comp::Pf => nvidia_ga104::mm_tc_pf::CBUF_BYTES,
            Comp::V4 => nvidia_ga104::mm_tc_v4::CBUF_BYTES,
            Comp::V5 => nvidia_ga104::mm_tc_v5::CBUF_BYTES,
            Comp::V6 => nvidia_ga104::mm_tc_v6::CBUF_BYTES,
            Comp::V7 => nvidia_ga104::mm_tc_v7::CBUF_BYTES,
            Comp::V8 | Comp::V8n => nvidia_ga104::mm_tc_v8::CBUF_BYTES,
            Comp::V8r => nvidia_ga104::mm_tc_v8r::CBUF_BYTES,
            Comp::Tc128V4 => nvidia_ga104::mm_tc128_v4::CBUF_BYTES,
            Comp::Tc => nvidia_ga104::mm_tc::CBUF_BYTES,
        }
    }
    /// The journal names (`kt …`): same sets, sets run, qkv median, `mm_tc`'s qkv median beside it.
    /// P6-214: `at64` = the run at the 64 KiB step (its own names).
    fn kt(self, at64: bool) -> [&'static str; 4] {
        if at64 {
            return match self {
                Comp::Tc => [
                    "kt mm_tc@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc@64k seturi rulate",
                    "kt mm_tc@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc@64k)",
                ],
                Comp::Tc128 => [
                    "kt mm_tc128@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc128@64k seturi rulate",
                    "kt mm_tc128@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc128@64k)",
                ],
                Comp::Pf => [
                    "kt mm_tc_pf@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_pf@64k seturi rulate",
                    "kt mm_tc_pf@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_pf@64k)",
                ],
                Comp::V4 => [
                    "kt mm_tc_v4@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v4@64k seturi rulate",
                    "kt mm_tc_v4@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v4@64k)",
                ],
                Comp::V5 => [
                    "kt mm_tc_v5@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v5@64k seturi rulate",
                    "kt mm_tc_v5@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v5@64k)",
                ],
                Comp::V6 => [
                    "kt mm_tc_v6@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v6@64k seturi rulate",
                    "kt mm_tc_v6@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v6@64k)",
                ],
                Comp::V7 => [
                    "kt mm_tc_v7@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v7@64k seturi rulate",
                    "kt mm_tc_v7@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v7@64k)",
                ],
                Comp::V8 => [
                    "kt mm_tc_v8@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v8@64k seturi rulate",
                    "kt mm_tc_v8@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v8@64k)",
                ],
                Comp::V8n => [
                    "kt mm_tc_v8n@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v8n@64k seturi rulate",
                    "kt mm_tc_v8n@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v8n@64k)",
                ],
                Comp::V8r => [
                    "kt mm_tc_v8r@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc_v8r@64k seturi rulate",
                    "kt mm_tc_v8r@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc_v8r@64k)",
                ],
                Comp::Tc128V4 => [
                    "kt mm_tc128_v4@64k seturi cu aceiasi biti ca mm_tc",
                    "kt mm_tc128_v4@64k seturi rulate",
                    "kt mm_tc128_v4@64k qkv median ns",
                    "kt mm_tc qkv median ns (langa mm_tc128_v4@64k)",
                ],
            };
        }
        match self {
            // P6-214: `mm_tc` as a companion only at 64 KiB; never asked with `at64 == false`.
            Comp::Tc => [
                "kt mm_tc seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc seturi rulate (insotitor)",
                "kt mm_tc qkv median ns (insotitor)",
                "kt mm_tc qkv median ns (langa mm_tc)",
            ],
            Comp::Tc128 => [
                "kt mm_tc128 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc128 seturi rulate",
                "kt mm_tc128 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc128)",
            ],
            Comp::Pf => [
                "kt mm_tc_pf seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_pf seturi rulate",
                "kt mm_tc_pf qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_pf)",
            ],
            Comp::V4 => [
                "kt mm_tc_v4 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v4 seturi rulate",
                "kt mm_tc_v4 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v4)",
            ],
            Comp::V5 => [
                "kt mm_tc_v5 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v5 seturi rulate",
                "kt mm_tc_v5 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v5)",
            ],
            Comp::V6 => [
                "kt mm_tc_v6 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v6 seturi rulate",
                "kt mm_tc_v6 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v6)",
            ],
            Comp::V7 => [
                "kt mm_tc_v7 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v7 seturi rulate",
                "kt mm_tc_v7 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v7)",
            ],
            Comp::V8 => [
                "kt mm_tc_v8 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v8 seturi rulate",
                "kt mm_tc_v8 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v8)",
            ],
            Comp::V8n => [
                "kt mm_tc_v8n seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v8n seturi rulate",
                "kt mm_tc_v8n qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v8n)",
            ],
            Comp::V8r => [
                "kt mm_tc_v8r seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc_v8r seturi rulate",
                "kt mm_tc_v8r qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc_v8r)",
            ],
            Comp::Tc128V4 => [
                "kt mm_tc128_v4 seturi cu aceiasi biti ca mm_tc",
                "kt mm_tc128_v4 seturi rulate",
                "kt mm_tc128_v4 qkv median ns",
                "kt mm_tc qkv median ns (langa mm_tc128_v4)",
            ],
        }
    }
}

/// P6-197: the companion's program and its VRAM page (the same cbuf at +0, its QMD at +0x800).
struct Tc128 {
    kind: Comp,
    /// P6-214: `Some(bytes)` = launched with this shared-memory step (`to_bytes_with_sm_config`); `None` = the smallest
    /// step that fits (today's launch).
    step: Option<u32>,
    prog: Mapping,
    zone: Mapping,
}

impl Tc128 {
    /// P6-214: the name in the rows: the program's, `@64k` added for the run at the 64 KiB step.
    fn label(kind: Comp, step: Option<u32>) -> String {
        match step {
            Some(b) => format!("{}@{}k", kind.name(), b >> 10),
            None => String::from(kind.name()),
        }
    }
    fn name(&self) -> String {
        Self::label(self.kind, self.step)
    }
}

/// P6-197: what `qkv-513` gave with `mm_tc128` (filled only on the board).
#[derive(Default)]
struct Qkv128 {
    set: Option<Tc128Set>,
    /// A copy or a launch the board did not finish (the line is written).
    stopped: bool,
    /// P6-210: the companion cannot take qkv-513 (`mm_tc_v4` and a misaligned product): not launched.
    unfit: bool,
}

/// P6-197: one set, `mm_tc` then `mm_tc128`: the same bytes?, the first differing word, both judgments (times).
struct Tc128Set {
    set: usize,
    same: bool,
    words: usize,
    first_diff: Option<usize>,
    old: nvidia_ga104::mm_tc_probe::SetResult,
    new: nvidia_ga104::mm_tc_probe::SetResult,
}

impl Tc128Set {
    fn new(set: usize, old: &[u8], new: &[u8], ns_old: Vec<u64>, ns_new: Vec<u64>) -> GpuResult<Self> {
        use nvidia_ga104::mm_tc_probe as tp;
        let first_diff = old.chunks(4).zip(new.chunks(4)).position(|(a, b)| a != b);
        Ok(Tc128Set {
            set,
            same: old == new,
            words: old.len() / 4,
            first_diff,
            old: tp::judge_set(set, old, &ns_old)?,
            new: tp::judge_set(set, new, &ns_new)?,
        })
    }
}

/// P6-197: the cbuf (the same bytes as `mm_tc`'s, checked) and `mm_tc128`'s QMD: blocks of 256, grid ⌈N/128⌉ × ⌈M/128⌉,
/// 128 registers, 24 KiB shared + one barrier.
fn tc128_launch_bytes(
    a: &nvidia_ga104::mm_tc::Args,
    cb: &[u8; nvidia_ga104::mm_tc::CBUF_BYTES],
    t: &Tc128,
) -> GpuResult<([u8; nvidia_ga104::mm_tc::CBUF_BYTES], Vec<u8>)> {
    use nvidia_ga104::qmd::ComputeQmd;
    let cb2 = a.cbuf().ok_or(bad("mm_tc companion: argumente"))?;
    if &cb2 != cb {
        return Err(bad("mm_tc companion: cbuf diferit de al lui mm_tc"));
    }
    let k = t.kind;
    let q = ComputeQmd {
        program_va: t.prog.va,
        registers: k.registers(),
        cbuf0_va: t.zone.va,
        cbuf0_size: k.cbuf_bytes() as u32,
        grid: k.grid(a),
        block: k.block(),
        release: None,
    };
    // P6-214: the 64 KiB step when asked (the same QMD fields as `--treapta-pas`, on the board since os-431).
    let q = match t.step {
        Some(b) => q.to_bytes_with_sm_config(&k.shared(), b)?,
        None => q.to_bytes_with_shared(&k.shared())?,
    };
    Ok((cb2, q.to_vec()))
}

/// P6-197: the rows and the verdict of `mm_tc128` beside `mm_tc` (verdict = the same bits on every set run).
#[allow(clippy::too_many_arguments)]
fn tc128_report(p: &str, label: &str, t: &Tc128, r: &[Tc128Set], stopped: Option<usize>, unfit: &[usize], journal: &mut Journal, lines: &mut Vec<String>) {
    use nvidia_ga104::mm_tc_probe as tp;
    let n = t.name();
    let kt = t.kind.kt(t.step.is_some());
    // P6-210: the sets not launched because the product is not aligned for 128-bit reads.
    for &i in unfit {
        lines.push(format!("{}: kronos --gpu: {} set {} ({}): nealiniat (randuri nu multiplu de 4 sau adrese nu multiplu de 16 octeti), sarit", p, n, tp::set(i).map_or("?", |s| s.name), label));
    }
    for x in r {
        // P6-247: `set(i)` also names `qkv-513-dx` / `-dw` (6 / 7).
        let name = tp::set(x.set).map_or("?", |s| s.name);
        let t = match (x.old.median_ns(), x.new.median_ns()) {
            (Some(a), Some(b)) if b > 0 => format!("; mm_tc {} ns, {} {} ns = x{}.{:02}", a, n, b, a / b, (a % b) * 100 / b),
            _ => String::new(),
        };
        let bits = match (x.same, x.first_diff) {
            (true, _) => format!("ACEIASI BITI ({} cuvinte)", x.words),
            (false, Some(w)) => format!("DIFERA (primul cuvant diferit {} din {})", w, x.words),
            (false, None) => format!("DIFERA (lungimi diferite, {} cuvinte la mm_tc)", x.words),
        };
        lines.push(format!(
            "{}: kronos --gpu: {} set {} ({}): {}; {} {} fata de model{}",
            p,
            n,
            name,
            label,
            bits,
            n,
            if x.new.passes() { "in margine" } else { "IN AFARA marginii" },
            t
        ));
    }
    let same = r.iter().filter(|x| x.same).count();
    let qkv = r.iter().find(|x| x.set == tp::QKV_SET);
    journal.push(Event::Value { what: kt[0], value: same as u32 });
    journal.push(Event::Value { what: kt[1], value: r.len() as u32 });
    let speed = match qkv.and_then(|x| x.old.median_ns().zip(x.new.median_ns())) {
        Some((a, b)) if b > 0 => {
            journal.push(Event::Value { what: kt[2], value: b.min(u32::MAX as u64) as u32 });
            journal.push(Event::Value { what: kt[3], value: a.min(u32::MAX as u64) as u32 });
            let g = qkv.and_then(|x| x.new.gflops()).unwrap_or(0);
            format!("qkv-513 mm_tc {} ns, {} {} ns ({}.{:02} TFLOP/s) = x{}.{:02}", a, n, b, g / 1000, (g % 1000) / 10, a / b, (a % b) * 100 / b)
        }
        _ => String::from("qkv-513 fara timp (omis sau SIMULARE)"),
    };
    let verdict = match stopped {
        Some(i) => format!("NU A TERMINAT (la setul {})", i),
        None if same == r.len() && !r.is_empty() => String::from("TRECE"),
        None => String::from("NU TRECE"),
    };
    let skipped = if unfit.is_empty() { String::new() } else { format!(", {} sarite (nealiniate)", unfit.len()) };
    lines.push(format!("{}: kronos --gpu: proba {}: {}: {} / {} seturi cu aceiasi biti ca mm_tc{}, {}", p, n, verdict, same, r.len(), skipped, speed));
}

/// P6-137: one submission of the QMD at `qmd_va` (records in `ctx.rec`); `Ok(ns)` = its time, `Err(last)` = the
/// record never came.
#[allow(clippy::too_many_arguments)]
fn mm_tc_launch<V: Mmio + ?Sized, K: Clock + ?Sized>(
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    sys: &[SysMapping],
    qmd_va: u64,
) -> GpuResult<Result<u64, u32>> {
    put_bytes(st.mem.writer_mut(), ctx.rec.pa, &[0; 32])?;
    let first = TOKEN_FIRST + STRIDE * *subs;
    let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
    let mut pb = PushBuf::new(&mut words);
    transfer::encode_sequence(&mut pb, &[qmd_va], ctx.rec.va, first)?;
    let nw = pb.len();
    match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, maps, sys, doorbell)? {
        Ok(b) => {
            *subs += 1;
            Ok(Ok(transfer::analyze_sequence(&b, 1, first)?[0]))
        }
        Err(got) => Ok(Err(got)),
    }
}

/// P6-137: `qkv-513` (note C-160_P7d, step 4). Its ~82,6 MB buffer in VRAM (if the allocation fails: skipped). The inputs
/// (`[0, c)`, ~26 MB, zeros between A, B and the bias) go up through the arena's upload pages (8 MiB at a time) with the
/// copy program, then come back the same way and are compared byte for byte (nothing is launched on a wrong upload). The
/// read-back ranges (3 bands + the guard) are filled with NaN the same way. Then, on the board, 5 submissions of the same
/// cbuf + QMD, each with its time; the ranges come back through the arena and are judged on the CPU (~0,6 s, after the
/// clock). In SIMULARE the launches are skipped and the NaN ranges are read back and checked instead.
#[allow(clippy::too_many_arguments)]
fn mm_tc_qkv<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    prog_va: u64,
    zone: Mapping,
    t128: &[Tc128],
    q128s: &mut [Qkv128],
    si: usize,
) -> GpuResult<QkvOutcome> {
    use nvidia_ga104::{mm_tc as tc, mm_tc_probe as tp};
    // P6-247: `si` = `QKV_SET`, or `QKV_DX_SET` / `QKV_DW_SET` (`set(i)`, cyber-gpu `1cb1846`).
    let set = tp::set(si).ok_or(bad("mm_tc: set necunoscut"))?;
    let lay = set.layout();
    let big = match map_vram(st, lay.bytes as u64) {
        Ok(m) if m.va.is_multiple_of(PAGE) => m,
        Ok(_) => {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513 omis: bufferul din VRAM nu e aliniat la 4 KiB", p));
            return Ok(QkvOutcome::Skipped);
        }
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513 omis: {} MB de VRAM nu s-au putut lua ({:?})", p, lay.bytes / 1_000_000, e));
            return Ok(QkvOutcome::Skipped);
        }
    };
    // The window: the arena's upload pages (8 MiB), and one VRAM page for the copy's cbuf (+0) and QMD (+0x800).
    let win = I11_BOARD_BYTES;
    let host = st.mem.alloc_sys_mapped(&arena_pages(0, win / PAGE)?)?;
    let cz = map_vram(st, PAGE)?;
    let mut maps = maps.to_vec();
    maps.push(big);
    maps.push(cz);
    let sys = [ctx.sys.clone(), host.clone()];
    // One copy of `len` bytes (a multiple of 16) from `src` to `dst`; `false` = the board did not finish it.
    let copy = |vbar: &mut V, clock: &mut K, journal: &mut Journal, st: &mut GpuState, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, src: u64, dst: u64, len: u64| -> GpuResult<bool> {
        {
            let w = st.mem.writer_mut();
            put_bytes(w, cz.pa, &transfer::copy_cbuf(src, dst, len)?)?;
            put_bytes(w, cz.pa + 0x800, &transfer::copy_qmd(ctx.copy_va, cz.va, len)?.to_bytes()?)?;
        }
        Ok(mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, cz.va + 0x800)?.is_ok())
    };
    // 1. The inputs, up and back.
    let t0 = crate::time::since_boot_us();
    let mut inputs = alloc::vec![0u8; lay.c];
    set.write_inputs(|at, b| inputs[at..at + b.len()].copy_from_slice(b));
    let mut same = true;
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        let at = k as u64 * win;
        let len = (piece.len() as u64).next_multiple_of(16);
        ram_write(&host, piece);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + at, len)? {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513: urcarea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, k));
            return Ok(QkvOutcome::Stopped);
        }
    }
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        let at = k as u64 * win;
        let len = (piece.len() as u64).next_multiple_of(16);
        ram_write(&host, &[]);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + at, host.va, len)? {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513: recitirea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, k));
            return Ok(QkvOutcome::Stopped);
        }
        same &= ram_read(&host, piece.len() as u64) == piece;
    }
    let up_ms = t0.zip(crate::time::since_boot_us()).map_or(0, |(a, b)| b.saturating_sub(a) / 1000);
    let fnv = fnv1a_bytes(&inputs);
    drop(inputs);
    // P6-247: the row stays `qkv-513`'s alone (the backward sets say it in their line).
    if si == tp::QKV_SET {
        journal.push(Event::Value { what: "kt mm_tc qkv intrari recitite identic", value: u32::from(same) });
    }
    if !same {
        lines.push(format!(
            "{}: kronos --gpu: proba mm_tc: qkv-513: intrarile recitite din VRAM NU sunt cele urcate ({} octeti); nimic lansat",
            p, lay.c
        ));
        return Ok(QkvOutcome::Skipped);
    }
    lines.push(format!(
        "{}: kronos --gpu: proba mm_tc: qkv-513: {} octeti de intrari urcati si recititi identic din VRAM (fnv {:#010x}, asteptat {:#010x}) in {} ms; bufferul {} octeti",
        p, lay.c, fnv, set.inputs_fnv(), up_ms, lay.bytes
    ));
    // 2. The read-back ranges filled with NaN (from the window's start, each range its own copy).
    let fill = set.fill_ranges();
    for &(o, n) in &fill {
        let bytes: Vec<u8> = core::iter::repeat_n(tp::FILL.to_le_bytes(), n / 4).flatten().collect();
        ram_write(&host, &bytes);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513: umplerea cu NaN nu s-a terminat; proba oprita, fara reincercare", p));
            return Ok(QkvOutcome::Stopped);
        }
    }
    // 3. The launches (board only), each its own submission and time.
    let mut times = Vec::new();
    if permit.is_some() {
        let (cb, q) = tp::launch_bytes(si, prog_va, zone.va, big.va)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, zone.pa, &cb)?;
            put_bytes(w, zone.pa + tc::CBUF_BYTES as u64, &q)?;
        }
        let t0 = clock.now_us();
        for r in 0..tp::QKV_REPEATS {
            match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, zone.va + tc::CBUF_BYTES as u64)? {
                Ok(ns) => times.push(ns),
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: proba mm_tc: placa nu a terminat la setul {} (qkv-513), lansarea {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p, si, r, got
                    ));
                    return Ok(QkvOutcome::Stopped);
                }
            }
        }
        // P6-250 (C-157 P2zzl): the times of this set, against the cycle's max-speed window.
        let what = format!("mm_tc pe setul {}", set.name);
        if let Some(w) = super::gpu_golden::speed_window_line(p, &what, t0, clock.now_us(), journal) {
            lines.push(w);
        }
    }
    // 4. The ranges back, in order, glued (all of them fit the window together: ~0,15 MB).
    let mut out = Vec::with_capacity(set.read_len());
    for &(o, n) in &set.read_ranges() {
        let len = (n as u64).next_multiple_of(16);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, len)? {
            lines.push(format!("{}: kronos --gpu: proba mm_tc: qkv-513: citirea benzilor nu s-a terminat; proba oprita, fara reincercare", p));
            return Ok(QkvOutcome::Stopped);
        }
        out.extend_from_slice(&ram_read(&host, n as u64));
    }
    // 5. P6-197: `mm_tc128` on the same inputs (board only): the ranges NaN again, 5 launches, the ranges back, compared.
    // P6-210: every companion in turn (`mm_tc_pf`, then `mm_tc_v4`), each from NaN again.
    for (t, q128) in t128.iter().zip(q128s.iter_mut()) {
        if permit.is_none() {
            break;
        }
        if !t.kind.fits(&set.args(big.va)) {
            q128.unfit = true;
            continue;
        }
        for &(o, n) in &fill {
            let bytes: Vec<u8> = core::iter::repeat_n(tp::FILL.to_le_bytes(), n / 4).flatten().collect();
            ram_write(&host, &bytes);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
                lines.push(format!("{}: kronos --gpu: proba {}: qkv-513: umplerea cu NaN nu s-a terminat; proba oprita, fara reincercare", p, t.name()));
                q128.stopped = true;
                return Ok(QkvOutcome::Stopped);
            }
        }
        let (cb, _) = tp::launch_bytes(si, prog_va, zone.va, big.va)?;
        let (cb2, q2) = tc128_launch_bytes(&set.args(big.va), &cb, t)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, t.zone.pa, &cb2)?;
            put_bytes(w, t.zone.pa + tc::CBUF_BYTES as u64, &q2)?;
        }
        let mut times2 = Vec::new();
        let t0 = clock.now_us();
        for r in 0..tp::QKV_REPEATS {
            match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, t.zone.va + tc::CBUF_BYTES as u64)? {
                Ok(ns) => times2.push(ns),
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: proba {}: placa nu a terminat la setul {} (qkv-513), lansarea {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p,
                        t.name(),
                        si,
                        r,
                        got
                    ));
                    q128.stopped = true;
                    return Ok(QkvOutcome::Stopped);
                }
            }
        }
        // P6-250: the companion's times, against the same window.
        let what = format!("{} pe setul {}", t.name(), set.name);
        if let Some(w) = super::gpu_golden::speed_window_line(p, &what, t0, clock.now_us(), journal) {
            lines.push(w);
        }
        let mut out2 = Vec::with_capacity(set.read_len());
        for &(o, n) in &set.read_ranges() {
            let len = (n as u64).next_multiple_of(16);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, len)? {
                lines.push(format!("{}: kronos --gpu: proba {}: qkv-513: citirea benzilor nu s-a terminat; proba oprita, fara reincercare", p, t.name()));
                q128.stopped = true;
                return Ok(QkvOutcome::Stopped);
            }
            out2.extend_from_slice(&ram_read(&host, n as u64));
        }
        q128.set = Some(Tc128Set::new(si, &out, &out2, times.clone(), times2)?);
    }
    if permit.is_none() {
        let nan = out.chunks(4).all(|w| w == tp::FILL.to_le_bytes());
        lines.push(format!(
            "{}: kronos --gpu: proba mm_tc qkv-513 (SIMULARE): benzile si garda umplute cu NaN si recitite ({} octeti, {}); cele {} lansari sarite in simulare (21,7 GFLOP fiecare pe procesorul emulat)",
            p,
            out.len(),
            if nan { "neatinse" } else { "ATINSE" },
            tp::QKV_REPEATS
        ));
        return Ok(QkvOutcome::Skipped);
    }
    Ok(QkvOutcome::Judged(tp::judge_set(si, &out, &times)?))
}

/// P6-141 (C-161 point 4, C-160 point 7 fifth part; cyber-gpu note `C-160_P7e_atentia-pe-placi-proba.md`, commit
/// `44a4d16`): the tiled training attention probe — the chain `attn_fwd_tile` (P, att) → `attn_back_tile0` (the rows of
/// dS) → `attn_back_tile` (dQ | dK | dV), alone on the board, outside the generation path and outside any training step.
/// The 5 small sets of `attn_tile_probe::SETS` judge the correctness (three submissions each, in order, each with its
/// time; each buffer ≤ 2,2 MB in host RAM, the arena's upload pages); the sixth, `atentie-513-b4` (4 × 512, 12 heads ×
/// 64, ~151 MB), gives the first time of the training attention: its buffer in VRAM (skipped if it does not fit), the
/// inputs uploaded through the arena with the copy program and read back byte for byte before any launch, the read-back
/// bands + the guards filled with NaN, then 5 submissions of every program, the bands read back and judged on the CPU
/// (after the clock). In SIMULARE the launches of the big set are skipped (too big for the emulated CPU); everything
/// before them runs. New for the board: these three programs (SHA-256 checked first), 55 / 50 / 73 registers, blocks of
/// 256, 37 / 32 / 32 KiB of shared memory + one barrier; cbuf 0x100 bytes. A launch the board does not finish stops the
/// probe (no retry).
#[allow(clippy::too_many_arguments)]
fn attn_tile_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use nvidia_ga104::attn_tile_probe as ap;
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    for prog in ap::PROGS {
        let sha = sha256_hex(prog.binary());
        if sha != prog.sha256() {
            lines.push(format!(
                "{}: kronos --gpu: proba atentiei pe placi: SHA-256 al programului {} = {} nu e cel din nota ({}): nimic trimis",
                p,
                prog.name(),
                sha,
                prog.sha256()
            ));
            return Err(bad("atentie pe placi: SHA-256 gresit"));
        }
    }
    // P6-149 (note C-157_P2m): the second generation beside the first, only when the list asked for it; P6-160 (note
    // C-157_P2n): the third after the second, likewise.
    let tile2 = super::gpu_tur::ATTN_TILE2_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let tile3 = tile2 && super::gpu_tur::ATTN_TILE3_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-168 (note C-157_P2q): the fourth after the third, likewise.
    let tile4 = tile3 && super::gpu_tur::ATTN_TILE4_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let gens: Vec<AttnGen> = [(tile2, AttnGen::Two), (tile3, AttnGen::Three), (tile4, AttnGen::Four)].into_iter().filter(|g| g.0).map(|g| g.1).collect();
    for &g in &gens {
        for k in 0..3 {
            let sha = sha256_hex(g.binary(k));
            if sha != g.sha256(k) {
                lines.push(format!(
                    "{}: kronos --gpu: proba atentiei pe placi: SHA-256 al programului nou {} = {} nu e cel din nota ({}): nimic trimis",
                    p,
                    g.name(k),
                    sha,
                    g.sha256(k)
                ));
                return Err(bad(match g {
                    AttnGen::Two => "atentie2: SHA-256 gresit",
                    AttnGen::Three => "atentie3: SHA-256 gresit",
                    AttnGen::Four => "atentie4: SHA-256 gresit",
                }));
            }
        }
        lines.push(format!(
            "{}: kronos --gpu: {} ceruta ({}): programele noi {} ({} / {} / {} registre, {} / {} / {} octeti partajati, SHA-256 verificate), dupa lantul vechi pe fiecare set, pe acelasi buffer",
            p,
            g.tag(),
            label,
            [0, 1, 2].map(|k| g.name(k)).join(" -> "),
            g.registers(0),
            g.registers(1),
            g.registers(2),
            g.shared_bytes(0),
            g.shared_bytes(1),
            g.shared_bytes(2)
        ));
    }
    lines.push(format!(
        "{}: kronos --gpu: proba atentiei pe placi ({}): programele {} ({} / {} / {} registre, {} / {} / {} octeti partajati, blocuri de 256, SHA-256 verificate), {} seturi mici cate trei trimiteri, apoi {}",
        p,
        label,
        ap::PROGS.map(|x| x.name()).join(" -> "),
        ap::Prog::Fwd.registers(),
        ap::Prog::Rows.registers(),
        ap::Prog::Cols.registers(),
        ap::Prog::Fwd.shared().bytes,
        ap::Prog::Rows.shared().bytes,
        ap::Prog::Cols.shared().bytes,
        ap::TIMING_SET,
        ap::SETS[ap::TIMING_SET].name
    ));
    // The programs (each aligned to a page), then one VRAM page per program: the cbuf at +0 (0x100 bytes), the QMD at
    // +0x800.
    let mut maps = ctx.maps.clone();
    let mut prog_va = [0u64; 3];
    let mut zones: Vec<Mapping> = Vec::with_capacity(3);
    for (k, prog) in ap::PROGS.iter().enumerate() {
        let m = map_vram(st, program_map_bytes(prog.binary().len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, prog.binary())?;
        prog_va[k] = m.va;
        maps.push(m);
        let z = map_vram(st, PAGE)?;
        maps.push(z);
        zones.push(z);
    }
    // P6-149 / P6-160: the new programs and their pages likewise (only when asked), one entry per generation.
    let mut newer: Vec<GenOnBoard> = Vec::with_capacity(gens.len());
    for &g in &gens {
        let mut gb = GenOnBoard { which: g, prog_va: [0; 3], zones: Vec::with_capacity(3) };
        for k in 0..3 {
            let m = map_vram(st, program_map_bytes(g.binary(k).len() as u64))?;
            put_bytes(st.mem.writer_mut(), m.pa, g.binary(k))?;
            gb.prog_va[k] = m.va;
            maps.push(m);
            let z = map_vram(st, PAGE)?;
            maps.push(z);
            gb.zones.push(z);
        }
        newer.push(gb);
    }
    // P6-227 (note C-157_P2zm): `attn_back_tile_tc` and `attn_back_tile_tc16` after the newest chain, only when asked
    // (and only with the third generation, whose `attn_back_tile_3` they replace).
    let tc_on = tile3 && super::gpu_tur::ATTN_TC_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let mut tcs: Vec<TcOnBoard> = Vec::new();
    if tc_on {
        // P6-233 (note C-157_P2zq): with `--atentie-tc16-v4`, `attn_back_tile_tc16_v4` and `_v4pf` too (indices 2, 3).
        let v4_on = super::gpu_tur::ATTN_TC_V4_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
        let progs: &[AttnTcProg] = if v4_on { &[AttnTcProg::Tc, AttnTcProg::Tc16, AttnTcProg::V4, AttnTcProg::V4pf] } else { &[AttnTcProg::Tc, AttnTcProg::Tc16] };
        for &which in progs {
            let sha = sha256_hex(which.binary());
            if sha != which.sha256() {
                lines.push(format!(
                    "{}: kronos --gpu: atentie-tc: SHA-256 al programului nou {} = {} nu e cel din nota ({}): nimic trimis",
                    p,
                    which.name(),
                    sha,
                    which.sha256()
                ));
                return Err(bad("atentie-tc: SHA-256 gresit"));
            }
            let m = map_vram(st, program_map_bytes(which.binary().len() as u64))?;
            put_bytes(st.mem.writer_mut(), m.pa, which.binary())?;
            maps.push(m);
            let z = map_vram(st, PAGE)?;
            maps.push(z);
            tcs.push(TcOnBoard { which, prog_va: m.va, zone: z });
        }
        lines.push(format!(
            "{}: kronos --gpu: atentie-tc ceruta ({}): programele noi attn_back_tile_tc si attn_back_tile_tc16 ({} / {} registre, {} / {} octeti partajati, argumentele la 0x700 intr-un cbuf de 0x800, SHA-256 verificate), dupa lantul {} pe fiecare set, pe aceleasi P / dS",
            p,
            label,
            AttnTcProg::Tc.registers(),
            AttnTcProg::Tc16.registers(),
            AttnTcProg::Tc.shared().bytes,
            AttnTcProg::Tc16.shared().bytes,
            newer.last().map_or("vechi", |g| g.which.tag())
        ));
        if super::gpu_tur::ATTN_TC_64K_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            lines.push(format!(
                "{}: kronos --gpu: atentie-tc@64k ceruta ({}): dupa fiecare program pe circuitele speciale, acelasi program inca o data cu treapta memoriei comune 64 KiB (campurile lui --treapta-pas, ca in pasul mixt #513), comparat cuvant cu cuvant cu rularea de azi; nimic nou pentru placa",
                p, label
            ));
        }
        if v4_on {
            lines.push(format!(
                "{}: kronos --gpu: atentie-tc16-v4 ceruta ({}): programele noi attn_back_tile_tc16_v4 si attn_back_tile_tc16_v4pf ({} / {} registre, {} octeti partajati, citiri de 128 de biti, SHA-256 verificate), dupa tc16 pe aceleasi P / dS, doar pe seturile aliniate (fits)",
                p,
                label,
                AttnTcProg::V4.registers(),
                AttnTcProg::V4pf.registers(),
                AttnTcProg::V4.shared().bytes
            ));
        }
    }
    // P6-234 (note C-157_P2zs): `attn_back_tile0_tc` (dS with dP on the tensor cores) after them, only when asked.
    let mut tc0: Option<Tc0OnBoard> = None;
    if tc_on && super::gpu_tur::ATTN_TC0_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::attn_back_tile0_tc as a0;
        let sha = sha256_hex(a0::ATTN_BACK_TILE0_TC);
        if sha != a0::ATTN_BACK_TILE0_TC_SHA256 {
            lines.push(format!(
                "{}: kronos --gpu: atentie-tc0: SHA-256 al programului nou attn_back_tile0_tc = {} nu e cel din nota ({}): nimic trimis",
                p,
                sha,
                a0::ATTN_BACK_TILE0_TC_SHA256
            ));
            return Err(bad("atentie-tc0: SHA-256 gresit"));
        }
        let m = map_vram(st, program_map_bytes(a0::ATTN_BACK_TILE0_TC.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, a0::ATTN_BACK_TILE0_TC)?;
        maps.push(m);
        let z = map_vram(st, PAGE)?;
        maps.push(z);
        tc0 = Some(Tc0OnBoard { prog_va: m.va, zone: z, v4_va: None });
        lines.push(format!(
            "{}: kronos --gpu: atentie-tc0 ceruta ({}): programul nou attn_back_tile0_tc ({} registre, {} octeti partajati, argumentele la 0x700 intr-un cbuf de 0x800, SHA-256 verificat), dS cu dP = dO*V^T pe circuitele speciale, dupa programele dQ / dK / dV pe aceleasi P / dO / V, langa dS-ul F32 al lantului {}",
            p,
            label,
            a0::REGISTERS,
            a0::SHARED.bytes,
            newer.last().map_or("vechi", |g| g.which.tag())
        ));
        // P6-235 (note C-157_P2zt): `attn_back_tile0_tc_v4` (128-bit loads / stores) after it, only when asked.
        if super::gpu_tur::ATTN_TC0_V4_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            use nvidia_ga104::attn_back_tile0_tc_v4 as a4;
            let sha = sha256_hex(a4::ATTN_BACK_TILE0_TC_V4);
            if sha != a4::ATTN_BACK_TILE0_TC_V4_SHA256 {
                lines.push(format!(
                    "{}: kronos --gpu: atentie-tc0-v4: SHA-256 al programului nou attn_back_tile0_tc_v4 = {} nu e cel din nota ({}): nimic trimis",
                    p,
                    sha,
                    a4::ATTN_BACK_TILE0_TC_V4_SHA256
                ));
                return Err(bad("atentie-tc0-v4: SHA-256 gresit"));
            }
            let m4 = map_vram(st, program_map_bytes(a4::ATTN_BACK_TILE0_TC_V4.len() as u64))?;
            put_bytes(st.mem.writer_mut(), m4.pa, a4::ATTN_BACK_TILE0_TC_V4)?;
            maps.push(m4);
            if let Some(t) = tc0.as_mut() {
                t.v4_va = Some(m4.va);
            }
            lines.push(format!(
                "{}: kronos --gpu: atentie-tc0-v4 ceruta ({}): programul nou attn_back_tile0_tc_v4 ({} registre, {} octeti partajati, {} citiri si {} scrieri de 128 de biti, SHA-256 verificat), dupa attn_back_tile0_tc pe aceleasi intrari, doar pe seturile aliniate (fits), comparat cuvant cu cuvant cu dS-ul lui attn_back_tile0_tc",
                p,
                label,
                a4::REGISTERS,
                a4::SHARED.bytes,
                a4::LOADS_128,
                a4::STORES_128
            ));
        }
    }
    let mut tc0_sets: Vec<Tc0Set> = Vec::new();
    let mut tc_sets: Vec<TcSet> = Vec::new();
    let mut stopped_tc: Option<usize> = None;
    let small = ap::SETS[..ap::TIMING_SET].iter().map(|s| s.layout().bytes as u64).max().unwrap_or(PAGE);
    let buf = st.mem.alloc_sys_mapped(&arena_pages(0, small.div_ceil(PAGE))?)?;
    if !buf.va.is_multiple_of(PAGE) {
        return Err(bad("atentie pe placi: bufferul nu e aliniat la 4 KiB"));
    }
    let mut probe = ap::Probe::default();
    // Per new generation: its pairs (set by set) and where it stopped.
    let mut pairs: Vec<Vec<ap::PairResult>> = gens.iter().map(|_| Vec::new()).collect();
    let mut stopped: Option<usize> = None;
    let mut stopped2: Option<(usize, usize)> = None;
    'sets: for (i, set) in ap::SETS[..ap::TIMING_SET].iter().enumerate() {
        let sys = [ctx.sys.clone(), buf.clone()];
        ram_write(&buf, &set.buffer()); // the outputs and the guards start as NaN
        let mut ns = [0u64; 3];
        for (k, prog) in ap::PROGS.iter().enumerate() {
            let (cb, q) = ap::launch_bytes(set, *prog, prog_va[k], zones[k].va, buf.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, zones[k].pa, &cb)?;
                put_bytes(w, zones[k].pa + 0x800, &q)?;
            }
            match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, zones[k].va + 0x800)? {
                Ok(t) => ns[k] = t,
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: proba atentiei pe placi: placa nu a terminat la setul {} ({}), programul {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p,
                        i,
                        set.name,
                        prog.name(),
                        got
                    ));
                    stopped = Some(i);
                    break 'sets;
                }
            }
        }
        let mut out = Vec::with_capacity(set.read_len());
        for (o, n) in set.read_ranges() {
            out.extend_from_slice(&ram_read_at(&buf, o as u64, n as u64));
        }
        probe.add(ap::judge_set(i, &out, [&ns[..1], &ns[1..2], &ns[2..]])?);
        // P6-147 (note C-157_P2l): the time of each launch on the small sets too (fixed costs vs work on the board).
        const T: [&str; 3] = ["kt atentie set: timp fwd (ns)", "kt atentie set: timp back0 (ns)", "kt atentie set: timp back1 (ns)"];
        for (k, what) in T.iter().enumerate() {
            journal.push(Event::Value { what: *what, value: u32::try_from(ns[k]).unwrap_or(u32::MAX) });
        }
        lines.push(format!(
            "{}: kronos --gpu: kt atentie set {} ({}): timp fwd {} ns, back0 {} ns, back1 {} ns",
            p, i, set.name, ns[0], ns[1], ns[2]
        ));
        // P6-227: the time of the newest chain's dQ / dK / dV program on this set (beside the tensor-core programs).
        let mut last_back_ns = ns[2];
        // P6-234: likewise its dS program (`attn_back_tile0_*`), beside `attn_back_tile0_tc`.
        let mut last_back0_ns = ns[1];
        // P6-149 / P6-160: the same set again for every new generation, the buffer rewritten each time.
        for (gi, gb) in newer.iter().enumerate() {
            ram_write(&buf, &set.buffer());
            let mut ns2 = [0u64; 3];
            for k in 0..3 {
                let (cb, q) = gb.which.launch_bytes(set, k, gb.prog_va[k], gb.zones[k].va, buf.va)?;
                {
                    let w = st.mem.writer_mut();
                    put_bytes(w, gb.zones[k].pa, &cb)?;
                    put_bytes(w, gb.zones[k].pa + 0x800, &q)?;
                }
                match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, gb.zones[k].va + 0x800)? {
                    Ok(t) => ns2[k] = t,
                    Err(got) => {
                        lines.push(format!(
                            "{}: kronos --gpu: {}: placa nu a terminat la setul {} ({}), programul nou {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                            p,
                            gb.which.tag(),
                            i,
                            set.name,
                            gb.which.name(k),
                            got
                        ));
                        stopped2 = Some((gi, i));
                        break 'sets;
                    }
                }
            }
            let mut out2 = Vec::with_capacity(set.read_len());
            for (o, n) in set.read_ranges() {
                out2.extend_from_slice(&ram_read_at(&buf, o as u64, n as u64));
            }
            pairs[gi].push(ap::judge_pair_set(i, &out, &out2, [&ns2[..1], &ns2[1..2], &ns2[2..]])?);
            last_back_ns = ns2[2];
            last_back0_ns = ns2[1];
        }
        // P6-227: the tensor-core programs on the P / dS the newest chain left in the buffer.
        if !tcs.is_empty() {
            match tc_small_set(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, &maps, &sys, &buf, set, i, &tcs, last_back_ns)? {
                Some(r) => tc_sets.push(r),
                None => {
                    stopped_tc = Some(i);
                    break 'sets;
                }
            }
            // P6-234: then dS on the tensor cores, on the same P / dO / V (the F32 dS read first, then overwritten).
            if let Some(t0) = tc0.as_ref() {
                match tc0_small_set(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, &maps, &sys, &buf, set, i, t0, last_back0_ns)? {
                    Some(r) => tc0_sets.push(r),
                    None => {
                        stopped_tc = Some(i);
                        break 'sets;
                    }
                }
            }
        }
    }
    let mut tc_big: Option<TcBig> = None;
    if stopped.is_none() && stopped2.is_none() && stopped_tc.is_none() {
        match attn_tile_big(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, &maps, prog_va, &zones, &newer, &tcs, tc0.as_ref(), &mut tc_big)? {
            AttnBig::Judged(r, r2) => {
                probe.add(*r);
                for (gi, x) in r2.into_iter().enumerate() {
                    match x {
                        Ok(r2) => pairs[gi].push(*r2),
                        Err(()) => {
                            stopped2 = Some((gi, ap::TIMING_SET));
                            break;
                        }
                    }
                }
            }
            AttnBig::Skipped => {}
            AttnBig::Stopped => stopped = Some(ap::TIMING_SET),
        }
    }
    probe.journal(journal);
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let verdict = match (stopped, stopped2) {
        (Some(i), _) => format!("NU A TERMINAT (la setul {})", i),
        (None, Some((_, i))) if i < ap::TIMING_SET => format!("OPRITA DE UN LANT NOU (la setul {})", i),
        _ if probe.passes() => String::from("TRECE"),
        _ => String::from("NU TRECE"),
    };
    let pp = probe.per_prog();
    let worst = |w: u32| if w == u32::MAX { String::from("-") } else { format!("{}", w) };
    lines.push(format!(
        "{}: kronos --gpu: proba atentiei pe placi: {}: {} / {} seturi in margine; exacte fata de model: fwd {} / {} (cel mai rau {} promile), back0 {} / {}, back1 {} / {}; {}",
        p,
        verdict,
        probe.in_margin(),
        probe.sets.len(),
        pp[0].0,
        pp[0].1,
        worst(pp[0].2),
        pp[1].0,
        pp[1].1,
        pp[2].0,
        pp[2].1,
        if probe.timing_skipped() { "atentie-513-b4 omis" } else { "atentie-513-b4 cronometrat" }
    ));
    // P6-149 / P6-160: every new generation's rows (journal `kt atentie2 …` / `kt atentie3 …`, its time beside the
    // generation before it) and its verdict.
    let probe2 = ap::Probe2 { sets: pairs.first().cloned().unwrap_or_default() };
    // P6-168: the third generation's probe, kept for the fourth's times (beside the third of the same run).
    let mut probe3_kept: Option<ap::Probe3> = None;
    for (gi, gb) in newer.iter().enumerate() {
        let sets = pairs[gi].clone();
        let (rows, passes, in_margin, identical, identical_sets, pp2, skipped) = match gb.which {
            AttnGen::Two => {
                probe2.journal(journal);
                lines.extend(probe2.lines(label, Some(&probe)).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
                (sets.len(), probe2.passes(), probe2.in_margin(), probe2.identical(), probe2.identical_sets(), probe2.per_prog(), probe2.timing_skipped())
            }
            AttnGen::Three => {
                let probe3 = ap::Probe3 { sets };
                probe3.journal(journal);
                lines.extend(probe3.lines(label, Some(&probe2)).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
                let row = (probe3.sets.len(), probe3.passes(), probe3.in_margin(), probe3.identical(), probe3.identical_sets(), probe3.per_prog(), probe3.timing_skipped());
                probe3_kept = Some(probe3);
                row
            }
            AttnGen::Four => {
                // The note C-157_P2q: judged against the first generation, its times beside the third of this run.
                let probe4 = ap::Probe4 { sets };
                probe4.journal(journal);
                lines.extend(probe4.lines(label, probe3_kept.as_ref()).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
                (probe4.sets.len(), probe4.passes(), probe4.in_margin(), probe4.identical(), probe4.identical_sets(), probe4.per_prog(), probe4.timing_skipped())
            }
        };
        let verdict2 = match (stopped, stopped2) {
            (_, Some((g, i))) if g == gi => format!("NU A TERMINAT (la setul {})", i),
            (_, Some((g, i))) if g < gi => format!("NERULAT DUPA SETUL {} (un lant nou de dinainte s-a oprit)", i),
            (Some(i), _) => format!("NERULAT DUPA SETUL {} (lantul vechi s-a oprit)", i),
            _ if passes => String::from("TRECE"),
            _ => String::from("NU TRECE"),
        };
        let n = gb.which.short();
        lines.push(format!(
            "{}: kronos --gpu: {} (vechi langa nou): {}: {} / {} seturi in margine; identice cu vechii: {} ({} / {} seturi); exacte fata de model: fwd{} {} / {} (cel mai rau {} promile), back0_{} {} / {}, back1_{} {} / {}; {}",
            p,
            gb.which.tag(),
            verdict2,
            in_margin,
            rows,
            if identical { "DA" } else { "NU" },
            identical_sets,
            rows,
            n,
            pp2[0].0,
            pp2[0].1,
            worst(pp2[0].2),
            n,
            pp2[1].0,
            pp2[1].1,
            n,
            pp2[2].0,
            pp2[2].1,
            if skipped { "atentie-513-b4 omis" } else { "atentie-513-b4 cronometrat" }
        ));
    }
    if !tcs.is_empty() {
        tc_verdict(p, label, journal, lines, &tc_sets, tc_big.as_ref(), stopped.or(stopped2.map(|x| x.1)), stopped_tc);
    }
    if tc0.is_some() {
        tc0_verdict(p, label, journal, lines, &tc0_sets, tc_big.as_ref().and_then(|b| b.t0.as_ref()), stopped.is_some() || stopped2.is_some() || stopped_tc.is_some());
    }
    if tc0.as_ref().is_some_and(|t| t.v4_va.is_some()) {
        tc0_v4_verdict(p, label, journal, lines, &tc0_sets, tc_big.as_ref().and_then(|b| b.t0.as_ref()), stopped.is_some() || stopped2.is_some() || stopped_tc.is_some());
    }
    match (stopped, stopped2, stopped_tc) {
        (None, None, None) => Ok(()),
        _ => Err(GpuError::Timeout { waited_us: WAIT_US }),
    }
}

/// P6-262 (C-168 point 4, step 3; cyber-gpu note `C-168_P4a_atentie-fwd-tc-proba.md`, commit `b060477`): the probe
/// `--atentie-fwd-tc` — `attn_fwd_tc` (the attention forward on the tensor cores, f16 products) alone on the board,
/// beside `attn_fwd_tile3` on the same inputs, outside any training step. The four small aligned sets of
/// `attn_fwd_tc_probe::SETS` in host RAM (whole), then `atentie-513-b4` in VRAM (inputs up and back through the arena,
/// 5 launches of each program, the bands back). Before EACH program the ranges of `fwd_ranges` (P rows, att rows and
/// their guards) are filled with NaN; `attn_fwd_tc` must leave the tiles above the diagonal tile unwritten. The verdict
/// is `attn_fwd_tc_probe::judge` (a margin against `attn_fwd_tile3`, not the same bits). New for the board: only the
/// program `attn_fwd_tc` (SHA-256 checked first), at the 64 KiB shared step. A launch the board does not finish stops the
/// probe (no retry). In SIMULARE the small sets run on the emulated CPU; the big set's launches are skipped.
#[allow(clippy::too_many_arguments)]
fn attn_fwd_tc_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<bool> {
    use nvidia_ga104::attn_fwd_tc as tc;
    use nvidia_ga104::attn_fwd_tc_probe as fp;
    use nvidia_ga104::attn_tile_probe as ap;
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let old = AttnGen::Three;
    for (name, bin, want) in [(old.name(0), old.binary(0), old.sha256(0)), ("attn_fwd_tc", tc::ATTN_FWD_TC, tc::ATTN_FWD_TC_SHA256)] {
        let sha = sha256_hex(bin);
        if sha != want {
            lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: SHA-256 al programului {} = {} nu e cel din nota ({}): nimic trimis", p, name, sha, want));
            return Err(bad("atentie-fwd-tc: SHA-256 gresit"));
        }
    }
    lines.push(format!(
        "{}: kronos --gpu: atentie-fwd-tc ceruta ({}): programul nou attn_fwd_tc ({} registre, {} octeti partajati, treapta 64 KiB, argumentele la 0x700 intr-un cbuf de 0x800, SHA-256 verificat) langa {} pe aceleasi intrari; {} seturi mici aliniate, apoi {}",
        p,
        label,
        tc::REGISTERS,
        tc::SHARED.bytes,
        old.name(0),
        fp::TIMING_SET,
        fp::SETS[fp::TIMING_SET].name
    ));
    // The two programs, each with one VRAM page: the cbuf at +0 (up to 0x800 bytes), the QMD at +0x800.
    let mut maps = ctx.maps.clone();
    let mut prog_va = [0u64; 2];
    let mut zones: Vec<Mapping> = Vec::with_capacity(2);
    for (k, bin) in [old.binary(0), tc::ATTN_FWD_TC].into_iter().enumerate() {
        let m = map_vram(st, program_map_bytes(bin.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, bin)?;
        prog_va[k] = m.va;
        maps.push(m);
        let z = map_vram(st, PAGE)?;
        maps.push(z);
        zones.push(z);
    }
    // cbuf + QMD of program `k` (0 = tile3, 1 = tc) on `set` with its buffer at `buf_va`.
    let bytes = |k: usize, set: &ap::ProbeSet, buf_va: u64| -> GpuResult<(Vec<u8>, Vec<u8>)> {
        if k == 0 {
            let (cb, q) = ap::launch_bytes3(set, ap::PROGS3[0], prog_va[0], zones[0].va, buf_va)?;
            Ok((cb.to_vec(), q.to_vec()))
        } else {
            let (cb, q) = fp::launch_bytes(set, prog_va[1], zones[1].va, buf_va)?;
            Ok((cb.to_vec(), q.to_vec()))
        }
    };
    let names = [old.name(0), "attn_fwd_tc"];
    let mut probe = fp::Probe::default();
    let mut stopped: Option<usize> = None;
    // 1. The small sets, whole, in host RAM.
    let small = fp::SETS[..fp::TIMING_SET].iter().map(|s| s.layout().bytes as u64).max().unwrap_or(PAGE);
    let buf = st.mem.alloc_sys_mapped(&arena_pages(0, small.div_ceil(PAGE))?)?;
    if !buf.va.is_multiple_of(PAGE) {
        return Err(bad("atentie-fwd-tc: bufferul nu e aliniat la 4 KiB"));
    }
    'sets: for (i, set) in fp::SETS[..fp::TIMING_SET].iter().enumerate() {
        if !fp::fits(set) {
            return Err(bad("atentie-fwd-tc: set nealiniat in nota"));
        }
        let sys = [ctx.sys.clone(), buf.clone()];
        ram_write(&buf, &set.buffer());
        let ranges = fp::fwd_ranges(set);
        let mut outs: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
        let mut ns = [0u64; 2];
        for k in 0..2 {
            for &(o, n) in &ranges {
                let fill: Vec<u8> = core::iter::repeat_n(fp::FILL.to_le_bytes(), n / 4).flatten().collect();
                ram_write_at(&buf, o as u64, &fill);
            }
            let (cb, q) = bytes(k, set, buf.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, zones[k].pa, &cb)?;
                put_bytes(w, zones[k].pa + 0x800, &q)?;
            }
            match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, zones[k].va + 0x800)? {
                Ok(t) => ns[k] = t,
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: atentie-fwd-tc: placa nu a terminat la setul {} ({}), programul {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p, i, set.name, names[k], got
                    ));
                    stopped = Some(i);
                    break 'sets;
                }
            }
            for &(o, n) in &ranges {
                outs[k].extend_from_slice(&ram_read_at(&buf, o as u64, n as u64));
            }
        }
        probe.add(fp::judge(set, &outs[0], &outs[1], &ns[..1], &ns[1..])?);
    }
    // 2. `atentie-513-b4` in VRAM (the time).
    if stopped.is_none() {
        match attn_fwd_tc_big(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, &maps, &zones, &bytes, names)? {
            Some(Ok(r)) => probe.add(r),
            Some(Err(())) => stopped = Some(fp::TIMING_SET),
            None => {}
        }
    }
    lines.extend(probe.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    journal.push(Event::Value { what: "kt atentie-fwd-tc seturi in margine", value: probe.in_limits() as u32 });
    if let Some(s) = probe.sets.iter().find(|s| s.name == fp::SETS[fp::TIMING_SET].name) {
        journal.push(Event::Value { what: "kt atentie-fwd-tc 513-b4 tile3 (ns)", value: s.median_tile3().map_or(0, |x| u32::try_from(x).unwrap_or(u32::MAX)) });
        journal.push(Event::Value { what: "kt atentie-fwd-tc 513-b4 tc (ns)", value: s.median_tc().map_or(0, |x| u32::try_from(x).unwrap_or(u32::MAX)) });
    }
    let timed = probe.sets.iter().any(|s| s.name == fp::SETS[fp::TIMING_SET].name);
    let verdict = match stopped {
        Some(i) => format!("NU A TERMINAT (la setul {})", i),
        None if probe.passes() => String::from("TRECE"),
        None => String::from("NU TRECE"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-fwd-tc (vechi langa nou, {}): {}: {} / {} seturi in margine (O <= 1e-3, P <= 2e-3 fata de {}); {}",
        p,
        label,
        verdict,
        probe.in_limits(),
        probe.sets.len(),
        old.name(0),
        if timed { "atentie-513-b4 cronometrat" } else { "atentie-513-b4 omis" }
    ));
    Ok(stopped.is_none() && probe.passes())
}

/// P6-262: `atentie-513-b4` of the `--atentie-fwd-tc` probe: its buffer in VRAM (if the allocation fails: skipped,
/// `None`), the inputs up and back through the arena (nothing is launched on a wrong upload), then for each program the
/// ranges filled with NaN, 5 launches, the ranges back. `Some(Err(()))` = the board did not finish (a line says where).
/// In SIMULARE the launches are skipped and the NaN ranges are read back and checked instead (`None`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn attn_fwd_tc_big<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    zones: &[Mapping],
    bytes: &dyn Fn(usize, &nvidia_ga104::attn_tile_probe::ProbeSet, u64) -> GpuResult<(Vec<u8>, Vec<u8>)>,
    names: [&str; 2],
) -> GpuResult<Option<Result<nvidia_ga104::attn_fwd_tc_probe::SetResult, ()>>> {
    use nvidia_ga104::attn_fwd_tc_probe as fp;
    let set = &fp::SETS[fp::TIMING_SET];
    let lay = set.layout();
    let big = match map_vram(st, lay.bytes as u64) {
        Ok(m) if m.va.is_multiple_of(PAGE) => m,
        Ok(_) => {
            lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {} omis: bufferul din VRAM nu e aliniat la 4 KiB", p, set.name));
            return Ok(None);
        }
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {} omis: {} MB de VRAM nu s-au putut lua ({:?})", p, set.name, lay.bytes / 1_000_000, e));
            return Ok(None);
        }
    };
    let win = I11_BOARD_BYTES;
    let host = st.mem.alloc_sys_mapped(&arena_pages(0, win / PAGE)?)?;
    let cz = map_vram(st, PAGE)?;
    let mut maps = maps.to_vec();
    maps.push(big);
    maps.push(cz);
    let sys = [ctx.sys.clone(), host.clone()];
    let copy = |vbar: &mut V, clock: &mut K, journal: &mut Journal, st: &mut GpuState, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, src: u64, dst: u64, len: u64| -> GpuResult<bool> {
        {
            let w = st.mem.writer_mut();
            put_bytes(w, cz.pa, &transfer::copy_cbuf(src, dst, len)?)?;
            put_bytes(w, cz.pa + 0x800, &transfer::copy_qmd(ctx.copy_va, cz.va, len)?.to_bytes()?)?;
        }
        Ok(mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, cz.va + 0x800)?.is_ok())
    };
    // The inputs (everything before P), up and back.
    let mut inputs = alloc::vec![0u8; lay.p];
    set.write_inputs(|at, b| inputs[at..at + b.len()].copy_from_slice(b));
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        ram_write(&host, piece);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + k as u64 * win, (piece.len() as u64).next_multiple_of(16))? {
            lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: urcarea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, set.name, k));
            return Ok(Some(Err(())));
        }
    }
    let mut same = true;
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        ram_write(&host, &[]);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + k as u64 * win, host.va, (piece.len() as u64).next_multiple_of(16))? {
            lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: recitirea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, set.name, k));
            return Ok(Some(Err(())));
        }
        same &= ram_read(&host, piece.len() as u64) == piece;
    }
    drop(inputs);
    if !same {
        lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: intrarile recitite din VRAM NU sunt cele urcate; nimic lansat", p, set.name));
        return Ok(None);
    }
    lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: {} octeti de intrari urcati si recititi identic din VRAM; bufferul {} octeti", p, set.name, lay.p, lay.bytes));
    let ranges = fp::fwd_ranges(set);
    let groups = spans(&ranges, win);
    let mut outs: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut times: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    for k in 0..2 {
        for &(start, len, _, _) in &groups {
            let fill: Vec<u8> = core::iter::repeat_n(fp::FILL.to_le_bytes(), len as usize / 4).flatten().collect();
            ram_write(&host, &fill);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + start, len)? {
                lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: umplerea cu NaN nu s-a terminat; proba oprita, fara reincercare", p, set.name));
                return Ok(Some(Err(())));
            }
        }
        if permit.is_some() {
            let (cb, q) = bytes(k, set, big.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, zones[k].pa, &cb)?;
                put_bytes(w, zones[k].pa + 0x800, &q)?;
            }
            for r in 0..set.repeats {
                match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, zones[k].va + 0x800)? {
                    Ok(ns) => times[k].push(ns),
                    Err(got) => {
                        lines.push(format!(
                            "{}: kronos --gpu: atentie-fwd-tc: placa nu a terminat la setul {} ({}), programul {}, lansarea {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                            p, fp::TIMING_SET, set.name, names[k], r, got
                        ));
                        return Ok(Some(Err(())));
                    }
                }
            }
        }
        for &(start, len, first, count) in &groups {
            ram_write(&host, &[]);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + start, host.va, len)? {
                lines.push(format!("{}: kronos --gpu: atentie-fwd-tc: {}: citirea benzilor nu s-a terminat; proba oprita, fara reincercare", p, set.name));
                return Ok(Some(Err(())));
            }
            for &(o, n) in &ranges[first..first + count] {
                outs[k].extend_from_slice(&ram_read_at(&host, o as u64 - start, n as u64));
            }
        }
    }
    if permit.is_none() {
        let nan = outs.iter().all(|o| o.chunks(4).all(|w| w == fp::FILL.to_le_bytes()));
        lines.push(format!(
            "{}: kronos --gpu: atentie-fwd-tc {} (SIMULARE): benzile si garzile umplute cu NaN si recitite de doua ori ({} octeti in {} intervale, {} copii, {}); cele {} lansari sarite in simulare (prea mari pentru procesorul emulat)",
            p,
            set.name,
            outs[0].len(),
            ranges.len(),
            groups.len(),
            if nan { "neatinse" } else { "ATINSE" },
            2 * set.repeats
        ));
        return Ok(None);
    }
    Ok(Some(Ok(fp::judge(set, &outs[0], &outs[1], &times[0], &times[1])?)))
}

/// P6-160: a newer generation of the tiled attention programs (`attn_*_2`, `attn_*_3`), run after the first on the
/// same buffer and compared with it word for word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttnGen {
    Two,
    Three,
    /// P6-168: `attn_fwd_tile3` → `attn_back_tile0_3` → `attn_back_tile_4` (only the last program new).
    Four,
}

impl AttnGen {
    fn tag(self) -> &'static str {
        match self {
            Self::Two => "atentie2",
            Self::Three => "atentie3",
            Self::Four => "atentie4",
        }
    }
    /// "2" / "3" (the suffix in the verdict line: fwd2, back0_2, …).
    fn short(self) -> &'static str {
        match self {
            Self::Two => "2",
            Self::Three => "3",
            Self::Four => "4",
        }
    }
    fn name(self, k: usize) -> &'static str {
        use nvidia_ga104::attn_tile_probe as ap;
        match self {
            Self::Two => ap::PROGS2[k].name(),
            Self::Three => ap::PROGS3[k].name(),
            Self::Four => ap::PROGS4[k].name(),
        }
    }
    fn binary(self, k: usize) -> &'static [u8] {
        use nvidia_ga104::attn_tile_probe as ap;
        match self {
            Self::Two => ap::PROGS2[k].binary(),
            Self::Three => ap::PROGS3[k].binary(),
            Self::Four => ap::PROGS4[k].binary(),
        }
    }
    fn sha256(self, k: usize) -> &'static str {
        use nvidia_ga104::attn_tile_probe as ap;
        match self {
            Self::Two => ap::PROGS2[k].sha256(),
            Self::Three => ap::PROGS3[k].sha256(),
            Self::Four => ap::PROGS4[k].sha256(),
        }
    }
    fn registers(self, k: usize) -> u32 {
        use nvidia_ga104::attn_tile_probe as ap;
        match self {
            Self::Two => ap::PROGS2[k].registers(),
            Self::Three => ap::PROGS3[k].registers(),
            Self::Four => ap::PROGS4[k].registers(),
        }
    }
    fn shared_bytes(self, k: usize) -> u32 {
        use nvidia_ga104::attn_tile_probe as ap;
        match self {
            Self::Two => ap::PROGS2[k].shared().bytes,
            Self::Three => ap::PROGS3[k].shared().bytes,
            Self::Four => ap::PROGS4[k].shared().bytes,
        }
    }
    #[allow(clippy::type_complexity)]
    fn launch_bytes(
        self,
        set: &nvidia_ga104::attn_tile_probe::ProbeSet,
        k: usize,
        prog_va: u64,
        cbuf_va: u64,
        buf_va: u64,
    ) -> GpuResult<(Vec<u8>, Vec<u8>)> {
        use nvidia_ga104::attn_tile_probe as ap;
        let (cb, q) = match self {
            Self::Two => ap::launch_bytes2(set, ap::PROGS2[k], prog_va, cbuf_va, buf_va)?,
            Self::Three => ap::launch_bytes3(set, ap::PROGS3[k], prog_va, cbuf_va, buf_va)?,
            Self::Four => ap::launch_bytes4(set, ap::PROGS4[k], prog_va, cbuf_va, buf_va)?,
        };
        Ok((cb.to_vec(), q.to_vec()))
    }
}

/// P6-160: one new generation's programs and control pages in VRAM.
struct GenOnBoard {
    which: AttnGen,
    prog_va: [u64; 3],
    zones: Vec<Mapping>,
}

/// P6-227 (cyber-gpu note C-157_P2zm): the two tensor-core programs for dQ, dK, dV (`attn_back_tile_3`'s job, f16 inputs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttnTcProg {
    Tc,
    Tc16,
    /// P6-233 (cyber-gpu note C-157_P2zq): tc16 with 128-bit loads; only where `fits` (aligned shapes and addresses).
    V4,
    /// P6-233: V4 with the next tile read ahead.
    V4pf,
}

impl AttnTcProg {
    fn name(self) -> &'static str {
        match self {
            Self::Tc => "attn_back_tile_tc",
            Self::Tc16 => "attn_back_tile_tc16",
            Self::V4 => "attn_back_tile_tc16_v4",
            Self::V4pf => "attn_back_tile_tc16_v4pf",
        }
    }
    fn binary(self) -> &'static [u8] {
        match self {
            Self::Tc => nvidia_ga104::attn_back_tile_tc::ATTN_BACK_TILE_TC,
            Self::Tc16 => nvidia_ga104::attn_back_tile_tc16::ATTN_BACK_TILE_TC16,
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::ATTN_BACK_TILE_TC16_V4,
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::ATTN_BACK_TILE_TC16_V4PF,
        }
    }
    fn sha256(self) -> &'static str {
        match self {
            Self::Tc => nvidia_ga104::attn_back_tile_tc::ATTN_BACK_TILE_TC_SHA256,
            Self::Tc16 => nvidia_ga104::attn_back_tile_tc16::ATTN_BACK_TILE_TC16_SHA256,
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::ATTN_BACK_TILE_TC16_V4_SHA256,
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::ATTN_BACK_TILE_TC16_V4PF_SHA256,
        }
    }
    fn registers(self) -> u32 {
        match self {
            Self::Tc => nvidia_ga104::attn_back_tile_tc::REGISTERS,
            Self::Tc16 => nvidia_ga104::attn_back_tile_tc16::REGISTERS,
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::REGISTERS,
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::REGISTERS,
        }
    }
    fn shared(self) -> nvidia_ga104::qmd::SharedMemory {
        match self {
            Self::Tc => nvidia_ga104::attn_back_tile_tc::SHARED,
            Self::Tc16 => nvidia_ga104::attn_back_tile_tc16::SHARED,
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::SHARED,
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::SHARED,
        }
    }
    /// P6-233: only the 128-bit-load programs have a rule: `fits` (T and d multiples of 4, the addresses of 16 bytes).
    fn fits(self, set: &nvidia_ga104::attn_tile_probe::ProbeSet, buf_va: u64) -> bool {
        let a = set.back_args(buf_va, nvidia_ga104::attn_back::Mode::Columns);
        match self {
            Self::Tc | Self::Tc16 => true,
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::fits(&a),
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::fits(&a),
        }
    }
    /// The cbuf (0x800 bytes, the arguments at 0x700 — laid out by the Cyber-GPU module) and the QMD of this program on
    /// `set`, whose buffer is at `buf_va`; the same arguments and grid as `attn_back_tile_3`.
    /// P6-238 (cyber-gpu note C-157_P2zw, point 3): `step = Some(bytes)` = the QMD with this shared-memory step
    /// (`to_bytes_with_sm_config`, as the mixed step runs it); `None` = the smallest step that fits (today's probe).
    fn launch_bytes(self, set: &nvidia_ga104::attn_tile_probe::ProbeSet, prog_va: u64, cbuf_va: u64, buf_va: u64, step: Option<u32>) -> GpuResult<(Vec<u8>, Vec<u8>)> {
        use nvidia_ga104::attn_back_tile_tc as at;
        use nvidia_ga104::qmd::ComputeQmd;
        let a = set.back_args(buf_va, nvidia_ga104::attn_back::Mode::Columns);
        let cb = match self {
            Self::Tc => at::cbuf(&a),
            Self::Tc16 => nvidia_ga104::attn_back_tile_tc16::cbuf(&a),
            Self::V4 => nvidia_ga104::attn_back_tile_tc16_v4::cbuf(&a),
            Self::V4pf => nvidia_ga104::attn_back_tile_tc16_v4pf::cbuf(&a),
        }
        .ok_or(bad("atentie-tc: argumentele refuzate de modulul programului"))?;
        let q = ComputeQmd {
            program_va: prog_va,
            registers: self.registers(),
            cbuf0_va: cbuf_va,
            cbuf0_size: at::CBUF_BYTES as u32,
            grid: at::grid(&a),
            block: at::BLOCK,
            release: None,
        };
        let q = match step {
            Some(b) => q.to_bytes_with_sm_config(&self.shared(), b)?,
            None => q.to_bytes_with_shared(&self.shared())?,
        };
        Ok((cb.to_vec(), q.to_vec()))
    }
}

/// P6-227: one tensor-core program and its control page (cbuf at +0, 0x800 bytes; QMD at +0x800).
struct TcOnBoard {
    which: AttnTcProg,
    prog_va: u64,
    zone: Mapping,
}

/// P6-227: one small set, both tensor-core programs (index 0 = tc, 1 = tc16).
struct TcSet {
    verdict: Vec<gpu_compute::mm_tc::Verdict>,
    /// P6-230 (cyber-gpu note C-157_P2zo): the same values judged with `attn_tc::judge_nominal` (binary16 subnormals
    /// counted at their nominal exponent, as the os-335 adder aligns them).
    nominal: Vec<gpu_compute::mm_tc::Verdict>,
    guard_ok: Vec<bool>,
    rel_ppm: Vec<Option<u32>>,
    ns: Vec<u64>,
    same_words: bool,
    /// P6-233: v4, v4pf on this set: `None` = not aligned (skipped); else (ns, the same words as tc16, guard untouched).
    v4: Vec<Option<(u64, bool, bool)>>,
    /// P6-238 (C-157_P2zw point 3): per program in `tcs` order, run once more at the 64 KiB step: `None` = not run
    /// (not aligned); else (ns at 64 KiB, ns today, the same words as today's run of the same program, guard untouched).
    at64k: Vec<Option<(u64, u64, bool, bool)>>,
}

impl TcSet {
    fn in_margin(&self) -> bool {
        self.verdict.len() == 2
            && self.verdict.iter().all(|v| matches!(v, gpu_compute::mm_tc::Verdict::InMargine { .. }))
            && self.guard_ok.iter().all(|&g| g)
    }
    /// P6-230: in margin under the nominal-exponent margin (and the guards untouched).
    fn in_margin_nominal(&self) -> bool {
        self.nominal.len() == 2
            && self.nominal.iter().all(|v| matches!(v, gpu_compute::mm_tc::Verdict::InMargine { .. }))
            && self.guard_ok.iter().all(|&g| g)
    }
    fn worst(&self, k: usize) -> Option<u32> {
        match self.verdict.get(k) {
            Some(gpu_compute::mm_tc::Verdict::InMargine { worst_permille, .. }) => Some(*worst_permille),
            _ => None,
        }
    }
}

/// P6-227: `atentie-513-b4` with the tensor-core programs (board only).
#[derive(Default)]
struct TcBig {
    /// Median ns per launch: tc, tc16.
    ns: Vec<u64>,
    /// Median ns of the newest chain's dQ / dK / dV program on the same set.
    f32_ns: u64,
    guard_ok: Vec<bool>,
    finite: Vec<bool>,
    rel_ppm: Vec<Option<u32>>,
    same_words: bool,
    words: usize,
    stopped: Option<String>,
    /// P6-233: v4, v4pf on `atentie-513-b4`: `None` = not aligned; else (median ns, the same bands as tc16, guard untouched).
    v4: Vec<Option<(u64, bool, bool)>>,
    /// P6-234: `attn_back_tile0_tc` on `atentie-513-b4` (only with `--atentie-tc0`).
    t0: Option<Tc0Big>,
    /// P6-238: per program in `tcs` order at the 64 KiB step: `None` = not aligned; else (median ns at 64 KiB, median ns
    /// today, the same bands as today's run of the same program, guard untouched). Empty = not asked.
    at64k: Vec<Option<(u64, u64, bool, bool)>>,
}

/// P6-234 (cyber-gpu note C-157_P2zs): `attn_back_tile0_tc` and its control page (cbuf at +0, 0x800 bytes; QMD at +0x800).
struct Tc0OnBoard {
    prog_va: u64,
    zone: Mapping,
    /// P6-235 (cyber-gpu note C-157_P2zt): `attn_back_tile0_tc_v4` when `--atentie-tc0-v4` asked for it.
    v4_va: Option<u64>,
}

/// P6-235: `attn_back_tile0_tc_v4` after `attn_back_tile0_tc` on one set (or the `atentie-513-b4` rows).
#[derive(Default, Clone)]
enum Tc0V4 {
    /// Not asked.
    #[default]
    Off,
    /// `attn_back_tile0_tc_v4::fits` said no: not launched.
    Unaligned,
    /// Launched: time (median on the big set), words equal bit for bit to tc's dS / of how many, the guard untouched.
    Ran { ns: u64, same: usize, words: usize, guard_ok: bool },
    /// The board did not finish (or a copy did not).
    Stopped(String),
}

impl Tc0V4 {
    /// The v4 words equal to tc's everywhere, the guard untouched (`None` = not launched).
    fn ok(&self) -> Option<bool> {
        match self {
            Tc0V4::Ran { same, words, guard_ok, .. } => Some(same == words && *guard_ok),
            Tc0V4::Stopped(_) => Some(false),
            _ => None,
        }
    }
}

/// P6-234: one small set with `attn_back_tile0_tc`, beside the newest chain's F32 dS.
struct Tc0Set {
    ns: u64,
    f32_ns: u64,
    /// Words under (and on) the diagonal equal bit for bit to `attn_tc::back0_model` / how many such words.
    exact: usize,
    lower: usize,
    /// NaN left under (or on) the diagonal (the program must write all of them).
    nan_lower: usize,
    /// Words above the diagonal that are not +0 / −0 (the model writes zeros there), of `upper`.
    upper_nonzero: usize,
    upper: usize,
    guard_ok: bool,
    /// ‖dS_tc − dS_F32‖ / ‖dS_F32‖ under the diagonal, in millionths.
    rel_ppm: Option<u32>,
    /// P6-235: `attn_back_tile0_tc_v4` on the same inputs.
    v4: Tc0V4,
}

impl Tc0Set {
    /// Passes: every word under the diagonal written and finite, the guard untouched, within 1e-3 of the F32 dS.
    fn passes(&self) -> bool {
        self.nan_lower == 0 && self.guard_ok && self.rel_ppm.is_some_and(|r| r <= 1000)
    }
}

/// P6-234: `attn_back_tile0_tc` on `atentie-513-b4` (board only): the rows T − 1 of the banded texts, every head.
#[derive(Default)]
struct Tc0Big {
    ns: u64,
    f32_ns: u64,
    rel_ppm: Option<u32>,
    finite: bool,
    guard_ok: bool,
    words: usize,
    stopped: Option<String>,
    /// P6-235: `attn_back_tile0_tc_v4` on the same rows.
    v4: Tc0V4,
}

/// P6-227: the median of some times (0 if none).
fn median_ns(v: &[u64]) -> u64 {
    let mut s = v.to_vec();
    s.sort_unstable();
    s.get(s.len() / 2).copied().unwrap_or(0)
}

/// P6-227: little-endian f32 words.
fn f32_words(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()
}

/// P6-227: ‖a − b‖ / ‖b‖ in millionths (`None`: a non-finite value, different lengths or ‖b‖ = 0).
fn rel_ppm(a: &[f32], b: &[f32]) -> Option<u32> {
    if a.len() != b.len() || a.iter().chain(b).any(|x| !x.is_finite()) {
        return None;
    }
    let (mut num, mut den) = (0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let d = f64::from(*x) - f64::from(*y);
        num += d * d;
        den += f64::from(*y) * f64::from(*y);
    }
    if den == 0.0 {
        return None;
    }
    let r2 = ((num / den) * 1e12) as u64;
    Some(u32::try_from(r2.isqrt()).unwrap_or(u32::MAX))
}

/// P6-227: millionths as `x,yy·10^-4` (`-` for none).
fn ppm_text(v: Option<u32>) -> String {
    match v {
        Some(x) => format!("{},{:02}e-4", x / 100, x % 100),
        None => String::from("-"),
    }
}

/// P6-227: one small set with both tensor-core programs, on the buffer the newest chain left (P and dS in place).
/// `None`: the board did not finish a launch (the line is written).
#[allow(clippy::too_many_arguments)]
fn tc_small_set<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    sys: &[SysMapping],
    buf: &SysMapping,
    set: &nvidia_ga104::attn_tile_probe::ProbeSet,
    i: usize,
    tcs: &[TcOnBoard],
    f32_ns: u64,
) -> GpuResult<Option<TcSet>> {
    use nvidia_ga104::attn_tile_probe as ap;
    let lay = set.layout();
    let shape = set.shape();
    let (nq, no, np) = shape.sizes().ok_or(bad("atentie-tc: forma setului"))?;
    let whole = ram_read(buf, lay.end as u64);
    let take = |o: usize, n: usize| f32_words(&whole[o..o + 4 * n]);
    let (qkv, dout, pm, ds, f32dq) = (take(lay.qkv, nq), take(lay.dout, no), take(lay.p, np), take(lay.ds, np), take(lay.dqkv, nq));
    let span = 4 * nq + 4 * ap::GUARD;
    let fill: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), span / 4).flatten().collect();
    let mut r = TcSet { verdict: Vec::new(), nominal: Vec::new(), guard_ok: Vec::new(), rel_ppm: Vec::new(), ns: Vec::new(), same_words: false, v4: Vec::new(), at64k: Vec::new() };
    let mut words: Vec<Vec<u8>> = Vec::new();
    // P6-238: today's words and time of every program that ran (in `tcs` order; `None` = not aligned).
    let mut today: Vec<Option<(u64, Vec<u8>)>> = Vec::new();
    for (k, tcb) in tcs.iter().enumerate() {
        // P6-233: v4 / v4pf only where their 128-bit loads are aligned; otherwise skipped (never launched).
        if k >= 2 && !tcb.which.fits(set, buf.va) {
            r.v4.push(None);
            today.push(None);
            continue;
        }
        ram_write_at(buf, lay.dqkv as u64, &fill);
        let (cb, q) = tcb.which.launch_bytes(set, tcb.prog_va, tcb.zone.va, buf.va, None)?;
        {
            let w = st.mem.writer_mut();
            put_bytes(w, tcb.zone.pa, &cb)?;
            put_bytes(w, tcb.zone.pa + 0x800, &q)?;
        }
        let ns = match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, maps, sys, tcb.zone.va + 0x800)? {
            Ok(t) => t,
            Err(got) => {
                lines.push(format!(
                    "{}: kronos --gpu: atentie-tc: placa nu a terminat la setul {} ({}), programul nou {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                    p,
                    i,
                    set.name,
                    tcb.which.name(),
                    got
                ));
                return Ok(None);
            }
        };
        let got = ram_read_at(buf, lay.dqkv as u64, span as u64);
        today.push(Some((ns, got[..4 * nq].to_vec())));
        if k >= 2 {
            let guard = got[4 * nq..].chunks(4).all(|w| w == ap::FILL.to_le_bytes());
            let same = words.get(1).is_some_and(|w| w[..] == got[..4 * nq]);
            r.v4.push(Some((ns, same, guard)));
            continue;
        }
        r.ns.push(ns);
        r.guard_ok.push(got[4 * nq..].chunks(4).all(|w| w == ap::FILL.to_le_bytes()));
        let v = f32_words(&got[..4 * nq]);
        r.verdict.push(gpu_compute::attn_tc::judge(&shape, &qkv, &pm, &dout, &ds, &v));
        r.nominal.push(gpu_compute::attn_tc::judge_nominal(&shape, &qkv, &pm, &dout, &ds, &v));
        r.rel_ppm.push(rel_ppm(&v, &f32dq));
        words.push(got[..4 * nq].to_vec());
    }
    r.same_words = words.len() >= 2 && words[0] == words[1];
    // P6-238 (C-157_P2zw point 3): every program that ran, once more with the shared-memory step at 64 KiB (the step the
    // mixed #513 step runs them at: 3–4 blocks per unit for tc16 instead of 1), dqkv filled with NaN again, its words
    // against today's run of the same program.
    if super::gpu_tur::ATTN_TC_64K_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        for (tcb, t) in tcs.iter().zip(today.iter()) {
            let Some((ns0, w0)) = t else {
                r.at64k.push(None);
                continue;
            };
            ram_write_at(buf, lay.dqkv as u64, &fill);
            let (cb, q) = tcb.which.launch_bytes(set, tcb.prog_va, tcb.zone.va, buf.va, Some(STEP_64K))?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, tcb.zone.pa, &cb)?;
                put_bytes(w, tcb.zone.pa + 0x800, &q)?;
            }
            let ns = match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, maps, sys, tcb.zone.va + 0x800)? {
                Ok(t) => t,
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: atentie-tc@64k: placa nu a terminat la setul {} ({}), programul {} la treapta 64 KiB: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p,
                        i,
                        set.name,
                        tcb.which.name(),
                        got
                    ));
                    return Ok(None);
                }
            };
            let got = ram_read_at(buf, lay.dqkv as u64, span as u64);
            let guard = got[4 * nq..].chunks(4).all(|w| w == ap::FILL.to_le_bytes());
            r.at64k.push(Some((ns, *ns0, got[..4 * nq] == w0[..], guard)));
        }
        let parts: Vec<String> = tcs
            .iter()
            .zip(r.at64k.iter())
            .map(|(tcb, x)| match x {
                Some((ns, ns0, same, guard)) => format!(
                    "{} {} ns (azi {} ns), {}{}",
                    tcb.which.name(),
                    ns,
                    ns0,
                    if *same { "ACEIASI BITI" } else { "DIFERIT" },
                    if *guard { "" } else { ", garda ATINSA" }
                ),
                None => format!("{} nealiniat (sarit)", tcb.which.name()),
            })
            .collect();
        lines.push(format!("{}: kronos --gpu: kt atentie-tc@64k set {} ({}): {}", p, i, set.name, parts.join("; ")));
    }
    let worst = |k: usize| verdict_text(r.verdict.get(k));
    fn verdict_text(v: Option<&gpu_compute::mm_tc::Verdict>) -> String {
        match v {
        Some(gpu_compute::mm_tc::Verdict::InMargine { worst_permille, .. }) => format!("{} promile", worst_permille),
        Some(gpu_compute::mm_tc::Verdict::NuTrece { index, got, reference, margin }) => {
            format!("NU (primul element in afara, nr. {} pe cap: {:e}, modelul {:e}, marginea {:e})", index, got, reference, margin)
        }
        Some(gpu_compute::mm_tc::Verdict::Invalid) => String::from("INVALID (lungime gresita sau valoare nefinita)"),
        None => String::from("-"),
        }
    }
    lines.push(format!(
        "{}: kronos --gpu: kt atentie-tc set {} ({}): tc {} ns, tc16 {} ns (dQ/dK/dV F32 {} ns); in margine: tc {}, tc16 {}; garda dqkv {}; tc16 = tc cuvant cu cuvant: {}; abaterea fata de F32: tc {}, tc16 {}",
        p,
        i,
        set.name,
        r.ns.first().copied().unwrap_or(0),
        r.ns.get(1).copied().unwrap_or(0),
        f32_ns,
        worst(0),
        worst(1),
        if r.guard_ok.iter().all(|&g| g) { "neatinsa" } else { "ATINSA" },
        if r.same_words { "DA" } else { "NU" },
        ppm_text(r.rel_ppm.first().copied().flatten()),
        ppm_text(r.rel_ppm.get(1).copied().flatten())
    ));
    // P6-230 (C-157_P2zo): the second margin, beside the first, never instead of it.
    lines.push(format!(
        "{}: kronos --gpu: kt atentie-tc set {} ({}): marginea-subnormale (attn_tc::judge_nominal) {}: tc {}, tc16 {}",
        p,
        i,
        set.name,
        if r.in_margin_nominal() { "in margine" } else { "NU" },
        verdict_text(r.nominal.first()),
        verdict_text(r.nominal.get(1))
    ));
    const T: [&str; 2] = ["kt atentie-tc set: timp tc (ns)", "kt atentie-tc set: timp tc16 (ns)"];
    for (k, what) in T.iter().enumerate() {
        journal.push(Event::Value { what: *what, value: u32::try_from(r.ns.get(k).copied().unwrap_or(0)).unwrap_or(u32::MAX) });
    }
    // P6-233 (C-157_P2zq): v4 / v4pf beside tc16, word for word.
    if !r.v4.is_empty() {
        let one = |x: Option<&Option<(u64, bool, bool)>>| match x {
            Some(Some((ns, same, guard))) => format!(
                "{} ns, {}{}",
                ns,
                if *same { "ACEIASI BITI ca tc16" } else { "DIFERIT de tc16" },
                if *guard { "" } else { ", garda ATINSA" }
            ),
            Some(None) => String::from("nealiniat (sarit)"),
            None => String::from("-"),
        };
        lines.push(format!(
            "{}: kronos --gpu: kt atentie-tc16-v4 set {} ({}): tc16 {} ns; v4 {}; v4pf {}",
            p,
            i,
            set.name,
            r.ns.get(1).copied().unwrap_or(0),
            one(r.v4.first()),
            one(r.v4.get(1))
        ));
    }
    Ok(Some(r))
}

/// P6-227: the rows and the verdict of the tensor-core programs. `before`: the set where a chain before them stopped.
#[allow(clippy::too_many_arguments)]
fn tc_verdict(p: &str, label: &str, journal: &mut Journal, lines: &mut Vec<String>, sets: &[TcSet], big: Option<&TcBig>, before: Option<usize>, stopped: Option<usize>) {
    use nvidia_ga104::attn_tile_probe as ap;
    let n = sets.len();
    let in_margin = sets.iter().filter(|s| s.in_margin()).count();
    let in_nominal = sets.iter().filter(|s| s.in_margin_nominal()).count();
    let same = sets.iter().filter(|s| s.same_words).count();
    let worst = |k: usize| sets.iter().filter_map(|s| s.worst(k)).max();
    let max_rel = sets.iter().flat_map(|s| s.rel_ppm.iter().copied()).try_fold(0u32, |m, x| x.map(|x| m.max(x)));
    journal.push(Event::Value { what: "kt atentie-tc seturi in margine", value: in_margin as u32 });
    journal.push(Event::Value { what: "kt atentie-tc seturi in marginea-subnormale", value: in_nominal as u32 });
    journal.push(Event::Value { what: "kt atentie-tc seturi tc16 = tc", value: same as u32 });
    journal.push(Event::Value { what: "kt atentie-tc abatere fata de F32 (milionimi)", value: max_rel.unwrap_or(u32::MAX) });
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc vechi langa nou ({}): {} / {} seturi in margine (gpu_compute::attn_tc::judge, marginea mm_tc pe cap; cel mai rau tc {} / tc16 {} promile), {} / {} seturi cu tc16 = tc cuvant cu cuvant, abaterea fata de F32 (norma relativa) cel mult {}",
        p,
        label,
        in_margin,
        n,
        worst(0).map_or(String::from("-"), |w| format!("{}", w)),
        worst(1).map_or(String::from("-"), |w| format!("{}", w)),
        same,
        n,
        ppm_text(max_rel)
    ));
    let mut big_ok = true;
    let big_text = match big {
        None => String::from("atentie-513-b4 omis"),
        Some(b) if b.stopped.is_some() => {
            big_ok = false;
            format!("atentie-513-b4: {}", b.stopped.as_deref().unwrap_or(""))
        }
        Some(b) => {
            let ok = b.guard_ok.iter().all(|&g| g) && b.finite.iter().all(|&f| f) && b.same_words;
            big_ok = ok;
            for (k, what) in ["kt atentie-tc mare timp tc (ns)", "kt atentie-tc mare timp tc16 (ns)"].iter().enumerate() {
                journal.push(Event::Value { what: *what, value: u32::try_from(b.ns.get(k).copied().unwrap_or(0)).unwrap_or(u32::MAX) });
            }
            journal.push(Event::Value { what: "kt atentie-tc mare timp F32 (ns)", value: u32::try_from(b.f32_ns).unwrap_or(u32::MAX) });
            let us = |ns: u64| format!("{},{:02} ms", ns / 1_000_000, (ns / 10_000) % 100);
            let tc = b.ns.first().copied().unwrap_or(0);
            let tc16 = b.ns.get(1).copied().unwrap_or(0);
            lines.push(format!(
                "{}: kronos --gpu: kt atentie-tc atentie-513-b4: pe lansare (mediana a {}) tc {}, tc16 {}, dQ/dK/dV F32 {}; pe pasul #513 (x 12): tc {}, tc16 {}, F32 {}; benzile dqkv ({} cuvinte): tc16 = tc {}, garzi {}, abaterea fata de F32 tc {} / tc16 {}",
                p,
                ap::REPEATS,
                us(tc),
                us(tc16),
                us(b.f32_ns),
                us(12 * tc),
                us(12 * tc16),
                us(12 * b.f32_ns),
                b.words,
                if b.same_words { "DA" } else { "NU" },
                if b.guard_ok.iter().all(|&g| g) { "neatinse" } else { "ATINSE" },
                ppm_text(b.rel_ppm.first().copied().flatten()),
                ppm_text(b.rel_ppm.get(1).copied().flatten())
            ));
            String::from("atentie-513-b4 cronometrat")
        }
    };
    let verdict = match (stopped, before) {
        (Some(i), _) => format!("NU A TERMINAT (la setul {})", i),
        (None, Some(i)) if n < ap::TIMING_SET => format!("NERULAT DUPA SETUL {} (un lant de dinainte s-a oprit)", i),
        // P6-230: the verdict follows the nominal-exponent margin (C-157_P2zo); the old count stays written beside it.
        _ if n == ap::TIMING_SET && in_nominal == n && same == n && big_ok => String::from("TRECE"),
        _ => String::from("NU TRECE"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc (vechi langa nou): {}: {} / {} seturi in marginea-subnormale (marginea veche {} / {}); tc16 = tc: {} ({} / {} seturi); abaterea fata de F32 cel mult {}; {}",
        p,
        verdict,
        in_nominal,
        n,
        in_margin,
        n,
        if same == n && n > 0 { "DA" } else { "NU" },
        same,
        n,
        ppm_text(max_rel),
        big_text
    ));
    tc_v4_verdict(p, label, journal, lines, sets, big);
    tc_64k_verdict(p, label, journal, lines, sets, big);
}

/// P6-238 (cyber-gpu note C-157_P2zw, point 3): the rows of the same tensor-core programs at the 64 KiB step (only when
/// they ran): the same words as today's run on every set, and on `atentie-513-b4` the median per launch beside today's.
fn tc_64k_verdict(p: &str, label: &str, journal: &mut Journal, lines: &mut Vec<String>, sets: &[TcSet], big: Option<&TcBig>) {
    if sets.iter().all(|s| s.at64k.is_empty()) {
        return;
    }
    const NAMES: [&str; 4] = ["tc", "tc16", "v4", "v4pf"];
    let progs = sets.iter().map(|s| s.at64k.len()).max().unwrap_or(0).min(NAMES.len());
    let mut ok = true;
    let mut parts: Vec<String> = Vec::new();
    for (k, name) in NAMES.iter().enumerate().take(progs) {
        let ran: Vec<&(u64, u64, bool, bool)> = sets.iter().filter_map(|s| s.at64k.get(k).and_then(|x| x.as_ref())).collect();
        let same = ran.iter().filter(|x| x.2 && x.3).count();
        ok &= same == ran.len() && !ran.is_empty();
        parts.push(format!("{}@64k = {} pe {} / {} seturi", name, name, same, ran.len()));
    }
    let all_same = sets.iter().filter(|s| !s.at64k.is_empty() && s.at64k.iter().flatten().all(|x| x.2 && x.3)).count();
    journal.push(Event::Value { what: "kt atentie-tc@64k seturi cu aceiasi biti", value: all_same as u32 });
    let ms = |ns: u64| format!("{},{:02} ms", ns / 1_000_000, (ns / 10_000) % 100);
    let big_text = match big {
        Some(b) if b.stopped.is_none() && !b.at64k.is_empty() => {
            const WHAT: [&str; 4] = ["kt atentie-tc@64k mare timp tc (ns)", "kt atentie-tc@64k mare timp tc16 (ns)", "kt atentie-tc@64k mare timp v4 (ns)", "kt atentie-tc@64k mare timp v4pf (ns)"];
            let mut t: Vec<String> = Vec::new();
            for (k, name) in NAMES.iter().enumerate().take(b.at64k.len()) {
                match b.at64k.get(k) {
                    Some(Some((ns, ns0, same, guard))) => {
                        ok &= *same && *guard;
                        journal.push(Event::Value { what: WHAT[k], value: u32::try_from(*ns).unwrap_or(u32::MAX) });
                        let ratio = if *ns == 0 { String::from("-") } else { format!("x{},{:02}", ns0 / ns, (ns0 % ns) * 100 / ns) };
                        t.push(format!(
                            "{} {} (azi {}, {} fata de azi; pe pasul #513 x 12: {}; benzile {}{})",
                            name,
                            ms(*ns),
                            ms(*ns0),
                            ratio,
                            ms(12 * ns),
                            if *same { "= azi" } else { "DIFERITE de azi" },
                            if *guard { "" } else { ", garzi ATINSE" }
                        ));
                    }
                    Some(None) => t.push(format!("{} nealiniat (sarit)", name)),
                    None => t.push(format!("{} -", name)),
                }
            }
            format!("atentie-513-b4 pe lansare (mediana a {}): {}", nvidia_ga104::attn_tile_probe::REPEATS, t.join(", "))
        }
        Some(b) if b.stopped.is_some() => {
            ok = false;
            String::from("atentie-513-b4 oprit")
        }
        _ => String::from("atentie-513-b4 omis"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc@64k (aceleasi programe la treapta 64 KiB, {}): {}: {}; {}",
        p,
        label,
        if ok { "TRECE" } else { "NU TRECE" },
        parts.join(", "),
        big_text
    ));
}

/// P6-233 (cyber-gpu note C-157_P2zq): the rows of `attn_back_tile_tc16_v4` / `_v4pf` beside tc16 (only when they ran):
/// how many aligned sets gave the same words as tc16, and on `atentie-513-b4` the median per launch and the speed-up.
fn tc_v4_verdict(p: &str, label: &str, journal: &mut Journal, lines: &mut Vec<String>, sets: &[TcSet], big: Option<&TcBig>) {
    if sets.iter().all(|s| s.v4.is_empty()) {
        return;
    }
    let names = ["v4", "v4pf"];
    let mut ok = true;
    let mut parts: Vec<String> = Vec::new();
    for (k, name) in names.iter().enumerate() {
        let ran: Vec<&(u64, bool, bool)> = sets.iter().filter_map(|s| s.v4.get(k).and_then(|x| x.as_ref())).collect();
        let same = ran.iter().filter(|x| x.1 && x.2).count();
        let skipped = sets.iter().filter(|s| matches!(s.v4.get(k), Some(None))).count();
        ok &= same == ran.len() && !ran.is_empty();
        journal.push(Event::Value { what: if k == 0 { "kt atentie-tc16-v4 seturi v4 = tc16" } else { "kt atentie-tc16-v4 seturi v4pf = tc16" }, value: same as u32 });
        parts.push(format!("{} = tc16 pe {} / {} seturi aliniate ({} sarite)", name, same, ran.len(), skipped));
    }
    let big_text = match big {
        Some(b) if b.stopped.is_none() && !b.v4.is_empty() => {
            let tc16 = b.ns.get(1).copied().unwrap_or(0);
            let ms = |ns: u64| format!("{},{:02} ms", ns / 1_000_000, (ns / 10_000) % 100);
            let ratio = |ns: u64| if ns == 0 { String::from("-") } else { format!("x{},{:02}", tc16 / ns, (tc16 % ns) * 100 / ns) };
            let mut t: Vec<String> = Vec::new();
            for (k, name) in names.iter().enumerate() {
                match b.v4.get(k) {
                    Some(Some((ns, same, guard))) => {
                        ok &= *same && *guard;
                        journal.push(Event::Value { what: if k == 0 { "kt atentie-tc16-v4 mare timp v4 (ns)" } else { "kt atentie-tc16-v4 mare timp v4pf (ns)" }, value: u32::try_from(*ns).unwrap_or(u32::MAX) });
                        t.push(format!("{} {} ({} fata de tc16, benzile {}{})", name, ms(*ns), ratio(*ns), if *same { "= tc16" } else { "DIFERITE de tc16" }, if *guard { "" } else { ", garzi ATINSE" }));
                    }
                    Some(None) => t.push(format!("{} nealiniat (sarit)", name)),
                    None => t.push(format!("{} -", name)),
                }
            }
            format!("atentie-513-b4 pe lansare (mediana a {}): tc16 {}, {}", nvidia_ga104::attn_tile_probe::REPEATS, ms(tc16), t.join(", "))
        }
        Some(b) if b.stopped.is_some() => {
            ok = false;
            String::from("atentie-513-b4 oprit")
        }
        _ => String::from("atentie-513-b4 omis"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc16-v4 (vechi langa nou, {}): {}: {}; {}",
        p,
        label,
        if ok { "TRECE" } else { "NU TRECE" },
        parts.join(", "),
        big_text
    ));
}

/// P6-141: what happened to `atentie-513-b4`.
enum AttnBig {
    /// The old chain judged; P6-149 / P6-160: each new chain's pair in order (`Err(())` = a launch or a copy of that
    /// chain the board did not finish — the line is written, and nothing after it ran).
    #[allow(clippy::type_complexity)]
    Judged(
        alloc::boxed::Box<nvidia_ga104::attn_tile_probe::SetResult>,
        Vec<Result<alloc::boxed::Box<nvidia_ga104::attn_tile_probe::PairResult>, ()>>,
    ),
    /// Not run (no VRAM, a wrong upload, or SIMULARE): the `kt atentie timp omis` row says so.
    Skipped,
    /// A copy or a launch the board did not finish (the line is written).
    Stopped,
}

/// P6-141: consecutive `(offset, len)` ranges (ascending) grouped into spans of at most `win` bytes: `(start, len,
/// first range, ranges)`; each span starts at its first range and ends at its last range's end, rounded up to 16.
fn spans(ranges: &[(usize, usize)], win: u64) -> Vec<(u64, u64, usize, usize)> {
    let mut out: Vec<(u64, u64, usize, usize)> = Vec::new();
    for (k, &(o, n)) in ranges.iter().enumerate() {
        let (o, n) = (o as u64, n as u64);
        if let Some(last) = out.last_mut() {
            let len = (o + n - last.0).next_multiple_of(16);
            if len <= win {
                last.1 = len;
                last.3 += 1;
                continue;
            }
        }
        out.push((o, n.next_multiple_of(16), k, 1));
    }
    out
}

/// P6-141: `atentie-513-b4` (note C-160_P7e, step 4). Its ~151 MB buffer in VRAM (if the allocation fails: skipped). The
/// inputs (qkv and dO, ~25 MB) go up through the arena's upload pages (8 MiB at a time) with the copy program, then come
/// back the same way and are compared byte for byte (nothing is launched on a wrong upload). The read-back ranges (bands
/// of P / att / dS / dqkv + the 4 guards) are filled with NaN in spans of at most 8 MiB (whatever lies between two
/// ranges of a span is output the programs rewrite whole). Then, on the board, 5 × Fwd, 5 × Rows, 5 × Cols, each its
/// own submission and time; the ranges come back through the arena and are judged on the CPU (after the clock). In
/// SIMULARE the launches are skipped and the NaN ranges are read back and checked instead.
#[allow(clippy::too_many_arguments)]
fn attn_tile_big<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    prog_va: [u64; 3],
    zones: &[Mapping],
    newer: &[GenOnBoard],
    tcs: &[TcOnBoard],
    tc0: Option<&Tc0OnBoard>,
    tc_big: &mut Option<TcBig>,
) -> GpuResult<AttnBig> {
    use nvidia_ga104::attn_tile_probe as ap;
    let set = &ap::SETS[ap::TIMING_SET];
    let lay = set.layout();
    let big = match map_vram(st, lay.bytes as u64) {
        Ok(m) if m.va.is_multiple_of(PAGE) => m,
        Ok(_) => {
            lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: {} omis: bufferul din VRAM nu e aliniat la 4 KiB", p, set.name));
            return Ok(AttnBig::Skipped);
        }
        Err(e) => {
            lines.push(format!(
                "{}: kronos --gpu: proba atentiei pe placi: {} omis: {} MB de VRAM nu s-au putut lua ({:?})",
                p,
                set.name,
                lay.bytes / 1_000_000,
                e
            ));
            return Ok(AttnBig::Skipped);
        }
    };
    // The window: the arena's upload pages (8 MiB), and one VRAM page for the copy's cbuf (+0) and QMD (+0x800).
    let win = I11_BOARD_BYTES;
    let host = st.mem.alloc_sys_mapped(&arena_pages(0, win / PAGE)?)?;
    let cz = map_vram(st, PAGE)?;
    let mut maps = maps.to_vec();
    maps.push(big);
    maps.push(cz);
    let sys = [ctx.sys.clone(), host.clone()];
    // One copy of `len` bytes (a multiple of 16) from `src` to `dst`; `false` = the board did not finish it.
    let copy = |vbar: &mut V, clock: &mut K, journal: &mut Journal, st: &mut GpuState, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, src: u64, dst: u64, len: u64| -> GpuResult<bool> {
        {
            let w = st.mem.writer_mut();
            put_bytes(w, cz.pa, &transfer::copy_cbuf(src, dst, len)?)?;
            put_bytes(w, cz.pa + 0x800, &transfer::copy_qmd(ctx.copy_va, cz.va, len)?.to_bytes()?)?;
        }
        Ok(mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, cz.va + 0x800)?.is_ok())
    };
    // 1. The inputs (everything before P), up and back.
    let t0 = crate::time::since_boot_us();
    let mut inputs = alloc::vec![0u8; lay.p];
    set.write_inputs(|at, b| inputs[at..at + b.len()].copy_from_slice(b));
    let mut same = true;
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        let at = k as u64 * win;
        let len = (piece.len() as u64).next_multiple_of(16);
        ram_write(&host, piece);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + at, len)? {
            lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: {}: urcarea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, set.name, k));
            return Ok(AttnBig::Stopped);
        }
    }
    for (k, piece) in inputs.chunks(win as usize).enumerate() {
        let at = k as u64 * win;
        let len = (piece.len() as u64).next_multiple_of(16);
        ram_write(&host, &[]);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + at, host.va, len)? {
            lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: {}: recitirea bucatii {} nu s-a terminat; proba oprita, fara reincercare", p, set.name, k));
            return Ok(AttnBig::Stopped);
        }
        same &= ram_read(&host, piece.len() as u64) == piece;
    }
    let up_ms = t0.zip(crate::time::since_boot_us()).map_or(0, |(a, b)| b.saturating_sub(a) / 1000);
    let fnv = ap::fnv(&inputs);
    drop(inputs);
    journal.push(Event::Value { what: "kt atentie mare intrari recitite identic", value: u32::from(same) });
    if !same {
        lines.push(format!(
            "{}: kronos --gpu: proba atentiei pe placi: {}: intrarile recitite din VRAM NU sunt cele urcate ({} octeti); nimic lansat",
            p, set.name, lay.p
        ));
        return Ok(AttnBig::Skipped);
    }
    lines.push(format!(
        "{}: kronos --gpu: proba atentiei pe placi: {}: {} octeti de intrari urcati si recititi identic din VRAM (fnv {:#010x}, asteptat {:#010x}) in {} ms; bufferul {} octeti",
        p,
        set.name,
        lay.p,
        fnv,
        set.inputs_fnv(),
        up_ms,
        lay.bytes
    ));
    // 2. The read-back ranges filled with NaN, span by span.
    let ranges = set.read_ranges();
    // Spans of at most the window (8 MiB): few submissions (in SIMULARE every submission moves the whole buffer).
    let groups = spans(&ranges, win);
    for &(start, len, _, _) in &groups {
        let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), len as usize / 4).flatten().collect();
        ram_write(&host, &bytes);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + start, len)? {
            lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: {}: umplerea cu NaN nu s-a terminat; proba oprita, fara reincercare", p, set.name));
            return Ok(AttnBig::Stopped);
        }
    }
    // 3. The launches (board only): 5 × each program, in order, each its own submission and time.
    let mut times: [Vec<u64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    if permit.is_some() {
        for (k, prog) in ap::PROGS.iter().enumerate() {
            let (cb, q) = ap::launch_bytes(set, *prog, prog_va[k], zones[k].va, big.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, zones[k].pa, &cb)?;
                put_bytes(w, zones[k].pa + 0x800, &q)?;
            }
            for r in 0..set.repeats {
                match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, zones[k].va + 0x800)? {
                    Ok(ns) => times[k].push(ns),
                    Err(got) => {
                        lines.push(format!(
                            "{}: kronos --gpu: proba atentiei pe placi: placa nu a terminat la setul {} ({}), programul {}, lansarea {}: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                            p,
                            ap::TIMING_SET,
                            set.name,
                            prog.name(),
                            r,
                            got
                        ));
                        return Ok(AttnBig::Stopped);
                    }
                }
            }
        }
    }
    // 4. The ranges back, span by span, in order, glued.
    let mut out = Vec::with_capacity(set.read_len());
    for &(start, len, first, count) in &groups {
        ram_write(&host, &[]);
        if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + start, host.va, len)? {
            lines.push(format!("{}: kronos --gpu: proba atentiei pe placi: {}: citirea benzilor nu s-a terminat; proba oprita, fara reincercare", p, set.name));
            return Ok(AttnBig::Stopped);
        }
        for &(o, n) in &ranges[first..first + count] {
            out.extend_from_slice(&ram_read_at(&host, o as u64 - start, n as u64));
        }
    }
    if permit.is_none() {
        let nan = out.chunks(4).all(|w| w == ap::FILL.to_le_bytes());
        lines.push(format!(
            "{}: kronos --gpu: proba atentiei pe placi {} (SIMULARE): benzile si garzile umplute cu NaN si recitite ({} octeti in {} intervale, {} copii, {}); cele {} lansari sarite in simulare (prea mari pentru procesorul emulat)",
            p,
            set.name,
            out.len(),
            ranges.len(),
            groups.len(),
            if nan { "neatinse" } else { "ATINSE" },
            3 * set.repeats
        ));
        return Ok(AttnBig::Skipped);
    }
    let r = ap::judge_set(ap::TIMING_SET, &out, [&times[0], &times[1], &times[2]])?;
    // P6-149 (note C-157_P2m, step 3) / P6-160 (note C-157_P2n): the inputs stay in VRAM; for every new generation the
    // read-back ranges filled with NaN again, then 5 × each of its programs, the ranges back, judged beside the old
    // read-back.
    let mut pairs: Vec<Result<alloc::boxed::Box<ap::PairResult>, ()>> = Vec::with_capacity(newer.len());
    // P6-227: the times of the newest chain's dQ / dK / dV program (beside the tensor-core programs).
    let mut last_back_times: Vec<u64> = times[2].clone();
    // P6-234: and of its dS program (beside `attn_back_tile0_tc`).
    let mut last_back0_times: Vec<u64> = times[1].clone();
    'gens: for gb in newer {
        let tag = gb.which.tag();
        let stop2 = |lines: &mut Vec<String>, pairs: &mut Vec<Result<alloc::boxed::Box<ap::PairResult>, ()>>, what: &str| {
            lines.push(format!("{}: kronos --gpu: {}: {}: {}; proba oprita, fara reincercare", p, tag, set.name, what));
            pairs.push(Err(()));
        };
        for &(start, len, _, _) in &groups {
            let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), len as usize / 4).flatten().collect();
            ram_write(&host, &bytes);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + start, len)? {
                stop2(lines, &mut pairs, "umplerea din nou cu NaN nu s-a terminat");
                break 'gens;
            }
        }
        let mut times2: [Vec<u64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for k in 0..3 {
            let (cb, q) = gb.which.launch_bytes(set, k, gb.prog_va[k], gb.zones[k].va, big.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, gb.zones[k].pa, &cb)?;
                put_bytes(w, gb.zones[k].pa + 0x800, &q)?;
            }
            for rep in 0..set.repeats {
                match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, gb.zones[k].va + 0x800)? {
                    Ok(ns) => times2[k].push(ns),
                    Err(got) => {
                        let what = format!("placa nu a terminat programul nou {}, lansarea {}: ultima inregistrare = {:#x}", gb.which.name(k), rep, got);
                        stop2(lines, &mut pairs, &what);
                        break 'gens;
                    }
                }
            }
        }
        let mut out2 = Vec::with_capacity(set.read_len());
        for &(start, len, first, count) in &groups {
            ram_write(&host, &[]);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + start, host.va, len)? {
                stop2(lines, &mut pairs, "citirea benzilor lantului nou nu s-a terminat");
                break 'gens;
            }
            for &(o, n) in &ranges[first..first + count] {
                out2.extend_from_slice(&ram_read_at(&host, o as u64 - start, n as u64));
            }
        }
        let r2 = ap::judge_pair_set(ap::TIMING_SET, &out, &out2, [&times2[0], &times2[1], &times2[2]])?;
        pairs.push(Ok(alloc::boxed::Box::new(r2)));
        last_back_times = times2[2].clone();
        last_back0_times = times2[1].clone();
    }
    // P6-227 (note C-157_P2zm, step 1): after every chain finished, the tensor-core programs on the P / dS the newest
    // chain left in VRAM: the dqkv bands (row T − 1 of the banded texts) and their guard filled with NaN before each
    // program, 5 launches each, the bands back. Judged here: tc16 = tc word for word, the distance from the F32 bands;
    // the margin is judged on the small sets only (the CPU model of this set would take too long).
    if !tcs.is_empty() && pairs.len() == newer.len() && pairs.iter().all(|x| x.is_ok()) {
        let (t, c) = (set.t as usize, set.c as usize);
        let mut dq: Vec<(usize, usize)> = Vec::new();
        for b in set.texts() {
            for tp in set.dq_rows() {
                dq.push((lay.dqkv + 4 * (b * t + tp) * 3 * c, 4 * 3 * c));
            }
        }
        dq.push((lay.dqkv_guard, 4 * ap::GUARD));
        let mut tb = TcBig { f32_ns: median_ns(&last_back_times), ..TcBig::default() };
        // The F32 bands (the newest chain's dqkv).
        let mut f32_band: Vec<u8> = Vec::new();
        for &(o, n) in &dq {
            ram_write(&host, &[]);
            if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, (n as u64).next_multiple_of(16))? {
                tb.stopped = Some(String::from("citirea benzilor dqkv F32 nu s-a terminat"));
                *tc_big = Some(tb);
                return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
            }
            f32_band.extend_from_slice(&ram_read(&host, n as u64));
        }
        let mut bands: Vec<Vec<u8>> = Vec::with_capacity(tcs.len());
        let mut v4_bands: Vec<Option<(u64, Vec<u8>)>> = Vec::new();
        for (k, tcb) in tcs.iter().enumerate() {
            // P6-233: v4 / v4pf only where aligned (never launched otherwise).
            if k >= 2 && !tcb.which.fits(set, big.va) {
                v4_bands.push(None);
                continue;
            }
            for &(o, n) in &dq {
                let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), n / 4).flatten().collect();
                ram_write(&host, &bytes);
                if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
                    tb.stopped = Some(format!("umplerea benzilor dqkv cu NaN inainte de {} nu s-a terminat", tcb.which.name()));
                    *tc_big = Some(tb);
                    return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                }
            }
            let (cb, q) = tcb.which.launch_bytes(set, tcb.prog_va, tcb.zone.va, big.va, None)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, tcb.zone.pa, &cb)?;
                put_bytes(w, tcb.zone.pa + 0x800, &q)?;
            }
            let mut ts: Vec<u64> = Vec::new();
            for rep in 0..set.repeats {
                match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, tcb.zone.va + 0x800)? {
                    Ok(ns) => ts.push(ns),
                    Err(got) => {
                        tb.stopped = Some(format!("placa nu a terminat programul nou {}, lansarea {}: ultima inregistrare = {:#x}", tcb.which.name(), rep, got));
                        *tc_big = Some(tb);
                        return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                    }
                }
            }
            let med = median_ns(&ts);
            if k < 2 {
                tb.ns.push(med);
            }
            let mut band: Vec<u8> = Vec::new();
            for &(o, n) in &dq {
                ram_write(&host, &[]);
                if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, (n as u64).next_multiple_of(16))? {
                    tb.stopped = Some(format!("citirea benzilor dqkv dupa {} nu s-a terminat", tcb.which.name()));
                    *tc_big = Some(tb);
                    return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                }
                band.extend_from_slice(&ram_read(&host, n as u64));
            }
            if k < 2 {
                bands.push(band);
            } else {
                v4_bands.push(Some((med, band)));
            }
        }
        // P6-238 (C-157_P2zw point 3): every program that ran, 5 launches more at the 64 KiB step (the step of the
        // mixed #513 step), the bands filled with NaN before, read back and compared with today's bands of the same program.
        if super::gpu_tur::ATTN_TC_64K_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            let mut v4_it = v4_bands.iter();
            for (k, tcb) in tcs.iter().enumerate() {
                let today: Option<(u64, &Vec<u8>)> = if k < 2 {
                    bands.get(k).map(|b| (tb.ns.get(k).copied().unwrap_or(0), b))
                } else {
                    v4_it.next().and_then(|x| x.as_ref().map(|(ns, b)| (*ns, b)))
                };
                let Some((ns0, band0)) = today else {
                    tb.at64k.push(None);
                    continue;
                };
                for &(o, n) in &dq {
                    let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), n / 4).flatten().collect();
                    ram_write(&host, &bytes);
                    if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
                        tb.stopped = Some(format!("umplerea benzilor dqkv cu NaN inainte de {}@64k nu s-a terminat", tcb.which.name()));
                        *tc_big = Some(tb);
                        return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                    }
                }
                let (cb, q) = tcb.which.launch_bytes(set, tcb.prog_va, tcb.zone.va, big.va, Some(STEP_64K))?;
                {
                    let w = st.mem.writer_mut();
                    put_bytes(w, tcb.zone.pa, &cb)?;
                    put_bytes(w, tcb.zone.pa + 0x800, &q)?;
                }
                let mut ts: Vec<u64> = Vec::new();
                for rep in 0..set.repeats {
                    match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, tcb.zone.va + 0x800)? {
                        Ok(ns) => ts.push(ns),
                        Err(got) => {
                            tb.stopped = Some(format!("placa nu a terminat programul {} la treapta 64 KiB, lansarea {}: ultima inregistrare = {:#x}", tcb.which.name(), rep, got));
                            *tc_big = Some(tb);
                            return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                        }
                    }
                }
                let mut band: Vec<u8> = Vec::new();
                for &(o, n) in &dq {
                    ram_write(&host, &[]);
                    if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, (n as u64).next_multiple_of(16))? {
                        tb.stopped = Some(format!("citirea benzilor dqkv dupa {}@64k nu s-a terminat", tcb.which.name()));
                        *tc_big = Some(tb);
                        return Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs));
                    }
                    band.extend_from_slice(&ram_read(&host, n as u64));
                }
                let gg = 4 * ap::GUARD;
                let same = band[..band.len() - gg] == band0[..band0.len() - gg];
                let guard = band[band.len() - gg..].chunks(4).all(|w| w == ap::FILL.to_le_bytes());
                tb.at64k.push(Some((median_ns(&ts), ns0, same, guard)));
            }
        }
        let g = 4 * ap::GUARD;
        let vals = |b: &[u8]| -> Vec<f32> { f32_words(&b[..b.len() - g]) };
        let f32v = vals(&f32_band);
        for band in &bands {
            let v = vals(band);
            tb.guard_ok.push(band[band.len() - g..].chunks(4).all(|w| w == ap::FILL.to_le_bytes()));
            tb.finite.push(v.iter().all(|x| x.is_finite()));
            tb.rel_ppm.push(rel_ppm(&v, &f32v));
        }
        tb.same_words = bands.len() == 2 && bands[0][..bands[0].len() - g] == bands[1][..bands[1].len() - g];
        for x in &v4_bands {
            tb.v4.push(x.as_ref().map(|(ns, band)| {
                let same = bands.get(1).is_some_and(|b1| b1[..b1.len() - g] == band[..band.len() - g]);
                (*ns, same, band[band.len() - g..].chunks(4).all(|w| w == ap::FILL.to_le_bytes()))
            }));
        }
        tb.words = f32v.len();
        *tc_big = Some(tb);
    }
    // P6-234 (note C-157_P2zs, step 1): after the dQ / dK / dV programs, `attn_back_tile0_tc` on the same P / dO / V
    // in VRAM: the F32 dS rows T − 1 of the banded texts (every head, all under the diagonal) read first, then those rows
    // and the dS guard filled with NaN, 5 launches, the rows back: the distance from F32, finite, the guard.
    if let (Some(t0), Some(tb)) = (tc0, tc_big.as_mut()) {
        if pairs.len() == newer.len() && pairs.iter().all(|x| x.is_ok()) && tb.stopped.is_none() {
            let (t, h) = (set.t as usize, set.heads as usize);
            let mut rows: Vec<(usize, usize)> = Vec::new();
            for b in set.texts() {
                for hh in 0..h {
                    rows.push((lay.ds + 4 * ((b * h + hh) * t + (t - 1)) * t, 4 * t));
                }
            }
            rows.push((lay.ds_guard, 4 * ap::GUARD));
            let mut r0 = Tc0Big { f32_ns: median_ns(&last_back0_times), ..Tc0Big::default() };
            let read_rows = |vbar: &mut V, clock: &mut K, journal: &mut Journal, st: &mut GpuState, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32| -> GpuResult<Option<Vec<u8>>> {
                let mut band: Vec<u8> = Vec::new();
                for &(o, n) in &rows {
                    ram_write(&host, &[]);
                    if !copy(vbar, clock, journal, st, l, doorbell, subs, big.va + o as u64, host.va, (n as u64).next_multiple_of(16))? {
                        return Ok(None);
                    }
                    band.extend_from_slice(&ram_read(&host, n as u64));
                }
                Ok(Some(band))
            };
            'tc0: {
                let Some(f32_band) = read_rows(vbar, clock, journal, st, l, doorbell, subs)? else {
                    r0.stopped = Some(String::from("citirea randurilor dS F32 nu s-a terminat"));
                    break 'tc0;
                };
                for &(o, n) in &rows {
                    let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), n / 4).flatten().collect();
                    ram_write(&host, &bytes);
                    if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
                        r0.stopped = Some(String::from("umplerea randurilor dS cu NaN nu s-a terminat"));
                        break 'tc0;
                    }
                }
                let a = set.back_args(big.va, nvidia_ga104::attn_back::Mode::Rows);
                let (cb, q) = tc0_launch_bytes(&a, t0.prog_va, t0.zone.va)?;
                {
                    let w = st.mem.writer_mut();
                    put_bytes(w, t0.zone.pa, &cb)?;
                    put_bytes(w, t0.zone.pa + 0x800, &q)?;
                }
                let mut ts: Vec<u64> = Vec::new();
                for rep in 0..set.repeats {
                    match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, t0.zone.va + 0x800)? {
                        Ok(ns) => ts.push(ns),
                        Err(got) => {
                            r0.stopped = Some(format!("placa nu a terminat programul nou attn_back_tile0_tc, lansarea {}: ultima inregistrare = {:#x}", rep, got));
                            break 'tc0;
                        }
                    }
                }
                r0.ns = median_ns(&ts);
                let Some(band) = read_rows(vbar, clock, journal, st, l, doorbell, subs)? else {
                    r0.stopped = Some(String::from("citirea randurilor dS dupa attn_back_tile0_tc nu s-a terminat"));
                    break 'tc0;
                };
                let g = 4 * ap::GUARD;
                let v = f32_words(&band[..band.len() - g]);
                let f = f32_words(&f32_band[..f32_band.len() - g]);
                r0.guard_ok = band[band.len() - g..].chunks(4).all(|w| w == ap::FILL.to_le_bytes());
                r0.finite = v.iter().all(|x| x.is_finite());
                r0.rel_ppm = rel_ppm(&v, &f);
                r0.words = v.len();
                // P6-235 (note C-157_P2zt): then `attn_back_tile0_tc_v4` on the same inputs, only when aligned: the rows and
                // the guard filled with NaN again, 5 launches, the rows back, word for word against tc's.
                if let Some(v4_va) = t0.v4_va {
                    if !nvidia_ga104::attn_back_tile0_tc_v4::fits(&a) {
                        r0.v4 = Tc0V4::Unaligned;
                        break 'tc0;
                    }
                    for &(o, n) in &rows {
                        let bytes: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), n / 4).flatten().collect();
                        ram_write(&host, &bytes);
                        if !copy(vbar, clock, journal, st, l, doorbell, subs, host.va, big.va + o as u64, (n as u64).next_multiple_of(16))? {
                            r0.v4 = Tc0V4::Stopped(String::from("umplerea randurilor dS cu NaN (v4) nu s-a terminat"));
                            break 'tc0;
                        }
                    }
                    let (cb, q) = tc0_v4_launch_bytes(&a, v4_va, t0.zone.va)?;
                    {
                        let w = st.mem.writer_mut();
                        put_bytes(w, t0.zone.pa, &cb)?;
                        put_bytes(w, t0.zone.pa + 0x800, &q)?;
                    }
                    let mut ts4: Vec<u64> = Vec::new();
                    for rep in 0..set.repeats {
                        match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, &maps, &sys, t0.zone.va + 0x800)? {
                            Ok(ns) => ts4.push(ns),
                            Err(got) => {
                                r0.v4 = Tc0V4::Stopped(format!("placa nu a terminat programul nou attn_back_tile0_tc_v4, lansarea {}: ultima inregistrare = {:#x}", rep, got));
                                break 'tc0;
                            }
                        }
                    }
                    let Some(band4) = read_rows(vbar, clock, journal, st, l, doorbell, subs)? else {
                        r0.v4 = Tc0V4::Stopped(String::from("citirea randurilor dS dupa attn_back_tile0_tc_v4 nu s-a terminat"));
                        break 'tc0;
                    };
                    let body = band.len() - g;
                    r0.v4 = Tc0V4::Ran {
                        ns: median_ns(&ts4),
                        same: band4[..body].chunks(4).zip(band[..body].chunks(4)).filter(|(x, y)| x == y).count(),
                        words: body / 4,
                        guard_ok: band4[body..].chunks(4).all(|w| w == ap::FILL.to_le_bytes()),
                    };
                }
            }
            tb.t0 = Some(r0);
        }
    }
    Ok(AttnBig::Judged(alloc::boxed::Box::new(r), pairs))
}

/// P6-234 (cyber-gpu note C-157_P2zs): the cbuf (0x800 bytes, the arguments at 0x700) and the QMD of `attn_back_tile0_tc`
/// for `a` (mode 0 = rows, the same arguments and grid as `attn_back_tile0_3`).
fn tc0_launch_bytes(a: &nvidia_ga104::attn_back::Args, prog_va: u64, cbuf_va: u64) -> GpuResult<(Vec<u8>, Vec<u8>)> {
    use nvidia_ga104::attn_back_tile0_tc as a0;
    use nvidia_ga104::qmd::ComputeQmd;
    let cb = a0::cbuf(a).ok_or(bad("atentie-tc0: argumentele refuzate de modulul programului"))?;
    let q = ComputeQmd {
        program_va: prog_va,
        registers: a0::REGISTERS,
        cbuf0_va: cbuf_va,
        cbuf0_size: a0::CBUF_BYTES as u32,
        grid: a0::grid(a),
        block: a0::BLOCK,
        release: None,
    }
    .to_bytes_with_shared(&a0::SHARED)?;
    Ok((cb.to_vec(), q.to_vec()))
}

/// P6-235 (cyber-gpu note C-157_P2zt): the cbuf and the QMD of `attn_back_tile0_tc_v4` (the same cbuf as tc when `fits`;
/// the same registers, shared memory, grid and block — only the program differs).
fn tc0_v4_launch_bytes(a: &nvidia_ga104::attn_back::Args, prog_va: u64, cbuf_va: u64) -> GpuResult<(Vec<u8>, Vec<u8>)> {
    use nvidia_ga104::attn_back_tile0_tc_v4 as a4;
    use nvidia_ga104::qmd::ComputeQmd;
    let cb = a4::cbuf(a).ok_or(bad("atentie-tc0-v4: argumentele refuzate de modulul programului (nealiniate)"))?;
    let q = ComputeQmd {
        program_va: prog_va,
        registers: a4::REGISTERS,
        cbuf0_va: cbuf_va,
        cbuf0_size: a4::CBUF_BYTES as u32,
        grid: a4::grid(a),
        block: a4::BLOCK,
        release: None,
    }
    .to_bytes_with_shared(&a4::SHARED)?;
    Ok((cb.to_vec(), q.to_vec()))
}

/// P6-234 (cyber-gpu note C-157_P2zs, step 1): one small set with `attn_back_tile0_tc`, on the buffer the newest chain
/// left (P, dO, V and its F32 dS in place): the F32 dS read, the dS region and its guard filled with NaN, one launch, dS
/// back. `None`: the board did not finish (the line is written).
#[allow(clippy::too_many_arguments)]
fn tc0_small_set<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    maps: &[Mapping],
    sys: &[SysMapping],
    buf: &SysMapping,
    set: &nvidia_ga104::attn_tile_probe::ProbeSet,
    i: usize,
    t0: &Tc0OnBoard,
    f32_ns: u64,
) -> GpuResult<Option<Tc0Set>> {
    use nvidia_ga104::attn_tile_probe as ap;
    let lay = set.layout();
    let shape = set.shape();
    let (nq, no, np) = shape.sizes().ok_or(bad("atentie-tc0: forma setului"))?;
    let whole = ram_read(buf, lay.end as u64);
    let take = |o: usize, n: usize| f32_words(&whole[o..o + 4 * n]);
    let (qkv, dout, pm, f32ds) = (take(lay.qkv, nq), take(lay.dout, no), take(lay.p, np), take(lay.ds, np));
    let span = 4 * np + 4 * ap::GUARD;
    let fill: Vec<u8> = core::iter::repeat_n(ap::FILL.to_le_bytes(), span / 4).flatten().collect();
    ram_write_at(buf, lay.ds as u64, &fill);
    let a = set.back_args(buf.va, nvidia_ga104::attn_back::Mode::Rows);
    let (cb, q) = tc0_launch_bytes(&a, t0.prog_va, t0.zone.va)?;
    {
        let w = st.mem.writer_mut();
        put_bytes(w, t0.zone.pa, &cb)?;
        put_bytes(w, t0.zone.pa + 0x800, &q)?;
    }
    let ns = match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, maps, sys, t0.zone.va + 0x800)? {
        Ok(t) => t,
        Err(got) => {
            lines.push(format!(
                "{}: kronos --gpu: atentie-tc0: placa nu a terminat la setul {} ({}), programul nou attn_back_tile0_tc: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                p, i, set.name, got
            ));
            return Ok(None);
        }
    };
    let got = ram_read_at(buf, lay.ds as u64, span as u64);
    let v = f32_words(&got[..4 * np]);
    let model = gpu_compute::attn_tc::back0_model(&shape, &qkv, &pm, &dout, a.scale);
    let (b, h, t) = (shape.batch, shape.heads, shape.t);
    let mut r = Tc0Set {
        ns,
        f32_ns,
        exact: 0,
        lower: 0,
        nan_lower: 0,
        upper_nonzero: 0,
        upper: 0,
        guard_ok: got[4 * np..].chunks(4).all(|w| w == ap::FILL.to_le_bytes()),
        rel_ppm: None,
        v4: Tc0V4::Off,
    };
    let (mut lo_v, mut lo_f) = (Vec::new(), Vec::new());
    for bh in 0..b * h {
        for row in 0..t {
            for j in 0..t {
                let at = (bh * t + row) * t + j;
                if j <= row {
                    r.lower += 1;
                    r.nan_lower += usize::from(v[at].is_nan());
                    r.exact += usize::from(model.as_ref().is_some_and(|m| m[at].to_bits() == v[at].to_bits()));
                    lo_v.push(v[at]);
                    lo_f.push(f32ds[at]);
                } else {
                    r.upper += 1;
                    r.upper_nonzero += usize::from(v[at] != 0.0);
                }
            }
        }
    }
    r.rel_ppm = rel_ppm(&lo_v, &lo_f);
    journal.push(Event::Value { what: "kt atentie-tc0 set: timp tc0 (ns)", value: u32::try_from(ns).unwrap_or(u32::MAX) });
    lines.push(format!(
        "{}: kronos --gpu: kt atentie-tc0 set {} ({}): attn_back_tile0_tc {} ns (dS F32 {} ns); abaterea fata de F32 {}; NaN sub diagonala {}; deasupra diagonalei {}; garda dS {}; exacte fata de attn_tc::back0_model {} / {}{}",
        p,
        i,
        set.name,
        ns,
        f32_ns,
        ppm_text(r.rel_ppm),
        r.nan_lower,
        if r.upper_nonzero == 0 { String::from("zerouri") } else { format!("{} / {} NU sunt zero", r.upper_nonzero, r.upper) },
        if r.guard_ok { "neatinsa" } else { "ATINSA" },
        r.exact,
        r.lower,
        if model.is_none() { " (modelul a refuzat intrarile)" } else { "" }
    ));
    // P6-235 (note C-157_P2zt): `attn_back_tile0_tc_v4` on the same inputs, only when aligned, dS (and its guard) filled
    // with NaN again; its words against tc's, bit for bit.
    if let Some(v4_va) = t0.v4_va {
        if !nvidia_ga104::attn_back_tile0_tc_v4::fits(&a) {
            r.v4 = Tc0V4::Unaligned;
            lines.push(format!("{}: kronos --gpu: kt atentie0tc-v4 set {} ({}): sarit (nealiniat)", p, i, set.name));
        } else {
            ram_write_at(buf, lay.ds as u64, &fill);
            let (cb, q) = tc0_v4_launch_bytes(&a, v4_va, t0.zone.va)?;
            {
                let w = st.mem.writer_mut();
                put_bytes(w, t0.zone.pa, &cb)?;
                put_bytes(w, t0.zone.pa + 0x800, &q)?;
            }
            let ns4 = match mm_tc_launch(vbar, clock, journal, st, l, permit, doorbell, subs, ctx, maps, sys, t0.zone.va + 0x800)? {
                Ok(t) => t,
                Err(got) => {
                    lines.push(format!(
                        "{}: kronos --gpu: atentie-tc0-v4: placa nu a terminat la setul {} ({}), programul nou attn_back_tile0_tc_v4: ultima inregistrare = {:#x}; proba oprita, fara reincercare",
                        p, i, set.name, got
                    ));
                    return Ok(None);
                }
            };
            let got4 = ram_read_at(buf, lay.ds as u64, span as u64);
            let same = got4[..4 * np].chunks(4).zip(got[..4 * np].chunks(4)).filter(|(x, y)| x == y).count();
            let guard4 = got4[4 * np..].chunks(4).all(|w| w == ap::FILL.to_le_bytes());
            r.v4 = Tc0V4::Ran { ns: ns4, same, words: np, guard_ok: guard4 };
            journal.push(Event::Value { what: "kt atentie0tc-v4 set: timp v4 (ns)", value: u32::try_from(ns4).unwrap_or(u32::MAX) });
            lines.push(format!(
                "{}: kronos --gpu: kt atentie0tc-v4 set {} ({}): attn_back_tile0_tc_v4 {} ns (tc {} ns); = tc: {} / {} cuvinte{}; garda dS {}",
                p,
                i,
                set.name,
                ns4,
                ns,
                same,
                np,
                if same == np { " (ACEIASI BITI)" } else { " (DIFERA)" },
                if guard4 { "neatinsa" } else { "ATINSA" }
            ));
        }
    }
    Ok(Some(r))
}

/// P6-235 (cyber-gpu note C-157_P2zt): the verdict of `attn_back_tile0_tc_v4` beside `attn_back_tile0_tc` (the aligned
/// small sets + `atentie-513-b4`): passes = the same words as tc on every aligned set and on the big rows, guards untouched.
fn tc0_v4_verdict(p: &str, label: &str, journal: &mut Journal, lines: &mut Vec<String>, sets: &[Tc0Set], big: Option<&Tc0Big>, stopped: bool) {
    use nvidia_ga104::attn_tile_probe as ap;
    let ran = sets.iter().filter(|s| s.v4.ok().is_some()).count();
    let good = sets.iter().filter(|s| s.v4.ok() == Some(true)).count();
    let skipped = sets.iter().filter(|s| matches!(s.v4, Tc0V4::Unaligned)).count();
    let (same, words) = sets.iter().fold((0usize, 0usize), |a, s| match s.v4 {
        Tc0V4::Ran { same, words, .. } => (a.0 + same, a.1 + words),
        _ => a,
    });
    let mut ok = !stopped && sets.len() == ap::TIMING_SET && ran > 0 && good == ran;
    journal.push(Event::Value { what: "kt atentie0tc-v4 = tc (cuvinte)", value: u32::try_from(same).unwrap_or(u32::MAX) });
    journal.push(Event::Value { what: "kt atentie0tc-v4 sarit (nealiniat)", value: u32::try_from(skipped).unwrap_or(u32::MAX) });
    let big_text = match big.map(|b| (b, &b.v4)) {
        Some((b, Tc0V4::Ran { ns, same, words, guard_ok })) if b.stopped.is_none() => {
            ok &= same == words && *guard_ok;
            journal.push(Event::Value { what: "kt atentie0tc-v4 timp (ns)", value: u32::try_from(*ns).unwrap_or(u32::MAX) });
            let ms = |ns: u64| format!("{},{:02} ms", ns / 1_000_000, (ns / 10_000) % 100);
            let ratio = |a: u64, b: u64| if b == 0 { String::from("-") } else { format!("x{},{:02}", a / b, (a % b) * 100 / b) };
            format!(
                "atentie-513-b4 pe lansare (mediana a {}): attn_back_tile0_tc_v4 {} fata de attn_back_tile0_tc {} ({} mai rapid; Cyber-GPU: x1,2-1,8, sub x1,15 nu se leaga) si dS F32 {} ({}); randurile T-1 = tc: {} / {} cuvinte{}",
                ap::REPEATS,
                ms(*ns),
                ms(b.ns),
                ratio(b.ns, *ns),
                ms(b.f32_ns),
                ratio(b.f32_ns, *ns),
                same,
                words,
                if *guard_ok { "" } else { ", garda ATINSA" }
            )
        }
        Some((_, Tc0V4::Unaligned)) => String::from("atentie-513-b4 nealiniat (v4 sarit)"),
        Some((_, Tc0V4::Stopped(why))) => {
            ok = false;
            format!("atentie-513-b4 oprit: {}", why)
        }
        Some((b, _)) if b.stopped.is_some() => {
            ok = false;
            format!("atentie-513-b4 oprit (tc): {}", b.stopped.as_deref().unwrap_or("-"))
        }
        _ => String::from("atentie-513-b4 omis"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc0-v4 (vechi langa nou, {}): {}: v4 = tc pe {} / {} seturi aliniate ({} sarite), {} / {} cuvinte; seturile mici in total: v4 {} ns, tc {} ns; {}",
        p,
        label,
        if ok { "TRECE" } else { "NU TRECE" },
        good,
        ran,
        skipped,
        same,
        words,
        sets.iter().map(|s| if let Tc0V4::Ran { ns, .. } = s.v4 { ns } else { 0 }).sum::<u64>(),
        sets.iter().filter(|s| matches!(s.v4, Tc0V4::Ran { .. })).map(|s| s.ns).sum::<u64>(),
        big_text
    ));
}

/// P6-234 (cyber-gpu note C-157_P2zs): the verdict of `attn_back_tile0_tc` beside the F32 dS (small sets + `atentie-513-b4`).
fn tc0_verdict(p: &str, label: &str, journal: &mut Journal, lines: &mut Vec<String>, sets: &[Tc0Set], big: Option<&Tc0Big>, stopped: bool) {
    use nvidia_ga104::attn_tile_probe as ap;
    let n = sets.len();
    let ok_sets = sets.iter().filter(|s| s.passes()).count();
    let max_rel = sets.iter().map(|s| s.rel_ppm).try_fold(0u32, |m, x| x.map(|x| m.max(x)));
    let nan: usize = sets.iter().map(|s| s.nan_lower).sum();
    let (exact, lower): (usize, usize) = sets.iter().fold((0, 0), |a, s| (a.0 + s.exact, a.1 + s.lower));
    let upper_zero = sets.iter().all(|s| s.upper_nonzero == 0);
    let guard = sets.iter().all(|s| s.guard_ok);
    let mut ok = !stopped && n == ap::TIMING_SET && ok_sets == n;
    journal.push(Event::Value { what: "kt atentie-tc0 exacte (model)", value: u32::try_from(exact).unwrap_or(u32::MAX) });
    journal.push(Event::Value { what: "kt atentie-tc0 abatere (x1e6)", value: max_rel.unwrap_or(u32::MAX) });
    journal.push(Event::Value { what: "kt atentie-tc0 garda (NaN sub diagonala)", value: u32::try_from(nan).unwrap_or(u32::MAX) });
    let big_text = match big {
        Some(b) if b.stopped.is_none() => {
            ok &= b.finite && b.guard_ok && b.rel_ppm.is_some_and(|r| r <= 1000);
            journal.push(Event::Value { what: "kt atentie-tc0 timp (ns)", value: u32::try_from(b.ns).unwrap_or(u32::MAX) });
            journal.push(Event::Value { what: "kt atentie-tc0 timp dS F32 (ns)", value: u32::try_from(b.f32_ns).unwrap_or(u32::MAX) });
            let ms = |ns: u64| format!("{},{:02} ms", ns / 1_000_000, (ns / 10_000) % 100);
            let ratio = if b.ns == 0 { String::from("-") } else { format!("x{},{:02}", b.f32_ns / b.ns, (b.f32_ns % b.ns) * 100 / b.ns) };
            format!(
                "atentie-513-b4 pe lansare (mediana a {}): attn_back_tile0_tc {} fata de dS F32 {} ({} mai rapid; x12 pe pas: {} fata de {}); randurile T-1 ({} cuvinte) fata de F32 {}{}{}",
                ap::REPEATS,
                ms(b.ns),
                ms(b.f32_ns),
                ratio,
                ms(12 * b.ns),
                ms(12 * b.f32_ns),
                b.words,
                ppm_text(b.rel_ppm),
                if b.finite { "" } else { ", valori NEFINITE" },
                if b.guard_ok { "" } else { ", garda ATINSA" }
            )
        }
        Some(b) => {
            ok = false;
            format!("atentie-513-b4 oprit: {}", b.stopped.as_deref().unwrap_or("-"))
        }
        None => String::from("atentie-513-b4 omis"),
    };
    lines.push(format!(
        "{}: kronos --gpu: atentie-tc0 (vechi langa nou, {}): {}: {} / {} seturi bune (abaterea fata de dS F32 cel mult {}, sub 1e-3; NaN sub diagonala {}; garda {}; deasupra diagonalei {}); exacte fata de attn_tc::back0_model {} / {} cuvinte; seturile mici in total: attn_back_tile0_tc {} ns, dS F32 {} ns; {}",
        p,
        label,
        if ok { "TRECE" } else { "NU TRECE" },
        ok_sets,
        n,
        ppm_text(max_rel),
        nan,
        if guard { "neatinsa" } else { "ATINSA" },
        if upper_zero { "zerouri" } else { "NU doar zerouri" },
        exact,
        lower,
        sets.iter().map(|s| s.ns).sum::<u64>(),
        sets.iter().map(|s| s.f32_ns).sum::<u64>(),
        big_text
    ));
}

/// P6-137: FNV-1a (32 bit) of bytes (the same as `mm_tc_probe::fnv`).
fn fnv1a_bytes(b: &[u8]) -> u32 {
    nvidia_ga104::mm_tc_probe::fnv(b)
}

/// P6-136 (C-161 point 4, C-160 point 6 second part; cyber-gpu note `C-160_P6b_pasul-de-aur-pe-placa.md`, commit
/// `dd67483`): the small golden step, whole, on the board — Kronos's model of the golden step (2 layers, C 64, batch
/// 2 × 32), 10 steps of `sgd` (73 launches per step) and of `adamw_513` (75), each step ONE submission of the library's
/// list (`train_step::Plan::step` → `pack` → `encode_sequence_fresh`), every tensor in the arena. The CPU writes the
/// uploads once per recipe, then only reads back the losses (and the norm at `adamw_513`) after every step, the gradient
/// and the weights after step 1, the weights after step 10. The arena, the control zone: host RAM (the arena's upload
/// pages, no longer used once the texts are done). Verdict: `train_step::judge` against PyTorch's files (the hub's
/// tolerances). New for the board: 11 programs that never ran (their SHA-256 checked first). A step the board does not
/// finish, or a non-finite norm, stops the probe (no retry).
#[allow(clippy::too_many_arguments)]
fn golden_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    use gpu_compute::train::{self, AdamW, Recipe};
    use nvidia_ga104::train_step::{self as ts, Outcome, Plan};
    const STEPS: u32 = 10;
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    // The data first: without it nothing is sent.
    let data = match crate::shell::antrenare::golden_data() {
        Ok(d) => d,
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: pasul de aur mic pe placa: NU A RULAT: {}; nimic trimis", p, e));
            return Err(bad("pas-aur: datele lipsesc"));
        }
    };
    if ts::check_programs().is_err() {
        lines.push(format!("{}: kronos --gpu: pasul de aur mic pe placa: SHA-256 al unui program nu e cel din nota: nimic trimis", p));
        return Err(bad("pas-aur: SHA-256 gresit"));
    }
    // Plan at a provisional base only to know the size; the real one at the arena's address.
    let probe_plan = Plan::new(train::MIC, 2, 32, &data.inputs, nvidia_ga104::mmu::VA_BASE)?;
    let arena_bytes = probe_plan.bytes().next_multiple_of(PAGE);
    let max_launches = probe_plan.launches_per_step(Recipe::AdamW513).max(probe_plan.launches_per_step(Recipe::Sgd));
    let zone_bytes = (max_launches as u64 * ts::SLOT).next_multiple_of(PAGE);
    if 16 * (max_launches as u64 + 1) > ctx.rec.size || max_launches as u32 + 1 > STRIDE {
        return Err(bad("pas-aur: prea putine inregistrari"));
    }
    if (arena_bytes + zone_bytes) / PAGE > I11_BOARD_BYTES / PAGE {
        return Err(bad("pas-aur: arena nu incape in paginile de urcare"));
    }
    lines.push(format!(
        "{}: kronos --gpu: pasul de aur mic pe placa ({}): datele din {}; {} programe (SHA-256 verificate), arena {} KiB + zona de control {} KiB in RAM; {} pasi x sgd ({} lansari) si adamw_513 ({} lansari), cate o trimitere pe pas",
        p,
        label,
        data.dir,
        ts::PROGRAMS.len(),
        arena_bytes >> 10,
        zone_bytes >> 10,
        STEPS,
        probe_plan.launches_per_step(Recipe::Sgd),
        probe_plan.launches_per_step(Recipe::AdamW513)
    ));
    // The plan's programs in VRAM (each mapping page-aligned, so 256-byte aligned). P6-152: `Plan::programs()` gives
    // exactly `ts::PROGRAMS` (13) for `Plan::new`; `Plan::new_p_only` would add `attn_p`.
    let plan_programs = probe_plan.programs();
    let mut progs: Vec<(ts::Prog, Mapping)> = Vec::with_capacity(plan_programs.len());
    for &prog in plan_programs.iter() {
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    let arena = st.mem.alloc_sys_mapped(&arena_pages(0, arena_bytes / PAGE)?)?;
    let zone = st.mem.alloc_sys_mapped(&arena_pages(arena_bytes / PAGE, zone_bytes / PAGE)?)?;
    if !arena.va.is_multiple_of(PAGE) || !zone.va.is_multiple_of(PAGE) {
        return Err(bad("pas-aur: arena nu e aliniata la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    let sys = [ctx.sys.clone(), arena.clone(), zone.clone()];
    let prog_va = |want: ts::Prog| progs.iter().find(|(q, _)| *q == want).map_or(0, |(_, pm)| pm.va);
    let read_f32s = |va: u64, n: usize| ts::f32s(&ram_read_at(&arena, va - arena.va, 4 * n as u64));
    let mut stopped: Option<String> = None;
    let mut all_pass = true;
    for (recipe, name, refs) in [(Recipe::Sgd, "sgd", &data.sgd), (Recipe::AdamW513, "adamw_513", &data.adamw)] {
        let plan = Plan::new(train::MIC, 2, 32, &data.inputs, arena.va)?;
        // The uploads, once, from zero: one image of the whole arena (zeros between the pieces).
        let mut image = alloc::vec![0u8; arena_bytes as usize];
        for (va, bytes) in plan.uploads(&data.initial, &data.inputs, &data.targets)? {
            let at = (va - arena.va) as usize;
            image[at..at + bytes.len()].copy_from_slice(&bytes);
        }
        ram_write(&arena, &image);
        drop(image);
        let opt = AdamW::recipe_513(&data.initial);
        let n = plan.launches_per_step(recipe);
        let mut out = Outcome::default();
        for step in 1..=STEPS {
            let list = plan.step(recipe, &opt, step)?;
            let (zb, qmds) = ts::pack(&list, prog_va, zone.va)?;
            ram_write(&zone, &zb);
            put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (list.len() + 1)])?;
            let first = TOKEN_FIRST + STRIDE * *subs;
            let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(list.len())];
            let mut pb = PushBuf::new(&mut words);
            transfer::encode_sequence_fresh(&mut pb, &qmds, ctx.rec.va, first)?;
            let nw = pb.len();
            let last = first + list.len() as u32;
            let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, list.len() as u64 + 1, last, &maps, &sys, doorbell)? {
                Ok(b) => b,
                Err(got) => {
                    stopped = Some(format!("{}: placa nu a terminat la pasul {} (ultima inregistrare {:#x}, asteptat {:#x})", name, step, got, last));
                    break;
                }
            };
            *subs += 1;
            out.step_ns.push(transfer::analyze_sequence(&b, list.len(), first)?.iter().sum());
            out.losses.push(Outcome::mean_loss(&read_f32s(plan.loss_rows, 2 * 32)));
            if recipe == Recipe::AdamW513 {
                let norm = read_f32s(plan.res, 1)[0];
                if !norm.is_finite() {
                    stopped = Some(format!("{}: norma gradientului nu e finita la pasul {} ({:?})", name, step, norm));
                    out.norms.push(norm);
                    break;
                }
                out.norms.push(norm);
            }
            if step == 1 {
                out.grad1 = plan.unflat(&read_f32s(plan.g, plan.n_params))?;
                out.after1 = plan.unflat(&read_f32s(plan.w, plan.n_params))?;
            }
        }
        if stopped.is_some() {
            lines.push(format!("{}: kronos --gpu: pasul de aur mic pe placa: {}; proba oprita, fara reincercare", p, stopped.as_deref().unwrap_or("")));
            all_pass = false;
            break;
        }
        out.after_last = plan.unflat(&read_f32s(plan.w, plan.n_params))?;
        let v = ts::judge(recipe, n, &out, refs);
        v.journal(journal);
        lines.extend(v.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
        all_pass &= v.passes();
    }
    lines.push(format!(
        "{}: kronos --gpu: pasul de aur mic pe placa: {}",
        p,
        match &stopped {
            Some(_) => "NU A TERMINAT (vezi randul de mai sus)",
            None if all_pass => "TRECE (sgd si adamw_513: aceleasi numere ca PyTorch)",
            None => "NU TRECE (vezi randurile pas de aur mic)",
        }
    ));
    match stopped {
        Some(_) => Err(GpuError::Timeout { waited_us: WAIT_US }),
        None => Ok(()),
    }
}

/// P6-164 (C-161, command-center note of 23 Nov 18:16; Adrian's order of 23 Nov 20:11 / 20:36: the whole training step
/// on the real board first): the step of `antrenare mini` (P6-157 … P6-162) — the small model (2 layers, E 64, 4 heads)
/// on Kronos's vocabulary (16 384 symbols, 1 150 720 weights), batch 2 × 32, AdamW lr 1e-3 (betas 0.9 / 0.999, eps 1e-8,
/// decay 0.1), clipping at 1.0 — done by the board's own list (`train_step::Plan::new`, the 13 programs of the small
/// golden step that passed on the board in os-351), 10 steps (1 in SIMULARE: ~145 s per step in QEMU), one submission
/// per step. Before every step the CPU writes the arena (the weights the board left, m, v, the step's batch and segment
/// lists); after it reads the per-row losses, the gradient norm, the weights and m, v. Cyber-GPU's CPU oracle
/// (`train::loss_and_grad` + clipping + AdamW) runs beside it on the same batches from the same start: every step says
/// how far the board is from the oracle (tolerance 1e-3, as `--placa-sim`). The arena (26.4 MiB: the logits and their
/// gradient alone are 8 MiB) does not fit the 8 MiB upload pages: it lives in a separate pinned host-RAM run below
/// 4 GiB (`ram_vram_host`, as the RAM ↔ VRAM probe), with the control zone after it. New for the board: only the large
/// shape of the step (the output layer with 16 384 symbols); no new program, no register. Nothing is written to any
/// disk (the data is only read). A step the board does not finish, or a non-finite norm, stops the probe (no retry).
#[allow(clippy::too_many_arguments)]
/// P6-215 (cyber-gpu note `C-157_P2zf_pas-mixt-rapid.md`): `mm_tc_pf` and `mm_tc_v4` in VRAM beside `mm_tc` for
/// `train_step::pack_mixed_fast` (SHA-256 checked first; each with the tail page, like every program).
fn map_fast_tc(st: &mut GpuState) -> GpuResult<(Mapping, Mapping)> {
    use nvidia_ga104::{mm_tc_pf as pf, mm_tc_v4 as v4};
    if sha256_hex(pf::MM_TC_PF) != pf::MM_TC_PF_SHA256 || sha256_hex(v4::MM_TC_V4) != v4::MM_TC_V4_SHA256 {
        return Err(bad("pasul mixt rapid: SHA-256 mm_tc_pf / mm_tc_v4 gresit"));
    }
    let pm = map_vram(st, program_map_bytes(pf::MM_TC_PF.len() as u64))?;
    put_bytes(st.mem.writer_mut(), pm.pa, pf::MM_TC_PF)?;
    let vm = map_vram(st, program_map_bytes(v4::MM_TC_V4.len() as u64))?;
    put_bytes(st.mem.writer_mut(), vm.pa, v4::MM_TC_V4)?;
    Ok((pm, vm))
}

/// P6-215: how many launches of one step got each `mm_tc` program (`mm_tc_v4 135, mm_tc_pf 12`).
fn fast_picks_text(picks: &[Option<nvidia_ga104::train_step::TcPick>]) -> String {
    use nvidia_ga104::train_step::TcPick;
    let parts: Vec<String> = [TcPick::V8, TcPick::V8n, TcPick::V7, TcPick::V8Gelu, TcPick::V8nGelu, TcPick::V7Gelu, TcPick::V6, TcPick::V5, TcPick::V4, TcPick::Pf, TcPick::Tc, TcPick::Gelu, TcPick::AttnTc, TcPick::AttnTc16, TcPick::AttnTc16V4, TcPick::AttnTc16V4pf, TcPick::Attn0Tc, TcPick::Attn0TcV4, TcPick::AttnFwdTc]
        .iter()
        .map(|k| (k, picks.iter().filter(|x| **x == Some(*k)).count()))
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{} {}", k.name(), n))
        .collect();
    if parts.is_empty() { String::from("niciuna") } else { parts.join(", ") }
}

fn train_mini_probe<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    // P6-169: the resident arena (in VRAM) is its own probe.
    if super::gpu_tur::TRAIN_MINI_VRAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        // P6-178: the same run twice from the same state — the batch through the VRAM window, then through RAM.
        if super::gpu_tur::TRAIN_MINI_RAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            let window = train_mini_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, false, false)?;
            let ram = train_mini_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, false, true)?;
            return ram_verdict(p, journal, lines, &window, &ram);
        }
        if !super::gpu_tur::TRAIN_MINI_FUSED_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
            let mini = train_mini_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, false, false)?;
            // P6-172: #513's step only after the small model's resident run passed in the same power-on.
            if super::gpu_tur::TRAIN_513_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
                if !mini.pass {
                    lines.push(format!("{}: kronos --gpu: antrenare #513 pe placa: NU A RULAT: antrenarea mini cu arena in VRAM nu a trecut; nimic trimis", p));
                } else {
                    train_513_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx)?;
                }
            }
            return Ok(());
        }
        // P6-175: the same run twice from the same state — without gluing, then glued — compared bit for bit.
        let plain = train_mini_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, false, false)?;
        let glued = train_mini_resident(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, ctx, true, false)?;
        return fused_verdict(p, journal, lines, &plain, &glued);
    }
    use crate::shell::antrenare_mini as am;
    use gpu_compute::train::{self, AdamW, Batch, Recipe};
    use nvidia_ga104::train_step::{self as ts, Outcome, Plan};
    const TOL: f64 = 1e-3;
    // P6-167 (`--mixt`): every matmul on `mm_tc`; the verdict on the loss and the norm (the note C-160_P7f), the
    // weights only shown (f16 + AdamW move them ~1e-3).
    let mixed = super::gpu_tur::TRAIN_MINI_MIXED_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let (bsz, tl) = (2usize, 32usize);
    let steps: usize = if permit.is_some() { 10 } else { 1 };
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let tag = if mixed { "antrenare mini pe placa (mixt)" } else { "antrenare mini pe placa" };
    let data = match am::board_batches(bsz, tl, steps) {
        Ok(d) => d,
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: {}; nimic trimis", p, tag, e));
            journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
            return Err(bad("antrenare-mini: datele"));
        }
    };
    if ts::check_programs().is_err() {
        lines.push(format!("{}: kronos --gpu: {}: SHA-256 al unui program nu e cel din nota: nimic trimis", p, tag));
        return Err(bad("antrenare-mini: SHA-256 gresit"));
    }
    if mixed && sha256_hex(nvidia_ga104::mm_tc::MM_TC) != nvidia_ga104::mm_tc::MM_TC_SHA256 {
        lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc nu e cel din nota: nimic trimis", p, tag));
        return Err(bad("antrenare-mini: SHA-256 mm_tc gresit"));
    }
    let a = am::arch(am::VOCAB, tl);
    // The arena's size and the launches over every step's plan (the segment lists depend on the batch).
    let base0 = nvidia_ga104::mmu::VA_BASE;
    let (mut arena_bytes, mut n_launch, mut zone_need) = (0u64, 0usize, 0u64);
    let mut programs: Option<&'static [ts::Prog]> = None;
    let adam0 = AdamW::new(&am::initial(&a, 0), 1e-3, 0.9, 0.999, 1e-8, 0.1);
    for (x, _) in &data.batches {
        let pl = Plan::new(a, bsz, tl, x, base0)?;
        arena_bytes = arena_bytes.max(pl.bytes());
        if mixed {
            // The mixed list's length and zone (its `mm_tc` slots are larger) do not depend on the AdamW state.
            let list = pl.step_mixed(Recipe::AdamW513, &adam0, 1, ts::LOSS_SCALE)?;
            n_launch = n_launch.max(list.len());
            zone_need = zone_need.max(list.iter().map(ts::MixedLaunch::slot).sum());
        } else {
            n_launch = n_launch.max(pl.launches_per_step(Recipe::AdamW513));
        }
        if programs.is_some_and(|q| q != pl.programs()) {
            return Err(bad("antrenare-mini: planurile pasilor au programe diferite"));
        }
        programs = Some(pl.programs());
    }
    let programs = programs.ok_or(bad("antrenare-mini: niciun pas"))?;
    let arena_bytes = arena_bytes.next_multiple_of(PAGE);
    drop(adam0);
    let zone_bytes = (n_launch as u64 * ts::SLOT).max(zone_need).next_multiple_of(PAGE);
    if 16 * (n_launch as u64 + 1) > ctx.rec.size || n_launch as u32 + 1 > STRIDE {
        return Err(bad("antrenare-mini: prea putine inregistrari"));
    }
    let Some(host) = ram_vram_host((arena_bytes + zone_bytes) / PAGE) else {
        lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: nu am {} KiB RAM contigua sub 4 GiB; nimic trimis", p, tag, (arena_bytes + zone_bytes) >> 10));
        journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
        return Err(GpuError::OutOfMemory { requested: arena_bytes + zone_bytes });
    };
    let count: usize = a.tensors().iter().map(|t| t.elements()).sum();
    lines.push(format!(
        "{}: kronos --gpu: {} ({}): modelul vocabular {}, context {}, E {}, {} capete, {} straturi ({} greutati), lot {} x {}, AdamW lr 1e-3, taiere 1,0; {}; {} programe{} (SHA-256 verificate), {} lansari pe pas, cate o trimitere; arena {} KiB + zona de control {} KiB in RAM fixata; {} pasi, oracolul pe procesor la fiecare pas",
        p,
        tag,
        label,
        a.vocab,
        a.ctx,
        a.n_emb,
        a.heads,
        a.layers,
        count,
        bsz,
        tl,
        data.source,
        programs.len(),
        if mixed { " + mm_tc (toate inmultirile: intrari f16, sume f32, scara erorii S = 1024)" } else { "" },
        n_launch,
        arena_bytes >> 10,
        zone_bytes >> 10,
        steps
    ));
    let mut progs: Vec<(ts::Prog, Mapping)> = Vec::with_capacity(programs.len());
    for &prog in programs.iter() {
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    // P6-167: `mm_tc` in VRAM beside the step's programs (its own mapping, 256-aligned like every program's).
    let tc_map = if mixed {
        let pm = map_vram(st, program_map_bytes(nvidia_ga104::mm_tc::MM_TC.len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, nvidia_ga104::mm_tc::MM_TC)?;
        Some(pm)
    } else {
        None
    };
    // P6-215 (`--mixt-rapid`): the fast `mm_tc` programs beside `mm_tc` (pack_mixed_fast picks per product).
    let fast_map = if mixed && super::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        Some(map_fast_tc(st)?)
    } else {
        None
    };
    let mut fast_picks = String::new();
    let pages = |first: u64, n: u64| (first..first + n).map(|i| host + i * PAGE).collect::<Vec<u64>>();
    let arena = st.mem.alloc_sys_mapped(&pages(0, arena_bytes / PAGE))?;
    let zone = st.mem.alloc_sys_mapped(&pages(arena_bytes / PAGE, zone_bytes / PAGE))?;
    if !arena.va.is_multiple_of(PAGE) || !zone.va.is_multiple_of(PAGE) {
        return Err(bad("antrenare-mini: arena nu e aliniata la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    maps.extend(tc_map.iter().copied());
    maps.extend(fast_map.iter().flat_map(|(a, b)| [*a, *b]));
    let sys = [ctx.sys.clone(), arena.clone(), zone.clone()];
    let prog_va = |want: ts::Prog| progs.iter().find(|(q, _)| *q == want).map_or(0, |(_, pm)| pm.va);
    let arena_va = arena.va;
    // The arena through the direct map (contiguous run): written before the doorbell, read after the last record.
    let put = |va: u64, bytes: &[u8]| {
        let at = va - arena_va;
        ram_vram_bytes(host + at, bytes.len() as u64).copy_from_slice(bytes);
    };
    let put_f32s = |va: u64, set: &[Vec<f32>]| {
        let bytes: Vec<u8> = set.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
        put(va, &bytes);
    };
    let read_f32s = |va: u64, n: usize| ts::f32s(ram_vram_bytes(host + (va - arena_va), 4 * n as u64));
    ram_vram_bytes(host, arena_bytes).fill(0);
    let mut params = am::initial(&a, 0);
    let mut adam = AdamW::new(&params, 1e-3, 0.9, 0.999, 1e-8, 0.1);
    let (mut op, mut oa) = (params.clone(), adam.clone());
    let (mut worst_loss, mut worst_w, mut worst_norm, mut tc_per_step) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    let (mut board_ns, mut host_us, mut losses): (Vec<u64>, Vec<u64>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new());
    let mut stopped: Option<String> = None;
    for (k, (x, y)) in data.batches.iter().enumerate() {
        let step = k as u32 + 1;
        let t0 = crate::time::since_boot_us();
        let plan = Plan::new(a, bsz, tl, x, arena_va)?;
        for (va, bytes) in plan.uploads(&params, x, y)? {
            put(va, &bytes);
        }
        // `uploads` starts m and v at zero (the golden step's first step); here they carry the run's AdamW state.
        put_f32s(plan.m, &adam.m);
        put_f32s(plan.v, &adam.v);
        let t = adam.t + 1;
        let (zb, qmds, n_list) = match tc_map {
            Some(tc) => {
                // The note C-160_P7f: 1/S at `res + 16` after every `uploads`, then the mixed list and its zone.
                for (va, bytes) in plan.mixed_uploads(ts::LOSS_SCALE)? {
                    put(va, &bytes);
                }
                let list = plan.step_mixed(Recipe::AdamW513, &adam, t, ts::LOSS_SCALE)?;
                tc_per_step = list.iter().filter(|x| matches!(x, ts::MixedLaunch::Tc { .. })).count();
                let (zb, qmds) = match fast_map {
                    Some((pf, v4)) => {
                        let progs = ts::TcPrograms { tc: tc.va, pf: pf.va, v4: v4.va, gelu: 0 };
                        let (zb, qmds, picks) = ts::pack_mixed_fast(&list, prog_va, &progs, None, zone.va)?;
                        fast_picks = fast_picks_text(&picks);
                        (zb, qmds)
                    }
                    None => ts::pack_mixed(&list, prog_va, tc.va, zone.va)?,
                };
                (zb, qmds, list.len())
            }
            None => {
                let list = plan.step(Recipe::AdamW513, &adam, t)?;
                let (zb, qmds) = ts::pack(&list, prog_va, zone.va)?;
                (zb, qmds, list.len())
            }
        };
        if zb.len() as u64 > zone_bytes {
            return Err(bad("antrenare-mini: zona de control prea mare"));
        }
        ram_write(&zone, &zb);
        put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (n_list + 1)])?;
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(n_list)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence_fresh(&mut pb, &qmds, ctx.rec.va, first)?;
        let nw = pb.len();
        let last = first + n_list as u32;
        let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, n_list as u64 + 1, last, &maps, &sys, doorbell)? {
            Ok(b) => b,
            Err(got) => {
                stopped = Some(format!("placa nu a terminat la pasul {} (ultima inregistrare {:#x}, asteptat {:#x})", step, got, last));
                break;
            }
        };
        *subs += 1;
        board_ns.push(transfer::analyze_sequence(&b, n_list, first)?.iter().sum());
        let loss = Outcome::mean_loss(&read_f32s(plan.loss_rows, bsz * tl));
        let norm = read_f32s(plan.res, 1)[0];
        if !norm.is_finite() || !loss.is_finite() {
            stopped = Some(format!("pasul {}: pierderea {:?} / norma gradientului {:?} nu e un numar{}", step, loss, norm, if mixed { " (scara S prea mare?)" } else { "" }));
            break;
        }
        // The mixed list's `res[0]` is S × the norm (the gradients carry the loss scale).
        let norm = if mixed { norm / ts::LOSS_SCALE } else { norm };
        params = plan.unflat(&read_f32s(plan.w, plan.n_params))?;
        adam.m = plan.unflat(&read_f32s(plan.m, plan.n_params))?;
        adam.v = plan.unflat(&read_f32s(plan.v, plan.n_params))?;
        adam.t = t;
        if let Some(us) = t0.zip(crate::time::since_boot_us()).map(|(p0, p1)| p1.saturating_sub(p0)) {
            host_us.push(us);
        }
        losses.push(loss);
        // The oracle on the same batch, from its own copy.
        let batch = Batch { inputs: x, targets: y, batch: bsz, len: tl };
        let (ol, mut og) = train::loss_and_grad(&a, &op, &batch)?;
        let onorm = train::clip_grad_norm(&mut og, 1.0);
        oa.step(&mut op, &og)?;
        let dn = ((norm as f64 - onorm as f64) / (onorm as f64).abs().max(1e-12)).abs();
        worst_norm = worst_norm.max(dn);
        let dl = ((loss as f64 - ol as f64) / (ol as f64).abs().max(1e-12)).abs();
        let dw = am::rel_dev(&params, &op);
        worst_loss = worst_loss.max(dl);
        worst_w = worst_w.max(dw);
        lines.push(format!(
            "{}: kronos --gpu: {}: pasul {}: pierderea {:.4} (oracolul {:.4}, abaterea {:.1e}), greutatile fata de oracol {:.1e}, norma gradientului {:.4} (oracolul {:.4}, abaterea {:.1e}); placa {} us, pasul intreg {} ms (urcari + placa + citiri, fara oracol)",
            p,
            tag,
            step,
            loss,
            ol,
            dl,
            dw,
            norm,
            onorm,
            dn,
            board_ns.last().copied().unwrap_or(0) / 1000,
            host_us.last().copied().unwrap_or(0) / 1000
        ));
    }
    let us = median_us(&board_ns);
    let host_ms = median_us(&host_us.iter().map(|u| u * 1000).collect::<Vec<u64>>()) / 1000;
    let tok_s = if us > 0 { (bsz * tl) as u64 * 1_000_000 / us } else { 0 };
    journal.push(Event::Value { what: KT_MINI_STEPS, value: losses.len() as u32 });
    journal.push(Event::Value { what: KT_MINI_US, value: us as u32 });
    journal.push(Event::Value { what: KT_MINI_HOST_MS, value: host_ms as u32 });
    journal.push(Event::Value { what: KT_MINI_TOK_S, value: tok_s as u32 });
    journal.push(Event::Value { what: KT_MINI_LOSS_E9, value: (worst_loss * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_W_E9, value: (worst_w * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_NORM_E9, value: (worst_norm * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_MIXED, value: u32::from(mixed) });
    journal.push(Event::Value { what: KT_MINI_VRAM, value: 0 });
    journal.push(Event::Value { what: KT_MINI_TC, value: tc_per_step as u32 });
    if let Some(why) = &stopped {
        lines.push(format!("{}: kronos --gpu: {}: {}; proba oprita, fara reincercare", p, tag, why));
        lines.push(format!("{}: kronos --gpu: {}: NU A TERMINAT (vezi randul de mai sus)", p, tag));
        journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
        return Err(GpuError::Timeout { waited_us: WAIT_US });
    }
    if let Some(l0) = losses.first() {
        lines.push(format!("{}: kronos --gpu: {}: pierderea la pornire {:.4} (ghicit la intamplare: ln {} = {:.4})", p, tag, l0, a.vocab, am::ln(a.vocab as f64)));
    }
    lines.push(format!(
        "{}: kronos --gpu: {}: timpul pe pas (median, {} pasi): placa {} us = {} simboluri/s la antrenare (lot {} x {}); pasul intreg {} ms (urcarile arenei, placa, citirile greutatilor si ale lui m, v; fara oracol)",
        p,
        tag,
        losses.len(),
        us,
        tok_s,
        bsz,
        tl,
        host_ms
    ));
    let pass = if mixed {
        worst_loss <= ts::MIX_TOL_LOSS_REL && worst_norm <= ts::MIX_TOL_NORM_REL
    } else {
        worst_loss <= TOL && worst_w <= TOL
    };
    journal.push(Event::Value { what: KT_MINI_PASS, value: u32::from(pass) });
    if mixed {
        lines.push(format!(
            "{}: kronos --gpu: {}: lista placii mixte ({}) fata de oracol: {} — cea mai mare abatere a pierderii {:.1e} (toleranta 1e-3), a normei {:.1e} (toleranta 2e-3); greutatile {:.1e} (doar afisat) in {} pasi; {} lansari mm_tc pe pas",
            p,
            tag,
            label,
            if pass { "ACELEASI NUMERE (mixt)" } else { "DIFERA" },
            worst_loss,
            worst_norm,
            worst_w,
            losses.len(),
            tc_per_step
        ));
        if fast_map.is_some() {
            lines.push(format!("{}: kronos --gpu: {}: alegerile mm_tc pe pas (--mixt-rapid, pack_mixed_fast): {}", p, tag, fast_picks));
        }
    } else {
        lines.push(format!(
            "{}: kronos --gpu: {}: lista placii ({}) fata de oracol: {} — cea mai mare abatere a pierderii {:.1e}, a greutatilor {:.1e} in {} pasi (toleranta 1e-3)",
            p,
            tag,
            label,
            if pass { "ACELEASI NUMERE" } else { "DIFERA" },
            worst_loss,
            worst_w,
            losses.len()
        ));
    }
    lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, if pass { "TRECE (pasul intreg de antrenare pe placa = oracolul)" } else { "NU TRECE (vezi randurile de mai sus)" }));
    Ok(())
}

/// P6-169 (center's note 23 Nov 23:36, point 2; cyber-gpu note C-160_P8, `f2e5097`): the step of `antrenare mini` as in
/// [`train_mini_probe`] (F32, the 13 programs of os-380), but with the **resident arena**: the plan's arena (26.4 MiB:
/// weights, gradient, m, v, activations) lives in VRAM for the whole run. Before step 1 the CPU fills it in the pinned
/// host run (`Plan::uploads`) and one launch of the copy program (on the board since I11, the upload of #513's
/// weights) carries it RAM → VRAM; between steps only `Plan::batch_uploads` (~1.3 KiB, written straight into VRAM) and,
/// after every step, only `Plan::step_reads` (the per-row losses + the norm, 260 B, read through the VRAM window of
/// os-345). At the end one copy VRAM → RAM brings the weights back for the oracle. The oracle (F32, CPU) runs beside
/// it: the loss and the norm at every step, the weights at the end. New for the board: the arena in VRAM and the two
/// large copies; no new program, no new register.
#[allow(clippy::too_many_arguments)]
fn train_mini_resident<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
    fused: bool,
    via_ram: bool,
) -> GpuResult<ResidentRun> {
    use crate::shell::antrenare_mini as am;
    use gpu_compute::train::{self, AdamW, Batch, Recipe};
    use nvidia_ga104::train_step::{self as ts, Outcome, Plan};
    const TOL: f64 = 1e-3;
    let (bsz, tl) = (2usize, 32usize);
    // SIMULARE: 2 steps, so the second batch goes through `batch_uploads` too.
    let steps: usize = if permit.is_some() { 10 } else { 2 };
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let tag = match (fused, via_ram) {
        (true, _) => "antrenare mini pe placa (arena in VRAM, lipit)",
        (false, true) => "antrenare mini pe placa (arena in VRAM, lot prin RAM)",
        (false, false) => "antrenare mini pe placa (arena in VRAM)",
    };
    if fused && via_ram {
        return Err(bad("antrenare-mini: lotul prin RAM nu merge cu pasul lipit"));
    }
    // P6-175: `ln_back_res` + `mm_train_gelu` glued into the step (the same plan, the same arena, 2·L + L launches fewer).
    let glue = |pl: Plan| if fused { pl.with_fused_back().with_fused_gelu() } else { pl };
    journal.push(Event::Value { what: KT_MINI_VRAM, value: 1 });
    journal.push(Event::Value { what: KT_MINI_FUSED, value: u32::from(fused) });
    journal.push(Event::Value { what: KT_MINI_RAM, value: u32::from(via_ram) });
    journal.push(Event::Value { what: KT_MINI_MIXED, value: 0 });
    let data = match am::board_batches(bsz, tl, steps) {
        Ok(d) => d,
        Err(e) => {
            lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: {}; nimic trimis", p, tag, e));
            journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
            return Err(bad("antrenare-mini: datele"));
        }
    };
    if ts::check_programs().is_err() {
        lines.push(format!("{}: kronos --gpu: {}: SHA-256 al unui program nu e cel din nota: nimic trimis", p, tag));
        return Err(bad("antrenare-mini: SHA-256 gresit"));
    }
    let a = am::arch(am::VOCAB, tl);
    let (x0, y0) = data.batches.first().ok_or(bad("antrenare-mini: niciun pas"))?;
    // The shape first (any base): the resident arena's size and every batch's launch count.
    let probe0 = glue(Plan::new(a, bsz, tl, x0, nvidia_ga104::mmu::VA_BASE)?).resident()?;
    let mut n_launch = 0usize;
    for (x, _) in &data.batches {
        let pl = probe0.rebatch(x)?;
        n_launch = n_launch.max(pl.launches_per_step(Recipe::AdamW513));
    }
    let arena_bytes = probe0.bytes().next_multiple_of(PAGE);
    // P6-178: through RAM the list carries the copies in front and behind (5 + 2 at `antrenare mini`).
    if via_ram {
        let v = nvidia_ga104::batch_ram::via_ram(&probe0, x0, y0, 0)?;
        n_launch += nvidia_ga104::batch_ram::extra_launches(&v);
        if v.staging.len() as u64 > arena_bytes {
            return Err(bad("antrenare-mini: zona de tranzit mai mare decat arena"));
        }
    }
    let zone_bytes = (n_launch as u64 * ts::SLOT).next_multiple_of(PAGE);
    if 16 * (n_launch as u64 + 1) > ctx.rec.size || n_launch as u32 + 1 > STRIDE {
        return Err(bad("antrenare-mini: prea putine inregistrari"));
    }
    let varena = map_vram(st, arena_bytes)?;
    let plan0 = glue(Plan::new(a, bsz, tl, x0, varena.va)?).resident()?;
    if plan0.bytes() > arena_bytes || plan0.programs() != probe0.programs() || plan0.fused_programs() != probe0.fused_programs() || plan0.fused_back() != fused || plan0.fused_gelu() != fused {
        return Err(bad("antrenare-mini: planul rezident difera intre adrese"));
    }
    let Some(host) = ram_vram_host((arena_bytes + zone_bytes) / PAGE) else {
        lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: nu am {} KiB RAM contigua sub 4 GiB; nimic trimis", p, tag, (arena_bytes + zone_bytes) >> 10));
        journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
        return Err(GpuError::OutOfMemory { requested: arena_bytes + zone_bytes });
    };
    let traffic = plan0.traffic();
    let count: usize = a.tensors().iter().map(|t| t.elements()).sum();
    lines.push(format!(
        "{}: kronos --gpu: {} ({}): modelul vocabular {}, context {}, E {}, {} capete, {} straturi ({} greutati), lot {} x {}, AdamW lr 1e-3, taiere 1,0; {}; {} programe (SHA-256 verificate), {} lansari pe pas, cate o trimitere; arena {} KiB in VRAM (facuta o data), zona de control {} KiB in RAM fixata; {} pasi, oracolul pe procesor la fiecare pas (pierderea, norma), greutatile la sfarsit",
        p,
        tag,
        label,
        a.vocab,
        a.ctx,
        a.n_emb,
        a.heads,
        a.layers,
        count,
        bsz,
        tl,
        data.source,
        plan0.programs().len() + plan0.fused_programs().len(),
        n_launch,
        arena_bytes >> 10,
        zone_bytes >> 10,
        steps
    ));
    lines.push(format!(
        "{}: kronos --gpu: {}: octeti pe legatura: o data {} KiB sus; pe pas {} B sus / {} B jos (drumul os-380: {} KiB sus / {} KiB jos pe pas); la sfarsit {} KiB jos",
        p,
        tag,
        traffic.once_up >> 10,
        traffic.step_up,
        traffic.step_down,
        traffic.today_step_up >> 10,
        traffic.today_step_down >> 10,
        traffic.final_down >> 10
    ));
    let mut progs: Vec<(ts::Prog, Mapping)> = Vec::with_capacity(plan0.programs().len() + plan0.fused_programs().len());
    for &prog in plan0.programs().iter().chain(plan0.fused_programs().iter()) {
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    let pages = |first: u64, n: u64| (first..first + n).map(|i| host + i * PAGE).collect::<Vec<u64>>();
    let stage = st.mem.alloc_sys_mapped(&pages(0, arena_bytes / PAGE))?;
    let ctl = st.mem.alloc_sys_mapped(&pages(arena_bytes / PAGE, zone_bytes / PAGE))?;
    if !stage.va.is_multiple_of(PAGE) || !ctl.va.is_multiple_of(PAGE) {
        return Err(bad("antrenare-mini: zona RAM nu e aliniata la 4 KiB"));
    }
    let mut maps = ctx.maps.clone();
    maps.push(varena);
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    let sys = [ctx.sys.clone(), stage.clone(), ctl.clone()];
    let prog_va = |want: ts::Prog| progs.iter().find(|(q, _)| *q == want).map_or(0, |(_, pm)| pm.va);
    // One launch of the copy program: `len` bytes from `src` to `dst` (GPU VAs), its own submission; the board time.
    let copy = |st: &mut GpuState, vbar: &mut V, clock: &mut K, journal: &mut Journal, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, src: u64, dst: u64, len: u64| -> GpuResult<Option<u64>> {
        let (cb_va, cb_pa) = (ctx.launches.va, ctx.launches.pa);
        {
            let w = st.mem.writer_mut();
            put_bytes(w, cb_pa, &transfer::copy_cbuf(src, dst, len)?)?;
            put_bytes(w, cb_pa + 256, &transfer::copy_qmd(ctx.copy_va, cb_va, len)?.to_bytes()?)?;
            put_bytes(w, ctx.rec.pa, &[0; 32])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[cb_va + 256], ctx.rec.va, first)?;
        let nw = pb.len();
        match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => {
                *subs += 1;
                Ok(Some(transfer::analyze_sequence(&b, 1, first)?[0]))
            }
            Err(_) => Ok(None),
        }
    };
    // 1. The arena, once: filled in the host run, then RAM → VRAM.
    let params0 = am::initial(&a, 0);
    ram_vram_bytes(host, arena_bytes).fill(0);
    for (va, bytes) in plan0.uploads(&params0, x0, y0)? {
        ram_vram_bytes(host + (va - varena.va), bytes.len() as u64).copy_from_slice(&bytes);
    }
    let Some(up_ns) = copy(st, vbar, clock, journal, l, doorbell, subs, stage.va, varena.va, arena_bytes)? else {
        lines.push(format!("{}: kronos --gpu: {}: urcarea arenei RAM -> VRAM nu s-a terminat; proba oprita, fara reincercare", p, tag));
        journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
        return Err(GpuError::Timeout { waited_us: WAIT_US });
    };
    journal.push(Event::Value { what: KT_MINI_UP_US, value: (up_ns / 1000) as u32 });
    lines.push(format!("{}: kronos --gpu: {}: arena urcata o data RAM -> VRAM: {} KiB in {} us (placa)", p, tag, arena_bytes >> 10, up_ns / 1000));
    // The VRAM reads after a step: the per-row losses and the norm, through the VRAM window.
    let read_vram_f32s = |st: &mut GpuState, vbar: &mut V, journal: &mut Journal, va: u64, n: usize| -> GpuResult<Vec<f32>> {
        let GpuState { zone, .. } = &*st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
        let pa = varena.pa + (va - varena.va);
        let r = (0..n).map(|i| vw.read_u32(pa + 4 * i as u64).map(f32::from_bits)).collect::<GpuResult<Vec<f32>>>();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))
    };
    let adam = AdamW::new(&params0, 1e-3, 0.9, 0.999, 1e-8, 0.1);
    let (mut op, mut oa) = (params0.clone(), adam.clone());
    let (mut worst_loss, mut worst_norm) = (0.0f64, 0.0f64);
    let (mut board_ns, mut host_us, mut losses): (Vec<u64>, Vec<u64>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new());
    let mut norms: Vec<f32> = Vec::new();
    let mut final_w: Vec<f32> = Vec::new();
    let (mut bytes_up, mut bytes_down) = (0u64, 0u64);
    let mut stopped: Option<String> = None;
    let mut parts: [Vec<u64>; 5] = Default::default();
    let mut kicks: [Vec<u64>; 6] = Default::default();
    // P6-174 (cyber-gpu notes `C-157_P2_os-386_pasul-intreg.md`, `C-157_P2_taieturi-pasul-intreg.md`; nothing new for the
    // board): (1) the journal as in the generation — the routine rows (VRAM window, TLB, doorbell and their notes) stay
    // in memory during the steps (`GenHold` + the routine, `flush_live` once per step), instead of one stick write each
    // (~16.5 ms, 22 per step in os-386); (2) the second row of cuts — the pushbuffer through a write shadow (the list is
    // the same every step), the records zeroed once (step 1; the payloads grow by `STRIDE` per submission, an old record
    // never passes for a new one), only the first + the last record read except on the `RecordPlan::DIET` steps, and the
    // planned wait. The same tools run on the board in `kronos --gpu`.
    let mut push_shadow = kf::WriteShadow::new(l.push.pa, l.push.size)?;
    let mut wait_plan = nvidia_ga104::kronos_diet2::WaitPlan::new();
    let (mut rec_words, mut polls_sum, mut full_steps) = (0u64, 0u64, 0usize);
    // P6-179: the board time of the fully read steps split by launch (tag, program, stage), Cyber-GPU `12b622d`.
    let mut prof: Option<nvidia_ga104::train_profile::StepProfile> = None;
    let writes0 = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed);
    let dropped0 = super::gpu_i6::DROPPED.load(core::sync::atomic::Ordering::Relaxed);
    let jus0 = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed);
    journal.flush_live();
    let hold = super::gpu_i6::GenHold::start();
    journal.set_routine(Some(kf::gen_routine(&GEN_NOTES)));
    let mut run_steps = || -> GpuResult<()> {
    for (k, (x, y)) in data.batches.iter().enumerate() {
        let step = k as u32 + 1;
        let t0 = crate::time::since_boot_us();
        let mut marks: [Option<u64>; 5] = [None; 5];
        let plan = plan0.rebatch(x)?;
        if plan.fused_back() != fused || plan.fused_gelu() != fused || plan.w != plan0.w || plan.m != plan0.m || plan.v != plan0.v || plan.loss_rows != plan0.loss_rows || plan.res != plan0.res {
            return Err(bad("antrenare-mini: adresele planului rezident s-au mutat"));
        }
        marks[0] = crate::time::since_boot_us();
        // P6-178: through RAM the batch goes into the start of `stage` (pinned RAM, unused between the arena's two
        // big copies) every step, step 1 too, so every step is the same; the copies in the list carry it.
        let via = if via_ram { Some(nvidia_ga104::batch_ram::via_ram(&plan, x, y, stage.va)?) } else { None };
        if let Some(v) = &via {
            ram_vram_bytes(host, v.staging.len() as u64).copy_from_slice(&v.staging);
            if k > 0 {
                bytes_up += v.copies_in.iter().map(|c| c.bytes).sum::<u64>();
            }
        } else if k > 0 {
            let w = st.mem.writer_mut();
            for (va, bytes) in plan.batch_uploads(x, y)? {
                bytes_up += bytes.len() as u64;
                put_bytes(w, varena.pa + (va - varena.va), &bytes)?;
            }
        }
        marks[1] = crate::time::since_boot_us();
        let list = plan.step(Recipe::AdamW513, &adam, step)?;
        let (zb, qmds) = match &via {
            Some(v) => nvidia_ga104::batch_ram::pack_via_ram(&list, prog_va, ctx.copy_va, ctl.va, v)?,
            None => ts::pack(&list, prog_va, ctl.va)?,
        };
        if zb.len() as u64 > zone_bytes {
            return Err(bad("antrenare-mini: zona de control prea mare"));
        }
        ram_write(&ctl, &zb);
        let n_list = qmds.len();
        if prof.is_none() {
            let pr = nvidia_ga104::train_profile::StepProfile::from_launches(&list)?;
            let pr = match &via {
                Some(v) => pr.with_copies(v.copies_in.len(), v.copies_out.len())?,
                None => pr,
            };
            if pr.launches() != n_list {
                return Err(bad("antrenare-mini: profilul pasului nu are cate lansari are lista"));
            }
            prof = Some(pr);
        }
        if k == 0 {
            // P6-174: the records on zero once, for the longest list of the run.
            put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (n_launch.max(n_list) + 1)])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(n_list)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence_fresh(&mut pb, &qmds, ctx.rec.va, first)?;
        let nw = pb.len();
        let last = first + n_list as u32;
        marks[2] = crate::time::since_boot_us();
        let all = nvidia_ga104::kronos_diet2::RecordPlan::DIET.reads_all(k);
        let mut polls = 0u32;
        let d2 = super::gpu_i11::Diet2Wait { plan: &wait_plan, all, polls: &mut polls };
        let b = match super::gpu_i11::run_records_d2(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, n_list as u64 + 1, last, &maps, &sys, doorbell, Some(&mut push_shadow), Some(d2)) {
            Ok(Ok(b)) => b,
            Err(e) => {
                wait_plan.forget();
                return Err(e);
            }
            Ok(Err(got)) => {
                wait_plan.forget();
                stopped = Some(format!("placa nu a terminat la pasul {} (ultima inregistrare {:#x}, asteptat {:#x})", step, got, last));
                break;
            }
        };
        *subs += 1;
        marks[3] = crate::time::since_boot_us();
        let step_kick = super::gpu_i10::kick_gaps();
        if let Some(g) = step_kick {
            for (i, v) in g.iter().enumerate() {
                kicks[i].push(*v);
            }
        }
        let busy: u64 = if all {
            full_steps += 1;
            let ns = transfer::analyze_sequence(&b, n_list, first)?;
            // P6-179: the same records the step's time comes from; a list of another length is refused there.
            if let Some(pr) = prof.as_mut() {
                pr.add_step(&ns)?;
            }
            ns.iter().sum()
        } else {
            nvidia_ga104::kronos_diet2::ends_span(&b[..16], &b[16 * n_list..16 * (n_list + 1)], n_list, first)?
        };
        wait_plan.observe(busy / 1000);
        rec_words += if all { 4 * (n_list as u64 + 1) } else { 8 };
        polls_sum += u64::from(polls);
        board_ns.push(busy);
        let [(lr_va, lr_n), (res_va, res_n)] = plan.step_reads();
        let (rows, norm) = match &via {
            // P6-178: the two copies at the end of the list brought them into the staging area (a copy that did not
            // arrive leaves 0 — the area is written with zeros with the batch — and the oracle catches it).
            Some(v) => nvidia_ga104::batch_ram::results(ram_vram_bytes(host, v.staging.len() as u64), v)?,
            None => {
                let rows = read_vram_f32s(st, vbar, journal, lr_va, (lr_n / 4) as usize)?;
                let norm = read_vram_f32s(st, vbar, journal, res_va, (res_n / 4) as usize)?[0];
                (rows, norm)
            }
        };
        bytes_down += lr_n + res_n;
        let loss = Outcome::mean_loss(&rows);
        marks[4] = crate::time::since_boot_us();
        // P6-174: once per step, as once per token in the generation (in a `GenHold` nothing goes to the stick here).
        journal.flush_live();
        if let Some(us) = t0.zip(marks[4]).map(|(p0, p1)| p1.saturating_sub(p0)) {
            host_us.push(us);
        }
        // P6-173: the five pieces of the host step (each = the time since the previous mark).
        let step_parts = mini_parts(t0, &marks);
        if let Some(sp) = step_parts {
            for (i, v) in sp.iter().enumerate() {
                parts[i].push(*v);
            }
        }
        if !norm.is_finite() || !loss.is_finite() {
            stopped = Some(format!("pasul {}: pierderea {:?} / norma gradientului {:?} nu e un numar", step, loss, norm));
            break;
        }
        losses.push(loss);
        norms.push(norm);
        let batch = Batch { inputs: x, targets: y, batch: bsz, len: tl };
        let (ol, mut og) = train::loss_and_grad(&a, &op, &batch)?;
        let onorm = train::clip_grad_norm(&mut og, 1.0);
        oa.step(&mut op, &og)?;
        let dn = ((norm as f64 - onorm as f64) / (onorm as f64).abs().max(1e-12)).abs();
        let dl = ((loss as f64 - ol as f64) / (ol as f64).abs().max(1e-12)).abs();
        worst_norm = worst_norm.max(dn);
        worst_loss = worst_loss.max(dl);
        lines.push(format!(
            "{}: kronos --gpu: {}: pasul {}: pierderea {:.4} (oracolul {:.4}, abaterea {:.1e}), norma gradientului {:.4} (oracolul {:.4}, abaterea {:.1e}); placa {} us, pasul intreg {} us (lotul sus + placa + pierderile jos, fara oracol){}{}",
            p,
            tag,
            step,
            loss,
            ol,
            dl,
            norm,
            onorm,
            dn,
            board_ns.last().copied().unwrap_or(0) / 1000,
            host_us.last().copied().unwrap_or(0),
            step_parts.map_or(String::new(), |sp| format!("; bucatile: {}", mini_parts_text(&sp))),
            step_kick.map_or(String::new(), |g| format!("; trimiterea: {}", kick_gaps_text(&g)))
        ));
    }
    Ok(())
    };
    let steps_r = run_steps();
    drop(hold);
    journal.set_routine(None);
    let stick_writes = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(writes0);
    let dropped = super::gpu_i6::DROPPED.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(dropped0);
    let journal_us = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(jus0);
    journal.push(Event::Value { what: KT_MINI_STICK, value: stick_writes.min(u32::MAX as usize) as u32 });
    journal.push(Event::Value { what: KT_MINI_JOURNAL_US, value: journal_us.min(u32::MAX as u64) as u32 });
    journal.push(Event::Value { what: KT_MINI_REC_WORDS, value: rec_words.min(u32::MAX as u64) as u32 });
    lines.push(format!(
        "{}: kronos --gpu: {}: taieturile pasului (P6-174): jurnalul in pasi {} scrieri pe stick ({} us de jurnal, {} randuri de rutina aruncate); pushbuffer prin umbra; inregistrarile pe zero o data, citite toate la {} pasi, altfel prima + ultima ({} cuvinte citite in total); asteptarea planificata ({} citiri)",
        p, tag, stick_writes, journal_us, dropped, full_steps, rec_words, polls_sum
    ));
    // P6-179: once, at the end (~30 rows): the total, the stages, the 8 longest tags, the programs.
    let prof_steps = prof.as_ref().map_or(0, |pr| pr.steps());
    journal.push(Event::Value { what: KT_MINI_PROFILE, value: prof_steps as u32 });
    if let Some(pr) = &prof {
        for row in pr.lines(8) {
            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
        }
    }
    steps_r?;
    // 2. At the end: VRAM → RAM once, the weights against the oracle's.
    let mut worst_w = f64::INFINITY;
    if stopped.is_none() {
        match copy(st, vbar, clock, journal, l, doorbell, subs, varena.va, stage.va, arena_bytes)? {
            Some(down_ns) => {
                let wb = ts::f32s(ram_vram_bytes(host + (plan0.w - varena.va), 4 * plan0.n_params as u64));
                let board_w = plan0.unflat(&wb)?;
                final_w = wb.clone();
                worst_w = am::rel_dev(&board_w, &op);
                lines.push(format!(
                    "{}: kronos --gpu: {}: arena adusa inapoi VRAM -> RAM la sfarsit in {} us (placa); greutatile dupa {} pasi fata de oracol: {:.1e}",
                    p,
                    tag,
                    down_ns / 1000,
                    losses.len(),
                    worst_w
                ));
            }
            None => stopped = Some(String::from("copia arenei VRAM -> RAM de la sfarsit nu s-a terminat")),
        }
    }
    let us = median_us(&board_ns);
    let host_med_us = median_us(&host_us.iter().map(|u| u * 1000).collect::<Vec<u64>>());
    let tok_s = if us > 0 { (bsz * tl) as u64 * 1_000_000 / us } else { 0 };
    let full_tok_s = if host_med_us > 0 { (bsz * tl) as u64 * 1_000_000 / host_med_us } else { 0 };
    let per_step = |b: u64| if steps > 1 { b / (steps as u64 - 1).max(1) } else { b };
    journal.push(Event::Value { what: KT_MINI_STEPS, value: losses.len() as u32 });
    journal.push(Event::Value { what: KT_MINI_US, value: us as u32 });
    journal.push(Event::Value { what: KT_MINI_HOST_MS, value: (host_med_us / 1000) as u32 });
    journal.push(Event::Value { what: KT_MINI_HOST_US, value: host_med_us as u32 });
    let part_med: Option<[u64; 5]> = (!parts[0].is_empty()).then(|| core::array::from_fn(|i| median_us(&parts[i].iter().map(|u| u * 1000).collect::<Vec<u64>>())));
    if let Some(pm) = part_med {
        for (i, v) in pm.iter().enumerate() {
            journal.push(Event::Value { what: KT_MINI_PARTS[i], value: (*v).min(u32::MAX as u64) as u32 });
        }
    }
    journal.push(Event::Value { what: KT_MINI_TOK_S, value: tok_s as u32 });
    journal.push(Event::Value { what: KT_MINI_LOSS_E9, value: (worst_loss * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_W_E9, value: (worst_w * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_NORM_E9, value: (worst_norm * 1e9).min(u32::MAX as f64) as u32 });
    journal.push(Event::Value { what: KT_MINI_STEP_UP, value: per_step(bytes_up) as u32 });
    journal.push(Event::Value { what: KT_MINI_STEP_DOWN, value: (bytes_down / losses.len().max(1) as u64) as u32 });
    if let Some(why) = &stopped {
        lines.push(format!("{}: kronos --gpu: {}: {}; proba oprita, fara reincercare", p, tag, why));
        lines.push(format!("{}: kronos --gpu: {}: NU A TERMINAT (vezi randul de mai sus)", p, tag));
        journal.push(Event::Value { what: KT_MINI_PASS, value: 0 });
        return Err(GpuError::Timeout { waited_us: WAIT_US });
    }
    if let Some(l0) = losses.first() {
        lines.push(format!("{}: kronos --gpu: {}: pierderea la pornire {:.4} (ghicit la intamplare: ln {} = {:.4})", p, tag, l0, a.vocab, am::ln(a.vocab as f64)));
    }
    lines.push(format!(
        "{}: kronos --gpu: {}: timpul pe pas (median, {} pasi): placa {} us = {} simboluri/s la antrenare (lot {} x {}); pasul intreg {} us = {} simboluri/s (lotul sus, placa, pierderile + norma jos; fara oracol)",
        p,
        tag,
        losses.len(),
        us,
        tok_s,
        bsz,
        tl,
        host_med_us,
        full_tok_s
    ));
    if let Some(pm) = part_med {
        let biggest = (0..5).max_by_key(|&i| pm[i]).unwrap_or(0);
        lines.push(format!(
            "{}: kronos --gpu: {}: pasul intreg pe bucati (median, {} pasi): {}; cea mai mare: {} ({} us din {} us)",
            p,
            tag,
            parts[0].len(),
            mini_parts_text(&pm),
            MINI_PART_NAMES[biggest],
            pm[biggest],
            host_med_us
        ));
    }
    if !kicks[0].is_empty() {
        let km: [u64; 6] = core::array::from_fn(|i| median_us(&kicks[i].iter().map(|u| u * 1000).collect::<Vec<u64>>()));
        for (i, v) in km.iter().enumerate() {
            journal.push(Event::Value { what: KT_MINI_KICKS[i], value: (*v).min(u32::MAX as u64) as u32 });
        }
        lines.push(format!("{}: kronos --gpu: {}: trimiterea + asteptarea pe bucati (median, {} pasi): {}", p, tag, kicks[0].len(), kick_gaps_text(&km)));
    }
    let pass = worst_loss <= TOL && worst_norm <= TOL && worst_w <= TOL;
    journal.push(Event::Value { what: KT_MINI_PASS, value: u32::from(pass) });
    lines.push(format!(
        "{}: kronos --gpu: {}: lista placii ({}) fata de oracol: {} — cea mai mare abatere a pierderii {:.1e}, a normei {:.1e} in {} pasi, a greutatilor la sfarsit {:.1e} (toleranta 1e-3)",
        p,
        tag,
        label,
        if pass { "ACELEASI NUMERE" } else { "DIFERA" },
        worst_loss,
        worst_norm,
        losses.len(),
        worst_w
    ));
    lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, if pass { "TRECE (antrenarea cu arena in VRAM = oracolul)" } else { "NU TRECE (vezi randurile de mai sus)" }));
    Ok(ResidentRun { losses, norms, weights: final_w, launches: n_launch, pass, board_us: us, host_us: host_med_us, parts: part_med })
}

/// P6-172 (command-center note of 23 Nov 23:36, point 3): the journal rows of the #513 training step on the board.
const KT_513_ARENA_MIB: &str = "kt antrenare-513 arena in VRAM (MiB)";
const KT_513_UP_MS: &str = "kt antrenare-513 urcarea arenei pe bucati (ms, placa)";
const KT_513_LAUNCHES: &str = "kt antrenare-513 lansari pe pas";
const KT_513_SUBS: &str = "kt antrenare-513 trimiteri pe pas";
const KT_513_STEPS: &str = "kt antrenare-513 pasi";
const KT_513_PROFILE: &str = "kt antrenare-513 profilul pasului (pasi cititi intregi)";
const KT_513_US: &str = "kt antrenare-513 us pe pas placa (median, din inregistrari)";
const KT_513_HOST_US: &str = "kt antrenare-513 us pe pas intreg (median)";
const KT_513_TOK_S: &str = "kt antrenare-513 simboluri/s placa";
const KT_513_LOSS1_E4: &str = "kt antrenare-513 pierderea la pasul 1 (x1e4)";
const KT_513_LOSSN_E4: &str = "kt antrenare-513 pierderea la ultimul pas (x1e4)";
const KT_513_PASS: &str = "kt antrenare-513 verdict (1/0)";
const KT_513_ORACLE: &str = "kt antrenare-513 = oracolul procesorului in pasii 1-9 (1/0)";
/// P6-185: 1 = the step built with `Plan::new_tiles3` (`--placi3`).
const KT_513_TILES3: &str = "kt antrenare-513 atentia pe placi gen 3 (1/0)";
/// P6-188: 1 = the step's `mm_train` launches on `mm_train_tile` (`--mm-placi`).
const KT_513_MM_TILE: &str = "kt antrenare-513 inmultirile pe mm_train_tile (1/0)";
/// P6-189: 1 = the step's `colsum` launches split over rows (`--colsum-impartit`).
const KT_513_COLSUM_SPLIT: &str = "kt antrenare-513 colsum impartit pe randuri (1/0)";
const KT_513_FUSED_TILE: &str = "kt antrenare-513 lipit pe mm_train_tile_gelu (1/0)";
/// P6-190: the night race's rows with the witness's bits, rows that differ, and the step time the winners would save.
const KT_513_RACE_SAME: &str = "kt cursa mm randuri cu aceiasi biti";
const KT_513_RACE_DIFFER: &str = "kt cursa mm randuri diferite";
const KT_513_RACE_SAVED_US: &str = "kt cursa mm pasul ar scadea cu (us, DEDUS)";
/// P6-195: the lone `mm_tv_64x64k8` probe finished with the witness's bits (1) or not (0).
const KT_513_PROBE_64K8: &str = "kt proba 64x64k8 terminata cu aceiasi biti (1/0)";
/// P6-195: the variant held since os-413 (`RACE_HOLD`), probed alone with `--proba-64k8`.
const PROBE_64K8_NAME: &str = "mm_tv_64x64k8";
/// P6-201: the occupancy probe's journal rows.
const KT_513_OCC_SAME: &str = "kt proba ocuparii rulari cu aceiasi biti (din 10)";
const KT_513_OCC_RATIO: &str = "kt proba ocuparii mm_train_tile 64 / 8 KiB (la mie)";

/// P6-201: one occupancy row: `ocupare <program> <N> KiB: <us> us, <TFLOP/s>, <W> W, ACEIASI BITI` (the note's format).
fn occ_row(name: &str, step_bytes: u32, f: &RaceForm, ns: Option<u64>, mw: Option<u32>, bits: RaceBits) -> String {
    let flops = 2 * u64::from(f.m) * u64::from(f.n) * u64::from(f.k);
    let (us, tf) = match ns {
        Some(n) if n > 0 => (format!("{}.{} us", n / 1000, (n % 1000) / 100), format!("{}.{:02} TFLOP/s", flops / n / 1000, (flops * 100 / n / 1000) % 100)),
        _ => (String::from("- us"), String::from("- TFLOP/s")),
    };
    let w = mw.map_or(String::from("- W"), |m| format!("{}.{} W", m / 1000, (m % 1000) / 100));
    let b = match bits {
        RaceBits::Same => "ACEIASI BITI",
        RaceBits::Differ => "DIFERA",
        RaceBits::Unfinished => "NETERMINATA",
        RaceBits::Skipped => "SARITA",
    };
    format!("ocupare {} {} KiB: {}, {}, {}, {}", name, step_bytes >> 10, us, tf, w, b)
}

/// P6-201: the witness (`mm_train_tile`, variant 0) at 64 KiB over 8 KiB, per mille (both runs with the same bits).
fn occ_witness_ratio(rows: &[(usize, u32, Option<u64>, RaceBits, Option<u32>)]) -> Option<u32> {
    let t = |kib: u32| rows.iter().find(|r| r.0 == 0 && r.1 == kib << 10 && r.3 == RaceBits::Same).and_then(|r| r.2);
    let (a, b) = (t(8)?, t(64)?);
    (a > 0).then(|| (b * 1000 / a).min(u64::from(u32::MAX)) as u32)
}

/// P6-201: the verdict rows of the occupancy probe — `proba ocuparii: k / 10 aceiasi biti`, the witness's ratio and
/// H1 / H2 by the note's thresholds (H1: 64 KiB <= 0.85 of 8 KiB; H2: every step within +-3 %), and per program the
/// time at each step relative to its 8 KiB run.
fn occ_verdict(rows: &[(usize, u32, Option<u64>, RaceBits, Option<u32>)], planned: usize, stopped: bool) -> Vec<String> {
    use nvidia_ga104::mm_train_variants::VARIANTS;
    let mut out = Vec::new();
    let same = rows.iter().filter(|r| r.3 == RaceBits::Same).count();
    out.push(format!(
        "proba ocuparii: {} / {} aceiasi biti{}",
        same,
        planned,
        if stopped { " (NU A TERMINAT: proba oprita la prima rulare neterminata)" } else if same < rows.len() { " (DIFERA sau SARITA: STOP, nu se leaga nimic)" } else { "" }
    ));
    let mut seen: Vec<usize> = Vec::new();
    for r in rows {
        if seen.contains(&r.0) {
            continue;
        }
        seen.push(r.0);
        let base = rows.iter().find(|x| x.0 == r.0 && x.1 == 8 << 10 && x.3 == RaceBits::Same).and_then(|x| x.2);
        let parts: Vec<String> = rows
            .iter()
            .filter(|x| x.0 == r.0)
            .map(|x| match (x.2, base, x.3) {
                (Some(t), Some(b), RaceBits::Same) if b > 0 => format!("{} KiB {} la mie", x.1 >> 10, t * 1000 / b),
                _ => format!("{} KiB -", x.1 >> 10),
            })
            .collect();
        let name = VARIANTS.get(r.0).map_or("?", |v| v.name);
        out.push(format!("proba ocuparii: {}: timpul fata de 8 KiB: {}", name, parts.join(", ")));
    }
    let all_same = same == planned && !stopped;
    let witness: Vec<u64> = rows.iter().filter(|r| r.0 == 0 && r.3 == RaceBits::Same).filter_map(|r| r.2).collect();
    let verdict = match occ_witness_ratio(rows) {
        _ if !all_same => String::from("fara verdict H1 / H2 (bitii nu sunt aceiasi la toate rularile)"),
        Some(q) if q <= 850 => format!("H1: mm_train_tile la 64 KiB = {} la mie din 8 KiB (<= 850): treapta mai mare ajuta, ocuparea era o frana", q),
        Some(q) => {
            let (lo, hi) = (witness.iter().copied().min().unwrap_or(0), witness.iter().copied().max().unwrap_or(0));
            if lo > 0 && hi * 1000 <= lo * 1030 {
                format!("H2: mm_train_tile la toate treptele in +-3 % (64 / 8 KiB = {} la mie): placa urca deja singura treapta, ocuparea nu e buba", q)
            } else {
                format!("intre H1 si H2: mm_train_tile 64 / 8 KiB = {} la mie (peste 850, dar treptele nu sunt in +-3 %)", q)
            }
        }
        None => String::from("fara verdict H1 / H2 (lipseste timpul la 8 sau 64 KiB)"),
    };
    out.push(format!("proba ocuparii: {}", verdict));
    out
}

/// P6-196: 1 = #513's `mm_train` shapes on the os-416 race's pick (`--mm-alegere`), and how many shapes left the witness.
const KT_513_MM_CHOICE: &str = "kt antrenare-513 inmultirile pe varianta aleasa de cursa (1/0)";
/// P6-205: 1 = #513's step packed with one SM shared-memory step (`--treapta-pas`), and that step in KiB.
const KT_513_SM_STEP: &str = "kt antrenare-513 o treapta a memoriei comune pe tot pasul (1/0)";
const KT_513_SM_STEP_KIB: &str = "kt antrenare-513 treapta pasului KiB";
/// P6-213: `--mm-model` = the choice from the waves model (1) instead of the os-416 race.
const KT_513_MM_MODEL: &str = "kt antrenare-513 alegerea din modelul valurilor (1/0)";
/// P6-213: the SM shared-memory step the waves model judges at (64 KiB: what `--treapta-pas` picks for #513).
const MM_MODEL_STEP: u32 = 64 << 10;
/// P6-208: #513's batch (sequences of 512 per step) actually run: 2 by default, N after `--lot-513 N` if it fits.
const KT_513_BATCH: &str = "kt antrenare-513 lot (secvente pe pas)";
/// P6-208 (DECIZII 25 Nov 11:52/11:55): does #513's arena, with the launch zone, fit in the largest free VRAM piece
/// while keeping `reserve` free? (No fixed ceiling: the board is ours alone in this OS.)
fn batch_fits(arena: u64, zone: u64, reserve: u64, largest: u64) -> bool {
    arena.saturating_add(zone).saturating_add(reserve) <= largest
}
/// P6-198: 1 = #513's mixed step (`--mixt-513`, every product on `mm_tc`, S = 2^14), 0 = the F32 step.
const KT_513_MIXED: &str = "kt antrenare-513 pasul mixt (1/0)";
/// P6-198: the `mm_tc` launches of one mixed #513 step (147 expected on the board).
const KT_513_MIXED_TC: &str = "kt antrenare-513 lansari mm_tc pe pas";
/// P6-216 (note C-157_P2zg): #513's batch and results through pinned RAM (1/0), and the step outside the board (median).
const KT_513_VIA_RAM: &str = "kt antrenare-513 lot prin RAM (1/0)";
/// P6-231: 1 = dQ / dK / dV on `attn_back_tile_tc16` in the fast mixed step (`--atentie-tc16`).
const KT_513_ATTN_TC16: &str = "kt antrenare-513 atentia pe f16 (1/0)";
/// P6-237: 1 when #513's fast mixed step ran with `attn_back_tile_tc16_v4` on the aligned launches (`--atentie-v4`).
const KT_513_ATTN_V4: &str = "kt antrenare-513 atentia pe f16 v4 (1/0)";
/// P6-241: dS of #513's fast mixed step on `attn_back_tile0_tc` / `_v4` (`--atentie-ds`).
const KT_513_ATTN_DS: &str = "kt antrenare-513 dS pe f16 (1/0)";
/// P6-245: 1 when #513's fast mixed step ran with `mm_tc_v5` on the aligned products (`--pas-v5`).
const KT_513_MM_V5: &str = "kt antrenare-513 mm_tc_v5 in pas (1/0)";
/// P6-247: 1 when #513's fast mixed step ran with `mm_tc_v6` first on the aligned products (`--pas-v6`).
const KT_513_MM_V6: &str = "kt antrenare-513 mm_tc_v6 in pas (1/0)";
/// P6-254: 1 when #513's fast mixed step ran with `mm_tc_v7` first on the aligned products (`--pas-v7`).
const KT_513_MM_V7: &str = "kt antrenare-513 mm_tc_v7 in pas (1/0)";
/// P6-259: 1 when #513's fast mixed step ran with v7 / v8 / v8n picked by shape on the aligned products (`--pas-v8`).
const KT_513_MM_V8: &str = "kt antrenare-513 mm_tc_v8 in pas (1/0)";
/// P6-264: 1 when #513's fast mixed step ran with the long-sum guard (`--pas-v8-garda`, k >= 8 192 on v7).
const KT_513_MM_V8_LONG_K: &str = "kt antrenare-513 garda suma lunga in pas (1/0)";
/// P6-265: 1 when #513's fast mixed step ran with `attn_fwd_tc` on the aligned forward attention (`--pas-atentie-fwd-tc`).
const KT_513_ATTN_FWD_TC: &str = "kt antrenare-513 attn_fwd_tc in pas (1/0)";
/// P6-266: 1 when #513's fast mixed step ran with the residual backward glued (`--pas-rezidual-lipit`, `ln_back_res`).
const KT_513_FUSED_BACK_FAST: &str = "kt antrenare-513 rezidual lipit in pas rapid (1/0)";
/// P6-268: 1 when #513's fast mixed step ran with GELU' glued into the fast products' tail (`--pas-gelu-lipit`).
const KT_513_FUSED_GELU_FAST: &str = "kt antrenare-513 gelu lipit in pas rapid (1/0)";
const KT_513_OUTSIDE_US: &str = "kt antrenare-513 us in afara placii pe pas (median)";
/// P6-217 (note C-157_P2zg step 2): two batch slots (1/0), and the parts of steps 2..N sent while the step before ran.
const KT_513_SLOTS: &str = "kt antrenare-513 doua locuri de lot (1/0)";
const KT_513_AHEAD: &str = "kt antrenare-513 parti trimise in avans";
const KT_513_MM_CHOICE_FORMS: &str = "kt antrenare-513 forme pe o varianta aleasa";
/// P6-196 (C-165, centre note of 25 Nov 02:10): the 60 race rows of os-416 (hub
/// `rezultate/os-automat/2026-11-25/ciclu-auto-20261125-0135.txt`), copied as the OS wrote them (`race_row`), behind a
/// `os-416: ` prefix so `mm_train_variants::parse_race` reads them. All 60 with the witness's bits; `mm_tv_128x128`
/// fastest on all 15 shapes.
const OS416_RACE: &str = "\
os-416: cursa: mm_train_tile | 1024x2304x768 f6 (x12 in pas) | 2839.4 us median | 1.27 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x2304x768 f6 (x12 in pas) | 2717.2 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x2304x768 f6 (x12 in pas) | 2710.5 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x2304x768 f6 (x12 in pas) | 1890.2 us median | 1.91 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x768 f6 (x12 in pas) | 841.4 us median | 1.43 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x768 f6 (x12 in pas) | 903.2 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x768 f6 (x12 in pas) | 904.3 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x768 f6 (x12 in pas) | 633.1 us median | 1.90 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x3072x768 f6 (x12 in pas) | 3801.5 us median | 1.27 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x3072x768 f6 (x12 in pas) | 3640.8 us median | 1.32 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x3072x768 f6 (x12 in pas) | 3616.8 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x3072x768 f6 (x12 in pas) | 2534.0 us median | 1.90 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x3072 f6 (x12 in pas) | 3702.4 us median | 1.30 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x3072 f6 (x12 in pas) | 3557.6 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x3072 f6 (x12 in pas) | 3546.0 us median | 1.36 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x3072 f6 (x12 in pas) | 2465.6 us median | 1.95 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x16384x768 f2 (x1 in pas) | 20312.7 us median | 1.26 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x16384x768 f2 (x1 in pas) | 19461.2 us median | 1.32 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x16384x768 f2 (x1 in pas) | 18111.9 us median | 1.42 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x16384x768 f2 (x1 in pas) | 14096.8 us median | 1.82 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 16384x768x1024 f1 (x1 in pas) | 19949.5 us median | 1.29 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 16384x768x1024 f1 (x1 in pas) | 17999.8 us median | 1.43 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 16384x768x1024 f1 (x1 in pas) | 17897.8 us median | 1.43 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 16384x768x1024 f1 (x1 in pas) | 12165.2 us median | 2.11 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x16384 f0 (x1 in pas) | 19692.5 us median | 1.30 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x16384 f0 (x1 in pas) | 17595.5 us median | 1.46 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x16384 f0 (x1 in pas) | 17589.8 us median | 1.46 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x16384 f0 (x1 in pas) | 12920.0 us median | 1.99 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x3072x768 f0 (x12 in pas) | 3488.6 us median | 1.38 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x3072x768 f0 (x12 in pas) | 3602.0 us median | 1.34 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x3072x768 f0 (x12 in pas) | 3553.0 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x3072x768 f0 (x12 in pas) | 2437.0 us median | 1.98 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 768x3072x1024 f1 (x12 in pas) | 3775.8 us median | 1.27 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 768x3072x1024 f1 (x12 in pas) | 3609.2 us median | 1.33 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 768x3072x1024 f1 (x12 in pas) | 3555.3 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 768x3072x1024 f1 (x12 in pas) | 2429.5 us median | 1.98 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x3072 f0 (x12 in pas) | 3691.6 us median | 1.30 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x3072 f0 (x12 in pas) | 3555.9 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x3072 f0 (x12 in pas) | 3553.3 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x3072 f0 (x12 in pas) | 2436.1 us median | 1.98 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 3072x768x1024 f1 (x12 in pas) | 3596.6 us median | 1.34 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 3072x768x1024 f1 (x12 in pas) | 3579.4 us median | 1.34 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 3072x768x1024 f1 (x12 in pas) | 3576.0 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 3072x768x1024 f1 (x12 in pas) | 2429.5 us median | 1.98 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x768 f0 (x12 in pas) | 834.0 us median | 1.44 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x768 f0 (x12 in pas) | 900.6 us median | 1.34 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x768 f0 (x12 in pas) | 899.0 us median | 1.34 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x768 f0 (x12 in pas) | 626.3 us median | 1.92 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 768x768x1024 f1 (x12 in pas) | 921.5 us median | 1.31 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 768x768x1024 f1 (x12 in pas) | 1194.1 us median | 1.01 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 768x768x1024 f1 (x12 in pas) | 1188.4 us median | 1.01 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 768x768x1024 f1 (x12 in pas) | 818.6 us median | 1.47 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 1024x768x2304 f0 (x12 in pas) | 2778.1 us median | 1.30 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 1024x768x2304 f0 (x12 in pas) | 2676.8 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 1024x768x2304 f0 (x12 in pas) | 2675.3 us median | 1.35 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 1024x768x2304 f0 (x12 in pas) | 1840.7 us median | 1.96 TFLOP/s | biti =\n\
os-416: cursa: mm_train_tile | 2304x768x1024 f1 (x12 in pas) | 2748.7 us median | 1.31 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x64 | 2304x768x1024 f1 (x12 in pas) | 2975.4 us median | 1.21 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_64x128 | 2304x768x1024 f1 (x12 in pas) | 2971.4 us median | 1.21 TFLOP/s | biti =\n\
os-416: cursa: mm_tv_128x128 | 2304x768x1024 f1 (x12 in pas) | 2427.7 us median | 1.49 TFLOP/s | biti =\n\
";

/// P6-196: the variant per `mm_train` shape of #513's step for `--mm-alegere`: the os-416 race read back with
/// `parse_race` -> `MmChoice::from_race` (Cyber-GPU's rule: the fastest with the witness's bits, only at >= 3 % gain).
/// A shape of `forms` the race never ran (none on the board; all of the small model in SIMULARE) gets the variant the
/// race chose most often. Returns (the choice, shapes from the race, shapes filled in, that variant's index, the step
/// saving the race deduces in ns).
fn mm_choice_513(forms: &[nvidia_ga104::mm_train_variants::Form]) -> GpuResult<(nvidia_ga104::mm_train_variants::MmChoice, usize, usize, usize, u64)> {
    use nvidia_ga104::mm_train_variants as mv;
    let race = mv::parse_race(OS416_RACE);
    let table = race.choice()?;
    let mut votes = [0usize; mv::VARIANTS.len()];
    for (_, v) in table.entries() {
        votes[v] += 1;
    }
    // The most chosen variant (ties -> the lower index); 0 (the witness) when the race chose nothing.
    let common = (1..votes.len()).fold(0usize, |best, i| if votes[i] > votes[best] || (best == 0 && votes[i] > 0) { i } else { best });
    let mut choice = mv::MmChoice::new();
    let (mut from_race, mut filled) = (0usize, 0usize);
    for f in forms {
        let in_race = race.counts.iter().any(|c| (c.m, c.n, c.k, c.flags) == (f.m, f.n, f.k, f.flags));
        let v = if in_race { table.of(f.m, f.n, f.k, f.flags) } else { common };
        if v == 0 {
            continue;
        }
        choice.set(f.m, f.n, f.k, f.flags, v)?;
        if in_race {
            from_race += 1;
        } else {
            filled += 1;
        }
    }
    Ok((choice, from_race, filled, common, race.saved_ns(&table)))
}

/// P6-172: the pieces of the arena that are not zero before the first step, written into one upload piece
/// `[cva, cva + buf.len())` of the arena: the weights (tensor by tensor, straight from `params`, so the GiB of the
/// weights are never copied into one buffer — the kernel heap is 1 GiB), then the small `pieces` (the batch, the
/// segment lists, the result words). The gradient, m and v stay zero (`buf` comes in zeroed).
fn fill_arena_piece(buf: &mut [u8], cva: u64, w_vas: &[u64], params: &[Vec<f32>], pieces: &[(u64, Vec<u8>)]) {
    let end = cva + buf.len() as u64;
    for (&va, t) in w_vas.iter().zip(params) {
        let tend = va + 4 * t.len() as u64;
        if tend <= cva || va >= end {
            continue;
        }
        let first = (cva.saturating_sub(va) / 4) as usize;
        let last = (((end.min(tend)) - va) / 4) as usize;
        for (i, x) in t[first..last].iter().enumerate() {
            let at = (va + 4 * (first + i) as u64 - cva) as usize;
            buf[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    for (va, bytes) in pieces {
        let pend = va + bytes.len() as u64;
        if pend <= cva || *va >= end {
            continue;
        }
        let (from, to) = (cva.max(*va), end.min(pend));
        buf[(from - cva) as usize..(to - cva) as usize].copy_from_slice(&bytes[(from - va) as usize..(to - va) as usize]);
    }
}

/// P6-190 (C-165, cyber-gpu note `C-165_cursa-mm.md`): one shape of the race — the step's first `mm_train` /
/// `mm_train_tile` launch of that (M, N, K, flags), its A, B and bias addresses (real arena data after the steps), and
/// how many launches of the step have it.
#[derive(Clone, Copy)]
struct RaceForm {
    m: u32,
    n: u32,
    k: u32,
    flags: u32,
    count: u32,
    a: u64,
    b: u64,
    bias: u64,
}

/// P6-190: the race's shapes from one step's list, in the order of `mm_train_variants::forms`.
fn race_forms(list: &[nvidia_ga104::train_step::Launch]) -> Vec<RaceForm> {
    use nvidia_ga104::train_step::Prog as TP;
    let u32_at = |cb: &[u8; 0x100], o: usize| u32::from_le_bytes([cb[o], cb[o + 1], cb[o + 2], cb[o + 3]]);
    let u64_at = |cb: &[u8; 0x100], o: usize| u64::from(u32_at(cb, o)) | (u64::from(u32_at(cb, o + 4)) << 32);
    nvidia_ga104::mm_train_variants::forms(list)
        .iter()
        .filter_map(|f| {
            let l = list.iter().find(|l| {
                matches!(l.prog, TP::MmTrain | TP::MmTrainTile)
                    && (u32_at(&l.cbuf, 0x48), u32_at(&l.cbuf, 0x4c), u32_at(&l.cbuf, 0x50), u32_at(&l.cbuf, 0x54)) == (f.m, f.n, f.k, f.flags)
            })?;
            Some(RaceForm { m: f.m, n: f.n, k: f.k, flags: f.flags, count: f.count, a: u64_at(&l.cbuf, 0x28), b: u64_at(&l.cbuf, 0x30), bias: u64_at(&l.cbuf, 0x40) })
        })
        .collect()
}

/// P6-190: what one variant gave on one shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RaceBits {
    /// C (and the guard after it) the same bits as the witness's C.
    Same,
    /// Another C, or a guard word changed: never used in the step.
    Differ,
    /// The board did not finish the launches (the race stops there).
    Unfinished,
    /// Not run: its SHA-256 is not the note's, or C does not fit the scratch piece.
    Skipped,
}

impl RaceBits {
    fn text(self) -> &'static str {
        match self {
            RaceBits::Same => "=",
            RaceBits::Differ => "DIFERA",
            RaceBits::Unfinished => "NETERMINAT",
            RaceBits::Skipped => "SARIT",
        }
    }
}

/// P6-190: FNV-1a over bytes, 64 bits (the witness's C against each variant's, piece by piece).
fn fnv64(mut h: u64, b: &[u8]) -> u64 {
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// P6-190: one race row of the report: `cursa: varianta · forma · us median · TFLOP/s · biti`.
fn race_row(name: &str, f: &RaceForm, ns: Option<u64>, bits: RaceBits) -> String {
    let flops = 2 * u64::from(f.m) * u64::from(f.n) * u64::from(f.k);
    let (us, tf) = match ns {
        Some(n) if n > 0 => (format!("{}.{} us", n / 1000, (n % 1000) / 100), format!("{}.{:02} TFLOP/s", flops / n / 1000, (flops * 100 / n / 1000) % 100)),
        _ => (String::from("- us"), String::from("- TFLOP/s")),
    };
    format!("cursa: {} | {}x{}x{} f{} (x{} in pas) | {} median | {} | biti {}", name, f.m, f.n, f.k, f.flags, f.count, us, tf, bits.text())
}

/// P6-172 (command-center note of 23 Nov 23:36, point 3; plan `docs/plans/P6-172.md`): the training step of Kronos #513
/// (`train::M513`: 12 layers, E 768, 12 heads, context 512, vocabulary 16 384) done by the board's own list with the
/// resident arena in VRAM, for the speed comparison with Windows / PyTorch. The same road as `--antrenare-mini --vram`
/// (os-386: the plan made once, the arena in VRAM, per step only the batch up and the losses + norm down), with what the
/// large shape needs: (1) the arena (GiB) is uploaded in pieces of 64 MiB through one pinned host run (no GiB of
/// contiguous RAM below 4 GiB) — each piece filled from the start weights and the batch, then one launch of the copy
/// program; (2) a step longer than one submission's records (STRIDE) goes out in parts, each waited for before the next
/// (the board runs them in order; the board time is their sum). The CPU cannot do #513's step in useful time, so on the
/// board there is no oracle: the verdict is the loss at step 1 ≈ ln 16 384 (start weights ±0.0346), every loss and norm
/// finite, the last loss below the first, and every weight finite at the end. In SIMULARE the same code runs on the
/// small model (the board simulator is too slow for #513) with pieces of 1 MiB and parts of 32 launches, beside the
/// oracle (≤ 1e-3) and with the pieces checked byte for byte against the plan's own uploads. New for the board: only the
/// large shape (the GiB arena, the parts); no new program, no new register.
#[allow(clippy::too_many_arguments)]
/// P6-215: the shared-memory step of every launch in #513's fast mixed step (the note C-157_P2zf: `Some(64 << 10)`).
const MIXED_FAST_STEP: u32 = 64 << 10;

fn train_513_resident<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<bool> {
    use crate::shell::antrenare_mini as am;
    use gpu_compute::layer::EmbSegments;
    use gpu_compute::train::{self, AdamW, Batch, Recipe};
    use nvidia_ga104::train_step::{self as ts, Outcome, Plan};
    const TOL: f64 = 1e-3;
    // P6-198: the mixed step's tolerances against the F32 oracle (loss, norm), as `--antrenare-mini --mixt` (C-160_P7f).
    const TOL_MIXED: (f64, f64) = (1e-3, 2e-3);
    // P6-208 (DECIZII 25 Nov 11:52 + 11:55): no fixed ceiling any more (it was 6 GiB, the Windows rule of Kronos); the
    // arena takes what fits in the largest free VRAM piece after the driver's start, keeping at least this much free.
    const ARENA_RESERVE: u64 = 256 << 20;
    let board = permit.is_some();
    let rec_cap = ((ctx.rec.size / 16) as usize).saturating_sub(1).min(STRIDE as usize - 1);
    let (a, mut bsz, tl, steps, piece, split) = if board {
        (train::M513, 2usize, 512usize, 10usize, 64u64 << 20, rec_cap)
    } else {
        (am::arch(am::VOCAB, 32), 2usize, 32usize, 2usize, 1u64 << 20, 32usize.min(rec_cap))
    };
    let label = if board { "placa" } else { "SIMULARE" };
    let tag = if board { "antrenare #513 pe placa (arena in VRAM)" } else { "antrenare #513 pe placa (arena in VRAM; SIMULARE pe modelul mic)" };
    journal.push(Event::Value { what: KT_513_PASS, value: 0 });
    if split == 0 {
        return Err(bad("antrenare-513: zona inregistrarilor prea mica"));
    }
    // P6-208 (centre note 25 Nov 14:40, point 2): the batch asked by `--lot-513 N`; before anything is sent, the arena
    // of N (the same plan options, a zero batch: the resident plan's addresses do not depend on the tokens) is checked
    // against the largest free VRAM piece; if it does not fit, back to the old batch (2) with the reason in the report.
    {
        use core::sync::atomic::Ordering::SeqCst;
        let want = super::gpu_tur::TRAIN_513_BATCH.load(SeqCst);
        if want != 0 && want != bsz {
            let t3 = super::gpu_tur::TRAIN_513_TILES3_IN_CYCLE.load(SeqCst);
            let mt = t3 && super::gpu_tur::TRAIN_513_MM_TILE_IN_CYCLE.load(SeqCst);
            let cs = mt && super::gpu_tur::TRAIN_513_COLSUM_SPLIT_IN_CYCLE.load(SeqCst);
            let need = |b: usize| -> GpuResult<(u64, u64)> {
                let x = alloc::vec![0u32; b * tl];
                let base = nvidia_ga104::mmu::VA_BASE;
                let pl = if t3 { Plan::new_tiles3(a, b, tl, &x, base)? } else { Plan::new(a, b, tl, &x, base)? };
                let pl = if mt { pl.with_mm_tile() } else { pl };
                let pl = if cs { pl.with_colsum_split() } else { pl };
                let r = pl.resident()?;
                let n = r.launches_per_step(Recipe::AdamW513);
                Ok((r.bytes().next_multiple_of(PAGE), (n as u64 * ts::SLOT_TC.max(ts::SLOT)).next_multiple_of(PAGE)))
            };
            let (free, largest) = st.mem.vram_free();
            let pick = need(want).map(|(arena, zone)| (arena, zone, batch_fits(arena, zone, ARENA_RESERVE, largest)));
            match pick {
                Ok((arena, zone, true)) => {
                    lines.push(format!(
                        "{}: kronos --gpu: {}: lot {} x {} (--lot-513 {}): arena {} MiB + zona lansarilor {} KiB + rezerva {} MiB incap in cea mai lunga bucata libera de VRAM ({} MiB; VRAM liber {} MiB)",
                        p, tag, want, tl, want, arena >> 20, zone >> 10, ARENA_RESERVE >> 20, largest >> 20, free >> 20
                    ));
                    bsz = want;
                }
                Ok((arena, zone, false)) => lines.push(format!(
                    "{}: kronos --gpu: {}: lot {} x {} (--lot-513 {}) NU INCAPE: arena {} MiB + zona {} KiB + rezerva {} MiB > cea mai lunga bucata libera {} MiB (VRAM liber {} MiB); revin la lotul vechi {} x {}",
                    p, tag, want, tl, want, arena >> 20, zone >> 10, ARENA_RESERVE >> 20, largest >> 20, free >> 20, bsz, tl
                )),
                Err(e) => lines.push(format!(
                    "{}: kronos --gpu: {}: lot {} x {} (--lot-513 {}): planul nu s-a putut face ({:?}); revin la lotul vechi {} x {}",
                    p, tag, want, tl, want, e, bsz, tl
                )),
            }
        }
        journal.push(Event::Value { what: KT_513_BATCH, value: bsz as u32 });
    }
    // P6-261 (C-168 p.2): with `--date-reale`, #513's own start weights from the stick and the batches of oracle S141
    // (seed 1134, positions checked against `loturi.i64`), AdamW lr 5e-5; without them the step does not run.
    let real = super::gpu_tur::TRAIN_513_REAL_DATA_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let lr = if real { am::REAL_LR } else { 1e-3 };
    let mut real_w: Option<Vec<Vec<f32>>> = None;
    let data = if real {
        let self_ok = am::real_positions_self_test();
        lines.push(format!(
            "{}: kronos --gpu: {}: --date-reale (C-168 p.2, P6-261): tragerea S141 (samanta 1134, lot 2 si 6, pasii 1 si 10 = loturi.i64) verificata fara date: {}",
            p, tag, if self_ok { "MERGE" } else { "GRESIT" }
        ));
        let got = if self_ok {
            am::real_weights_513(&a).and_then(|(w, line)| am::real_batches_513(bsz, tl, steps).map(|d| (w, line, d)))
        } else {
            Err(String::from("--date-reale: generatorul nu trage pozitiile S141"))
        };
        match got {
            Ok((w, line, d)) => {
                lines.push(format!("{}: kronos --gpu: {}: {}; AdamW lr 5e-5 (reteta S141)", p, tag, line));
                real_w = Some(w);
                d
            }
            Err(e) => {
                lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: {}; nimic trimis", p, tag, e));
                return Ok(false);
            }
        }
    } else {
        match am::board_batches(bsz, tl, steps) {
            Ok(d) => d,
            Err(e) => {
                lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: {}; nimic trimis", p, tag, e));
                return Err(bad("antrenare-513: datele"));
            }
        }
    };
    if ts::check_programs().is_err() {
        lines.push(format!("{}: kronos --gpu: {}: SHA-256 al unui program nu e cel din nota: nimic trimis", p, tag));
        return Err(bad("antrenare-513: SHA-256 gresit"));
    }
    let (x0, y0) = data.batches.first().ok_or(bad("antrenare-513: niciun pas"))?;
    // P6-185 (cyber-gpu note C-157_P2v): with `--placi3`, the attention on the third-generation tiles (`attn_fwd_tile3`,
    // `attn_back_tile0_3`, `attn_back_tile_3`; on the board since os-375); the same arena, the same number of launches.
    let tiles3 = super::gpu_tur::TRAIN_513_TILES3_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-188 (cyber-gpu note C-157_P2x step 2): with `--mm-placi` (only after `--placi3`), the 147 `mm_train` launches on
    // `mm_train_tile` (the same cbuf; the same bits in SIMULARE); the same arena, the same launches in the same order.
    let mm_tile = tiles3 && super::gpu_tur::TRAIN_513_MM_TILE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-189 (cyber-gpu note C-157_P2y): with `--colsum-impartit` (only after `--mm-placi`), each `colsum` split over rows
    // (`colsum_part` + `colsum` over the partial sums): 385 -> 483 launches, a work zone after the arena counted by
    // `Plan::bytes`; not the same bits, judged against the oracle as before.
    let colsum_split = mm_tile && super::gpu_tur::TRAIN_513_COLSUM_SPLIT_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-193 (cyber-gpu note C-165_mm-tile-gelu): with `--lipit-placi` (only after `--mm-placi`), the step glued
    // (`ln_back_res`, GELU' written by "proj-dX") with the glued products on `mm_train_tile_gelu`: 385 -> 349 launches,
    // the same arena; the same bits as glued without tiles (SIMULARE), judged against the oracle as before.
    let fused_tile = mm_tile && super::gpu_tur::TRAIN_513_FUSED_TILE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-196 (C-165, os-416 race; cyber-gpu note C-165_cursa-mm steps 2-3): with `--mm-alegere` (only after `--mm-placi`
    // or `--colsum-impartit`, not with `--lipit-placi`), every `mm_train` shape of the step on the variant the os-416 race
    // picked (`Plan::with_mm_choice`): the same arena, the same launches; only the programs of the chosen shapes change.
    let mm_choice_on = mm_tile && !fused_tile && super::gpu_tur::TRAIN_513_MM_CHOICE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-213 (cyber-gpu `8fe7637`, note C-165_valuri-alegerea-64k, way 2): with `--mm-model` (only right after
    // `--treapta-pas`), the choice comes from the waves model at 64 KiB (`wave_model::predicted_choice`) instead of the
    // os-416 race: `mm_train_tile`, `mm_tv_64x64k8` or `mm_tv_128x128` per shape; the same programs, a new mix.
    let mm_model_on = mm_choice_on
        && super::gpu_tur::TRAIN_513_SM_STEP_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_513_MM_MODEL_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let mut model_ns: Option<u64> = None;
    let mm_choice = if mm_choice_on {
        // The step's shapes, from a probe of the `with_mm_tile` plan (the choice keeps the shapes and the order).
        let pl = if tiles3 { Plan::new_tiles3(a, bsz, tl, x0, nvidia_ga104::mmu::VA_BASE)? } else { Plan::new(a, bsz, tl, x0, nvidia_ga104::mmu::VA_BASE)? };
        let adam0 = AdamW { lr, beta1: 0.9, beta2: 0.999, eps: 1e-8, weight_decay: 0.1, t: 0, m: Vec::new(), v: Vec::new() };
        let list = pl.with_mm_tile().resident()?.step(Recipe::AdamW513, &adam0, 1)?;
        let forms = nvidia_ga104::mm_train_variants::forms(&list);
        if mm_model_on {
            let (c, ns) = nvidia_ga104::wave_model::predicted_choice(&forms, MM_MODEL_STEP)?;
            model_ns = Some(ns);
            let set = c.entries().filter(|(_, v)| *v != 0).count();
            Some((c, set, 0, 0, 0))
        } else {
            Some(mm_choice_513(&forms)?)
        }
    } else {
        None
    };
    let choice = mm_choice.as_ref().map(|(c, ..)| *c);
    // P6-266 (`--pas-rezidual-lipit`, cyber-gpu `e127322`, note C-168_P5a): the fast mixed step (`--pas-v8`) planned with
    // `with_fused_back` too: per layer `ln_back_res` (the os-415 program, in `check_programs`) instead of `ln_back` +
    // `vadd` at "ln2" and "ln1", 24 launches fewer at #513. Only on the mixed fast path (not with `--lipit-placi`,
    // `--mm-alegere` or the race); `v8_maps` is checked once it exists.
    let fused_fast = mm_tile
        && !fused_tile
        && !mm_choice_on
        && !super::gpu_tur::TRAIN_513_RACE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_513_MIXED_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_513_MM_V8_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_513_FUSED_BACK_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-268 (`--pas-gelu-lipit`, cyber-gpu `ea9bfd9`, note C-168_P5b): over P5a, `with_fused_gelu` too: "proj-dX" writes
    // GELU's backward itself (`MixedLaunch::TcGelu`), on the GELU' variant of the fast program it would take anyway.
    let gelu_fast_on = fused_fast && super::gpu_tur::TRAIN_513_FUSED_GELU_FAST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let build = |base: u64| -> GpuResult<Plan> {
        let pl = if tiles3 { Plan::new_tiles3(a, bsz, tl, x0, base)? } else { Plan::new(a, bsz, tl, x0, base)? };
        let pl = if fused_tile {
            pl.with_fused_back().with_fused_gelu().with_mm_tile_gelu()
        } else if let Some(c) = choice {
            pl.with_mm_choice(c)
        } else if mm_tile {
            pl.with_mm_tile()
        } else {
            pl
        };
        let pl = if colsum_split { pl.with_colsum_split() } else { pl };
        // P6-266: the order of cyber-gpu T306 (`new_tiles3` + `with_mm_tile` + `with_colsum_split` + `with_fused_back`).
        let pl = if fused_fast { pl.with_fused_back() } else { pl };
        // P6-268: the order of cyber-gpu T307 (P5a's plan + `with_fused_gelu`).
        Ok(if gelu_fast_on { pl.with_fused_gelu() } else { pl })
    };
    journal.push(Event::Value { what: KT_513_TILES3, value: tiles3 as u32 });
    journal.push(Event::Value { what: KT_513_FUSED_BACK_FAST, value: fused_fast as u32 });
    journal.push(Event::Value { what: KT_513_FUSED_GELU_FAST, value: gelu_fast_on as u32 });
    journal.push(Event::Value { what: KT_513_MM_TILE, value: mm_tile as u32 });
    journal.push(Event::Value { what: KT_513_COLSUM_SPLIT, value: colsum_split as u32 });
    journal.push(Event::Value { what: KT_513_FUSED_TILE, value: fused_tile as u32 });
    journal.push(Event::Value { what: KT_513_MM_CHOICE, value: mm_choice_on as u32 });
    journal.push(Event::Value { what: KT_513_MM_MODEL, value: mm_model_on as u32 });
    // P6-205 (cyber-gpu `8b6a72d`, note C-165_treapta-pe-pas; os-423's occupancy probe gave H1): with `--treapta-pas`
    // (only after `--mm-alegere`), every launch of the step packed with one SM shared-memory step
    // (`step_best_sm_step` -> `pack_with_sm_step`: MIN = TARGET = the step, MAX = 64 KiB); nothing else changes.
    let sm_step_on = mm_choice_on && super::gpu_tur::TRAIN_513_SM_STEP_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    journal.push(Event::Value { what: KT_513_SM_STEP, value: sm_step_on as u32 });
    // P6-190 (C-165, cyber-gpu note `C-165_cursa-mm.md`): with `--cursa-mm` (only after `--mm-placi`), after the steps,
    // every variant of `mm_train_variants::VARIANTS` on every `mm_train` shape of the step, against the witness.
    let race = mm_tile && super::gpu_tur::TRAIN_513_RACE_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-195 (cyber-gpu note `C-165_os413_64x64k8.md`, proposal 3): with `--proba-64k8` (only after `--cursa-mm`), instead
    // of the big race only the witness and `mm_tv_64x64k8` (held since os-413) on the qkv shape, with the tail page.
    let probe64 = race && super::gpu_tur::TRAIN_513_PROBE_64K8_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let probe64_vi = nvidia_ga104::mm_train_variants::VARIANTS.iter().position(|v| v.name == PROBE_64K8_NAME);
    // P6-201 (cyber-gpu note `C-165_proba-ocuparii.md`): with `--ocupare` (only after `--cursa-mm`), instead of the big
    // race the occupancy probe on the qkv shape: `occupancy::PROBE`'s 10 runs, each with the QMD's SM shared-memory step
    // chosen (`to_bytes_with_sm_config`), the bits against the first run (today's launch).
    let occ = race && !probe64 && super::gpu_tur::TRAIN_513_OCCUPANCY_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    // P6-198 (cyber-gpu note C-165_pas-mixt-513): with `--mixt-513` (only after `--mm-placi` or `--colsum-impartit`, last),
    // the mixed step: every product of the model on `mm_tc` (f16 inputs, f32 sums), the loss scale S = 2^14
    // (`LOSS_SCALE_513`), 1/S written once at `res + 16`; `res[0]` is then S times the norm. The same arena.
    let mixed = mm_tile && !fused_tile && !mm_choice_on && !race && super::gpu_tur::TRAIN_513_MIXED_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    let scale = ts::LOSS_SCALE_513;
    journal.push(Event::Value { what: KT_513_MIXED, value: mixed as u32 });
    // P6-216 (cyber-gpu `1ddf092`, note C-157_P2zg step 1): with `--lot-prin-ram` (only after `--mixt-513-rapid`), the
    // batch goes into pinned RAM (the start of `stage`, unused between the arena's big copies) and the losses + norm come
    // back there: `batch_ram::pack_mixed_fast_via_ram` = 5 copies in + the fast mixed list + 2 copies out, one submission
    // order, all at the 64 KiB step; nothing is written or read through the VRAM window during the steps.
    let via_ram_on = mixed
        && super::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst)
        && super::gpu_tur::TRAIN_513_VIA_RAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    journal.push(Event::Value { what: KT_513_VIA_RAM, value: via_ram_on as u32 });
    // P6-217 (note C-157_P2zg step 2): with `--doua-locuri` (only after `--lot-prin-ram`), a second batch slot at the end
    // of the arena (`Plan::with_batch_slots`), and per slot its own half of the staging piece, launch zone and records.
    let slots_on = via_ram_on && super::gpu_tur::TRAIN_513_SLOTS_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    journal.push(Event::Value { what: KT_513_SLOTS, value: slots_on as u32 });
    let n_slots: u64 = if slots_on { 2 } else { 1 };
    let stage_cap = piece / n_slots;
    let stage_off = |slot: usize| slot as u64 * stage_cap;
    let resident = |pl: Plan| -> GpuResult<Plan> {
        let r = pl.resident()?;
        if slots_on { r.with_batch_slots() } else { Ok(r) }
    };
    let probe0 = resident(build(nvidia_ga104::mmu::VA_BASE)?)?;
    let mut n_launch = 0usize;
    for (x, _) in &data.batches {
        let pr = probe0.rebatch(x)?;
        n_launch = n_launch.max(if mixed { pr.launches_per_step_mixed(Recipe::AdamW513) } else { pr.launches_per_step(Recipe::AdamW513) });
    }
    // P6-216: the 7 copies take a slot each in the zone and a record each.
    let mut stage_need = 0u64;
    if via_ram_on {
        let v = nvidia_ga104::batch_ram::via_ram(&probe0, x0, y0, 0)?;
        n_launch += nvidia_ga104::batch_ram::extra_launches(&v);
        stage_need = v.staging.len() as u64;
        if stage_need > stage_cap {
            lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: zona de tranzit a lotului ({} B) nu incape in bucata RAM ({} KiB); nimic trimis", p, tag, stage_need, stage_cap >> 10));
            return Err(bad("antrenare-513: zona de tranzit prea mare"));
        }
    }
    let arena_bytes = probe0.bytes().next_multiple_of(PAGE);
    // P6-198: an `mm_tc` launch takes `SLOT_TC` bytes of the zone (its larger cbuf), the others `SLOT`.
    let zone_bytes = (n_launch as u64 * if mixed { ts::SLOT_TC.max(ts::SLOT) } else { ts::SLOT }).next_multiple_of(PAGE);
    let (free, largest) = st.mem.vram_free();
    if !batch_fits(arena_bytes, zone_bytes, ARENA_RESERVE, largest) {
        lines.push(format!(
            "{}: kronos --gpu: {}: NU A RULAT: arena {} MiB + zona lansarilor {} KiB + rezerva {} MiB nu incap (VRAM liber {} MiB, cea mai lunga bucata {} MiB); nimic trimis",
            p, tag, arena_bytes >> 20, zone_bytes >> 10, ARENA_RESERVE >> 20, free >> 20, largest >> 20
        ));
        return Err(GpuError::OutOfMemory { requested: arena_bytes });
    }
    // P6-208 (DECIZII 25 Nov 11:55): the arena chosen and the free VRAM, in the report.
    lines.push(format!(
        "{}: kronos --gpu: {}: arena aleasa {} MiB (lot {} x {}) + zona lansarilor {} KiB; VRAM liber {} MiB, cea mai lunga bucata {} MiB, rezerva pastrata >= {} MiB (fara plafon fix; DECIZII 25 nov 11:52/11:55)",
        p, tag, arena_bytes >> 20, bsz, tl, zone_bytes >> 10, free >> 20, largest >> 20, ARENA_RESERVE >> 20
    ));
    let probe_programs = probe0.programs();
    let probe_extra = (probe0.fused_programs(), probe0.mm_tile_gelu_programs());
    let probe_choice = (probe0.mm_choice().copied(), probe0.mm_choice_programs().to_vec());
    drop(probe0);
    let varena = map_vram(st, arena_bytes)?;
    let plan0 = resident(build(varena.va)?)?;
    if slots_on && plan0.batch_slots() != 2 {
        return Err(bad("antrenare-513: planul nu are doua locuri de lot"));
    }
    if plan0.bytes() > arena_bytes || plan0.mm_tile() != mm_tile || plan0.colsum_split() != colsum_split || plan0.programs() != probe_programs
        || plan0.fused_back() != (fused_tile || fused_fast) || plan0.fused_gelu() != (fused_tile || gelu_fast_on) || plan0.mm_tile_gelu() != fused_tile
        || (plan0.fused_programs(), plan0.mm_tile_gelu_programs()) != probe_extra
        || (plan0.mm_choice().copied(), plan0.mm_choice_programs().to_vec()) != probe_choice
        || plan0.mm_choice().copied() != choice
    {
        return Err(bad("antrenare-513: planul rezident difera intre adrese"));
    }
    let Some(host) = ram_vram_host((piece + n_slots * zone_bytes) / PAGE) else {
        lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: nu am {} KiB RAM contigua sub 4 GiB; nimic trimis", p, tag, (piece + n_slots * zone_bytes) >> 10));
        return Err(GpuError::OutOfMemory { requested: piece + n_slots * zone_bytes });
    };
    let count: usize = a.tensors().iter().map(|t| t.elements()).sum();
    let n_parts = n_launch.div_ceil(split);
    journal.push(Event::Value { what: KT_513_ARENA_MIB, value: (arena_bytes >> 20) as u32 });
    journal.push(Event::Value { what: KT_513_LAUNCHES, value: n_launch as u32 });
    journal.push(Event::Value { what: KT_513_SUBS, value: n_parts as u32 });
    lines.push(format!(
        "{}: kronos --gpu: {} ({}): modelul vocabular {}, context {}, E {}, {} capete, {} straturi ({} greutati), lot {} x {}, AdamW lr {:e}, taiere 1,0; {}; {}{}{}{}{}; {} programe (SHA-256 verificate), {} lansari pe pas in {} trimiteri (cel mult {} lansari pe trimitere); arena {} MiB in VRAM (VRAM liber {} MiB), urcata o data pe bucati de {} KiB; {} pasi{}",
        p, tag, label, a.vocab, a.ctx, a.n_emb, a.heads, a.layers, count, bsz, tl, lr, data.source,
        if tiles3 { "atentia pe placi gen 3 (attn_fwd_tile3, attn_back_tile0_3, attn_back_tile_3; --placi3)" } else { "atentia de baza (attn_fwd, attn_back)" },
        if mixed { "; inmultirile pe mm_tc (pasul mixt, S = 16384; --mixt-513 in locul lui mm_train_tile de la --mm-placi)" } else if mm_tile { "; inmultirile pe mm_train_tile (blocuri 256 x 64 x 64; --mm-placi)" } else { "" },
        if fused_tile { "; lipit (ln_back_res, GELU' in proj-dX) cu proj-dX+gelu pe mm_train_tile_gelu (--lipit-placi)" } else { "" },
        if mm_choice_on { "; inmultirile pe varianta aleasa de cursa os-416 (Plan::with_mm_choice; --mm-alegere)" } else { "" },
        match plan0.colsum_scratch() {
            Some((_, _, words)) => format!("; colsum impartit pe randuri (colsum_part + colsum; zona de lucru {} KiB dupa arena; --colsum-impartit)", (2 * 4 * words as u64) >> 10),
            None => String::new(),
        },
        plan0.programs().len() + plan0.fused_programs().len() + plan0.mm_tile_programs().len() + plan0.mm_tile_gelu_programs().len() + plan0.mm_choice_programs().len() + plan0.colsum_split_programs().len(), n_launch, n_parts, split, arena_bytes >> 20, free >> 20, piece >> 10, steps,
        if board { "; fara oracol (procesorul nu face pasul #513 in timp util): pierderea la pasul 1 ~ ln 16 384, apoi scade" } else { ", oracolul pe procesor la fiecare pas, greutatile la sfarsit" }
    ));
    let mut progs: Vec<(ts::Prog, Mapping)> = Vec::new();
    // P6-196: the chosen race variants (`mm_choice_programs`) too, each with the tail page like every program (P6-194).
    let choice_progs: Vec<ts::Prog> = plan0.mm_choice_programs().to_vec();
    if let (Some((c, _, _, _, _)), Some(ns)) = (&mm_choice, model_ns) {
        use nvidia_ga104::mm_train_variants::VARIANTS;
        let picks: Vec<String> = c.entries().map(|(f, v)| format!("{}x{}x{} f{} -> {}", f.m, f.n, f.k, f.flags, VARIANTS[v].name)).collect();
        let mut per = [0usize; VARIANTS.len()];
        for (_, v) in c.entries() {
            per[v] += 1;
        }
        journal.push(Event::Value { what: KT_513_MM_CHOICE_FORMS, value: c.entries().filter(|(_, v)| *v != 0).count() as u32 });
        let counts: Vec<String> = (0..VARIANTS.len()).filter(|&i| per[i] > 0).map(|i| format!("{} {}", VARIANTS[i].name, per[i])).collect();
        lines.push(format!(
            "{}: kronos --gpu: {}: alegerea inmultirilor din modelul valurilor (--mm-model; wave_model::predicted_choice la {} KiB, din timpii os-423; DEDUS, nu o cursa): {} forme ({}); programe in plus: {}; inmultirile prezise ~{} us la ceasul cursei, ~{} us la ceasul pasului (x {} / 1000; Cyber-GPU: pasul ~204 ms, 195-215); {}",
            p, tag, MM_MODEL_STEP >> 10, c.entries().count(),
            if counts.is_empty() { String::from("niciuna") } else { counts.join(", ") },
            if choice_progs.is_empty() { String::from("niciunul") } else { choice_progs.iter().map(|q| q.name()).collect::<Vec<_>>().join(", ") },
            ns / 1000, ns * nvidia_ga104::wave_model::STEP_OVER_RACE_PERMILLE / 1_000_000, nvidia_ga104::wave_model::STEP_OVER_RACE_PERMILLE,
            if picks.is_empty() { String::from("nicio forma") } else { picks.join("; ") }
        ));
    } else if let Some((c, from_race, filled, common, saved_ns)) = &mm_choice {
        use nvidia_ga104::mm_train_variants::VARIANTS;
        let picks: Vec<String> = c.entries().map(|(f, v)| format!("{}x{}x{} f{} -> {}", f.m, f.n, f.k, f.flags, VARIANTS[v].name)).collect();
        journal.push(Event::Value { what: KT_513_MM_CHOICE_FORMS, value: (from_race + filled) as u32 });
        lines.push(format!(
            "{}: kronos --gpu: {}: alegerea inmultirilor (--mm-alegere, din cursa os-416 citita cu parse_race; cea mai rapida cu aceiasi biti, la >= 3 % castig): {} forme pe o varianta ({} din cursa, {} fara rand in cursa -> varianta cea mai des aleasa, {}); programe in plus: {}; in cursa, pasul ar fi scazut cu ~{} us (DEDUS, timpi izolati); {}",
            p, tag, from_race + filled, from_race, filled, VARIANTS[*common].name,
            if choice_progs.is_empty() { String::from("niciunul") } else { choice_progs.iter().map(|q| q.name()).collect::<Vec<_>>().join(", ") },
            saved_ns / 1000,
            if picks.is_empty() { String::from("nicio forma schimbata (pasul = --mm-placi)") } else { picks.join("; ") }
        ));
    }
    for &prog in plan0.programs().iter().chain(plan0.fused_programs()).chain(plan0.mm_tile_programs()).chain(plan0.mm_tile_gelu_programs()).chain(choice_progs.iter()).chain(plan0.colsum_split_programs()) {
        // `fused_programs` and `mm_tile_programs` never overlap `programs`, but a program loaded twice would only waste VRAM.
        if progs.iter().any(|(q, _)| *q == prog) {
            continue;
        }
        let pm = map_vram(st, program_map_bytes(prog.code().len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    // P6-266: the packer does not check that `ln_back_res` is loaded (note C-168_P5a): the OS does, before any launch.
    if fused_fast {
        // P6-268: with `--pas-gelu-lipit` also `mm_train_gelu` (note C-168_P5b: never launched in the mixed step, where it
        // becomes `TcGelu`; loaded anyway, harmless).
        let want: &[ts::Prog] = if gelu_fast_on { &[ts::Prog::LnBackRes, ts::Prog::MmTrainGelu] } else { &[ts::Prog::LnBackRes] };
        if plan0.fused_programs() != want || !progs.iter().any(|(q, _)| *q == ts::Prog::LnBackRes) {
            lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: ln_back_res nu e in programele planului sau nu e incarcat (--pas-rezidual-lipit); nimic trimis", p, tag));
            return Err(bad("antrenare-513: ln_back_res lipsa"));
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: reziduurile inapoi lipite in pasul rapid (--pas-rezidual-lipit, nota Cyber-GPU C-168_P5a): Plan::with_fused_back, pe fiecare strat ln_back_res in locul lui ln_back + vadd la ln2 si ln1; ln_back_res incarcat (SHA-256 verificat, acelasi binar ca os-415); {} lansari pe pas (Cyber-GPU, DEDUS: 24 mai putin, 491 -> 467 pe placa; vadd 48 -> 24, ln_back 25 -> 1, ln_back_res 24; oracolul lot 2 ACELEASI NUMERE; castig 0,5-1,1 ms pe pas la lot 6)",
            p, tag, n_launch
        ));
    }
    // P6-198: `mm_tc` beside the step's programs, with the tail page like every program (P6-194); SHA-256 checked.
    let tc_map: Option<Mapping> = if mixed {
        if sha256_hex(nvidia_ga104::mm_tc::MM_TC) != nvidia_ga104::mm_tc::MM_TC_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("antrenare-513: SHA-256 mm_tc gresit"));
        }
        let pm = map_vram(st, program_map_bytes(nvidia_ga104::mm_tc::MM_TC.len() as u64))?;
        put_bytes(st.mem.writer_mut(), pm.pa, nvidia_ga104::mm_tc::MM_TC)?;
        Some(pm)
    } else {
        None
    };
    // P6-215 (`--mixt-513-rapid`): the fast `mm_tc` programs, every launch at the 64 KiB step.
    let fast_map = if mixed && super::gpu_tur::TRAIN_MIXED_FAST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        match map_fast_tc(st) {
            Ok(m) => Some(m),
            Err(e) => {
                lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_pf / mm_tc_v4 nu e cel din nota: nimic trimis", p, tag));
                return Err(e);
            }
        }
    } else {
        None
    };
    // P6-231 (`--atentie-tc16`, notes C-157_P2zm step 2 / C-157_P2zp): dQ, dK, dV on `attn_back_tile_tc16` (SHA-256 first,
    // with the tail page), only with the fast mixed step.
    let attn16_map = if fast_map.is_some() && super::gpu_tur::TRAIN_513_ATTN_TC16_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::attn_back_tile_tc16 as a16;
        if sha256_hex(a16::ATTN_BACK_TILE_TC16) != a16::ATTN_BACK_TILE_TC16_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui attn_back_tile_tc16 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 attn_back_tile_tc16 gresit"));
        }
        let m = map_vram(st, program_map_bytes(a16::ATTN_BACK_TILE_TC16.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, a16::ATTN_BACK_TILE_TC16)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: atentia inapoi pe circuitele f16 (--atentie-tc16, nota Cyber-GPU C-157_P2zm pasul 2): attn_back_tile_tc16 ({} registre, {} octeti partajati, SHA-256 verificat) in locul celor 12 lansari attn_back_tile_3 din pas",
            p, tag, a16::REGISTERS, a16::SHARED.bytes
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_ATTN_TC16, value: u32::from(attn16_map.is_some()) });
    // P6-237 (`--atentie-v4`, note C-157_P2zv, after os-465): the winner of P2zq, `attn_back_tile_tc16_v4`, beside tc16
    // (SHA-256 first, with the tail page); `pack_mixed_fast_attn4` picks it on every aligned launch.
    let attn_v4_map = if attn16_map.is_some() && super::gpu_tur::TRAIN_513_ATTN_V4_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::attn_back_tile_tc16_v4 as av4;
        if sha256_hex(av4::ATTN_BACK_TILE_TC16_V4) != av4::ATTN_BACK_TILE_TC16_V4_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui attn_back_tile_tc16_v4 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 attn_back_tile_tc16_v4 gresit"));
        }
        let m = map_vram(st, program_map_bytes(av4::ATTN_BACK_TILE_TC16_V4.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, av4::ATTN_BACK_TILE_TC16_V4)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: atentia inapoi pe attn_back_tile_tc16_v4 unde forma e aliniata (--atentie-v4, nota Cyber-GPU C-157_P2zv; castigatoarea os-465): {} registre, citiri de 128 de biti, SHA-256 verificat; tc16 pe rest",
            p, tag, av4::REGISTERS
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_ATTN_V4, value: u32::from(attn_v4_map.is_some()) });
    // P6-241 (`--atentie-ds`, note C-157_P2zu; programs repaired in P2zx): the 12 dS launches on `attn_back_tile0_tc_v4`
    // where aligned, `attn_back_tile0_tc` for the rest (SHA-256 first, each with the tail page).
    let attn0_maps = if attn16_map.is_some() && super::gpu_tur::TRAIN_513_ATTN_DS_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::attn_back_tile0_tc as a0;
        use nvidia_ga104::attn_back_tile0_tc_v4 as a4;
        if sha256_hex(a0::ATTN_BACK_TILE0_TC) != a0::ATTN_BACK_TILE0_TC_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui attn_back_tile0_tc nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 attn_back_tile0_tc gresit"));
        }
        if sha256_hex(a4::ATTN_BACK_TILE0_TC_V4) != a4::ATTN_BACK_TILE0_TC_V4_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui attn_back_tile0_tc_v4 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 attn_back_tile0_tc_v4 gresit"));
        }
        let m0 = map_vram(st, program_map_bytes(a0::ATTN_BACK_TILE0_TC.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m0.pa, a0::ATTN_BACK_TILE0_TC)?;
        let m4 = map_vram(st, program_map_bytes(a4::ATTN_BACK_TILE0_TC_V4.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m4.pa, a4::ATTN_BACK_TILE0_TC_V4)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: dS al atentiei pe circuitele f16 (--atentie-ds, nota Cyber-GPU C-157_P2zu): attn_back_tile0_tc_v4 unde forma e aliniata, attn_back_tile0_tc pe rest ({} registre, {} octeti partajati, SHA-256 verificate) in locul celor 12 lansari attn_back_tile0_3 din pas",
            p, tag, a0::REGISTERS, a0::SHARED.bytes
        ));
        Some((m0, m4))
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_ATTN_DS, value: u32::from(attn0_maps.is_some()) });
    // P6-245 (`--pas-v5`, cyber-gpu `34961eb`, note C-157_P2zzf, after os-476: same bits, x1,41 over v4 at 64 KiB):
    // `mm_tc_v5` beside v4 (SHA-256 first, with the tail page); `pack_mixed_fast_attn5` picks it on every aligned product.
    let v5_map = if attn16_map.is_some() && super::gpu_tur::TRAIN_513_MM_V5_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::mm_tc_v5 as m5;
        if sha256_hex(m5::MM_TC_V5) != m5::MM_TC_V5_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_v5 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 mm_tc_v5 gresit"));
        }
        let m = map_vram(st, program_map_bytes(m5::MM_TC_V5.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, m5::MM_TC_V5)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: inmultirile aliniate ale pasului pe mm_tc_v5 (--pas-v5, nota Cyber-GPU C-157_P2zzf; os-476: aceiasi biti, x1,41 fata de v4 la 64 KiB): {} registre, {} octeti partajati, SHA-256 verificat; restul ca inainte (v4 / pf / tc)",
            p, tag, m5::REGISTERS, m5::SHARED.bytes
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_MM_V5, value: u32::from(v5_map.is_some()) });
    // P6-247 (`--pas-v6`, cyber-gpu `1cb1846`, note C-157_P2zzh): `mm_tc_v6` beside v5 (SHA-256 first, with the tail page);
    // `pack_mixed_fast_attn6` picks it before v5 on every aligned product.
    let v6_map = if v5_map.is_some() && super::gpu_tur::TRAIN_513_MM_V6_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::mm_tc_v6 as m6;
        if sha256_hex(m6::MM_TC_V6) != m6::MM_TC_V6_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_v6 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 mm_tc_v6 gresit"));
        }
        let m = map_vram(st, program_map_bytes(m6::MM_TC_V6.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, m6::MM_TC_V6)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: inmultirile aliniate ale pasului intai pe mm_tc_v6 (--pas-v6, nota Cyber-GPU C-157_P2zzh; scrierile f16 imperecheate): {} registre, {} octeti partajati, SHA-256 verificat; mm_tc_v5 langa el, restul ca inainte",
            p, tag, m6::REGISTERS, m6::SHARED.bytes
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_MM_V6, value: u32::from(v6_map.is_some()) });
    // P6-254 (`--pas-v7`, cyber-gpu `3206d58`, note C-157_P2zzr): `mm_tc_v7` beside v5 / v6 (SHA-256 first, with the tail
    // page); `pack_mixed_fast_attn7` picks it before v6 / v5 on every aligned product.
    let v7_map = if v6_map.is_some() && super::gpu_tur::TRAIN_513_MM_V7_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::mm_tc_v7 as m7;
        if sha256_hex(m7::MM_TC_V7) != m7::MM_TC_V7_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_v7 nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 mm_tc_v7 gresit"));
        }
        let m = map_vram(st, program_map_bytes(m7::MM_TC_V7.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, m7::MM_TC_V7)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: inmultirile aliniate ale pasului intai pe mm_tc_v7 (--pas-v7, nota Cyber-GPU C-157_P2zzr; randuri umplute in memoria partajata): {} registre, {} octeti partajati, SHA-256 verificat; mm_tc_v6 si mm_tc_v5 langa el, restul ca inainte",
            p, tag, m7::REGISTERS, m7::SHARED.bytes
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_MM_V7, value: u32::from(v7_map.is_some()) });
    // P6-259 (`--pas-v8`, cyber-gpu `c74237a`, note C-168_P3b): `mm_tc_v8` and `mm_tc_v8n` beside v7 (SHA-256 first, with
    // the tail page); `pack_mixed_fast_attn8` gives every aligned product the program `mm_tc_v8::pick` picks by waves.
    let v8_maps = if v7_map.is_some() && super::gpu_tur::TRAIN_513_MM_V8_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::mm_tc_v8 as m8;
        if sha256_hex(m8::MM_TC_V8) != m8::MM_TC_V8_SHA256 || sha256_hex(m8::MM_TC_V8N) != m8::MM_TC_V8N_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_v8 / mm_tc_v8n nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 mm_tc_v8 / mm_tc_v8n gresit"));
        }
        let a = map_vram(st, program_map_bytes(m8::MM_TC_V8.len() as u64))?;
        put_bytes(st.mem.writer_mut(), a.pa, m8::MM_TC_V8)?;
        let b = map_vram(st, program_map_bytes(m8::MM_TC_V8N.len() as u64))?;
        put_bytes(st.mem.writer_mut(), b.pa, m8::MM_TC_V8N)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: inmultirile aliniate ale pasului pe mm_tc_v7 / mm_tc_v8 / mm_tc_v8n, alese pe forma de mm_tc_v8::pick (--pas-v8, nota Cyber-GPU C-168_P3b / P3c; vitezele masurate in os-500 v8 {} % / v8n {} % fata de v7, {} SM-uri): mm_tc_v8 {} registre {} octeti partajati, mm_tc_v8n {} registre {} octeti partajati, SHA-256 verificate; restul ca inainte",
            p, tag, m8::RATES_OS500.v8_pct, m8::RATES_OS500.v8n_pct, nvidia_ga104::wave_model::SMS, m8::REGISTERS, m8::SHARED.bytes, m8::REGISTERS_N, m8::SHARED_N.bytes
        ));
        Some((a, b))
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_MM_V8, value: u32::from(v8_maps.is_some()) });
    // P6-266: `--pas-rezidual-lipit` only on the step `--pas-v8` built (the cycle sets both; the test build may not).
    if fused_fast && v8_maps.is_none() {
        lines.push(format!("{}: kronos --gpu: {}: NU A RULAT: --pas-rezidual-lipit cere pasul rapid cu --pas-v8; nimic trimis", p, tag));
        return Err(bad("antrenare-513: rezidual lipit fara --pas-v8"));
    }
    // P6-268 (`--pas-gelu-lipit`, cyber-gpu `ea9bfd9`, note C-168_P5b): the three GELU' variants of the fast products
    // (SHA-256 first: `mm_tc_fast_gelu::check` and the sums of the note), each with the tail page like every program.
    let gelu_maps = if gelu_fast_on {
        use nvidia_ga104::mm_tc_fast_gelu as fg;
        const NOTE: [&str; 3] = [
            "ed8082045e2255d96dbcca4f42bb44bb4e199a91e57e1dc03e95d57c756a10c2",
            "d0d061ce7711fdf9f1a9e56ea52162fc84752f23f53987e78907be694af05417",
            "81e6e4ab50145e08dfa04627a0fc46afb17ba331087f547ddb7bd3c3fa259714",
        ];
        let codes = [fg::MM_TC_V7_GELU, fg::MM_TC_V8_GELU, fg::MM_TC_V8N_GELU];
        if fg::check().is_err() || codes.iter().zip(NOTE).any(|(c, h)| sha256_hex(c) != h) {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui mm_tc_v7_gelu / mm_tc_v8_gelu / mm_tc_v8n_gelu nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 mm_tc_*_gelu gresit"));
        }
        let mut m: Vec<Mapping> = Vec::new();
        for c in codes {
            let pm = map_vram(st, program_map_bytes(c.len() as u64))?;
            put_bytes(st.mem.writer_mut(), pm.pa, c)?;
            m.push(pm);
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: GELU' lipit in coada inmultirilor rapide (--pas-gelu-lipit, nota Cyber-GPU C-168_P5b): Plan::with_fused_gelu peste --pas-rezidual-lipit, proj-dX+gelu pe varianta GELU a programului ales (mm_tc_v7_gelu {} / mm_tc_v8_gelu {} / mm_tc_v8n_gelu {} registre), SHA-256 verificate, pack_mixed_fast_attn10; {} lansari pe pas (Cyber-GPU, scris inainte: 12 mai putin, 467 -> 455 pe placa, gelu_tr 24 -> 12; lot 6 12 x mm_tc_v8n_gelu, lot 2 12 x mm_tc_v8_gelu; oracolul lot 2 ACELEASI NUMERE; castig 1,0-1,8 ms pe pas la lot 6, sub 0,5 ms = RATAT)",
            p, tag, fg::REGISTERS_V7, fg::REGISTERS_V8, fg::REGISTERS_V8N, n_launch
        ));
        Some((m[0], m[1], m[2]))
    } else {
        None
    };
    // P6-264 (`--pas-v8-garda`, cyber-gpu `9805b5c`, note C-168_P3e): products with k >= LONG_K_OS503 back on v7.
    let long_k = if v8_maps.is_some() && super::gpu_tur::TRAIN_513_MM_V8_LONG_K_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        lines.push(format!(
            "{}: kronos --gpu: {}: garda sumei lungi (--pas-v8-garda, nota Cyber-GPU C-168_P3e): produsele aliniate cu suma k >= {} inapoi pe mm_tc_v7 (pack_mixed_fast_attn8k; la #513 doar dX-ul capului); restul ca la --pas-v8",
            p, tag, nvidia_ga104::mm_tc_v8::LONG_K_OS503
        ));
        nvidia_ga104::mm_tc_v8::LONG_K_OS503
    } else {
        0
    };
    journal.push(Event::Value { what: KT_513_MM_V8_LONG_K, value: u32::from(long_k != 0) });
    // P6-265 (`--pas-atentie-fwd-tc`, cyber-gpu `9428d34`, note C-168_P4b): `attn_fwd_tc` (the os-504 binary, SHA-256
    // first) on every aligned `attn_fwd_tile3` launch of the step (`attn_fwd_tc_list` + `pack_mixed_fast_attn9`).
    let fwd_map = if v8_maps.is_some() && super::gpu_tur::TRAIN_513_ATTN_FWD_TC_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
        use nvidia_ga104::attn_fwd_tc as af;
        if sha256_hex(af::ATTN_FWD_TC) != af::ATTN_FWD_TC_SHA256 {
            lines.push(format!("{}: kronos --gpu: {}: SHA-256 al lui attn_fwd_tc nu e cel din nota: nimic trimis", p, tag));
            return Err(bad("pasul mixt rapid: SHA-256 attn_fwd_tc gresit"));
        }
        let m = map_vram(st, program_map_bytes(af::ATTN_FWD_TC.len() as u64))?;
        put_bytes(st.mem.writer_mut(), m.pa, af::ATTN_FWD_TC)?;
        lines.push(format!(
            "{}: kronos --gpu: {}: atentia inainte aliniata a pasului pe attn_fwd_tc (--pas-atentie-fwd-tc, nota Cyber-GPU C-168_P4b): {} registre, {} octeti partajati, SHA-256 verificat (acelasi binar ca proba os-504); lansarile nealiniate raman pe attn_fwd_tile3",
            p, tag, af::REGISTERS, af::SHARED.bytes
        ));
        Some(m)
    } else {
        None
    };
    journal.push(Event::Value { what: KT_513_ATTN_FWD_TC, value: u32::from(fwd_map.is_some()) });
    let mut fast_picks = String::new();
    let pages = |first: u64, n: u64| (first..first + n).map(|i| host + i * PAGE).collect::<Vec<u64>>();
    let stage = st.mem.alloc_sys_mapped(&pages(0, piece / PAGE))?;
    let ctl = st.mem.alloc_sys_mapped(&pages(piece / PAGE, zone_bytes / PAGE))?;
    if !stage.va.is_multiple_of(PAGE) || !ctl.va.is_multiple_of(PAGE) {
        return Err(bad("antrenare-513: zona RAM nu e aliniata la 4 KiB"));
    }
    // P6-217: slot 1's launch zone right after slot 0's, and every (slot, part) its own records page(s), so that the
    // next step's parts never touch the records or the zone of the step the board is running.
    let mut ctls: Vec<SysMapping> = alloc::vec![ctl.clone()];
    let mut recs: [Vec<Mapping>; 2] = Default::default();
    if slots_on {
        ctls.push(st.mem.alloc_sys_mapped(&pages((piece + zone_bytes) / PAGE, zone_bytes / PAGE))?);
        for r in recs.iter_mut() {
            for _ in 0..n_parts {
                r.push(map_vram(st, (16 * (split as u64 + 1)).next_multiple_of(PAGE))?);
            }
        }
    }
    // P6-190: the race's scratch C (the largest product the step can have + a 256-byte guard) and its 5 programs, mapped
    // before the step so they are in the submissions' map list.
    let race_c: Option<Mapping> = if race {
        let (r, v, e) = ((bsz * tl) as u64, a.vocab as u64, a.n_emb as u64);
        let most = [r * v, v * e, r * 4 * e, 4 * e * e, r * 3 * e, 3 * e * e, r * e, e * e].into_iter().max().unwrap_or(0);
        Some(map_vram(st, (4 * most + 256).next_multiple_of(PAGE))?)
    } else {
        None
    };
    // P6-194 (os-413): only the variants of the race list, indexed as `VARIANTS`; each with the tail page.
    // P6-199 (os-417): the full list (`race_list_full()`: `RACE_HOLD` brought back by `RACE_CLEARED_OS417`) — the
    // condition Cyber-GPU names for it (every program mapped with `program_map_bytes`) holds here.
    let mut race_progs: Vec<Option<Mapping>> = Vec::new();
    if race {
        for (vi, v) in nvidia_ga104::mm_train_variants::VARIANTS.iter().enumerate() {
            // P6-195: the probe maps only the witness and 64x64k8, in `VARIANTS`' order (64x64k8 last, as at os-413).
            let wanted = if probe64 {
                vi == 0 || Some(vi) == probe64_vi
            } else if occ {
                nvidia_ga104::occupancy::PROBE.iter().any(|r| r.variant == vi)
            } else {
                nvidia_ga104::mm_train_variants::race_list_full().any(|(i, _)| i == vi)
            };
            if !wanted {
                race_progs.push(None);
                continue;
            }
            let pm = map_vram(st, program_map_bytes(v.code.len() as u64))?;
            put_bytes(st.mem.writer_mut(), pm.pa, v.code)?;
            race_progs.push(Some(pm));
        }
    }
    let mut race_shapes: Vec<RaceForm> = Vec::new();
    let mut maps = ctx.maps.clone();
    maps.push(varena);
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    maps.extend(race_c);
    maps.extend(race_progs.iter().flatten().copied());
    maps.extend(tc_map);
    maps.extend(fast_map.iter().flat_map(|(a, b)| [*a, *b]));
    // P6-231: `attn_back_tile_tc16` in the submissions' map list too.
    maps.extend(attn16_map);
    // P6-237: `attn_back_tile_tc16_v4` too.
    maps.extend(attn_v4_map);
    // P6-241: `attn_back_tile0_tc` and `_v4` too.
    maps.extend(attn0_maps.iter().flat_map(|(a, b)| [*a, *b]));
    // P6-245: `mm_tc_v5` too.
    maps.extend(v5_map);
    // P6-247: `mm_tc_v6` too.
    maps.extend(v6_map);
    // P6-254: `mm_tc_v7` too.
    maps.extend(v7_map);
    // P6-259: `mm_tc_v8` / `mm_tc_v8n` too.
    if let Some((a, b)) = v8_maps {
        maps.push(a);
        maps.push(b);
    }
    // P6-265: `attn_fwd_tc` too.
    maps.extend(fwd_map);
    // P6-268: the three GELU' variants too.
    if let Some((a, b, c)) = gelu_maps {
        maps.extend([a, b, c]);
    }
    maps.extend(recs.iter().flatten().copied());
    let mut sys = alloc::vec![ctx.sys.clone(), stage.clone()];
    sys.extend(ctls.iter().cloned());
    let prog_va = |want: ts::Prog| progs.iter().find(|(q, _)| *q == want).map_or(0, |(_, pm)| pm.va);
    // One launch of the copy program, as in `train_mini_resident`.
    let copy = |st: &mut GpuState, vbar: &mut V, clock: &mut K, journal: &mut Journal, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, src: u64, dst: u64, len: u64| -> GpuResult<Option<u64>> {
        let (cb_va, cb_pa) = (ctx.launches.va, ctx.launches.pa);
        {
            let w = st.mem.writer_mut();
            put_bytes(w, cb_pa, &transfer::copy_cbuf(src, dst, len)?)?;
            put_bytes(w, cb_pa + 256, &transfer::copy_qmd(ctx.copy_va, cb_va, len)?.to_bytes()?)?;
            put_bytes(w, ctx.rec.pa, &[0; 32])?;
        }
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[cb_va + 256], ctx.rec.va, first)?;
        let nw = pb.len();
        match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => {
                *subs += 1;
                Ok(Some(transfer::analyze_sequence(&b, 1, first)?[0]))
            }
            Err(_) => Ok(None),
        }
    };
    // 1. The arena, once, in pieces: the start weights, the first batch, the segment lists, the result words.
    let params0 = real_w.take().unwrap_or_else(|| am::initial(&a, 0));
    let w_vas: Vec<u64> = (0..params0.len()).map(|i| plan0.t(plan0.w, i)).collect();
    let u = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut small = plan0.batch_uploads(x0, y0)?;
    let sp = EmbSegments::positions(bsz, tl).ok_or(bad("antrenare-513: pozitii"))?;
    small.push((plan0.seg_pos.off, u(&sp.off)));
    small.push((plan0.seg_pos.dst, u(&sp.dst)));
    small.push((plan0.seg_pos.rows, u(&sp.rows)));
    small.push((plan0.res, [0.0f32, 1.0].iter().flat_map(|x| x.to_le_bytes()).collect()));
    // SIMULARE: the pieces against the plan's own uploads (the whole arena image), byte for byte.
    let reference: Option<Vec<u8>> = if board {
        None
    } else {
        let mut img = alloc::vec![0u8; arena_bytes as usize];
        for (va, bytes) in plan0.uploads(&params0, x0, y0)? {
            let at = (va - varena.va) as usize;
            img[at..at + bytes.len()].copy_from_slice(&bytes);
        }
        Some(img)
    };
    let (mut up_ns, mut n_pieces, mut same_pieces) = (0u64, 0usize, 0usize);
    let mut off = 0u64;
    while off < arena_bytes {
        let len = piece.min(arena_bytes - off);
        let buf = ram_vram_bytes(host, len);
        buf.fill(0);
        fill_arena_piece(buf, varena.va + off, &w_vas, &params0, &small);
        if let Some(img) = &reference {
            if img[off as usize..(off + len) as usize] == buf[..] {
                same_pieces += 1;
            }
        }
        let Some(ns) = copy(st, vbar, clock, journal, l, doorbell, subs, stage.va, varena.va + off, len)? else {
            lines.push(format!("{}: kronos --gpu: {}: urcarea bucatii {} (RAM -> VRAM) nu s-a terminat; proba oprita, fara reincercare", p, tag, n_pieces + 1));
            return Err(GpuError::Timeout { waited_us: WAIT_US });
        };
        up_ns += ns;
        n_pieces += 1;
        off += len;
    }
    drop(reference);
    journal.push(Event::Value { what: KT_513_UP_MS, value: (up_ns / 1_000_000) as u32 });
    lines.push(format!(
        "{}: kronos --gpu: {}: arena urcata o data RAM -> VRAM: {} MiB in {} bucati, {} ms (placa){}",
        p, tag, arena_bytes >> 20, n_pieces, up_ns / 1_000_000,
        if board { String::new() } else { format!("; bucatile fata de urcarile planului: {} / {} identice octet cu octet", same_pieces, n_pieces) }
    ));
    let pieces_ok = board || same_pieces == n_pieces;
    // P6-198: 1/S at `res + 16`, once (the steps' lists never write it; `batch_uploads` does not touch `res`).
    let mut tc_per_step = 0usize;
    if mixed {
        let w = st.mem.writer_mut();
        for (va, bytes) in plan0.mixed_uploads(scale)? {
            put_bytes(w, varena.pa + (va - varena.va), &bytes)?;
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: pasul mixt (--mixt-513, nota Cyber-GPU C-165_pas-mixt-513): toate inmultirile modelului pe mm_tc (intrari f16, sume f32; SHA-256 verificat, mapat cu pagina de paza), scara erorii S = {} (LOSS_SCALE_513), 1/S scris o data la res + 16; norma citita = res[0] / S; m, v si greutatile raman f32",
            p, tag, scale as u32
        ));
    }
    // P6-217: slot 1's `res = [0, 1]` and 1/S, once (slot 0's came with the arena and the mixed uploads above).
    if slots_on {
        let s1 = plan0.slot(1)?;
        let w = st.mem.writer_mut();
        for (va, bytes) in s1.slot_uploads().into_iter().chain(s1.mixed_uploads(scale)?) {
            put_bytes(w, varena.pa + (va - varena.va), &bytes)?;
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: doua locuri de lot (--doua-locuri, nota Cyber-GPU C-157_P2zg pasul 2; Plan::with_batch_slots): locul 2 la capatul arenei (res = [0, 1] si 1/S scrise o data), fiecare loc cu jumatatea lui din bucata RAM pentru tranzit ({} KiB), zona lui de lansari ({} KiB) si inregistrarile lui ({} parti); pasul N+1 pregatit in celalalt loc cat placa lucreaza pasul N, partile lui trimise pe masura ce se termina partile pasului N (in bufferul de comenzi cel mult un pas)",
            p, tag, stage_cap >> 10, zone_bytes >> 10, n_parts
        ));
    }
    if via_ram_on {
        lines.push(format!(
            "{}: kronos --gpu: {}: lotul si rezultatele prin RAM fixata (--lot-prin-ram, nota Cyber-GPU C-157_P2zg pasul 1): zona de tranzit {} B la inceputul bucatii RAM, la fiecare pas (si la pasul 1) lotul scris acolo si adus in arena de 5 copieri, pierderile + norma duse inapoi de 2 copieri, toate in aceeasi trimitere cu pasul, la treapta {} KiB (pack_mixed_fast_via_ram; programul de copiere din os-397); fereastra VRAM neatinsa in timpul pasilor",
            p, tag, stage_need, MIXED_FAST_STEP >> 10
        ));
    }
    let read_vram_f32s = |st: &mut GpuState, vbar: &mut V, journal: &mut Journal, va: u64, n: usize| -> GpuResult<Vec<f32>> {
        let GpuState { zone, .. } = &*st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
        let pa = varena.pa + (va - varena.va);
        let r = (0..n).map(|i| vw.read_u32(pa + 4 * i as u64).map(f32::from_bits)).collect::<GpuResult<Vec<f32>>>();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))
    };
    // The optimizer's numbers only (the step's list does not read the CPU's m, v: they live in the arena) — on the
    // board no GiB-sized m, v on the heap.
    let adam = AdamW { lr, beta1: 0.9, beta2: 0.999, eps: 1e-8, weight_decay: 0.1, t: 0, m: Vec::new(), v: Vec::new() };
    let mut oracle = (!board).then(|| (params0.clone(), AdamW::new(&params0, lr, 0.9, 0.999, 1e-8, 0.1)));
    // P6-205: the step's shared-memory step, picked once on step 1's list (every step has the same programs; checked
    // again on every step's list before it is packed).
    let sm_step: Option<u32> = if sm_step_on {
        let l1 = plan0.step(Recipe::AdamW513, &adam, 1)?;
        let s = ts::step_best_sm_step(&l1, nvidia_ga104::occupancy::Reserve::OneKib);
        journal.push(Event::Value { what: KT_513_SM_STEP_KIB, value: s >> 10 });
        // P6-213: the waves model judged the shapes at 64 KiB; say so when the step's list picks another step.
        if mm_model_on && s != MM_MODEL_STEP {
            lines.push(format!(
                "{}: kronos --gpu: {}: ATENTIE: --mm-model a ales variantele la {} KiB, dar treapta pasului e {} KiB (alegerea poate sa nu fie cea mai buna; bitii nu se schimba)",
                p, tag, MM_MODEL_STEP >> 10, s >> 10
            ));
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: treapta pas: {} KiB pe {} lansari (--treapta-pas; step_best_sm_step peste {} programe, rezerva 1 KiB; MIN = TARGET = {} KiB, MAX = 64 KiB la fiecare lansare a pasului; restul QMD-urilor = pack)",
            p, tag, s >> 10, l1.len(), ts::step_kernels(&l1).len(), s >> 10
        ));
        Some(s)
    } else {
        None
    };
    drop(params0);
    let (mut worst_loss, mut worst_norm) = (0.0f64, 0.0f64);
    let (mut board_ns, mut host_us, mut losses, mut norms): (Vec<u64>, Vec<u64>, Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut stopped: Option<String> = None;
    // P6-182 (os-395 report, Cyber-GPU's ask: "the time per kind of program for #513's step"): the P6-179 profile on
    // #513's list, every step (each part's records are read whole).
    let mut prof: Option<nvidia_ga104::train_profile::StepProfile> = None;
    // P6-226: (profile launches, QMDs) when the mixed step's profile did not match its list.
    let mut mixed_prof_skip: Option<(usize, usize)> = None;
    // P6-192 (A4): the dashboard readings taken during the steps.
    let mut step_dash: Vec<super::gpu_golden::StepSample> = Vec::new();
    // P6-216: the pieces of the step outside the board, per step (µs): batch, list, submissions - board, results.
    let mut outside: [Vec<u64>; 4] = Default::default();
    journal.flush_live();
    let hold = super::gpu_i6::GenHold::start();
    journal.set_routine(Some(kf::gen_routine(&GEN_NOTES)));
    // P6-249 (Cyber-GPU P2zzj, os-481): the dashboard values after each step are held with the routine's writes
    // (Flush::Later, same max_pending) instead of one stick write each; flush_live / leaving the routine writes them.
    // The thermal watch decides on the values read, not on the ones written, so it is unchanged.
    journal.set_hold_values(true);
    // P6-217: one step made ready on the CPU — its batch in place (staging area or VRAM window), its list packed into its
    // slot's launch zone. Nothing here touches the other slot, so it can run while the board works on that one.
    struct Prep {
        slot: usize,
        plan: Plan,
        via: Option<nvidia_ga104::batch_ram::ViaRam>,
        qmds: Vec<u64>,
        list: Option<Vec<ts::Launch>>,
        tc: Option<usize>,
        picks: Option<String>,
        t0: Option<u64>,
        t_batch: Option<u64>,
        t_list: Option<u64>,
        // P6-226 (PREDARE Cyber-GPU cab6742, note C-157_P2zl): the mixed step's profile names (step 1 only).
        mprof: Option<nvidia_ga104::train_profile::StepProfile>,
    }
    let prepare = |st: &mut GpuState, k: usize| -> GpuResult<Prep> {
        let (x, y) = data.batches.get(k).ok_or(bad("antrenare-513: pasul nu exista"))?;
        let step = k as u32 + 1;
        let slot = if slots_on { k % 2 } else { 0 };
        let ctl = &ctls[slot];
        let t0 = crate::time::since_boot_us();
        let base = if slots_on { plan0.slot(slot)? } else { plan0.clone() };
        let plan = base.rebatch(x)?;
        if plan.w != base.w || plan.m != base.m || plan.v != base.v || plan.loss_rows != base.loss_rows || plan.res != base.res {
            return Err(bad("antrenare-513: adresele planului rezident s-au mutat"));
        }
        // P6-216: through RAM every step (step 1 too, so all steps are alike); otherwise through the VRAM window.
        let via = if via_ram_on { Some(nvidia_ga104::batch_ram::via_ram(&plan, x, y, stage.va + stage_off(slot))?) } else { None };
        if let Some(v) = &via {
            if v.staging.len() as u64 > stage_cap {
                return Err(bad("antrenare-513: zona de tranzit prea mare"));
            }
            // P6-217: only this slot's half of the piece; the board reads the other half (the other slot) meanwhile.
            ram_vram_bytes(host + stage_off(slot), v.staging.len() as u64).copy_from_slice(&v.staging);
        } else if k > 0 {
            let w = st.mem.writer_mut();
            for (va, bytes) in plan.batch_uploads(x, y)? {
                put_bytes(w, varena.pa + (va - varena.va), &bytes)?;
            }
        }
        let t_batch = crate::time::since_boot_us();
        let (mut tc, mut picks, mut mprof) = (None, None, None);
        let (zb, qmds, list) = match tc_map {
            // P6-198: the mixed list (the 1x1x1 c/S product included) and its zone. P6-226: its profile through
            // `StepProfile::from_mixed` (each `mm_tc` named after the program really chosen; the RAM copies around it).
            Some(tcm) => {
                let ml = plan.step_mixed(Recipe::AdamW513, &adam, step, scale)?;
                // P6-231: the 12 `attn_back_tile_3` launches onto `MixedLaunch::AttnTc` (nothing else changes).
                let ml = if attn16_map.is_some() { ts::attn_tc_list(ml)? } else { ml };
                // P6-241: and the 12 `attn_back_tile0_3` (dS) launches onto `MixedLaunch::Attn0Tc`.
                let ml = if attn0_maps.is_some() { ts::attn0_tc_list(ml)? } else { ml };
                // P6-265: and the aligned `attn_fwd_tile3` launches onto `MixedLaunch::AttnFwdTc`.
                let ml = if fwd_map.is_some() { ts::attn_fwd_tc_list(ml)? } else { ml };
                tc = Some(ml.iter().filter(|x| matches!(x, ts::MixedLaunch::Tc { .. })).count());
                let (zb, qmds, pk) = match fast_map {
                    Some((pf, v4)) => {
                        let progs = ts::TcPrograms { tc: tcm.va, pf: pf.va, v4: v4.va, gelu: 0 };
                        let attn = attn16_map.as_ref().map(|m| ts::AttnTcProgram::Tc16(m.va));
                        // P6-237: with `--atentie-v4`, the v4 winner on the aligned launches (`pack_mixed_fast_attn4`).
                        let fast = attn_v4_map.as_ref().map(|m| ts::AttnTcFast::V4(m.va));
                        // P6-241: with `--atentie-ds`, both dS programs (v4 picked on the aligned launches).
                        let attn0 = attn0_maps.as_ref().map_or(ts::Attn0Programs { tc: 0, v4: 0 }, |(m0, m4)| ts::Attn0Programs { tc: m0.va, v4: m4.va });
                        let with4 = fast.is_some() || attn0_maps.is_some();
                        // P6-245: with `--pas-v5`, `mm_tc_v5` on every aligned product (`pack_mixed_fast_attn5`).
                        let v5va = v5_map.as_ref().map_or(0, |m| m.va);
                        // P6-247: with `--pas-v6`, `mm_tc_v6` first (`pack_mixed_fast_attn6`, v5 beside it).
                        let v6va = v6_map.as_ref().map_or(0, |m| m.va);
                        // P6-254: with `--pas-v7`, `mm_tc_v7` first (`pack_mixed_fast_attn7`, v6 / v5 beside it).
                        let v7va = v7_map.as_ref().map_or(0, |m| m.va);
                        // P6-259: with `--pas-v8`, v7 / v8 / v8n picked by shape (`pack_mixed_fast_attn8`); P6-260: the rates measured in os-500.
                        let v8p = v8_maps.as_ref().map(|(a, b)| ts::V8Programs { v8: a.va, v8n: b.va, sms: nvidia_ga104::wave_model::SMS, rates: nvidia_ga104::mm_tc_v8::RATES_OS500 });
                        // P6-264 / P6-265: the long-sum guard and `attn_fwd_tc` (`pack_mixed_fast_attn9`; both 0 = `attn8`).
                        let fwd_va = fwd_map.as_ref().map_or(0, |m| m.va);
                        let attn9 = long_k != 0 || fwd_va != 0;
                        // P6-268: with `--pas-gelu-lipit`, the GELU' variants (`pack_mixed_fast_attn10`; `None` = `attn9`).
                        let gf = gelu_maps.as_ref().map(|(a, b, c)| ts::GeluFast { v7: a.va, v8: b.va, v8n: c.va });
                        let (zb, qmds, pk) = match (&via, attn, fast) {
                            (Some(v), at, f) if v8p.is_some() && gf.is_some() => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn10(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, long_k, fwd_va, gf, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v8p.is_some() && gf.is_some() => ts::pack_mixed_fast_attn10(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, long_k, fwd_va, gf, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if v8p.is_some() && attn9 => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn9(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, long_k, fwd_va, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v8p.is_some() && attn9 => ts::pack_mixed_fast_attn9(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, long_k, fwd_va, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if v8p.is_some() => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn8(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v8p.is_some() => ts::pack_mixed_fast_attn8(&ml, prog_va, &progs, v5va, v6va, v7va, v8p, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if v7va != 0 => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn7(&ml, prog_va, &progs, v5va, v6va, v7va, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v7va != 0 => ts::pack_mixed_fast_attn7(&ml, prog_va, &progs, v5va, v6va, v7va, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if v6va != 0 => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn6(&ml, prog_va, &progs, v5va, v6va, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v6va != 0 => ts::pack_mixed_fast_attn6(&ml, prog_va, &progs, v5va, v6va, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if v5va != 0 => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn5(&ml, prog_va, &progs, v5va, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if v5va != 0 => ts::pack_mixed_fast_attn5(&ml, prog_va, &progs, v5va, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, f) if with4 => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn4(&ml, prog_va, &progs, at, f, attn0, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, f) if with4 => ts::pack_mixed_fast_attn4(&ml, prog_va, &progs, at, f, attn0, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), None, None) => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram(&ml, prog_va, &progs, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, None, None) => ts::pack_mixed_fast(&ml, prog_va, &progs, Some(MIXED_FAST_STEP), ctl.va)?,
                            (Some(v), at, _) => nvidia_ga104::batch_ram::pack_mixed_fast_via_ram_attn(&ml, prog_va, &progs, at, Some(MIXED_FAST_STEP), ctx.copy_va, ctl.va, v)?,
                            (None, at, _) => ts::pack_mixed_fast_attn(&ml, prog_va, &progs, at, Some(MIXED_FAST_STEP), ctl.va)?,
                        };
                        // P6-268: `mm_tc_gelu` is not loaded (gelu: 0): a product that would fall on it is refused before
                        // anything is sent (at #513 there is none; note C-168_P5b).
                        if pk.contains(&Some(ts::TcPick::Gelu)) {
                            return Err(bad("antrenare-513: NU A RULAT: un produs proj-dX+gelu nealiniat ar cadea pe mm_tc_gelu, neincarcat (--pas-gelu-lipit); nimic trimis"));
                        }
                        picks = Some(fast_picks_text(&pk));
                        (zb, qmds, pk)
                    }
                    None => {
                        let (zb, qmds) = ts::pack_mixed(&ml, prog_va, tcm.va, ctl.va)?;
                        (zb, qmds, alloc::vec![None; ml.len()])
                    }
                };
                if k == 0 {
                    let pr = nvidia_ga104::train_profile::StepProfile::from_mixed(&ml, &pk)?;
                    mprof = Some(match &via {
                        Some(v) => pr.with_copies(v.copies_in.len(), v.copies_out.len())?,
                        None => pr,
                    });
                }
                (zb, qmds, None)
            }
            None => {
                let list = plan.step(Recipe::AdamW513, &adam, step)?;
                let (zb, qmds) = match sm_step {
                    // P6-205: the same step on every step's list (the programs never change between steps).
                    Some(sm) => {
                        if ts::step_best_sm_step(&list, nvidia_ga104::occupancy::Reserve::OneKib) != sm {
                            return Err(bad("antrenare-513: treapta memoriei comune difera intre pasi"));
                        }
                        ts::pack_with_sm_step(&list, prog_va, ctl.va, sm)?
                    }
                    None => ts::pack(&list, prog_va, ctl.va)?,
                };
                (zb, qmds, Some(list))
            }
        };
        if zb.len() as u64 > zone_bytes {
            return Err(bad("antrenare-513: zona de control prea mare"));
        }
        ram_write(ctl, &zb);
        let t_list = crate::time::since_boot_us();
        Ok(Prep { slot, plan, via, qmds, list, tc, picks, t0, t_batch, t_list, mprof })
    };
    // P6-217: one part of a step submitted and rung, not waited for: (what to wait for, first payload, launches).
    // `None` = the pushbuffer has no room for it until an earlier part is retired (nothing was submitted).
    let send_part = |st: &mut GpuState, vbar: &mut V, clock: &mut K, journal: &mut Journal, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, rec: Mapping, part: &[u64]| -> GpuResult<Option<(Sent, u32, usize)>> {
        let n = part.len();
        put_bytes(st.mem.writer_mut(), rec.pa, &alloc::vec![0u8; 16 * (n + 1)])?;
        let first = TOKEN_FIRST + STRIDE * *subs;
        let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(n)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence_fresh(&mut pb, part, rec.va, first)?;
        let nw = pb.len();
        match submit_records(vbar, clock, journal, st, l, permit, &words[..nw], rec, n as u64 + 1, first + n as u32, &maps, &sys, doorbell) {
            Ok(sent) => {
                *subs += 1;
                Ok(Some((sent, first, n)))
            }
            Err(GpuError::OutOfMemory { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    };
    // P6-217: the next step's parts already submitted (in order), and how many went while the step before still ran.
    let mut ahead: Vec<(Sent, u32, usize)> = Vec::new();
    let (mut sent_ahead, mut held_back, mut timed_out) = (0usize, 0usize, false);
    // P6-250 (C-157 P2zzl: "the lot-6 step with many flags may pass 600 s too"): the steps against the max-speed window.
    let steps_t0 = clock.now_us();
    let mut run_steps = || -> GpuResult<()> {
        let mut next_prep: Option<Prep> = None;
        let mut prev_done: Option<u64> = None;
        for (k, (x, y)) in data.batches.iter().enumerate() {
            let step = k as u32 + 1;
            let prep = match next_prep.take() {
                Some(pr) => pr,
                None => prepare(st, k)?,
            };
            let Prep { slot, plan, via, qmds, list, tc, picks, t0, t_batch, t_list, mprof } = prep;
            if let Some(t) = tc {
                tc_per_step = t;
            }
            if let Some(t) = picks {
                fast_picks = t;
            }
            if let (true, true, Some(list)) = (race, race_shapes.is_empty(), list.as_ref()) {
                race_shapes = race_forms(list);
            }
            if let (None, Some(list)) = (prof.as_ref(), list.as_ref()) {
                let pr = nvidia_ga104::train_profile::StepProfile::from_launches(list)?;
                if pr.launches() != qmds.len() {
                    return Err(bad("antrenare-513: profilul pasului nu are cate lansari are lista"));
                }
                prof = Some(pr);
            }
            // P6-226: the mixed step's profile. A count that differs from the QMDs leaves the run without a profile (the
            // line below says so) instead of stopping a run that is otherwise fine.
            if let (None, Some(pr)) = (prof.as_ref(), mprof) {
                if pr.launches() == qmds.len() {
                    prof = Some(pr);
                } else {
                    mixed_prof_skip = Some((pr.launches(), qmds.len()));
                }
            }
            let mut step_ns: Vec<u64> = Vec::with_capacity(qmds.len());
            let mut busy = 0u64;
            let mut t_wait0 = t_list;
            if slots_on {
                // P6-217: this step's parts not yet sent (all of step 1; any the pushbuffer had no room for), then the
                // next step made ready in the other slot while the board runs this one; each next part is sent as soon
                // as the same part of this step is done (never more than one step's pushbuffer in flight).
                let parts: Vec<&[u64]> = qmds.chunks(split).collect();
                if parts.len() > recs[slot].len() {
                    return Err(bad("antrenare-513: mai multe parti decat zone de inregistrari"));
                }
                while ahead.len() < parts.len() {
                    let j = ahead.len();
                    match send_part(st, vbar, clock, journal, l, doorbell, subs, recs[slot][j], parts[j])? {
                        Some(sn) => ahead.push(sn),
                        None => return Err(bad("antrenare-513: bufferul de comenzi nu are loc pentru o parte a pasului")),
                    }
                }
                let mine = core::mem::take(&mut ahead);
                if k + 1 < data.batches.len() {
                    next_prep = Some(prepare(st, k + 1)?);
                }
                t_wait0 = crate::time::since_boot_us();
                for (j, (sn, first, n)) in mine.into_iter().enumerate() {
                    match wait_records(vbar, clock, journal, st, l, permit, &sn)? {
                        Ok(b) => {
                            let ns = transfer::analyze_sequence(&b, n, first)?;
                            busy += ns.iter().sum::<u64>();
                            step_ns.extend_from_slice(&ns);
                        }
                        Err(got) => {
                            timed_out = true;
                            stopped = Some(format!("placa nu a terminat partea {} a pasului {} (ultima inregistrare {:#x}, asteptat {:#x}; doua locuri)", j + 1, step, got, first + n as u32));
                            return Ok(());
                        }
                    }
                    if let Some(np) = next_prep.as_ref() {
                        let np_parts: Vec<&[u64]> = np.qmds.chunks(split).collect();
                        if ahead.len() == j && j < np_parts.len() && j < recs[np.slot].len() {
                            match send_part(st, vbar, clock, journal, l, doorbell, subs, recs[np.slot][j], np_parts[j])? {
                                Some(sn) => {
                                    ahead.push(sn);
                                    sent_ahead += 1;
                                }
                                None => held_back += 1,
                            }
                        }
                    }
                }
            } else {
                // The step in parts of at most `split` launches, each its own submission, in order.
                for (j, part) in qmds.chunks(split).enumerate() {
                    let n = part.len();
                    put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (n + 1)])?;
                    let first = TOKEN_FIRST + STRIDE * *subs;
                    let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(n)];
                    let mut pb = PushBuf::new(&mut words);
                    transfer::encode_sequence_fresh(&mut pb, part, ctx.rec.va, first)?;
                    let nw = pb.len();
                    let last = first + n as u32;
                    match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, n as u64 + 1, last, &maps, &sys, doorbell)? {
                        Ok(b) => {
                            *subs += 1;
                            let ns = transfer::analyze_sequence(&b, n, first)?;
                            busy += ns.iter().sum::<u64>();
                            step_ns.extend_from_slice(&ns);
                        }
                        Err(got) => {
                            stopped = Some(format!("placa nu a terminat partea {} a pasului {} (ultima inregistrare {:#x}, asteptat {:#x})", j + 1, step, got, last));
                            return Ok(());
                        }
                    }
                }
            }
            board_ns.push(busy);
            let t_subs = crate::time::since_boot_us();
            if let Some(pr) = prof.as_mut() {
                pr.add_step(&step_ns)?;
            }
            let [(lr_va, lr_n), (res_va, res_n)] = plan.step_reads();
            let (rows, norm) = match &via {
                // P6-216: the two copies at the end of the submission brought them into the staging area (written with
                // zeros with the batch: a copy that did not arrive leaves 0, and the oracle / the loss check catch it).
                // P6-217: this step's slot's half (the next step's half was written meanwhile, not this one).
                Some(v) => nvidia_ga104::batch_ram::results(ram_vram_bytes(host + stage_off(slot), v.staging.len() as u64), v)?,
                None => {
                    let rows = read_vram_f32s(st, vbar, journal, lr_va, (lr_n / 4) as usize)?;
                    let norm = read_vram_f32s(st, vbar, journal, res_va, (res_n / 4) as usize)?[0];
                    (rows, norm)
                }
            };
            let t_res = crate::time::since_boot_us();
            // P6-216 (note C-157_P2zg: "outside the board" split): batch, list + packing, submissions minus the board,
            // results (µs; a mark the clock did not give leaves the step out). P6-217: with two slots the batch and the
            // list were made while the step before ran, and "submissions" is the wait for this step minus its board time.
            if let (Some(a0), Some(a1), Some(a2), Some(a3), Some(a4)) = (t0, t_batch, t_wait0, t_subs, t_res) {
                outside[0].push(a1.saturating_sub(a0));
                outside[1].push(t_list.unwrap_or(a1).saturating_sub(a1));
                outside[2].push(a3.saturating_sub(a2).saturating_sub(busy / 1000));
                outside[3].push(a4.saturating_sub(a3));
            }
            // P6-198: the mixed list's `res[0]` is S times the norm (S a power of two: the division is exact).
            let norm = if mixed { norm / scale } else { norm };
            let loss = Outcome::mean_loss(&rows);
            journal.flush_live();
            // P6-217: with two slots, the whole step = from the previous step's results to this one's (step 1: from its
            // preparation), since each step was made ready while the one before ran.
            let now = crate::time::since_boot_us();
            let start = if slots_on { prev_done.or(t0) } else { t0 };
            if let Some(us) = start.zip(now).map(|(a0, a1)| a1.saturating_sub(a0)) {
                host_us.push(us);
            }
            prev_done = now;
            if !norm.is_finite() || !loss.is_finite() {
                stopped = Some(format!("pasul {}: pierderea {:?} / norma gradientului {:?} nu e un numar{}", step, loss, norm, if mixed { " (scara S prea mare?)" } else { "" }));
                return Ok(());
            }
            losses.push(loss);
            norms.push(norm);
            let oracle_part = match oracle.as_mut() {
                Some((op, oa)) => {
                    let batch = Batch { inputs: x, targets: y, batch: bsz, len: tl };
                    let (ol, mut og) = train::loss_and_grad(&a, op, &batch)?;
                    let onorm = train::clip_grad_norm(&mut og, 1.0);
                    oa.step(op, &og)?;
                    let dn = ((norm as f64 - onorm as f64) / (onorm as f64).abs().max(1e-12)).abs();
                    let dl = ((loss as f64 - ol as f64) / (ol as f64).abs().max(1e-12)).abs();
                    worst_norm = worst_norm.max(dn);
                    worst_loss = worst_loss.max(dl);
                    format!(" (oracolul: pierderea {:.4}, abaterea {:.1e}; norma {:.4}, abaterea {:.1e})", ol, dl, onorm, dn)
                }
                None => String::new(),
            };
            lines.push(format!(
                "{}: kronos --gpu: {}: pasul {}: pierderea {:.4}, norma gradientului {:.4}{}; placa {} us ({} trimiteri), pasul intreg {} us",
                p, tag, step, loss, norm, oracle_part, busy / 1000, n_parts, host_us.last().copied().unwrap_or(0)
            ));
            // P6-192 (A4): the dashboard right after the step (outside its clock). GSP-RM writes the page every 500 ms
            // and the board works ~98 % of the time here, so the values are those of the board at work. The thermal
            // watch's stop ends the steps.
            if let Some(s) = super::gpu_tur::step_dashboard(p, &format!("in timpul pasilor #513 (dupa pasul {})", step), journal, lines) {
                let stop = s.stop.then(|| s.verdict.clone());
                step_dash.push(s);
                if let Some(v) = stop {
                    stopped = Some(format!("ceasul termic dupa pasul {}: {}", step, v));
                    return Ok(());
                }
            }
        }
        Ok(())
    };
    let steps_r = run_steps();
    // P6-217: a step stopped early (a number that is not one, the thermal watch) may leave the next step's parts in
    // flight: they are waited for (and retired) before anything else goes on the channel; after a part that did not
    // finish, the rest will not either, so they are left as today.
    if !timed_out {
        for (sn, ..) in core::mem::take(&mut ahead) {
            if wait_records(vbar, clock, journal, st, l, permit, &sn)?.is_err() {
                break;
            }
        }
    }
    drop(hold);
    journal.set_hold_values(false);
    journal.set_routine(None);
    steps_r?;
    lines.push(format!(
        "{}: kronos --gpu: {}: tabloul de dupa fiecare pas tinut in jurnal cu scrierile rutinei (P2zzj, set_hold_values; scris pe stick la golire / la iesirea din rutina)",
        p, tag
    ));
    if slots_on {
        journal.push(Event::Value { what: KT_513_AHEAD, value: sent_ahead as u32 });
        lines.push(format!(
            "{}: kronos --gpu: {}: doua locuri: {} parti ale pasilor 2..{} trimise cat placa lucra pasul dinainte ({} parti pe pas), {} amanate (fara loc in bufferul de comenzi; trimise dupa pasul dinainte)",
            p, tag, sent_ahead, data.batches.len(), n_parts, held_back
        ));
    }
    // 2. At the end: the weights region back VRAM → RAM in pieces — every weight finite; in SIMULARE against the oracle.
    let (mut bad_w, mut worst_w, mut read_w) = (0usize, 0.0f64, 0usize);
    if stopped.is_none() {
        let (w0, w_len) = (plan0.w, 4 * plan0.n_params as u64);
        let mut board_w: Vec<f32> = Vec::new();
        let mut off = 0u64;
        while off < w_len {
            let len = piece.min(w_len - off);
            if copy(st, vbar, clock, journal, l, doorbell, subs, w0 + off, stage.va, len)?.is_none() {
                stopped = Some(String::from("copia greutatilor VRAM -> RAM de la sfarsit nu s-a terminat"));
                break;
            }
            let ws = ts::f32s(ram_vram_bytes(host, len));
            bad_w += ws.iter().filter(|x| !x.is_finite()).count();
            read_w += ws.len();
            if !board {
                board_w.extend_from_slice(&ws);
            }
            off += len;
        }
        if let (Some((op, _)), None) = (&oracle, &stopped) {
            let flat: Vec<Vec<f32>> = plan0.unflat(&board_w)?;
            worst_w = am::rel_dev(&flat, op);
        }
    }
    let us = median_us(&board_ns);
    let host_med = median_us(&host_us.iter().map(|u| u * 1000).collect::<Vec<u64>>());
    let tok_s = if us > 0 { (bsz * tl) as u64 * 1_000_000 / us } else { 0 };
    let full_tok_s = if host_med > 0 { (bsz * tl) as u64 * 1_000_000 / host_med } else { 0 };
    journal.push(Event::Value { what: KT_513_STEPS, value: losses.len() as u32 });
    journal.push(Event::Value { what: KT_513_US, value: us.min(u32::MAX as u64) as u32 });
    journal.push(Event::Value { what: KT_513_HOST_US, value: host_med.min(u32::MAX as u64) as u32 });
    journal.push(Event::Value { what: KT_513_TOK_S, value: tok_s as u32 });
    if let (Some(f), Some(z)) = (losses.first(), losses.last()) {
        journal.push(Event::Value { what: KT_513_LOSS1_E4, value: (*f as f64 * 1e4).clamp(0.0, u32::MAX as f64) as u32 });
        journal.push(Event::Value { what: KT_513_LOSSN_E4, value: (*z as f64 * 1e4).clamp(0.0, u32::MAX as f64) as u32 });
    }
    if let Some(why) = &stopped {
        lines.push(format!("{}: kronos --gpu: {}: {}; proba oprita, fara reincercare", p, tag, why));
        lines.push(format!("{}: kronos --gpu: {}: NU A TERMINAT (vezi randul de mai sus)", p, tag));
        return Err(GpuError::Timeout { waited_us: WAIT_US });
    }
    let ln_v = am::ln(a.vocab as f64);
    let first = losses.first().copied().unwrap_or(f32::NAN) as f64;
    let last = losses.last().copied().unwrap_or(f32::NAN) as f64;
    let start_ok = ((first - ln_v) / ln_v).abs() <= 0.02;
    let falls = steps < 2 || last < first;
    lines.push(format!(
        "{}: kronos --gpu: {}: pierderea la pornire {:.4} (ghicit la intamplare: ln {} = {:.4}), la ultimul pas {:.4}; greutatile la sfarsit: {} citite, {} care nu sunt numere",
        p, tag, first, a.vocab, ln_v, last, read_w, bad_w
    ));
    lines.push(format!(
        "{}: kronos --gpu: {}: timpul pe pas (median, {} pasi): placa {} us = {} simboluri/s la antrenare (lot {} x {}; Windows / PyTorch ~39 300); pasul intreg {} us = {} simboluri/s",
        p, tag, losses.len(), us, tok_s, bsz, tl, host_med, full_tok_s
    ));
    let what = format!("{} (pasii)", tag);
    if let Some(w) = super::gpu_golden::speed_window_line(p, &what, steps_t0, clock.now_us(), journal) {
        lines.push(w);
    }
    if mixed {
        journal.push(Event::Value { what: KT_513_MIXED_TC, value: tc_per_step as u32 });
        lines.push(format!("{}: kronos --gpu: {}: pasul mixt: {} lansari mm_tc pe pas (din {}), S = {}", p, tag, tc_per_step, n_launch, scale as u32));
        if fast_map.is_some() {
            lines.push(format!(
                "{}: kronos --gpu: {}: alegerile mm_tc pe pas (--mixt-513-rapid, pack_mixed_fast{}, treapta memoriei comune {} KiB pe toate lansarile): {} (Cyber-GPU: {}, DEDUS{})",
                p, tag, if attn16_map.is_some() { "_attn" } else { "" }, MIXED_FAST_STEP >> 10, fast_picks,
                // P6-245: with `--pas-v5` the prediction of note C-157_P2zzf.
                if fwd_map.is_some() { "12 pe attn_fwd_tc, niciun attn_fwd_tile3; restul alegerilor neschimbat (lot 6 fara garda v7 12 / v8n 135, cu garda 13 / 134; lot 2 v7 73 / v8 24 / v8n 50); oracolul lot 2 ACELEASI NUMERE; placa 126-130 ms pe pas la lot 6, cu garda 121-126 (os-504: 140,5)" } else if long_k != 0 { "lot 6: v7 13 / v8n 134 (garda P3e), aceleasi numere ca os-503; placa 132,0-135,8 ms pe pas la lot 6 (os-503: 140,7); lot 2 neschimbat (v7 73 / v8 24 / v8n 50)" } else if v8_maps.is_some() { "lot 2: v7 73 / v8 24 / v8n 50, lot 6: v7 12 / v8 0 / v8n 135 (vitezele masurate in os-500, nota P3c); oracolul mixt ACELEASI NUMERE; placa ~133 ms pe pas la lot 6 (os-496: 147,7)" } else if v7_map.is_some() { "147 pe mm_tc_v7, 0 pe mm_tc_v6 / mm_tc_v5; oracolul mixt ACELEASI NUMERE la lot 2; placa 137-162 ms pe pas la lot 6 (os-487: 200,77)" } else if v6_map.is_some() { "147 pe mm_tc_v6, 0 pe mm_tc_v5; placa x0,885-0,979 fata de pasul cu v5 la lot 2, x0,867-0,976 la lot 6" } else if v5_map.is_some() { "147 pe mm_tc_v5, 0 pe mm_tc_v4; placa ~78-84 ms pe pas la lot 2 (os-473: 97,1), 202-222 ms la lot 6" } else { "147 pe mm_tc_v4, inmultirile ~115-136 ms" },
                if attn_v4_map.is_some() { "; atentia: 12 pe attn_back_tile_tc16_v4 asteptate (os-465: 0,52 ms fata de 0,87 ms pe lansare), aceiasi biti ca tc16" } else if attn16_map.is_some() { "; atentia: 12 pe attn_back_tile_tc16, ~2,2-4,0 ms mai putin pe pas, DEDUS" } else { "" }
            ));
        }
    }
    // P6-216 (note C-157_P2zg): "outside the board" = the whole step - the board, and its pieces (medians, µs).
    {
        let out_us: Vec<u64> = host_us.iter().zip(&board_ns).map(|(h, b)| h.saturating_sub(b / 1000) * 1000).collect();
        let out_med = median_us(&out_us);
        journal.push(Event::Value { what: KT_513_OUTSIDE_US, value: out_med.min(u32::MAX as u64) as u32 });
        let m = |v: &Vec<u64>| median_us(&v.iter().map(|u| u * 1000).collect::<Vec<u64>>());
        lines.push(format!(
            "{}: kronos --gpu: {}: in afara placii (pasul intreg - placa, median): {} us; pe bucati: lotul {} us, lista + impachetarea {} us, trimiterile (fara placa) {} us, rezultatele {} us; {}",
            p, tag, out_med, m(&outside[0]), m(&outside[1]), m(&outside[2]), m(&outside[3]),
            if via_ram_on { "lot + rezultate prin RAM fixata (--lot-prin-ram)" } else { "lot + rezultate prin fereastra VRAM" }
        ));
    }
    // P6-183 (os-395: "placa" > "pasul intreg" by ~3 %): the two clocks side by side, so the next report says which.
    lines.push(format!(
        "{}: kronos --gpu: {}: ceasurile: placa / pasul intreg = {} la mie (median; peste 1000 = un ceas gresit); {}",
        p,
        tag,
        if host_med > 0 { us * 1000 / host_med } else { 0 },
        crate::arch::x86_64::apic::clock_note()
    ));
    // P6-192 (A4): what the dashboard said during the steps (the clock, the power, the load, the reasons).
    if super::gpu_tur::step_dashboard_lent() {
        for row in super::gpu_golden::dash_summary(p, "in timpul pasilor #513", "tablou pasi-513", &step_dash) {
            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
        }
    }
    // P6-182: once, at the end: the total, the stages, the 8 longest tags, the programs.
    let prof_steps = prof.as_ref().map_or(0, |pr| pr.steps());
    journal.push(Event::Value { what: KT_513_PROFILE, value: prof_steps as u32 });
    if let Some(pr) = &prof {
        for row in pr.lines(8) {
            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
        }
    }
    if let Some((a, b)) = mixed_prof_skip {
        lines.push(format!("{}: kronos --gpu: {}: profilul pasului mixt sarit: {} lansari in profil, {} in lista (P6-226)", p, tag, a, b));
    }
    // P6-190 (C-165, decision H3): the night race. After the steps (the arena holds real weights, activations and
    // gradients, nothing reads it any more), for every `mm_train` shape of the step: the same A, B (and bias) as the
    // step's first launch of that shape, C redirected to a scratch piece. For each variant: SHA-256 checked, C and a
    // 256-byte guard filled with a sentinel (through RAM with the copy program), one warm-up launch + N timed ones in one
    // submission (each its own QMD, the variant's own grid / block / shared memory), C + guard read back through RAM and
    // compared bit for bit with the witness (`mm_train_tile`, index 0, on the board since os-410). Nothing new for the
    // board but the programs; it does not change #513's verdict. The fastest variant with the same bits per shape is
    // what `Plan::with_mm_choice` (Cyber-GPU, next) will take.
    if let Some(c_map) = race_c.filter(|_| stopped.is_none()) {
        use nvidia_ga104::mm_train::Args as MmArgs;
        use nvidia_ga104::mm_train_variants::{race_list_full, Variant, RACE_CLEARED_OS417, RACE_HOLD, VARIANTS};
        use nvidia_ga104::qmd::ComputeQmd;
        const SENTINEL: [u8; 4] = (-12345.0f32).to_le_bytes();
        let reps = (if board { 8usize } else { 2 }).min(rec_cap).min((zone_bytes / ts::SLOT) as usize);
        let shas: Vec<bool> = VARIANTS.iter().map(|v| sha256_hex(v.code) == v.sha256).collect();
        // P6-195: who runs and on which shapes — the race list on every shape, or (the probe) the witness and 64x64k8 on
        // the qkv shape only (the first with N = 3 K: 1024 x 2304 x 768 on the board, where os-413 stopped).
        // P6-201: each racer with its SM shared-memory step (`None` = today's QMD, the step of one block).
        let racers: Vec<(usize, &'static Variant, Option<u32>)> = if probe64 {
            VARIANTS.iter().enumerate().filter(|(i, _)| *i == 0 || Some(*i) == probe64_vi).map(|(i, v)| (i, v, None)).collect()
        } else if occ {
            nvidia_ga104::occupancy::PROBE.iter().filter(|r| r.variant < VARIANTS.len()).map(|r| (r.variant, &VARIANTS[r.variant], Some(r.step_bytes))).collect()
        } else {
            race_list_full().map(|(i, v)| (i, v, None)).collect()
        };
        if occ {
            race_shapes = race_shapes.iter().filter(|f| f.n == 3 * f.k).take(1).copied().collect();
            lines.push(format!(
                "{}: kronos --gpu: {}: proba ocuparii (P6-201, C-165, PREDARE Cyber-GPU 54f5ab8): in locul cursei mari, pe forma qkv ({}), {} rulari din occupancy::PROBE ({}); treapta memoriei comune = MIN = TARGET_SM_CONFIG_SHARED_MEM_SIZE din QMD, MAX ca azi; martorul bitilor = prima rulare (lansarea de azi)",
                p, tag,
                race_shapes.first().map_or(String::from("negasita"), |f| format!("{}x{}x{} f{}", f.m, f.n, f.k, f.flags)),
                racers.len(),
                racers.iter().map(|&(_, v, sb)| format!("{} {} KiB", v.name, sb.unwrap_or(0) >> 10)).collect::<Vec<_>>().join(", ")
            ));
        }
        if probe64 {
            race_shapes = race_shapes.iter().filter(|f| f.n == 3 * f.k).take(1).copied().collect();
            lines.push(format!(
                "{}: kronos --gpu: {}: proba 64x64k8 (P6-195, propunerea 3 Cyber-GPU dupa os-413): in locul cursei mari, doar martorul {} si {} pe forma qkv ({}); {} mapata ultima, ca la os-413, dar cu o pagina de paza dupa cod (qmd::program_map_bytes)",
                p, tag, VARIANTS[0].name, PROBE_64K8_NAME,
                race_shapes.first().map_or(String::from("negasita"), |f| format!("{}x{}x{} f{}", f.m, f.n, f.k, f.flags)),
                PROBE_64K8_NAME
            ));
        }
        lines.push(format!(
            "{}: kronos --gpu: {}: cursa mm (C-165, {}): {} forme ale lui mm_train din pas ({} lansari) x {} variante ({}); martorul {} (trecut pe placa la os-410); C pe o bucata de {} MiB in VRAM + 256 octeti de paza, umplute cu -12345.0; {} lansari pe varianta si forma (prima de incalzire), comparate bit cu bit cu martorul",
            p, tag, label, race_shapes.len(), race_shapes.iter().map(|f| f.count).sum::<u32>(), racers.len(),
            racers.iter().map(|&(i, v, _)| if shas[i] { String::from(v.name) } else { format!("{} SHA-256 GRESIT", v.name) }).collect::<Vec<_>>().join(", "),
            VARIANTS[0].name, c_map.size >> 20, reps
        ));
        // P6-194: the held variants (os-413: `mm_tv_64x64k8` did not finish), named once, never launched.
        // P6-199: those cleared by os-417 (finished with the tail page, the witness's bits) race again, named once.
        for &i in RACE_HOLD.iter().filter(|&&i| i < VARIANTS.len() && !probe64 && !occ) {
            if RACE_CLEARED_OS417.contains(&i) {
                lines.push(format!("{}: kronos --gpu: {}: cursa mm: {} intoarsa in cursa (RACE_CLEARED_OS417; os-417: a terminat cu pagina de paza, aceiasi biti ca martorul; P6-199)", p, tag, VARIANTS[i].name));
            } else {
                lines.push(format!("{}: kronos --gpu: {}: cursa mm: {} scoasa din cursa (RACE_HOLD; os-413: nu a terminat), nelansata", p, tag, VARIANTS[i].name));
            }
        }
        lines.push(format!("{}: kronos --gpu: {}: cursa mm: programele mapate cu o pagina de paza dupa cod (qmd::program_map_bytes, P6-194)", p, tag));
        // Sentinel → C piece by piece (the stage is refilled each time: the reads below overwrite it).
        let fill = |st: &mut GpuState, vbar: &mut V, clock: &mut K, journal: &mut Journal, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, len: u64| -> GpuResult<bool> {
            let mut off = 0u64;
            while off < len {
                let n = piece.min(len - off);
                for w in ram_vram_bytes(host, n).chunks_exact_mut(4) {
                    w.copy_from_slice(&SENTINEL);
                }
                if copy(st, vbar, clock, journal, l, doorbell, subs, stage.va, c_map.va + off, n)?.is_none() {
                    return Ok(false);
                }
                off += n;
            }
            Ok(true)
        };
        // C + guard back through RAM: (hash of C, guard intact, words still the sentinel inside C).
        let read = |st: &mut GpuState, vbar: &mut V, clock: &mut K, journal: &mut Journal, l: &mut Launched, doorbell: &mut Bell<'_, V>, subs: &mut u32, c_len: u64, len: u64| -> GpuResult<Option<(u64, bool, u64)>> {
            let (mut h, mut guard_ok, mut unwritten, mut off) = (0xcbf2_9ce4_8422_2325u64, true, 0u64, 0u64);
            while off < len {
                let n = piece.min(len - off);
                if copy(st, vbar, clock, journal, l, doorbell, subs, c_map.va + off, stage.va, n)?.is_none() {
                    return Ok(None);
                }
                let buf = ram_vram_bytes(host, n);
                let in_c = c_len.saturating_sub(off).min(n) as usize;
                h = fnv64(h, &buf[..in_c]);
                unwritten += buf[..in_c].chunks_exact(4).filter(|w| *w == SENTINEL).count() as u64;
                guard_ok &= buf[in_c..].chunks_exact(4).all(|w| w == SENTINEL);
                off += n;
            }
            Ok(Some((h, guard_ok, unwritten)))
        };
        let (mut rows_same, mut rows_differ, mut saved_ns, mut race_stopped) = (0u32, 0u32, 0u64, None::<String>);
        let mut probe64_bits: Option<RaceBits> = None;
        // P6-201: per occupancy run (variant, step, median ns, bits, board power mW).
        let mut occ_rows: Vec<(usize, u32, Option<u64>, RaceBits, Option<u32>)> = Vec::new();
        let mut race_dash: Vec<super::gpu_golden::StepSample> = Vec::new();
        'forms: for (fi, f) in race_shapes.iter().enumerate() {
            let c_len = 4 * u64::from(f.m) * u64::from(f.n);
            let len = c_len.next_multiple_of(256) + 256;
            let args = MmArgs { a: f.a, b: f.b, c: c_map.va, bias: f.bias, m: f.m, n: f.n, k: f.k, flags: f.flags };
            let Some(cb_args) = args.cbuf() else {
                lines.push(format!("{}: kronos --gpu: {}: cursa: forma {}x{}x{} f{}: argumentele refuzate; sarita", p, tag, f.m, f.n, f.k, f.flags));
                continue;
            };
            // SIMULARE: the shapes with the vocabulary (~67 M multiply-adds each) take minutes per launch on the emulated
            // CPU; the race path is the same on the small ones.
            if !board && u64::from(f.m) * u64::from(f.n) * u64::from(f.k) > 1 << 20 {
                lines.push(format!("{}: kronos --gpu: {}: cursa: forma {}x{}x{} f{}: sarita in SIMULARE (prea mare pentru procesorul emulat)", p, tag, f.m, f.n, f.k, f.flags));
                continue;
            }
            let mut cb = [0u8; 0x100];
            cb[..cb_args.len()].copy_from_slice(&cb_args);
            let mut witness: Option<u64> = None;
            let mut best: Option<(usize, u64)> = None;
            let mut witness_ns: Option<u64> = None;
            for (ri, &(vi, v, step)) in racers.iter().enumerate() {
                let skip = !shas[vi] || len > c_map.size || (ri > 0 && witness.is_none());
                // P6-201: a step the QMD refuses (smaller than the block's memory, above MAX) is not run.
                let refused = step.is_some_and(|sb| {
                    use nvidia_ga104::qmd::{sm_config_shared_units, SM_CONFIG_STEPS};
                    let sh = Variant::shared(v);
                    !SM_CONFIG_STEPS.contains(&sb) || sb < sh.bytes.next_multiple_of(0x100) || sm_config_shared_units(sb) > sm_config_shared_units(sh.max_bytes)
                });
                if skip || refused {
                    if occ {
                        lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, occ_row(v.name, step.unwrap_or(0), f, None, None, RaceBits::Skipped)));
                        occ_rows.push((vi, step.unwrap_or(0), None, RaceBits::Skipped, None));
                    } else {
                        lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, race_row(v.name, f, None, RaceBits::Skipped)));
                    }
                    continue;
                }
                if !fill(st, vbar, clock, journal, l, doorbell, subs, len)? {
                    race_stopped = Some(format!("umplerea lui C nu s-a terminat (forma {}, {})", fi + 1, v.name));
                    break 'forms;
                }
                // `reps` copies of the same cbuf + QMD, one slot each (as the step's control zone).
                let q = ComputeQmd { program_va: race_progs[vi].map_or(0, |m| m.va), registers: v.registers, cbuf0_va: 0, cbuf0_size: 0x100, grid: v.grid(&args), block: v.block(), release: None };
                let mut zb = alloc::vec![0u8; reps * ts::SLOT as usize];
                let mut qv = Vec::with_capacity(reps);
                for r in 0..reps {
                    let at = r * ts::SLOT as usize;
                    let cb_va = ctl.va + at as u64;
                    let qb = match step {
                        Some(sb) => ComputeQmd { cbuf0_va: cb_va, ..q }.to_bytes_with_sm_config(&Variant::shared(v), sb)?,
                        None => ComputeQmd { cbuf0_va: cb_va, ..q }.to_bytes_with_shared(&Variant::shared(v))?,
                    };
                    zb[at..at + 0x100].copy_from_slice(&cb);
                    zb[at + 0x100..at + 0x100 + qb.len()].copy_from_slice(&qb);
                    qv.push(cb_va + 0x100);
                }
                ram_write(&ctl, &zb);
                put_bytes(st.mem.writer_mut(), ctx.rec.pa, &alloc::vec![0u8; 16 * (reps + 1)])?;
                let first = TOKEN_FIRST + STRIDE * *subs;
                let mut words = alloc::vec![0u32; transfer::sequence_words_fresh(reps)];
                let mut pb = PushBuf::new(&mut words);
                transfer::encode_sequence_fresh(&mut pb, &qv, ctx.rec.va, first)?;
                let nw = pb.len();
                let last = first + reps as u32;
                let ns = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, reps as u64 + 1, last, &maps, &sys, doorbell)? {
                    Ok(b) => {
                        *subs += 1;
                        transfer::analyze_sequence(&b, reps, first)?
                    }
                    Err(got) => {
                        if occ {
                            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, occ_row(v.name, step.unwrap_or(0), f, None, None, RaceBits::Unfinished)));
                            occ_rows.push((vi, step.unwrap_or(0), None, RaceBits::Unfinished, None));
                        } else {
                            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, race_row(v.name, f, None, RaceBits::Unfinished)));
                        }
                        if Some(vi) == probe64_vi {
                            probe64_bits = Some(RaceBits::Unfinished);
                        }
                        race_stopped = Some(format!("placa nu a terminat {} pe forma {} (ultima inregistrare {:#x}, asteptat {:#x})", v.name, fi + 1, got, last));
                        break 'forms;
                    }
                };
                let med = median_of(ns.get(1..).filter(|t| !t.is_empty()).unwrap_or(&ns));
                let Some((h, guard_ok, unwritten)) = read(st, vbar, clock, journal, l, doorbell, subs, c_len, len)? else {
                    race_stopped = Some(format!("citirea lui C nu s-a terminat (forma {}, {})", fi + 1, v.name));
                    break 'forms;
                };
                let bits = if ri == 0 {
                    // The witness (the first racer: `mm_train_tile`, today's QMD in the race, the one-block step in the probe): every word written, the guard intact; otherwise the shape is not raced.
                    if guard_ok && unwritten == 0 {
                        witness = Some(h);
                        witness_ns = med;
                        RaceBits::Same
                    } else {
                        RaceBits::Differ
                    }
                } else if guard_ok && witness == Some(h) {
                    RaceBits::Same
                } else {
                    RaceBits::Differ
                };
                match bits {
                    RaceBits::Same => rows_same += 1,
                    _ => rows_differ += 1,
                }
                if bits == RaceBits::Same {
                    if let Some(t) = med {
                        if best.is_none_or(|(_, b)| t < b) {
                            best = Some((vi, t));
                        }
                    }
                }
                if occ {
                    // P6-201: the board's power right after the run (the page's ~500 ms average: only indicative).
                    let what = format!("dupa proba ocuparii {} {} KiB", v.name, step.unwrap_or(0) >> 10);
                    let mw = super::gpu_tur::step_dashboard(p, &what, journal, lines).and_then(|d| d.dash.avg_power_mw);
                    lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, occ_row(v.name, step.unwrap_or(0), f, med, mw, bits)));
                    occ_rows.push((vi, step.unwrap_or(0), med, bits, mw));
                    continue;
                }
                lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, race_row(v.name, f, med, bits)));
                if Some(vi) == probe64_vi {
                    probe64_bits = Some(bits);
                }
            }
            if let (Some((vi, t)), Some(w), false) = (best, witness_ns, occ) {
                saved_ns += w.saturating_sub(t) * u64::from(f.count);
                lines.push(format!(
                    "{}: kronos --gpu: {}: cursa: forma {}x{}x{} f{}: cea mai rapida cu aceiasi biti = {} ({}.{} us; martorul {}.{} us)",
                    p, tag, f.m, f.n, f.k, f.flags, VARIANTS[vi].name, t / 1000, (t % 1000) / 100, w / 1000, (w % 1000) / 100
                ));
            }
            // P6-192 (A4): the dashboard after each shape's variants; the thermal watch's stop ends the race (C-165).
            let what = format!("in timpul cursei (dupa forma {} din {})", fi + 1, race_shapes.len());
            if let Some(s) = super::gpu_tur::step_dashboard(p, &what, journal, lines) {
                let stop = s.stop.then(|| s.verdict.clone());
                race_dash.push(s);
                if let Some(v) = stop {
                    race_stopped = Some(format!("ceasul termic dupa forma {}: {}", fi + 1, v));
                    break 'forms;
                }
            }
        }
        journal.push(Event::Value { what: KT_513_RACE_SAME, value: rows_same });
        journal.push(Event::Value { what: KT_513_RACE_DIFFER, value: rows_differ });
        journal.push(Event::Value { what: KT_513_RACE_SAVED_US, value: (saved_ns / 1000).min(u32::MAX as u64) as u32 });
        lines.push(format!(
            "{}: kronos --gpu: {}: cursa mm: {}: {} randuri cu aceiasi biti, {} diferite; cu castigatoarele pe fiecare forma pasul ar scadea cu ~{} us (din timpii izolati; DEDUS)",
            p,
            tag,
            match &race_stopped {
                Some(why) => format!("NU A TERMINAT ({})", why),
                None => String::from("GATA"),
            },
            rows_same,
            rows_differ,
            saved_ns / 1000
        ));
        // P6-195: the probe's verdict, from its single row (the witness first, then 64x64k8).
        if probe64 {
            let verdict = match probe64_bits {
                Some(RaceBits::Same) => "TERMINA cu aceiasi biti ca martorul: blocajul de la os-413 venea (foarte probabil) din citirea in avans dupa capatul codului; varianta poate reveni in cursa",
                Some(RaceBits::Differ) => "a terminat, dar C DIFERA de martor: nu se foloseste",
                Some(RaceBits::Unfinished) => "NU A TERMINAT nici cu pagina de paza: cauza e in program; ramane scoasa din cursa",
                _ => "NELANSATA (martorul sau forma qkv lipsesc, ori SHA-256 gresit)",
            };
            journal.push(Event::Value { what: KT_513_PROBE_64K8, value: u32::from(probe64_bits == Some(RaceBits::Same)) });
            lines.push(format!("{}: kronos --gpu: {}: proba 64x64k8: {}: {}", p, tag, PROBE_64K8_NAME, verdict));
        }
        // P6-201: the occupancy probe's verdict (the note's rule: the bits first, then H1 / H2 on the witness's times).
        if occ {
            for row in occ_verdict(&occ_rows, racers.len(), race_stopped.is_some()) {
                lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
            }
            let same = occ_rows.iter().filter(|r| r.3 == RaceBits::Same).count() as u32;
            journal.push(Event::Value { what: KT_513_OCC_SAME, value: same });
            let ratio = occ_witness_ratio(&occ_rows);
            journal.push(Event::Value { what: KT_513_OCC_RATIO, value: ratio.unwrap_or(0) });
        }
        if super::gpu_tur::step_dashboard_lent() {
            for row in super::gpu_golden::dash_summary(p, "in timpul cursei", "tablou cursa", &race_dash) {
                lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
            }
        }
    }
    // P6-187 (command centre note 24 Nov 14:20, point 1): the fair Windows <-> OS bench — the same figures as PyTorch, one row.
    if losses.len() == steps {
        let version = crate::fs::read_all("/usb/VERSIUNE")
            .ok()
            .and_then(|v| String::from_utf8_lossy(&v).lines().next().map(|l| String::from(l.trim())))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| String::from("necunoscuta"));
        let bench = Bench {
            version: &version,
            model: if board { "#513" } else { "modelul mic (SIMULARE)" },
            batch: bsz,
            context: tl,
            precision: if mixed { "mixt (f16 pe mm_tc, sume f32)" } else { "F32" },
            data: if data.synthetic { String::from("sintetice MT19937 0x164") } else { data.source.clone() },
            attention: if tiles3 { "pe placi gen 3" } else { "de baza" },
            step_us: host_med,
            board_us: us,
            losses: &losses,
            norms: &norms,
        };
        for row in bench.lines() {
            lines.push(format!("{}: kronos --gpu: {}: {}", p, tag, row));
        }
    }
    // P6-184: on the synthetic batches with exactly the oracle's recipe, the verdict is the oracle (steps 1–9);
    // on Kronos's data the old one (the loss starts at ln V and falls).
    let recipe = board && data.synthetic && bsz == 2 && tl == 512 && steps == gpu_compute::train::ORACLE_513_SYNTH.len();
    // P6-221 (PREDARE Cyber-GPU `969a6b4`, note C-157_P2zi): the mixed step is judged against the mixed oracle
    // (f16 at the matmul inputs) on steps 1–6; the F32 oracle stays shown next to it (the price of f16), not judged.
    let f32_check = recipe.then(|| oracle_513_check(&losses, &norms, &gpu_compute::train::ORACLE_513_SYNTH, F32_JUDGED, if mixed { TOL_MIXED } else { (TOL, TOL) }));
    let mixed_check = (recipe && mixed)
        .then(|| oracle_513_check(&losses, &norms, &gpu_compute::train::ORACLE_513_MIXED_SYNTH, MIXED_JUDGED, TOL_MIXED));
    if let Some(o) = &f32_check {
        let verdict = if mixed {
            "doar afisat (pretul f16; verdictul pasului mixt e randul urmator)"
        } else if o.pass {
            "ACELEASI NUMERE"
        } else {
            "DIFERA"
        };
        lines.push(format!(
            "{}: kronos --gpu: {}: fata de oracolul procesorului (ORACLE_513_SYNTH, loturile sintetice): {} — pasii 1-9: abaterea pierderii {:.1e}, a normei {:.1e} (toleranta {}); pasul 10 doar afisat: {:.1e} / {:.1e}",
            p, tag, verdict, o.worst.0, o.worst.1,
            if mixed { "1e-3 pierderea, 2e-3 norma: pasul mixt" } else { "1e-3" }, o.last.0, o.last.1
        ));
    }
    if let Some(o) = &mixed_check {
        lines.push(format!(
            "{}: kronos --gpu: {}: fata de oracolul pasului mixt (ORACLE_513_MIXED_SYNTH, loturile sintetice): {} — pasii 1-6: abaterea pierderii {:.1e}, a normei {:.1e} (toleranta 1e-3 / 2e-3); pasii 7-10 doar afisati: {:.1e} / {:.1e}",
            p, tag, if o.pass { "ACELEASI NUMERE (mixt)" } else { "DIFERA" }, o.worst.0, o.worst.1, o.last.0, o.last.1
        ));
    }
    // P6-261: on the real data, the verdict is oracle S141 (F32) on all 10 steps, with the tolerances written before the
    // run (the F32 step 1e-3; the mixed step as on the synthetic batches: the loss 1e-3, the norm 2e-3); PyTorch's own
    // bf16 run is shown beside it (how far bf16 alone moves the numbers).
    let real_check = if real && board {
        am::s141_tables(bsz).map(|(f32_t, bf16_t)| {
            let o = oracle_513_check(&losses, &norms, f32_t, f32_t.len(), if mixed { TOL_MIXED } else { (TOL, TOL) });
            let b = oracle_513_check(&losses, &norms, bf16_t, bf16_t.len(), (f64::INFINITY, f64::INFINITY));
            let (bl, bn): (Vec<f32>, Vec<f32>) = bf16_t.iter().copied().unzip();
            let tb = oracle_513_check(&bl, &bn, f32_t, f32_t.len(), (f64::INFINITY, f64::INFINITY));
            lines.push(format!(
                "{}: kronos --gpu: {}: date reale fata de oracolul S141 F32 (lot {}, samanta 1134, lr 5e-5): {} — pasii 1-10: abaterea pierderii {:.1e}, a normei {:.1e} (toleranta {}); pasul 1 {:.6} (S141 {:.6}); fata de S141 bf16: {:.1e} / {:.1e}; bf16 al PyTorch fata de F32: {:.1e} / {:.1e}",
                p, tag, bsz, if !o.pass { "DIFERA" } else if mixed { "ACELEASI NUMERE (mixt)" } else { "ACELEASI NUMERE" }, o.worst.0, o.worst.1,
                if mixed { "pierderea 1e-3, norma 2e-3: pasul mixt" } else { "1e-3" },
                losses.first().copied().unwrap_or(f32::NAN), f32_t[0].0, b.worst.0, b.worst.1, tb.worst.0, tb.worst.1
            ));
            o
        })
    } else {
        None
    };
    let oracle = if real_check.is_some() { real_check } else if mixed { mixed_check } else { f32_check };
    if let Some(o) = &oracle {
        journal.push(Event::Value { what: KT_513_ORACLE, value: u32::from(o.pass) });
    }
    let pass = if real && board {
        oracle.as_ref().is_some_and(|o| o.pass) && bad_w == 0 && losses.len() == steps
    } else if let Some(o) = &oracle {
        start_ok && o.pass && bad_w == 0 && losses.len() == steps
    } else if board {
        start_ok && falls && bad_w == 0 && losses.len() == steps
    } else {
        // P6-198: the mixed step against the F32 oracle with the mixed step's tolerances (as `--antrenare-mini --mixt`):
        // the loss 1e-3, the norm 2e-3; the weights only shown.
        let same = if mixed {
            worst_loss <= TOL_MIXED.0 && worst_norm <= TOL_MIXED.1
        } else {
            worst_loss <= TOL && worst_norm <= TOL && worst_w <= TOL
        };
        lines.push(format!(
            "{}: kronos --gpu: {}: lista placii pe bucati si parti fata de oracol: {} — abaterea pierderii {:.1e}, a normei {:.1e}, a greutatilor la sfarsit {:.1e} ({})",
            p, tag, if !same { "DIFERA" } else if mixed { "ACELEASI NUMERE (mixt)" } else { "ACELEASI NUMERE" }, worst_loss, worst_norm, worst_w,
            if mixed { "toleranta pasului mixt: pierderea 1e-3, norma 2e-3; greutatile doar afisate" } else { "toleranta 1e-3" }
        ));
        // P6-184: the check of the oracle verdict (the small model cannot use the #513 oracle itself).
        let self_ok = oracle_513_self_test();
        lines.push(format!(
            "{}: kronos --gpu: {}: verdictul fata de oracolul #513 (P6-184), verificat pe numerele lui: {}",
            p, tag, if self_ok { "MERGE (oracolul si os-395 trec, o abatere de 2e-3 si 9 pasi nu; mixt: os-445 trece pe pasii 1-6, nu si fata de F32)" } else { "GRESIT" }
        ));
        // P6-192: the dashboard's summary, checked on its own numbers.
        let dash_ok = super::gpu_golden::dash_summary_self_test();
        lines.push(format!(
            "{}: kronos --gpu: {}: rezumatul tabloului (P6-192), verificat pe numerele lui: {}",
            p, tag, if dash_ok { "MERGE" } else { "GRESIT" }
        ));
        self_ok && dash_ok && pieces_ok && same && bad_w == 0 && losses.len() == steps
    };
    journal.push(Event::Value { what: KT_513_PASS, value: u32::from(pass) });
    lines.push(format!(
        "{}: kronos --gpu: {}: {}",
        p, tag,
        match (pass, board) {
            (true, true) if real => "TRECE (pasul #513 pe datele reale, din greutatile #513 = oracolul S141 in pasii 1-10, toate numerele finite)",
            (true, true) if oracle.is_some() => "TRECE (pasul #513 cu arena in VRAM = oracolul procesorului in pasii 1-9, toate numerele finite)",
            (true, true) => "TRECE (pasul #513 cu arena in VRAM: pierderea porneste de la ln V si scade, toate numerele finite)",
            (true, false) => "TRECE (pasul #513 pe bucati si parti = oracolul, bucatile = urcarile planului)",
            _ => "NU TRECE (vezi randurile de mai sus)",
        }
    ));
    Ok(pass)
}

/// P6-187: one training run as the fair Windows <-> OS bench reads it (`docs/BANC.md`): the same fields PyTorch writes.
struct Bench<'a> {
    version: &'a str,
    model: &'a str,
    batch: usize,
    context: usize,
    precision: &'a str,
    data: String,
    attention: &'a str,
    /// The whole step on the CPU clock (batch up + board + losses down), median, µs.
    step_us: u64,
    /// The board's own time per step, median, µs.
    board_us: u64,
    losses: &'a [f32],
    norms: &'a [f32],
}

impl Bench<'_> {
    /// Two rows: `banc antrenare:` with fixed ` | `-separated `key value` fields, and `banc rand tabel:` ready to paste.
    fn lines(&self) -> [String; 2] {
        let tokens = (self.batch * self.context) as u64;
        let tok_s = if self.step_us > 0 { tokens * 1_000_000 / self.step_us } else { 0 };
        let ms = |us: u64| format!("{}.{:03}", us / 1000, us % 1000);
        let list = |v: &[f32]| v.iter().map(|x| format!("{:.4}", x)).collect::<Vec<String>>().join(" ");
        let first = self.losses.first().map_or(String::from("-"), |x| format!("{:.4}", x));
        let last = self.losses.last().map_or(String::from("-"), |x| format!("{:.4}", x));
        [
            format!(
                "banc antrenare: sistem OS {} | model {} | lot {} x {} | precizie {} | loturi {} | atentia {} | pasi {} | ms pe pas {} | simb/s {} | placa ms pe pas {} | pierderea {} | norma {}",
                self.version, self.model, self.batch, self.context, self.precision, self.data, self.attention,
                self.losses.len(), ms(self.step_us), tok_s, ms(self.board_us), list(self.losses), list(self.norms)
            ),
            format!(
                "banc rand tabel: | OS {} | {} | {} x {} | {} | {} | {} | {} | {} | {} -> {} |",
                self.version, self.model, self.batch, self.context, self.precision, self.data, self.losses.len(), ms(self.step_us), tok_s, first, last
            ),
        ]
    }
}

/// P6-184 (PREDARE Cyber-GPU `8de6436`, note `C-157_P2u_oracol-513.md`): the board's #513 run on the synthetic batches
/// against the CPU oracle measured in the cloud (`gpu_compute::train::ORACLE_513_SYNTH`, 4 decimals). Steps 1–9: loss and
/// norm within 1e-3 relative; step 10 (where the run jumps, norm 0.93 → 1.65) only shown. Valid only for exactly that
/// recipe (M513, batch 2 × 512, 10 steps, the synthetic batches with seed 0x164, AdamW lr 1e-3) — the caller checks.
/// P6-221: the mixed step against `ORACLE_513_MIXED_SYNTH` on steps 1–6 only (from step 7 the synthetic run is unstable).
struct Oracle513 {
    /// Worst relative deviation over the judged steps: (loss, norm).
    worst: (f64, f64),
    /// Worst relative deviation (loss, norm) over the remaining steps, shown only.
    last: (f64, f64),
    pass: bool,
}

/// Steps judged against the F32 oracle (1–9) and against the mixed oracle (1–6).
const F32_JUDGED: usize = 9;
const MIXED_JUDGED: usize = 6;

fn oracle_513_check(losses: &[f32], norms: &[f32], o: &[(f32, f32)], judged: usize, tol: (f64, f64)) -> Oracle513 {
    let rel = |x: f32, o: f32| {
        let (x, o) = (x as f64, o as f64);
        if x.is_finite() { ((x - o) / o).abs() } else { f64::INFINITY }
    };
    let n = o.len();
    let judged = judged.min(n);
    let complete = losses.len() == n && norms.len() == n;
    let mut worst = (0.0f64, 0.0f64);
    for k in 0..judged.min(losses.len()).min(norms.len()) {
        worst.0 = worst.0.max(rel(losses[k], o[k].0));
        worst.1 = worst.1.max(rel(norms[k], o[k].1));
    }
    let mut last = (0.0f64, 0.0f64);
    if complete {
        for k in judged..n {
            last.0 = last.0.max(rel(losses[k], o[k].0));
            last.1 = last.1.max(rel(norms[k], o[k].1));
        }
    } else {
        last = (f64::INFINITY, f64::INFINITY);
    }
    Oracle513 { worst, last, pass: complete && worst.0 <= tol.0 && worst.1 <= tol.1 }
}

/// P6-184: the check of the check, run in SIMULARE (the small model cannot use the #513 oracle): the oracle's own
/// numbers must pass, the os-395 board numbers must pass, one step-5 loss off by 2e-3 must fail, 9 steps must fail.
/// P6-221: the mixed oracle passes itself, os-445 (the mixed step on the board) passes it on steps 1–6 but not the F32
/// oracle, one step-3 loss off by 2e-3 fails.
fn oracle_513_self_test() -> bool {
    use gpu_compute::train::{ORACLE_513_MIXED_SYNTH as M, ORACLE_513_SYNTH as O};
    let (l, n): (Vec<f32>, Vec<f32>) = O.iter().copied().unzip();
    // os-395 on the board (the note's table): steps 8–10 differ.
    let mut lb = l.clone();
    let mut nb = n.clone();
    lb[7] = 9.9389;
    nb[7] = 0.9363;
    lb[8] = 9.9930;
    lb[9] = 10.1774;
    nb[9] = 1.6549;
    let mut lw = l.clone();
    lw[4] *= 1.002;
    const T: (f64, f64) = (1e-3, 1e-3);
    const TM: (f64, f64) = (1e-3, 2e-3);
    let f32_ok = oracle_513_check(&l, &n, &O, F32_JUDGED, T).pass
        && oracle_513_check(&lb, &nb, &O, F32_JUDGED, T).pass
        && !oracle_513_check(&lw, &n, &O, F32_JUDGED, T).pass
        && !oracle_513_check(&l[..9], &n[..9], &O, F32_JUDGED, T).pass;
    // os-445 on the board (`--mixt-513 --mixt-513-rapid`, the rows copied by Cyber-GPU in `os445-mixt-rapid.txt`).
    let l445 = [9.8493f32, 9.8749, 9.8293, 9.9085, 9.8648, 9.8816, 9.8498, 9.9936, 9.9207, 10.0178];
    let n445 = [3.2731f32, 1.1021, 1.0246, 0.9175, 0.8956, 0.8954, 0.9028, 0.9309, 0.9357, 1.0812];
    let (lm, nm): (Vec<f32>, Vec<f32>) = M.iter().copied().unzip();
    let mut lmw = lm.clone();
    lmw[2] *= 1.002;
    let mixed_ok = oracle_513_check(&lm, &nm, &M, MIXED_JUDGED, TM).pass
        && oracle_513_check(&l445, &n445, &M, MIXED_JUDGED, TM).pass
        && !oracle_513_check(&l445, &n445, &O, F32_JUDGED, TM).pass
        && !oracle_513_check(&lmw, &nm, &M, MIXED_JUDGED, TM).pass;
    f32_ok && mixed_ok
}

/// P6-175: what one resident run leaves for the comparison glued / not glued.
struct ResidentRun {
    losses: Vec<f32>,
    norms: Vec<f32>,
    weights: Vec<f32>,
    launches: usize,
    pass: bool,
    board_us: u64,
    /// P6-178: the whole step (median us) and its pieces, for the comparison window / RAM.
    host_us: u64,
    parts: Option<[u64; 5]>,
}

/// P6-175 (cyber-gpu T200 / T201: the glued step = the plain step bit for bit): both runs against the oracle, and the
/// glued one against the plain one — the loss and the norm at every step, the weights at the end, by bits.
fn fused_verdict(p: &str, journal: &mut Journal, lines: &mut Vec<String>, plain: &ResidentRun, glued: &ResidentRun) -> GpuResult<()> {
    let tag = "antrenare mini pe placa (arena in VRAM, lipit)";
    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
    let steps = plain.losses.len().min(glued.losses.len());
    let same_steps = (0..steps).filter(|&k| plain.losses[k].to_bits() == glued.losses[k].to_bits() && plain.norms.get(k).map(|x| x.to_bits()) == glued.norms.get(k).map(|x| x.to_bits())).count();
    let same_w = !plain.weights.is_empty() && bits(&plain.weights) == bits(&glued.weights);
    let diff_w = plain.weights.iter().zip(glued.weights.iter()).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
    let same = same_steps == steps && steps == plain.losses.len() && steps == glued.losses.len() && same_w;
    journal.push(Event::Value { what: KT_MINI_FUSED_SAME, value: same_steps as u32 });
    lines.push(format!(
        "{}: kronos --gpu: {}: lipit fata de nelipit: {} lansari pe pas in loc de {}; placa {} us fata de {} us pe pas (median); {} / {} pasi cu aceeasi pierdere si norma bit cu bit; greutatile la sfarsit {} ({} diferite din {})",
        p,
        tag,
        glued.launches,
        plain.launches,
        glued.board_us,
        plain.board_us,
        same_steps,
        steps,
        if same_w { "ACEIASI BITI" } else { "ALTI BITI" },
        diff_w,
        plain.weights.len()
    ));
    let pass = same && plain.pass && glued.pass;
    lines.push(format!(
        "{}: kronos --gpu: {}: {}",
        p,
        tag,
        if pass { "TRECE (lipit = nelipit bit cu bit, ambele = oracolul)" } else { "NU TRECE (vezi randurile de mai sus)" }
    ));
    if pass {
        Ok(())
    } else {
        Err(bad("antrenare-mini: pasul lipit difera de cel nelipit"))
    }
}

/// P6-178 (cyber-gpu T209: through RAM = through the window bit for bit, the arena identical): both runs against the
/// oracle, and the RAM one against the window one — the loss and the norm at every step, the weights at the end, by
/// bits — plus where the whole step's time went (cyber-gpu predicts ~4 150 -> ~3 800 us on the board).
fn ram_verdict(p: &str, journal: &mut Journal, lines: &mut Vec<String>, window: &ResidentRun, ram: &ResidentRun) -> GpuResult<()> {
    let tag = "antrenare mini pe placa (arena in VRAM, lot prin RAM)";
    let steps = window.losses.len().min(ram.losses.len());
    let same_steps = (0..steps).filter(|&k| window.losses[k].to_bits() == ram.losses[k].to_bits() && window.norms.get(k).map(|x| x.to_bits()) == ram.norms.get(k).map(|x| x.to_bits())).count();
    let same_w = !window.weights.is_empty() && window.weights.len() == ram.weights.len() && window.weights.iter().zip(ram.weights.iter()).all(|(a, b)| a.to_bits() == b.to_bits());
    let diff_w = window.weights.iter().zip(ram.weights.iter()).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
    let same = same_steps == steps && steps == window.losses.len() && steps == ram.losses.len() && same_w;
    journal.push(Event::Value { what: KT_MINI_RAM_SAME, value: same_steps as u32 });
    lines.push(format!(
        "{}: kronos --gpu: {}: prin RAM fata de prin fereastra: {} lansari pe pas in loc de {} (copierile lotului si ale rezultatelor in lista); pasul intreg {} us fata de {} us, placa {} us fata de {} us pe pas (median); {} / {} pasi cu aceeasi pierdere si norma bit cu bit; greutatile la sfarsit {} ({} diferite din {})",
        p,
        tag,
        ram.launches,
        window.launches,
        ram.host_us,
        window.host_us,
        ram.board_us,
        window.board_us,
        same_steps,
        steps,
        if same_w { "ACEIASI BITI" } else { "ALTI BITI" },
        diff_w,
        window.weights.len()
    ));
    if let (Some(a), Some(b)) = (window.parts, ram.parts) {
        let row = (0..5).map(|i| format!("{} {} -> {} us", MINI_PART_NAMES[i], a[i], b[i])).collect::<Vec<String>>().join(", ");
        lines.push(format!("{}: kronos --gpu: {}: bucatile pasului (median, fereastra -> RAM): {}", p, tag, row));
    }
    let pass = same && window.pass && ram.pass;
    lines.push(format!(
        "{}: kronos --gpu: {}: {}",
        p,
        tag,
        if pass { "TRECE (prin RAM = prin fereastra bit cu bit, ambele = oracolul)" } else { "NU TRECE (vezi randurile de mai sus)" }
    ));
    if pass {
        Ok(())
    } else {
        Err(bad("antrenare-mini: lotul prin RAM difera de cel prin fereastra"))
    }
}

/// P6-99: positions per second × 100 with the check: every position's OS time (board + driver + journal) plus its check.
fn with_check_x100(split: &[kf::TokenSplit]) -> Option<u64> {
    let total: u64 = split.iter().map(|t| t.host_us + t.check_us).sum();
    (total > 0).then(|| (split.len() as u128 * 100 * 1_000_000 / total as u128) as u64)
}

/// The CPU engine's median time per position (us) and positions per second × 100 over all of them.
fn cpu_speed(us: &[u64]) -> (Option<u64>, Option<u64>) {
    if us.is_empty() {
        return (None, None);
    }
    let mut v = us.to_vec();
    v.sort_unstable();
    let total: u64 = us.iter().sum();
    let x100 = (total > 0).then(|| (us.len() as u128 * 100 * 1_000_000 / total as u128) as u64);
    (Some(v[v.len() / 2]), x100)
}

/// P6-173: the five pieces of one resident step (us), from the step's start and the five marks; `None` if the clock
/// was missing at any mark.
fn mini_parts(t0: Option<u64>, marks: &[Option<u64>; 5]) -> Option<[u64; 5]> {
    let mut prev = t0?;
    let mut out = [0u64; 5];
    for (i, m) in marks.iter().enumerate() {
        let now = (*m)?;
        out[i] = now.saturating_sub(prev);
        prev = now;
    }
    Some(out)
}

/// P6-173: `planul lotului 12 us, lotul sus 3 us, …`.
fn mini_parts_text(v: &[u64; 5]) -> String {
    MINI_PART_NAMES.iter().zip(v.iter()).map(|(n, u)| format!("{} {} us", n, u)).collect::<Vec<String>>().join(", ")
}

/// P6-173: `pushbuffer 5 us, golirea in VRAM 120 us, …`.
fn kick_gaps_text(v: &[u64; 6]) -> String {
    KICK_NAMES.iter().zip(v.iter()).map(|(n, u)| format!("{} {} us", n, u)).collect::<Vec<String>>().join(", ")
}
