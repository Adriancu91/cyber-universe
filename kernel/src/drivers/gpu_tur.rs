//! P6-42 (command C-55): `gpu tur` and `gpu lant`, the OS side of Cyber-GPU's C-54
//! (`integration/cyber-universe/C-54_gpu-tur-si-lant.md`, hub proposal `idei/2026-11-01_cyber-gpu-tur-si-lant.md`).
//!
//! - `gpu tur`: READ ONLY. `nvidia_ga104::read_tour::run_tour` through the read-only configuration adapter and the
//!   read-only 16 MiB BAR0 window: PCI identity, BOOT_0/42, GFW, the VBIOS layout, FWSEC, the I6 preparation. One line
//!   `fapt <key> = <value>` per fact (the format `compara_raport` of Cyber-GPU reads back), then the write count, which
//!   must be 0. On the allow-list of the automatic loop (C-53).
//! - `gpu lant`: the window steps one after the other. P6-55 (C-71): the whole window chain of Cyber-GPU
//!   (`run_chain_ready(WINDOW_CHAIN_C83, READY_IN_OS, ..)`, I6 to the first computation). The OS has the I6
//!   preparation, Bus Master on, FWSEC-FRTS, the WPR2 read after I6 (`i6-wpr2-citit`, reads only), Bus Master back and
//!   the tour after; P6-56 (C-72): I7 (Booter on SEC2, `gpu_i7.rs`) and `i7-citit`; step 2: I8 (`gpu_i8.rs`: SetSystemInfo + SetRegistry, the doorbell, the wait for GSP_INIT_DONE); step 2b: golden + I9 (`gpu_golden.rs`: RM messages with the GSP doorbell each, page tables in VRAM through the VRAM window); step 3: I10 (`gpu_i10.rs`: `add64` on the work channel, the TLB invalidate, the work-channel doorbell, the result bit for bit; the launch cost). The driver owns the order and the stop rules; this file only does each step. Writes are allowed
//!   only in the test-stick build (`gpu-scrie`) and only after `DA` (asked by the shell); in every other build the
//!   chain runs read only (it stops before the first writing step, then the tour). Refused by the automatic loop.
//!
//! In test builds both run on the gpu-sim models (`--sim`): the tour on `gpu_sim::observed` (the model built from the
//! values the workstation board gave), the chain with the falcon model reading the OS's real DMA area (as in P6-40).
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml).

#![cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell

use super::gpu::{self, ReadOnlyBar0Window, ReadOnlyConfig, TscClock, WINDOW_LOCK};
use super::gpu_i6::{self, finish};
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_i7;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_i8;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_golden;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_i10;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_i11;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
use super::gpu_i14;
use alloc::{format, string::String, vec, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, ConfigSpace, Mmio};
use gpu_core::PciAddress;
use nvidia_ga104::chain::{run_chain_ready, ChainReport, WINDOW_CHAIN_C83};

/// P6-62 (C-83): the plan steps the OS runs in the window chain: I5 to I14 (I14 with Kronos's tiny control model first,
/// `gpu_i14`; #513 comes with P6-60).
const READY_IN_OS: &[&str] = &["I5", "I6", "I7", "I8", "I9", "I10", "I11", "I12", "I13", "I14"];
use nvidia_ga104::gsp::falcon;
use nvidia_ga104::read_tour::run_tour;

/// The tour on any board (real or model): one line per fact, then a summary. Errors are returned as they are.
fn tour_on<C: ConfigSpace + ?Sized, M: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    cfg: &mut C,
    addr: Option<PciAddress>,
    bar0: &mut M,
    clock: &mut K,
    journal: &mut Journal,
    lines: &mut Vec<String>,
) -> GpuResult<()> {
    let mut t = run_tour(cfg, addr, bar0, clock, journal)?;
    i6_result_fact(&mut t.facts, bar0, journal);
    for f in &t.facts {
        lines.push(format!("{}", f));
    }
    lines.push(format!("{}: tur de citire (C-54): {} fapte, {} citiri jurnalizate", p, t.facts.len(), t.reads));
    Ok(())
}

/// C-72 (Adrian, 5 nov): the library's fact `i6.trece` is the I6 *pre-check* ("WPR2 down, I6 may run"), so after a
/// successful I6 on the board it said `nu`. In the OS tour it now means the I6 *result*: with WPR2 raised, the three
/// status reads of `read_frts_after_i6` against the FRTS region of the same layout `i6_check` uses (VRAM from the fact
/// `i6.vram`, sizes of the official files) decide `da`/`nu`; with WPR2 down it is `nu (inca nefacut ...)`. The
/// pre-check keeps its own fact, `i6.pregatire`. Reads only.
fn i6_result_fact<M: Mmio + ?Sized>(facts: &mut Vec<nvidia_ga104::read_tour::Fact>, bar0: &mut M, journal: &mut Journal) {
    use nvidia_ga104::fb_layout::{FbLayout, FbParams};
    use nvidia_ga104::firmware::{GspRole, GSP_570_144};
    let get = |facts: &[nvidia_ga104::read_tour::Fact], k: &str| facts.iter().find(|f| f.key == k).map(|f| f.value.clone());
    let (Some(wpr2), Some(vram)) = (get(facts, "i6.wpr2"), get(facts, "i6.vram")) else { return };
    let value = if !wpr2.starts_with("RIDICAT") {
        String::from("nu (inca nefacut: WPR2 coborat)")
    } else {
        let size = |r: GspRole| GSP_570_144.iter().find(|f| f.role == r).map(|f| f.size).unwrap_or(0);
        let mib: u64 = vram.trim_end_matches(" MiB").parse().unwrap_or(0);
        let frts = FbLayout::compute(&FbParams {
            fb_size: mib << 20,
            vga_workspace_reg: None,
            bootloader_size: size(GspRole::Bootloader),
            gsp_elf_size: size(GspRole::GspRm),
        })
        .map(|l| (l.frts.start, l.frts.end));
        // C-75: after I7, Booter widens WPR2 for GSP-RM (board, 5 nov: 0x1f4100000..0x1ffee0000); the library's
        // fact `i6.wpr2.etapa` names that stage, and it means I6 had passed before.
        let stage = get(facts, "i6.wpr2.etapa").unwrap_or_default();
        match frts.and_then(|f| falcon::read_frts_after_i6(bar0, journal, f)) {
            Ok(c) => format!("da (WPR2 = regiunea FRTS ceruta, capat: {})", c.form.label()),
            Err(_) if stage.starts_with("largit") => String::from("da (WPR2 largit de Booter pentru GSP-RM dupa I7: I6 trecuse)"),
            Err(e) => format!("nu (WPR2 ridicat, dar: {})", e),
        }
    };
    if let Some(i) = facts.iter().position(|f| f.key == "i6.trece") {
        let pre = core::mem::replace(&mut facts[i].value, value);
        facts.insert(i, nvidia_ga104::read_tour::Fact::new("i6.pregatire", pre));
    }
}

/// The chain report, one line per step, with the board name in front.
fn report_lines(p: &str, r: &ChainReport, lines: &mut Vec<String>) {
    lines.push(format!("{}: lantul pasilor (C-54):", p));
    for l in format!("{}", r).lines() {
        lines.push(format!("{}: {}", p, l.trim_start()));
    }
}

/// P6-55: the error of a step of the full chain that the OS does not have yet (I7-I10).
const NOT_IN_OS: &str = "pas fara implementare in OS";

/// P6-56 step 3 (I10): one write of the work-channel doorbell through its fence, journaled. The register is not read
/// (before/after in the journal are 0, with a note): RM only writes it.
#[cfg(feature = "gpu-scrie")]
fn ring_work_doorbell(db: &mut gpu_i6::hw::DoorbellFence, j: &mut Journal, off: u64, v: u32) -> GpuResult<()> {
    let before = db.rung;
    gpu_core::hal::checked_write32(db, off, v)?;
    if db.rung != before + 1 {
        return Err(GpuError::NotPermitted("soneria canalului refuzata de gard"));
    }
    j.push(gpu_core::diag::Event::Note("soneria canalului de lucru (I10): scriere fara recitire"));
    j.push(gpu_core::diag::Event::MmioWrite { bar: 0, offset: off, before: 0, written: v, after: 0 });
    Ok(())
}

/// P6-55 (C-71): step `i6-wpr2-citit` — WPR2 read after I6 with reads only (`falcon::read_frts_after_i6`, no permit)
/// and checked against the FRTS region asked for; the line names the form of the end the board wrote.
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
fn wpr2_step<M: Mmio + ?Sized>(
    p: &str,
    mmio: &mut M,
    j: &mut Journal,
    frts: (u64, u64),
    lines: &mut Vec<String>,
) -> GpuResult<()> {
    let c = falcon::read_frts_after_i6(mmio, j, frts)?;
    lines.push(format!(
        "{}: pas i6-wpr2-citit: WPR2 {:#x}..{:#x} = regiunea FRTS ceruta (capat: {})",
        p,
        c.wpr2.start,
        c.wpr2.end,
        c.form.label()
    ));
    Ok(())
}

/// The chain's error for a step whose detail is already in `lines` (the library's errors carry only static text).
fn step_failed(what: &'static str) -> GpuError {
    GpuError::Blocked(what)
}

// ---------------------------------------------------------------------------------------------------------------
// The real board.
// ---------------------------------------------------------------------------------------------------------------

/// `gpu tur`: read only, on the real board. The journal goes live to the stick when one is writable.
pub fn tour() -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
    // P6-85 (C-116): the laptop's GA106 has its own read-only path (`run_tour_ampere`); GA104 is unchanged.
    if let Some(d) = ga106_board() {
        return tour_ampere_board(p, d, false, lines);
    }
    let Some((d, bar0)) = gpu_i6::board(&mut lines) else { return finish(lines) };
    let live = match gpu_i6::live_open("gpu tur") {
        Ok(_) => true,
        Err(why) => {
            lines.push(format!("{}: jurnalul pe loc: nu (stick: {}); doar in log", p, why));
            false
        }
    };
    let mut j = gpu_i6::journal(live);
    let addr = PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
    let (refused, remaps, map_failed) = {
        let _guard = WINDOW_LOCK.lock();
        let mut cfg = ReadOnlyConfig { dev: &d, refused: 0 };
        let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
        if let Err(e) = tour_on(p, &mut cfg, Some(addr), &mut win, &mut TscClock, &mut j, &mut lines) {
            lines.push(format!("{}: tur de citire: {}", p, e));
        }
        (cfg.refused + win.refused + j.writes(), win.remaps, win.map_failed)
    };
    gpu_i6::close_and_check(p, false, &mut lines);
    if map_failed {
        lines.push(format!("{}: maparea ferestrei a esuat (citirile au dat FFFFFFFF)", p));
    }
    lines.push(format!("{}: {} pagini mapate pe rand (doar citire, fereastra de 16 MiB)", p, remaps));
    lines.push(gpu::writes_line(refused));
    finish(lines)
}

/// The line naming the chosen I8 variant (P6-78, C-108), for the screen and the result lines.
pub fn variant_line(nr: Option<u8>) -> String {
    match nr.and_then(nvidia_ga104::gsp::variants::get) {
        Some(v) => format!("varianta I8 {}: {} — {} (sursa: {})", v.nr, v.name, v.changes, v.source),
        // cyber-gpu e49ca76: without `--varianta` the library uses its default variant (1, the one that passed I8 on the board).
        None => {
            let d = nvidia_ga104::gsp::variants::default_variant();
            format!("varianta I8 implicita: {} ({})", d.nr, d.name)
        }
    }
}

/// `gpu variante` (P6-78, C-108): the numbered I8 variants from Cyber-GPU. No card access.
pub fn variants_list() -> Vec<String> {
    let mut l = Vec::new();
    l.push(format!(
        "variantele I8 (C-108, cyber-gpu {}): una pe pornire rece, cu `gpu lant --varianta N`; fara --varianta = varianta {} (implicita)",
        gpu::CYBER_GPU_REV,
        nvidia_ga104::gsp::variants::DEFAULT
    ));
    for v in nvidia_ga104::gsp::variants::list() {
        l.push(format!("{}. {} — {} (sursa: {})", v.nr, v.name, v.changes, v.source));
    }
    l.push(String::from("(nicio placa atinsa)"));
    l
}

/// `gpu lant` on the real board (the shell asked for `DA` first). Writes only in the `gpu-scrie` build.
/// P6-78 (C-108): `variant` = `--varianta N` (already checked by the shell); `None` = the library's default variant (1 since cyber-gpu e49ca76).
/// P6-106 (C-148 completion 1): set by the cycle (`ciclu::board_test`) around its chains — only then is the board asked for
/// its maximum speed (C-147 point 4). Off everywhere else: `gpu lant` and `kronos --gpu` by hand behave as before.
pub static BOOST_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-107 (C-154, cyber-gpu C-147 point 7): the driver's diet (the pushbuffer through a shadow, the records not zeroed)
/// for `kronos --gpu`. Set only by the cycle (`ciclu::board_test`) around its ordinary test (not the FLR trial's second
/// chain) and, in the test build, by `gpu dieta-sim on`. Off everywhere else: `kronos --gpu` by hand is as in os-323.
pub static DIET_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-110 (C-155 point 1, cyber-gpu `6f0fdbf`): P3, the glued launch list (88 launches per token at #513 instead of 196,
/// the new program `q8_fused`) for `kronos --gpu`. Set only by the cycle when the list asks for it (`test-placa … --lipit`)
/// and, in the test build, by `gpu lipit-sim on`. Then the same text runs twice: first the plain list (as in os-324), then
/// the glued one, and their logits are compared bit for bit. Off by default: every other run is exactly as in os-324.
pub static FUSED_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-112 (center's note 21 Nov 13:00, after os-328 passed): the cycle's ordinary test sends #513 with the glued list
/// alone (no comparison). Set by `ciclu::board_test` unless the list says `--lipit` (both, compared) or `--simplu` (the
/// plain list alone, as in os-324), and in the test build by `gpu lipit-sim singur`. Off by hand.
pub static FUSED_ONLY_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-114 (C-155 point 2, cyber-gpu `139b145`): the new LayerNorm (`layernorm256`: 256 threads, one read, `shfl` +
/// barrier) in the glued list (`TokenList::FusedLn`). Set only by the cycle when the list asks for it (`test-placa …
/// --ln-nou`) and, in the test build, by `gpu lipit-sim ln`. Then the same text runs twice: first the glued list with the
/// old LayerNorm (as in os-329), then with the new one, compared (the note's threshold: the same tokens and every logit
/// within 1e-4; not bit for bit, the sums run in another order). Off by default.
pub static LN_NEW_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-120 (center's proposal 21 Nov 19:25; cyber-gpu note `C-157_P2d_os-331.md`): the new LayerNorm alone
/// (`TokenList::FusedLn`, no comparison) — the cycle's default from os-336 on (it passed on the board twice, os-331 and
/// os-332). Set by `ciclu::board_test` unless the list picks another list (`--lipit`, `--simplu`, `--ln-nou`, `--ln-vechi`)
/// and, in the test build, by `gpu lipit-sim ln-singur`. Off by hand.
pub static LN_ONLY_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-124 (C-155 point 4, cyber-gpu `9d256ca`, note `C-155_cursa-pe-sferturi.md`): the race of the products in the token
/// list: the same text twice, the glued list (`TokenList::Fused`, old LayerNorm, as in os-329) then the glued list on
/// quarters (`TokenList::Fused32`: the 4 glued products per layer on `q8_fused32`, the logits on `q8_matvec32`; the same
/// 88 launches and cbufs). Compared: the same tokens and every logit within 1e-4 (not bit for bit), then the time per kind
/// of launch of both runs (a kind passes on quarters only if faster by at least 50 per mille). Set only by the cycle when
/// the list asks for it (`test-placa … --sferturi`) and, in the test build, by `gpu lipit-sim sferturi`. Off by default.
pub static QUARTERS_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-128 (C-157 point 2, cyber-gpu `feb0819`, note `C-157_P2g_sferturi.md`): the list Cyber-GPU chose after the race on
/// quarters passed on the board (cycle 22 Nov 05:06): the same text twice, `lipita-ln` (`TokenList::FusedLn`, the cycle's
/// default) then `lipita-ln-32` (`TokenList::FusedLn32`: the new LayerNorm and every product on quarters; the same 88
/// launches and cbufs, programs that all ran on the board already). Compared like `QUARTERS_IN_CYCLE` (the same tokens,
/// every logit within 1e-4, then the kinds of launch at 50 per mille). Set only by the cycle (`test-placa … --ln-32`)
/// and, in the test build, by `gpu lipit-sim ln-32`. Off by default.
pub static LN32_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-135 (cyber-gpu note `C-157_P2i_os-346_ln-32.md`, center's proposal 22 Nov 10:30): `lipita-ln-32` alone
/// (`TokenList::FusedLn32`, no comparison) — the cycle's default from this Release on (it passed on the board twice, 05:06
/// and 10:06). Set by `ciclu::board_test` unless the list picks another list (`--lipita-ln` = the default until now) and,
/// in the test build, by `gpu lipit-sim ln-32-singur`. Off by hand.
pub static LN32_ONLY_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-119 (C-157 point 3, cyber-gpu `367edf4`): the driver's second diet for `kronos --gpu` (two records read on most
/// positions, the planned wait, the logits' fingerprint after the token's clock, the argmax on bits). Nothing new for
/// the board. Set only by the cycle when the list asks for it (`test-placa … --dieta2`) and, in the test build, by
/// `gpu dieta-sim 2`. Off by default: every other run is as in os-332.
pub static DIET2_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-123 (C-155 point 3, cyber-gpu `3683ef4`): the argmax on the board, the trial (the note's point 3): every token of
/// `kronos --gpu` ends with the copy of the logits AND the `argmax` program (one launch more, its 16-byte result in the
/// logits' RAM arena); at every position the 4 words must equal `argmax_result` of the copied logits (threshold 0). The
/// token is still chosen on the processor, as before. Set only by the cycle when the list asks for it (`test-placa …
/// --argmax-placa`) and, in the test build, by `gpu argmax-sim on`. Off by default: every other run is as in os-339.
pub static ARGMAX_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-126 (C-155 point 4, cyber-gpu note C-157_P2f `dc4edf0`): the argmax on the board WITHOUT the copy of the logits:
/// every token of `kronos --gpu` ends with the `argmax` program only, the OS reads its 16 bytes and takes the token
/// from them (`kronos_gen::TopRunner`); after the board, the processor computes every position's token (the check after
/// the board, as today) and they must be the same. No logits are read (no print, no deviation). Set only by the cycle
/// when the list asks for it (`test-placa … --argmax-fara-copie`) and, in the test build, by `gpu argmax-sim fara-copie`.
pub static ARGMAX_TOP_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// P6-145 (C-157 point 7, cyber-gpu `0d22357`): after the texts of `kronos --gpu` with the argmax without the copy, the
/// same generation again in batches (`kronos_batch`): the token stays on the board (`embed_dev`), several positions on ONE
/// doorbell — first batches of 4, then the whole text (or as many positions as the pushbuffer area holds). Set only by the
/// cycle when the list asks for it (`test-placa … --argmax-fara-copie --dieta2 --lot`) and, in the test build, by
/// `gpu lot-sim on`. Off by default.
pub static BATCH_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-117 (C-161 point 2, cyber-gpu `3199fa2`): P8, the first tensor-core computation (`mma16_f16`, 6 sets), after
/// `kronos --gpu`'s texts. Set only by the cycle when the list asks for it (`test-placa … --p8`) and, in the test
/// build, by `gpu p8-sim on`. Off by default: every other run sends exactly what os-332 sent.
pub static P8_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-134 (C-161 point 4, cyber-gpu `3b4ca77`): the first backward program of the training on the board, the `colsum`
/// probe (5 fixed sets), after `kronos --gpu`'s texts (and after P8 when asked). Set only by the cycle when the list
/// asks for it (`test-placa … --colsum`) and, in the test build, by `gpu colsum-sim on`. Off by default.
pub static COLSUM_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-136 (C-161 point 4, cyber-gpu `dd67483`): the small golden step, whole, on the board (13 programs, 10 steps of
/// `sgd` and of `adamw_513`, judged against PyTorch), after `kronos --gpu`'s texts (and after P8 / `colsum` when asked).
/// Set only by the cycle when the list asks for it (`test-placa … --pas-aur`) and, in the test build, by
/// `gpu pas-aur-sim on`. Off by default.
pub static GOLDEN_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-137 (C-161 point 4, cyber-gpu `54563a5`): the `mm_tc` probe (the training matmul on the tensor cores: 5 fixed sets
/// for correctness + `qkv-513` for the first time of a fast training matmul), after `kronos --gpu`'s texts (and after
/// P8 / `colsum` / the golden step when asked). Set only by the cycle when the list asks for it (`test-placa … --mm-tc`)
/// and, in the test build, by `gpu mm-tc-sim on`. Off by default.
pub static MM_TC_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-141 (C-161 point 4, cyber-gpu `44a4d16`): the tiled training attention probe (`attn_fwd_tile` → `attn_back_tile0`
/// → `attn_back_tile` on 5 small sets + `atentie-513-b4` for the time), after `kronos --gpu`'s texts (and after the other
/// probes when asked). Set only by the cycle when the list asks for it (`test-placa … --atentie-placi`) and, in the test
/// build, by `gpu atentie-placi-sim on`. Off by default.
pub static ATTN_TILE_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-149 (cyber-gpu note C-157_P2m): with the tiled attention probe, after the old chain on every set, the second
/// generation (`attn_fwd_tile2` → `attn_back_tile0_2` → `attn_back_tile_2`) on the same buffer, judged with the same
/// margins and compared word for word with the old. Set only by the cycle (`test-placa … --atentie-placi --atentie2`)
/// and, in the test build, by `gpu atentie2-sim on`. Off by default.
pub static ATTN_TILE2_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-160 (cyber-gpu note C-157_P2n, `d885efd`): after the second generation, the third (`attn_fwd_tile3` →
/// `attn_back_tile0_3` → `attn_back_tile_3`, branch-free global loads) on the same buffer, judged with the same margins,
/// compared word for word with the first and timed beside the second. Only with [`ATTN_TILE2_IN_CYCLE`]. Set only by the
/// cycle (`test-placa … --atentie-placi --atentie2 --atentie3`) and, in the test build, by `gpu atentie3-sim on`.
pub static ATTN_TILE3_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-161 (C-157, cyber-gpu `9359f21`): the RAM ↔ VRAM probe with the copy engine — the copy object allocated on the
/// work channel right after I9 (one RM message), then, after `kronos --gpu`'s texts, one submission of copies host RAM ↔
/// VRAM timed and checked byte for byte (`kt ramvram …`). Set only by the cycle (`test-placa … --ram-vram`) and, in the
/// test build, by `gpu ram-vram-sim on`. Off by default.
pub static RAM_VRAM_IN_CYCLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-101: set only for the FLR trial's second chain (`test-placa --flr-proba`, `gpu lant --flr-proba`); taken by
/// `chain`. Without it no chain reads WPR2 for the reset nor writes anything for it.
pub static FLR_ARMED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-105 (C-152 point 5, C-151 point 1): set only around the cycle's test when the list asks for it
/// (`test-placa … --tablou`) or by `gpu lant --tablou` (with DA): the board's dashboard is started, read once and
/// stopped at the end of the chain, after I14 and the `ks` rows. Off by default: every other chain is as in os-322.
pub static RUSD_ARMED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-142 (C-157 point 5, cyber-gpu `1922eb2`): with `RUSD_ARMED`, only in the cycle (`test-placa … --tablou-viteza`):
/// the dashboard is started right after the work channel, BEFORE the max-speed request, read before it and 1 s after
/// it (`compare_boost`), kept on and read a third time at the end of the chain; every read goes through the thermal
/// watch. "Stop" at the first read = the max-speed request is not sent. Off by default.
pub static RUSD_EARLY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-105: the last dashboard read in this power-on (the `kd` screen lines), for `gpu stare`.
pub static LAST_DASHBOARD: crate::sched::Mutex<Option<Vec<String>>> = crate::sched::Mutex::new(None);

/// P6-105: `gpu stare` — the last dashboard read in this power-on; the board is not touched.
pub fn dashboard_status() -> Vec<String> {
    match LAST_DASHBOARD.lock().as_ref() {
        Some(l) => {
            let mut v = vec![String::from("gpu stare: tabloul de bord citit la sfarsitul ultimului lant (placa nu e atinsa acum):")];
            v.extend(l.iter().cloned());
            v
        }
        None => vec![String::from(
            "gpu stare: niciun tablou de bord citit in aceasta pornire (se citeste doar la sfarsitul lantului: `test-placa --tablou` sau `gpu lant --tablou`); placa nu e atinsa",
        )],
    }
}

/// P6-105: the 4 KiB page of system memory given to GSP-RM for the dashboard (contiguous, zero, 4 KiB aligned, below
/// 4 GiB, away from boot-time memory). Freed only when GSP-RM confirmed the stop.
#[cfg(feature = "gpu-scrie")]
struct RusdPage {
    phys: u64,
    ptr: *const u8,
}

#[cfg(feature = "gpu-scrie")]
impl RusdPage {
    fn new() -> Option<Self> {
        let phys = crate::mem::frame::alloc_contiguous_between(1, 16 << 20, 4 << 30)?;
        let ptr = crate::mem::map::phys_to_virt(phys);
        // SAFETY: the frame was just allocated for us (4 KiB, mapped through the HHDM); nothing else uses it yet.
        unsafe { core::ptr::write_bytes(ptr, 0, 4096) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        Some(RusdPage { phys, ptr })
    }
}

#[cfg(feature = "gpu-scrie")]
impl nvidia_ga104::gsp::rusd::RusdMem for RusdPage {
    fn read_u64(&mut self, o: usize) -> u64 {
        if o % 8 != 0 || o + 8 > 4096 {
            return 0;
        }
        // SAFETY: o + 8 <= 4096 and o is a multiple of 8 (checked above); the page is ours and mapped while it lives.
        let v = unsafe { core::ptr::read_volatile(self.ptr.add(o) as *const u64) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        v
    }
    fn read_bytes(&mut self, o: usize, out: &mut [u8]) {
        if o + out.len() > 4096 {
            out.fill(0);
            return;
        }
        for (i, b) in out.iter_mut().enumerate() {
            // SAFETY: o + out.len() <= 4096 (checked above), the same page.
            *b = unsafe { core::ptr::read_volatile(self.ptr.add(o + i)) };
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
    }
}

/// P6-105 (SIMULARE): the page as the RM model rendered it during the wait.
#[cfg(feature = "test-gpu-sim")]
struct SimRusdPage(Vec<u8>);

#[cfg(feature = "test-gpu-sim")]
impl nvidia_ga104::gsp::rusd::RusdMem for SimRusdPage {
    fn read_u64(&mut self, o: usize) -> u64 {
        self.0.get(o..o + 8).map_or(0, |b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    fn read_bytes(&mut self, o: usize, out: &mut [u8]) {
        match self.0.get(o..o + out.len()) {
            Some(b) => out.copy_from_slice(b),
            None => out.fill(0),
        }
    }
}

/// The page's address in SIMULARE (a system-memory address the RM model only records).
#[cfg(feature = "test-gpu-sim")]
const SIM_RUSD_PHYS: u64 = 0x0200_0000;

/// P6-142 (SIMULARE, DEDUS): the core clock the dashboard's model shows after an accepted max-speed request
/// (the RTX 3070 Ti's boost clock, NVIDIA's public sheet: 1 770 MHz).
#[cfg(feature = "test-gpu-sim")]
const SIM_BOOST_CLK_MHZ: u32 = 1770;

/// P6-142 (SIMULARE): the page as the RM model renders it now (all zero when the model is not polling).
#[cfg(feature = "test-gpu-sim")]
fn sim_render(rm: &gpu_sim::fake_rm::FakeRm, clock: &mut impl Clock, page: &mut SimRusdPage) {
    if let Some(m) = rm.rusd.as_ref() {
        let now = clock.now_us().saturating_mul(1000).max(1);
        let b = m.render(now);
        page.0[..b.len()].copy_from_slice(&b);
    }
}

/// P6-105 (SIMULARE): `gpu lant --sim --tablou-refuz` = the RM model refuses the start (as the board might refuse our
/// client, C-151 note).
#[cfg(feature = "test-gpu-sim")]
pub static SIM_RUSD_RULE: crate::sched::Mutex<Option<gpu_sim::fake_rm::RusdRule>> = crate::sched::Mutex::new(None);

/// The FLR permit's reason (C-152 point 1, Adrian's "Da" for step 1, 19 Nov 23:37).
#[cfg_attr(not(feature = "gpu-scrie"), allow(dead_code))]
const FLR_REASON: &str = "C-152: reset FLR in ciclu (aprobat de Adrian 19 nov)";

pub fn chain(variant: Option<u8>, i9: super::gpu_golden::I9Choice) -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
    let settings = match nvidia_ga104::gsp::variants::settings(variant) {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!("{}: {}; nimic scris in placa", p, e));
            return finish(lines);
        }
    };
    lines.push(format!("{}: {}", p, variant_line(variant)));
    lines.push(format!("{}: {}", p, super::gpu_golden::i9_line(i9)));
    let Some((d, bar0)) = gpu_i6::board(&mut lines) else { return finish(lines) };
    let writes_allowed = cfg!(feature = "gpu-scrie");
    #[cfg(feature = "gpu-scrie")]
    let permit = match nvidia_ga104::gsp::HardwareWritePermit::new("C-55 gpu lant: fereastra cu Adrian (P6-42)") {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!("{}: {}", p, e));
            return finish(lines);
        }
    };
    if !writes_allowed {
        lines.push(format!(
            "{}: acest sistem nu poate scrie in placa (doar stick-ul de test, gpu-scrie): lantul ruleaza doar citire",
            p
        ));
    }
    let live = match gpu_i6::live_open("gpu lant") {
        Ok(_) => true,
        Err(why) if writes_allowed => {
            lines.push(format!("{}: jurnalul pe stick nu se poate scrie ({}): STOP, nimic scris in placa", p, why));
            return finish(lines);
        }
        Err(why) => {
            lines.push(format!("{}: jurnalul pe loc: nu (stick: {}); doar in log", p, why));
            false
        }
    };
    let mut j = gpu_i6::journal(live);
    settings.journal(variant, &mut j);
    let mut clock = TscClock;
    let addr = PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
    let _guard = WINDOW_LOCK.lock();
    // P6-101 (C-151 point 3, C-152 point 1): before the chain, the reset from inside when WPR2 is already up (a chain
    // ran earlier in this power-on). WPR2 = 0: nothing written. Only when the FLR trial asked for it (`FLR_ARMED`, taken
    // here): every other chain is exactly as before (the speed and the reset are tested in separate Releases).
    #[cfg(feature = "gpu-scrie")]
    if FLR_ARMED.swap(false, core::sync::atomic::Ordering::SeqCst) {
        let flr = gpu_pci::flr::FlrPermit::new(FLR_REASON);
        let mut board = gpu_i6::hw::FlrBoardHw {
            cfg: gpu_i6::hw::FlrConfig::new(&d),
            bar0: ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW),
        };
        let r = nvidia_ga104::flr_reset::reset_if_wpr2_up(&mut board, &mut clock, &flr, Some(&permit), &mut j);
        let refused_flr = board.cfg.refused + board.bar0.refused;
        drop(board);
        lines.push(format!("{}: {}", p, r.row()));
        lines.push(format!("{}: resetul din interior (C-151): {}", p, r.verdict.meaning()));
        if refused_flr > 0 {
            lines.push(format!("{}: gardul FLR a REFUZAT {} scrieri", p, refused_flr));
        }
        if !r.verdict.chain_may_run() {
            drop(_guard);
            gpu_i6::close_and_check(p, false, &mut lines);
            lines.push(format!("{}: rezultat lant: oprit la resetul din interior ({}); fara a doua incercare", p, r.verdict.code()));
            return finish(lines);
        }
    }
    let mut prepared: Option<(nvidia_ga104::fwsec::FwsecLocation, nvidia_ga104::i6_check::I6Check)> = None;
    #[cfg(feature = "gpu-scrie")]
    let mut old: Option<u16> = None;
    #[cfg(feature = "gpu-scrie")]
    let bm = gpu_pci::bus_master::BusMasterPermit::new("C-55 gpu lant (P6-42)");
    #[cfg(feature = "gpu-scrie")]
    let mut i7: Vec<gpu_i7::I7Prep> = Vec::new();
    #[cfg(feature = "gpu-scrie")]
    let mut gs: Option<gpu_golden::GpuState> = None;
    #[cfg(feature = "gpu-scrie")]
    let mut launched: Option<gpu_i10::Launched> = None;
    #[cfg(feature = "gpu-scrie")]
    let mut boost: Option<nvidia_ga104::gsp::perf::BoostStart> = None;
    #[cfg(feature = "gpu-scrie")]
    let boost_wanted = BOOST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    #[cfg(feature = "gpu-scrie")]
    let rusd_wanted = RUSD_ARMED.load(core::sync::atomic::Ordering::SeqCst);
    #[cfg(feature = "gpu-scrie")]
    let rusd_early = rusd_wanted && RUSD_EARLY.load(core::sync::atomic::Ordering::SeqCst);
    #[cfg(feature = "gpu-scrie")]
    let mut early: Option<gpu_golden::EarlyDashboard<RusdPage>> = None;
    let mut refused = 0usize;
    // P6-107 (C-158 note of 21 Nov 08:45): which steps had writes refused by the OS fences (the total alone did not say).
    let mut refused_by_step: Vec<(&'static str, usize)> = Vec::new();
    let report = run_chain_ready(WINDOW_CHAIN_C83, READY_IN_OS, writes_allowed, &mut j, |s, j| {
        let before = refused;
        let r = match s.id {
        "pregatire-i6" => {
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let r = gpu_i6::prepare(p, &mut win, j, &mut clock, &mut lines);
            refused += win.refused;
            match r {
                Some((l, c)) if c.passes() => {
                    prepared = Some((l, c));
                    Ok(())
                }
                Some(_) => Err(step_failed("WPR2 deja ridicat: oprire completa (cablul scos 30 s) inainte")),
                None => Err(step_failed("pregatirea I6 nu trece (vezi randurile de mai sus)")),
            }
        }
        #[cfg(feature = "gpu-scrie")]
        "bus-master-pornit" => {
            let mut cfg = gpu_i6::hw::BusMasterOnlyConfig { dev: &d, refused: 0 };
            let o = gpu_pci::bus_master::set_bus_master(&mut cfg, &bm, j, true);
            refused += cfg.refused;
            let o = o?;
            old = Some(o);
            lines.push(format!("{}: Bus Master pornit (COMMAND {:#06x} -> {:#06x})", p, o, d.command()));
            Ok(())
        }
        #[cfg(feature = "gpu-scrie")]
        "i6-fwsec-frts" => {
            let Some((loc, c)) = prepared.as_ref() else { return Err(step_failed("lipseste pregatirea I6")) };
            let built = {
                let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
                let b = gpu_i6::build_image(p, &mut win, j, &mut clock, loc, c, &mut lines);
                refused += win.refused;
                b
            };
            let Some((u, img)) = built else { return Err(step_failed("FWSEC nu s-a putut copia din VBIOS")) };
            let image = match gpu_i6::dma_put(&img) {
                Ok(i) => i,
                Err(why) => {
                    lines.push(format!("{}: zona DMA: {}; nimic scris in registre", p, why));
                    return Err(step_failed("zona DMA"));
                }
            };
            lines.push(format!("{}: zona DMA: {:#x} octeti la adresa fizica {:#x} (sub 4 GiB, recitita)", p, image.len, image.phys));
            let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
            let ok = gpu_i6::i6_falcon(p, &mut rw, &mut clock, j, &u, &image, c.frts, Some(&permit), &mut lines);
            refused += rw.refused;
            if ok { Ok(()) } else { Err(step_failed("FWSEC-FRTS a esuat (vezi randul pasului I6)")) }
        }
        #[cfg(feature = "gpu-scrie")]
        "i6-wpr2-citit" => {
            let Some((_, c)) = prepared.as_ref() else { return Err(step_failed("lipseste pregatirea I6")) };
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let r = wpr2_step(p, &mut win, j, c.frts, &mut lines);
            refused += win.refused;
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i7-booter-gsp" => {
            // P6-56 (C-72): everything in RAM first (files from the stick, the DMA area, GspFwWprMeta, Booter); any
            // failure there stops the chain before the first SEC2 write.
            let Some((_, c)) = prepared.as_ref() else { return Err(step_failed("lipseste pregatirea I6")) };
            let fw = gpu_i7::stick_firmware(p, j, &mut lines)?;
            let sys = gpu_i8::system_info(&d)?;
            let prep = {
                let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
                let r = gpu_i7::prepare(p, &mut win, j, &fw, c, settings, &sys, &mut lines);
                refused += win.refused;
                r?
            };
            drop(fw);
            let mut rw = gpu_i6::hw::GspWriteWindow::with_sec2(bar0);
            let r = gpu_i7::booter(p, &mut rw, &mut clock, j, &prep, Some(&permit), &mut lines);
            refused += rw.refused;
            i7.push(prep); // the area stays pinned: GSP-RM reads it after I7
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i7-citit" => {
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let r = gpu_i7::after(p, &mut win, j, &mut lines);
            refused += win.refused;
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i8-gsp-init" => {
            // P6-56 step 2: the queues and the LibOS arguments were prepared at I7 (RAM); here 2 messages, the doorbell
            // (GSP window) and the wait for GSP_INIT_DONE with the sequencer through the same fence.
            let Some(prep) = i7.last_mut() else { return Err(step_failed("lipseste pregatirea I7")) };
            let sys = gpu_i8::system_info(&d)?;
            let mut rw = gpu_i6::hw::GspWriteWindow::with_sec2(bar0);
            let r = gpu_i8::init(p, &mut rw, &mut clock, j, prep, &sys, Some(&permit), &mut lines, |_| {});
            refused += rw.refused;
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i8e-golden" => {
            // P6-56 step 2b: the VRAM zone from the I6/I7 layout and the screen (Limine's framebuffer inside BAR1);
            // any doubt stops here, before the first VRAM write.
            let (Some(prep), Some((_, c))) = (i7.last_mut(), prepared.as_ref()) else {
                return Err(step_failed("lipseste pregatirea I7"));
            };
            let screen = crate::gfx::framebuffer::boot_info().zip(d.bar(1)).and_then(|((phys, size, ..), bar1)| {
                nvidia_ga104::vram_window::screen_vram_range(phys, size, bar1, gpu_golden::BAR1_LEN)
            });
            // P6-58: WPR2 read back (reads only) and checked against the layout before the zone is used.
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let wpr2 = gpu_golden::read_wpr2_after_i7(&mut win, j);
            refused += win.refused;
            let wpr2 = wpr2?;
            lines.push(format!("{}: golden: WPR2 dupa I7 {:#x}..{:#x}", p, wpr2.0, wpr2.1));
            let zone = gpu_golden::zone(&gpu_i7::layout(c)?, wpr2, c.fb_size, screen, gpu_golden::BAR1_LEN)?;
            lines.push(format!(
                "{}: golden: WPR2 = cel cerut de GspFwWprMeta; ecranul in VRAM {:x?}; zona driverului {:#x}-{:#x} (restul protejat)",
                p, screen, zone.start, zone.end
            ));
            let st = gs.insert(gpu_golden::GpuState::new(zone)?);
            let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
            let mut vf = gpu_i6::hw::VramFence::new(bar0);
            let r = gpu_golden::golden_step(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, |_| {});
            refused += rw.refused + vf.refused;
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i9-canal-lucru" => {
            let (Some(prep), Some(st)) = (i7.last_mut(), gs.as_mut()) else { return Err(step_failed("lipseste golden")) };
            let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
            let mut vf = gpu_i6::hw::VramFence::new(bar0);
            let r = gpu_golden::channel_step(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, i9, |_| {});
            // P6-106: only in the cycle, once, right after the work channel is open.
            if r.is_ok() && boost_wanted {
                // P6-142: the dashboard before the request (and the thermal watch's first read).
                if rusd_early {
                    early = early_dashboard_hw(p, &mut rw, &mut vf, &mut clock, j, prep, st, &permit, &mut lines);
                }
                if early.as_ref().is_some_and(|e| e.says_stop()) {
                    lines.push(format!("{}: ks: cererea de viteza maxima NU se trimite: ceasul termic spune oprire", p));
                } else {
                    match gpu_golden::boost_begin(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, |_| {}) {
                        Ok(b) => boost = Some(b),
                        Err(e) => lines.push(format!("{}: ks: cererea de viteza maxima nu s-a putut trimite: {}", p, e)),
                    }
                    if let (Some(e), true) = (early.as_mut(), boost.is_some()) {
                        clock.delay_us((nvidia_ga104::gsp::rusd_watch::after_wait_ms(e.plan.poll_ms) * 1000) as u32);
                        if let (Some(b), Some((a, _))) = (e.before, e.read(p, "dupa cererea de viteza maxima (1 s)", &mut clock, j, &mut lines)) {
                            lines.push(gpu_golden::boost_seen_line(p, &b, &a, j));
                        }
                    }
                }
            }
            // P6-161: only when the cycle asks for the RAM ↔ VRAM probe: the copy object on the work channel.
            if r.is_ok() && RAM_VRAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
                if let Err(e) = gpu_golden::copy_object_step(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, |_| {}) {
                    lines.push(format!("{}: motorul de copiere: obiectul nu s-a putut aloca ({}); proba RAM <-> VRAM nu ruleaza", p, e));
                }
            }
            refused += rw.refused + vf.refused;
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "i10-add64" => {
            // P6-56 step 3: the work-channel doorbell is written only here (and in the cost step), through its own
            // fence (that offset, that token), after golden and I9 passed in this chain.
            let Some(st) = gs.as_mut() else { return Err(step_failed("lipseste I9")) };
            let Some(token) = st.space.as_ref().map(|(_, w)| w.doorbell_token) else { return Err(step_failed("lipseste I9")) };
            let mut vf = gpu_i6::hw::VramFence::new(bar0);
            let mut db = gpu_i6::hw::DoorbellFence::new(bar0, token);
            let r = gpu_i10::add64_step(p, &mut vf, &mut clock, j, st, Some(&permit), &mut lines, &mut |_, j, off, v, _| {
                ring_work_doorbell(&mut db, j, off, v)
            });
            refused += vf.refused + db.refused;
            launched = Some(r?);
            Ok(())
        }
        #[cfg(feature = "gpu-scrie")]
        "i10-cost-lansare" => {
            let (Some(st), Some(l)) = (gs.as_mut(), launched.as_mut()) else { return Err(step_failed("lipseste add64")) };
            let Some(token) = st.space.as_ref().map(|(_, w)| w.doorbell_token) else { return Err(step_failed("lipseste I9")) };
            let mut vf = gpu_i6::hw::VramFence::new(bar0);
            let mut db = gpu_i6::hw::DoorbellFence::new(bar0, token);
            let r = gpu_i10::cost_step(p, &mut vf, &mut clock, j, st, l, Some(&permit), &mut lines, &mut |_, j, off, v, _| {
                ring_work_doorbell(&mut db, j, off, v)
            });
            refused += vf.refused + db.refused;
            r.map(|_| ())
        }
        #[cfg(feature = "gpu-scrie")]
        "i11-copiere" | "i12-q8-matvec" | "i13-strat" | "i14-simbol" => {
            // P6-62 (C-83): no new register: the VRAM window and the work-channel doorbell, as in I10.
            let (Some(st), Some(l)) = (gs.as_mut(), launched.as_mut()) else { return Err(step_failed("lipseste I10")) };
            let Some(token) = st.space.as_ref().map(|(_, w)| w.doorbell_token) else { return Err(step_failed("lipseste I9")) };
            let mut vf = gpu_i6::hw::VramFence::new(bar0);
            let mut db = gpu_i6::hw::DoorbellFence::new(bar0, token);
            let mut bell = |_: &mut gpu_i6::hw::VramFence, j: &mut Journal, off: u64, v: u32, _: &gpu_i10::Ring<'_>| {
                ring_work_doorbell(&mut db, j, off, v)
            };
            let r = match s.id {
                "i11-copiere" => {
                    gpu_i11::copy_step(p, &mut vf, &mut clock, j, st, l, gpu_i11::I11_BOARD_BYTES, Some(&permit), &mut lines, &mut bell)
                }
                "i12-q8-matvec" => gpu_i11::matvec_step(p, &mut vf, &mut clock, j, st, l, Some(&permit), &mut lines, &mut bell),
                "i13-strat" => {
                    gpu_i11::layer_step(p, &mut vf, &mut clock, j, st, l, gpu_i11::I13_BOARD, Some(&permit), &mut lines, &mut bell)
                }
                _ => gpu_i14::token_step(p, &mut vf, &mut clock, j, st, l, gpu_i14::MIC_BOARD, Some(&permit), &mut lines, &mut bell),
            };
            refused += vf.refused + db.refused;
            // P6-98 (C-148 point 3): the board's performance state, read once at the end of the chain (after I14 passed).
            if s.id == "i14-simbol" && r.is_ok() {
                if let Some(prep) = i7.last_mut() {
                    let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
                    let mut vf = gpu_i6::hw::VramFence::new(bar0);
                    // P6-142: the early dashboard's last read while the max-speed request still holds (after the texts).
                    if let Some(e) = early.as_mut() {
                        let _ = e.read(p, "la final (dupa texte, inainte de stergerea cererii)", &mut clock, j, &mut lines);
                    }
                    let r = match boost.take() {
                        Some(b) => gpu_golden::boost_end(p, &mut rw, &mut vf, &mut clock, j, prep, st, &b, Some(&permit), &mut lines, |_| {}),
                        None => gpu_golden::pstate_step(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, |_| {}),
                    };
                    if let Err(e) = r {
                        lines.push(format!("{}: ks: starea de performanta necitita: {}", p, e));
                    }
                    // P6-105 (C-151 point 1): the dashboard, only when armed, after everything above is on the stick.
                    if rusd_wanted {
                        match early.take() {
                            // P6-142: the early dashboard, read a third time and stopped.
                            Some(e) => early_finish_hw(p, &mut rw, &mut vf, &mut clock, j, prep, st, &permit, &mut lines, e, false),
                            None => dashboard_hw(p, &mut rw, &mut vf, &mut clock, j, prep, st, &permit, &mut lines),
                        }
                    }
                    refused += rw.refused + vf.refused;
                }
            }
            r
        }
        #[cfg(feature = "gpu-scrie")]
        "bus-master-oprit" => match old {
            None => Ok(()),
            Some(o) => {
                let mut cfg = gpu_i6::hw::BusMasterOnlyConfig { dev: &d, refused: 0 };
                let r = gpu_pci::bus_master::restore_command(&mut cfg, &bm, j, o);
                refused += cfg.refused;
                r?;
                let now = d.command();
                if now != o {
                    lines.push(format!("{}: COMMAND {:#06x}, nu {:#06x} -> oprire completa (cablul scos 30 s)", p, now, o));
                    return Err(GpuError::VerifyFailed { offset: 4, expected: o as u32, actual: now as u32 });
                }
                lines.push(format!("{}: Bus Master oprit la loc (COMMAND {:#06x}, ca inainte)", p, o));
                Ok(())
            }
        },
        "tur-dupa" => {
            let mut cfg = ReadOnlyConfig { dev: &d, refused: 0 };
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let r = tour_on(p, &mut cfg, Some(addr), &mut win, &mut clock, j, &mut lines);
            refused += cfg.refused + win.refused;
            r
        }
        // Writing steps in a build that cannot write never get here (the chain stops before them).
        _ => Err(GpuError::NotPermitted(NOT_IN_OS)),
        };
        if refused > before {
            refused_by_step.push((s.id, refused - before));
        }
        r
    });
    // P6-106: the chain stopped after the boost was accepted: clear it while the channel may still answer (else it
    // ends by itself after 600 s).
    #[cfg(feature = "gpu-scrie")]
    if let (Some(b), Some(prep), Some(st)) = (boost.take(), i7.last_mut(), gs.as_mut()) {
        let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
        let mut vf = gpu_i6::hw::VramFence::new(bar0);
        if let Err(e) = gpu_golden::boost_end(p, &mut rw, &mut vf, &mut clock, &mut j, prep, st, &b, Some(&permit), &mut lines, |_| {}) {
            lines.push(format!("{}: ks: stergerea cererii dupa oprire nu a mers ({}); se stinge singura dupa 600 s", p, e));
        }
        refused += rw.refused + vf.refused;
    }
    // P6-142: the chain stopped with the early dashboard still polling: its reads stopped all the same.
    #[cfg(feature = "gpu-scrie")]
    if let (Some(e), Some(prep), Some(st)) = (early.take(), i7.last_mut(), gs.as_mut()) {
        let mut rw = gpu_i6::hw::GspWriteWindow::new(bar0);
        let mut vf = gpu_i6::hw::VramFence::new(bar0);
        lines.push(format!("{}: kd lantul s-a oprit cu tabloul pornit devreme: opresc citirile", p));
        early_finish_hw(p, &mut rw, &mut vf, &mut clock, &mut j, prep, st, &permit, &mut lines, e, true);
        refused += rw.refused + vf.refused;
    }
    drop(_guard);
    report_lines(p, &report, &mut lines);
    if refused > 0 {
        lines.push(format!("{}: adaptoarele OS au REFUZAT {} scrieri cerute in afara gardului", p, refused));
        let by: Vec<String> = refused_by_step.iter().map(|(id, n)| format!("{} {}", id, n)).collect();
        let after = refused - refused_by_step.iter().map(|(_, n)| n).sum::<usize>();
        lines.push(format!(
            "{}: scrierile refuzate pe pasi: {}{}",
            p,
            if by.is_empty() { String::from("-") } else { by.join(", ") },
            if after > 0 { format!("; in afara pasilor (stergerea cererii de viteza dupa o oprire) {}", after) } else { String::new() }
        ));
    }
    gpu_i6::close_and_check(p, false, &mut lines);
    lines.push(format!(
        "{}: rezultat lant: {} ({} scrieri)",
        p,
        match &report.stopped_at {
            None => String::from("toti pasii au trecut"),
            Some((id, _)) => format!("oprit la {}", id),
        },
        report.writes
    ));
    finish(lines)
}

/// P6-105 on the board: page → start → 3 × the interval → read → stop. The lines also go to `LAST_DASHBOARD`.
#[cfg(feature = "gpu-scrie")]
#[allow(clippy::too_many_arguments)]
fn dashboard_hw(
    p: &str,
    rw: &mut gpu_i6::hw::GspWriteWindow,
    vf: &mut gpu_i6::hw::VramFence,
    clock: &mut TscClock,
    j: &mut Journal,
    prep: &mut gpu_i7::I7Prep,
    st: &mut gpu_golden::GpuState,
    permit: &nvidia_ga104::gsp::HardwareWritePermit,
    lines: &mut Vec<String>,
) {
    let plan = nvidia_ga104::gsp::rusd::RusdPlan { enabled: true, ..Default::default() };
    let Some(mut page) = RusdPage::new() else {
        lines.push(format!("{}: kd tabloul de bord: nicio pagina libera sub 4 GiB; nimic trimis", p));
        return;
    };
    let start = match gpu_golden::dashboard_begin(rw, vf, clock, j, prep, st, &plan, page.phys, gpu_golden::DASHBOARD_BEGIN, Some(permit), |_| {}) {
        Ok(s) => s,
        Err(e) => {
            // A transport error: the page may already be GSP-RM's; it is never reused in this power-on.
            lines.push(format!("{}: kd tabloul de bord: eroare la pornire ({}); pagina ramane rezervata", p, e));
            return;
        }
    };
    if start.polling() {
        clock.delay_us(plan.poll_ms.saturating_mul(3).saturating_mul(1000));
    }
    match gpu_golden::dashboard_finish(rw, vf, clock, j, prep, st, &start, &mut page, Some(permit), |_| {}) {
        Ok((dash, stop)) => {
            let l = gpu_golden::dashboard_lines(p, &start, dash.as_ref(), &stop);
            *LAST_DASHBOARD.lock() = Some(l.clone());
            lines.extend(l);
            if stop.page_free(&start) {
                crate::mem::frame::free_frame(page.phys);
            }
        }
        Err(e) => lines.push(format!("{}: kd tabloul de bord: eroare la oprire ({}); pagina ramane rezervata", p, e)),
    }
}

/// P6-142 on the board: page → start (before the max-speed request) → 3 × the interval → the "before" read.
/// `None` = no page, a refusal or a transport error (the line says which; the request then goes as before).
#[cfg(feature = "gpu-scrie")]
#[allow(clippy::too_many_arguments)]
fn early_dashboard_hw(
    p: &str,
    rw: &mut gpu_i6::hw::GspWriteWindow,
    vf: &mut gpu_i6::hw::VramFence,
    clock: &mut TscClock,
    j: &mut Journal,
    prep: &mut gpu_i7::I7Prep,
    st: &mut gpu_golden::GpuState,
    permit: &nvidia_ga104::gsp::HardwareWritePermit,
    lines: &mut Vec<String>,
) -> Option<gpu_golden::EarlyDashboard<RusdPage>> {
    let plan = nvidia_ga104::gsp::rusd::RusdPlan { enabled: true, ..Default::default() };
    let Some(page) = RusdPage::new() else {
        lines.push(format!("{}: kd tabloul de bord: nicio pagina libera sub 4 GiB; nimic trimis", p));
        return None;
    };
    let phys = page.phys;
    let t0 = clock.now_us();
    let start = match gpu_golden::dashboard_begin(rw, vf, clock, j, prep, st, &plan, phys, gpu_golden::DASHBOARD_EARLY, Some(permit), |_| {}) {
        Ok(s) => s,
        Err(e) => {
            lines.push(format!("{}: kd tabloul de bord (devreme): eroare la pornire ({}); pagina ramane rezervata", p, e));
            return None;
        }
    };
    let Some(mut e) = gpu_golden::EarlyDashboard::new(start, plan, page, phys, t0) else {
        lines.push(format!("{}: kd ceasul termic: limitele propuse refuzate de biblioteca", p));
        return None;
    };
    if !e.start.polling() {
        lines.push(format!("{}: kd tabloul de bord (devreme): placa nu a acceptat toate comenzile; fara citiri", p));
        return Some(e);
    }
    clock.delay_us(plan.poll_ms.saturating_mul(3).saturating_mul(1000));
    e.before = e.read(p, "inainte de cererea de viteza maxima", clock, j, lines).map(|(d, _)| d);
    Some(e)
}

/// P6-142 on the board: the thermal watch's summary, then the reads stopped (`rusd_stop_poll_only`); `read_last` = one more
/// read first (the chain stopped early; otherwise the last read was taken before the max-speed request was cleared).
#[cfg(feature = "gpu-scrie")]
#[allow(clippy::too_many_arguments)]
fn early_finish_hw(
    p: &str,
    rw: &mut gpu_i6::hw::GspWriteWindow,
    vf: &mut gpu_i6::hw::VramFence,
    clock: &mut TscClock,
    j: &mut Journal,
    prep: &mut gpu_i7::I7Prep,
    st: &mut gpu_golden::GpuState,
    permit: &nvidia_ga104::gsp::HardwareWritePermit,
    lines: &mut Vec<String>,
    mut e: gpu_golden::EarlyDashboard<RusdPage>,
    read_last: bool,
) {
    if read_last {
        let _ = e.read(p, "la oprirea lantului", clock, j, lines);
    }
    lines.push(gpu_golden::watch_summary_line(p, &e.watch, j));
    match gpu_golden::dashboard_finish(rw, vf, clock, j, prep, st, &e.start, &mut e.page, Some(permit), |_| {}) {
        Ok((dash, stop)) => {
            let l = gpu_golden::dashboard_lines(p, &e.start, dash.as_ref(), &stop);
            *LAST_DASHBOARD.lock() = Some(l.clone());
            lines.extend(l);
            if stop.page_free(&e.start) {
                crate::mem::frame::free_frame(e.phys);
            }
        }
        Err(err) => lines.push(format!("{}: kd tabloul de bord: eroare la oprire ({}); pagina ramane rezervata", p, err)),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// P6-85 (C-116): the ASUS TUF laptop (GA106). The library knows it only for reading; its firmware leaves the board
// with Memory Space off, so nothing can be read until the "minimal wake-up" (`gpu tur --trezire`): one write in the
// PCI Command register (Memory Space on) for the tour, put back after it — also when the tour fails. No GPU register
// is ever written: BAR0 is reached through the read-only window.
// ---------------------------------------------------------------------------------------------------------------

/// The first NVIDIA board, when the library knows it as a family that may not be written (GA106, the laptop).
fn ga106_board() -> Option<crate::pci::Device> {
    let d = gpu::display_devices().into_iter().find(|d| d.vendor == gpu::NVIDIA)?;
    let (fam, _) = nvidia_ga104::ids::lookup_ampere(d.vendor, d.device)?;
    (!fam.writes_allowed()).then_some(d)
}

/// The fact lines of a tour and its summary.
fn tour_lines(p: &str, t: &nvidia_ga104::read_tour::Tour, lines: &mut Vec<String>) {
    for f in &t.facts {
        lines.push(format!("{}", f));
    }
    lines.push(format!("{}: tur de citire (C-54): {} fapte, {} citiri jurnalizate", p, t.facts.len(), t.reads));
}

/// The warning shown before `DA` for `gpu tur --trezire`.
pub const TREZIRE_WARNING: &[&str] = &[
    "ATENTIE: gpu tur --trezire (C-116) face o singura scriere in configuratia PCI a placii video:",
    "  pornesc bitul Memory Space din registrul Command, ca sa se poata citi placa (BOOT_0, GFW, VBIOS, FWSEC);",
    "  la final Command e repus cum l-a lasat firmware-ul (si daca turul pica). Niciun registru al placii nu se scrie.",
    "  Jurnalul merge pe stick (gpu-jurnal-NNN.txt) si se trimite in hub dupa.",
];

/// The tour with the minimal wake-up on any configuration / BAR0 (real board or model). `writes_cfg` = the
/// configuration writes the adapter let through (2 when the wake-up ran, 0 when it was not needed).
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
fn awake_tour_on<C: ConfigSpace + ?Sized, M: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    cfg: &mut C,
    addr: Option<PciAddress>,
    bar0: &mut M,
    clock: &mut K,
    journal: &mut Journal,
    lines: &mut Vec<String>,
) {
    let permit = gpu_pci::mem_space::MemorySpacePermit::new("C-116 gpu tur --trezire (P6-85)");
    match nvidia_ga104::read_tour::run_tour_awake(cfg, addr, bar0, clock, journal, &permit) {
        Ok((t, wake)) => {
            tour_lines(p, &t, lines);
            lines.push(format!(
                "{}: trezirea minima: {}",
                p,
                match wake {
                    nvidia_ga104::read_tour::Wake::NotNeeded => "nu a fost nevoie (Memory Space era pornit), nicio scriere",
                    nvidia_ga104::read_tour::Wake::EnabledAndRestored => "Memory Space pornit pentru tur, apoi repus",
                }
            ));
        }
        Err(e) => lines.push(format!("{}: tur de citire (trezire C-116): {}", p, e)),
    }
}

/// `gpu tur` on the laptop (`awake` false: read only; with Memory Space off it stops with the reason) and
/// `gpu tur --trezire` (`awake` true: only in the test-stick build, after `DA`).
fn tour_ampere_board(p: &str, d: crate::pci::Device, awake: bool, mut lines: Vec<String>) -> Vec<String> {
    let addr = PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
    let (c, family) = {
        let mut cfg = ReadOnlyConfig { dev: &d, refused: 0 };
        match nvidia_ga104::probe::probe_config_ampere(&mut cfg, Some(addr)) {
            Ok(x) => x,
            Err(e) => {
                lines.push(format!("{}: {} cyber-gpu {}: {}", p, d.address(), gpu::CYBER_GPU_REV, e));
                return finish(lines);
            }
        }
    };
    lines.push(format!(
        "{}: {} {} (BAR0 {:#x}), familia {}, cyber-gpu {}",
        p, d.address(), c.model, c.bar0, family.name(), gpu::CYBER_GPU_REV
    ));
    if !awake && !c.memory_space_enabled() {
        lines.push(format!("{}: {}", p, nvidia_ga104::probe::MEMORY_SPACE_OFF));
        lines.push(format!("{}: {}", p, gpu::WAKE_HINT));
        lines.push(gpu::writes_line(0));
        return finish(lines);
    }
    if crate::time::since_boot_us().is_none() {
        lines.push(format!("{}: ceasul (TSC) nu e calibrat: limitele de timp nu se pot tine; nimic citit", p));
        return finish(lines);
    }
    let what = if awake { "gpu tur --trezire" } else { "gpu tur" };
    let live = match gpu_i6::live_open(what) {
        Ok(_) => true,
        Err(why) => {
            lines.push(format!("{}: jurnalul pe loc: nu (stick: {}); doar in log", p, why));
            false
        }
    };
    let mut j = gpu_i6::journal(live);
    let (refused, cfg_writes, remaps, map_failed) = {
        let _guard = WINDOW_LOCK.lock();
        let mut win = ReadOnlyBar0Window::with_len(c.bar0 & !0xFFF, nvidia_ga104::fuse::FUSE_WINDOW);
        let (refused, cfg_writes) = if awake {
            awake_on_board(p, &d, addr, &mut win, &mut j, &mut lines)
        } else {
            let mut cfg = ReadOnlyConfig { dev: &d, refused: 0 };
            match nvidia_ga104::read_tour::run_tour_ampere(&mut cfg, Some(addr), &mut win, &mut TscClock, &mut j) {
                Ok(t) => tour_lines(p, &t, &mut lines),
                Err(e) => lines.push(format!("{}: tur de citire: {}", p, e)),
            }
            (cfg.refused, 0)
        };
        (refused + win.refused, cfg_writes, win.remaps, win.map_failed)
    };
    let cmd = d.command();
    lines.push(format!(
        "{}: Command acum {:04x} (memorie {}, bus master {})",
        p, cmd, if cmd & 2 != 0 { "da" } else { "nu" }, if cmd & 4 != 0 { "da" } else { "nu" }
    ));
    gpu_i6::close_and_check(p, false, &mut lines);
    if map_failed {
        lines.push(format!("{}: maparea ferestrei a esuat (citirile au dat FFFFFFFF)", p));
    }
    lines.push(format!("{}: {} pagini mapate pe rand (doar citire, fereastra de 16 MiB)", p, remaps));
    lines.push(format!(
        "{}: scrieri: {} in configuratia PCI (registrul Command, trezirea C-116), 0 in registrele placii; {} cereri refuzate de adaptor",
        p, cfg_writes, refused
    ));
    finish(lines)
}

/// The wake-up on the real board: only the test-stick build has an adapter that can write the Command register.
/// Returns (refused, configuration writes let through).
#[cfg(feature = "gpu-scrie")]
fn awake_on_board(
    p: &str,
    d: &crate::pci::Device,
    addr: PciAddress,
    win: &mut ReadOnlyBar0Window,
    j: &mut Journal,
    lines: &mut Vec<String>,
) -> (usize, usize) {
    let mut cfg = gpu_i6::hw::MemorySpaceOnlyConfig { dev: d, refused: 0, written: 0 };
    awake_tour_on(p, &mut cfg, Some(addr), win, &mut TscClock, j, lines);
    (cfg.refused, cfg.written)
}

#[cfg(not(feature = "gpu-scrie"))]
fn awake_on_board(
    p: &str,
    _d: &crate::pci::Device,
    _addr: PciAddress,
    _win: &mut ReadOnlyBar0Window,
    _j: &mut Journal,
    lines: &mut Vec<String>,
) -> (usize, usize) {
    lines.push(format!("{}: {}", p, TREZIRE_REFUSED));
    (0, 0)
}

/// The refusal outside the test-stick build.
pub const TREZIRE_REFUSED: &str =
    "gpu tur --trezire: acest sistem nu poate scrie in placa (doar stick-ul de test, gpu-scrie); nimic scris";

/// P6-90 (C-117, ASUS os-301): the last `gpu tur --trezire` of this power-on that found facts, so `analiza` can put
/// them in the inventory when its own read-only tour finds Memory Space off (the laptop) — `analiza` never wakes the
/// board itself (0 writes).
#[derive(Clone)]
pub struct AwakeTour {
    /// PCI address of the board ("01:00.0"); None for the gpu-sim model.
    pub address: Option<String>,
    /// When the tour ran ("HH:MM:SS", from the RTC; "necunoscut" without it).
    pub when: String,
    pub lines: Vec<String>,
}

static LAST_AWAKE: crate::sched::Mutex<Option<AwakeTour>> = crate::sched::Mutex::new(None);

fn remember_awake(address: Option<String>, lines: &[String]) {
    if !lines.iter().any(|l| l.contains("fapt ")) {
        return; // a failed tour is not kept: an older good one stays the useful one
    }
    let when = match crate::drivers::rtc::now() {
        Some(t) => format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
        None => String::from("necunoscut"),
    };
    *LAST_AWAKE.lock() = Some(AwakeTour { address, when, lines: lines.to_vec() });
}

/// The last awake tour of this power-on for the board at `address` (None = the gpu-sim model).
pub fn last_awake(address: Option<&str>) -> Option<AwakeTour> {
    LAST_AWAKE.lock().as_ref().filter(|t| t.address.as_deref() == address).cloned()
}

/// `gpu tur --trezire` on the real board (the shell asked for `DA` first; outside `gpu-scrie` it refuses before).
pub fn tour_awake() -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
    let Some(d) = gpu::display_devices().into_iter().find(|d| d.vendor == gpu::NVIDIA) else {
        lines.push(String::from("nicio placa NVIDIA"));
        return finish(lines);
    };
    let address = d.address();
    let lines = tour_ampere_board(p, d, true, lines);
    remember_awake(Some(address), &lines);
    lines
}

/// `gpu tur --trezire --sim`: the wake-up and the tour on the gpu-sim model of the laptop (configuration from what
/// the laptop showed on 12 nov: `10de:2520`, Command 0; BAR0 = the observed workstation BAR0 with GA106 identity,
/// as in Cyber-GPU's own test), then the same on a model whose BOOT_0 does not answer (Command must still go back),
/// then the plain read-only tour on the laptop model (refused: Memory Space off).
#[cfg(feature = "test-gpu-sim")]
pub fn tour_awake_sim() -> Vec<String> {
    let p = "model";
    let mut lines = Vec::new();
    let live = gpu_i6::live_open("gpu tur --trezire --sim").is_ok();
    let mut j = gpu_i6::journal(live);
    let mut clock = gpu_sim::clock::SimClock::default();
    let mut cfg = gpu_sim::observed::observed_config();
    cfg.set_device_id(0x2520);
    cfg.write_raw(0x04, 0x0010_0000);
    cfg.writes = 0;
    let mut bar0 = gpu_sim::observed::observed_bar0();
    bar0.set(0, 0xb760_00a1);
    bar0.set(0xa00, 0x176a_1000);
    awake_tour_on(p, &mut cfg, None, &mut bar0, &mut clock, &mut j, &mut lines);
    lines.push(format!(
        "{}: laptop: {} scrieri in configuratie, {} in registre, Command dupa = {:04x}",
        p, cfg.writes, bar0.writes.len(), cfg.read_raw(0x04) & 0xffff
    ));
    // P6-90: the laptop model's awake tour is what `analiza --sim` may reuse (as the real one on the laptop).
    remember_awake(None, &lines);
    // BOOT_0 does not answer after the wake-up: the tour fails, Command must still be put back.
    let mut cfg2 = gpu_sim::pci::SimConfig::ga106_laptop_asus();
    let mut dead = gpu_sim::mmio::SimMmio::new(16 << 20);
    dead.set(0, 0xffff_ffff);
    let mut scratch = Journal::new(256);
    awake_tour_on("model nu raspunde", &mut cfg2, None, &mut dead, &mut clock, &mut scratch, &mut lines);
    lines.push(format!(
        "model nu raspunde: {} scrieri in configuratie, Command dupa = {:04x}",
        cfg2.writes, cfg2.read_raw(0x04) & 0xffff
    ));
    // The plain tour (no wake-up) on the laptop as the firmware leaves it: refused before any BAR0 read.
    let mut cfg3 = gpu_sim::pci::SimConfig::ga106_laptop_asus();
    let mut bar3 = gpu_sim::mmio::SimMmio::ga106_bar0();
    let mut scratch = Journal::new(256);
    match nvidia_ga104::read_tour::run_tour_ampere(&mut cfg3, None, &mut bar3, &mut clock, &mut scratch) {
        Ok(_) => lines.push(String::from("model fara trezire: turul a citit (NEASTEPTAT)")),
        Err(e) => lines.push(format!("model fara trezire: {}; {} citiri BAR0", e, bar3.reads.len())),
    }
    // The writing chain stays refused on GA106 (C-116 step 3).
    let mut cfg4 = gpu_sim::pci::SimConfig::ga106_laptop_asus();
    match nvidia_ga104::probe::probe_config(&mut cfg4, None) {
        Ok(_) => lines.push(String::from("model lant: GA106 acceptat de lant (NEASTEPTAT)")),
        Err(e) => lines.push(format!("model lant: GA106 refuzat de lant: {}", e)),
    }
    gpu_i6::close_and_check(p, false, &mut lines);
    gpu_i6::sim_lines(lines, "gpu tur --trezire")
}

// ---------------------------------------------------------------------------------------------------------------
// SIMULARE (test builds).
// ---------------------------------------------------------------------------------------------------------------

/// `gpu tur --sim`: the tour on the model of the workstation board built from its real answers (`gpu_sim::observed`).
#[cfg(feature = "test-gpu-sim")]
pub fn tour_sim() -> Vec<String> {
    let p = "model";
    let mut lines = Vec::new();
    let live = gpu_i6::live_open("gpu tur --sim").is_ok();
    let mut j = gpu_i6::journal(live);
    let mut cfg = gpu_sim::observed::observed_config();
    let mut bar0 = gpu_sim::observed::observed_bar0();
    let mut clock = gpu_sim::clock::SimClock::default();
    if let Err(e) = tour_on(p, &mut cfg, None, &mut bar0, &mut clock, &mut j, &mut lines) {
        lines.push(format!("{}: tur de citire: {}", p, e));
    }
    // P6-58: the VRAM zone of golden on the WPR2 values the board gave on 5 nov: after I7 (widened) it is the zone,
    // still the FRTS region (Booter did not widen it) or WPR2 down it is refused — before any VRAM write.
    let screen = nvidia_ga104::vram_window::screen_vram_range(0xe000_0000, 8192 * 1080, 0xe000_0000, gpu_golden::BAR1_LEN);
    match gpu_i7::layout_for(8 << 30) {
        Ok(l) => {
            for (what, wpr2) in [
                ("WPR2 largit dupa I7 (placa, 5 nov)", (0x1_f410_0000u64, 0x1_ffee_0000u64)),
                ("WPR2 doar FRTS (Booter nu l-a largit)", (0x1_ffe0_0000, 0x1_ffee_0000)),
            ] {
                let r = gpu_golden::zone(&l, wpr2, 8 << 30, screen, gpu_golden::BAR1_LEN);
                lines.push(match r {
                    Ok(z) => format!("{}: zona VRAM golden cu {}: {:#x}-{:#x}", p, what, z.start, z.end),
                    Err(e) => format!("{}: zona VRAM golden cu {}: REFUZATA ({})", p, what, e),
                });
            }
        }
        Err(e) => lines.push(format!("{}: zona VRAM golden: aranjamentul: {}", p, e)),
    }
    // C-72: the fact `i6.trece` after I6, on the register values the board gave on 5 nov (os-242), on a cold board
    // and on a WPR2 that is not the FRTS region. Reads only, on throw-away models.
    // C-75: and after I7, WPR2 widened by Booter as on the board (0x1f4100000..0x1ffee0000).
    for (what, wpr2, stage) in [
        ("model dupa I6 (WPR2 ridicat ca pe placa, 5 nov)", Some((0x01ff_e000u32, 0x01ff_ee00u32)), "FRTS (dupa I6)"),
        ("model dupa I7 (WPR2 largit ca pe placa, 5 nov)", Some((0x01f4_1000, 0x01ff_ee00)), "largit pentru GSP-RM (dupa I7)"),
        ("model fara I6", None, "coborat (pornire rece)"),
        ("model cu WPR2 gresit", Some((0x01ff_0000, 0x01ff_1000)), "alta forma (de analizat)"),
    ] {
        let mut m = gpu_sim::mmio::SimMmio::ga104_bar0();
        if let Some((lo, hi)) = wpr2 {
            m.set(0x1fa824, lo);
            m.set(0x1fa828, hi);
        }
        let raised = wpr2.map(|_| String::from("RIDICAT"));
        let mut facts = alloc::vec![
            nvidia_ga104::read_tour::Fact::new("i6.vram", "8192 MiB"),
            nvidia_ga104::read_tour::Fact::new("i6.wpr2", raised.unwrap_or_else(|| String::from("coborat"))),
            nvidia_ga104::read_tour::Fact::new("i6.trece", if wpr2.is_some() { "nu" } else { "da" }),
            nvidia_ga104::read_tour::Fact::new("i6.wpr2.etapa", stage),
        ];
        let mut scratch = Journal::new(64);
        i6_result_fact(&mut facts, &mut m, &mut scratch);
        for f in facts.iter().filter(|f| f.key != "i6.vram" && f.key != "i6.wpr2" && f.key != "i6.wpr2.etapa") {
            lines.push(format!("{}: {}", what, f));
        }
    }
    gpu_i6::close_and_check(p, false, &mut lines);
    lines.push(gpu::writes_line(cfg.writes + bar0.writes.len() + j.writes()));
    gpu_i6::sim_lines(lines, "gpu tur")
}

/// `gpu lant --sim [--eroare]` (the shell asked for `DA` first): the chain with writes allowed on the models. I6
/// preparation on the read model of P6-40, Bus Master on the observed configuration model (off at the start, as on
/// the workstation), FWSEC-FRTS with the SIM FWSEC through the OS's real DMA area and the falcon model, the tour after
/// on the observed BAR0 with the same configuration model (so it shows Bus Master back off). `fault`: FWSEC halts with
/// an error (the chain must stop there and still put Bus Master back). `writes_allowed` false = the read-only chain of
/// every build without `gpu-scrie`: it must stop before Bus Master and still run the tour.
#[cfg(feature = "test-gpu-sim")]
pub fn chain_sim(fault: bool, writes_allowed: bool, variant: Option<u8>, i9: super::gpu_golden::I9Choice, flr_trial: bool) -> Vec<String> {
    use gpu_pci::bus_master::{restore_command, set_bus_master, BusMasterPermit};
    use gpu_sim::fake_gsp::FakeGsp;
    let p = "model";
    let mut lines = Vec::new();
    let what = match (fault, writes_allowed) {
        (_, false) => "gpu lant --sim --doar-citire",
        (true, _) => "gpu lant --sim --eroare",
        _ => "gpu lant --sim",
    };
    if let Err(why) = gpu_i6::live_open(what) {
        lines.push(format!("{}: jurnalul pe disc nu se poate scrie ({}): STOP", p, why));
        return gpu_i6::sim_lines(lines, "gpu lant");
    }
    let mut j = gpu_i6::journal(true);
    let settings = match nvidia_ga104::gsp::variants::settings(variant) {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!("{}: {}; nimic scris", p, e));
            return gpu_i6::sim_lines(lines, "gpu lant");
        }
    };
    lines.push(format!("{}: {}", p, variant_line(variant)));
    lines.push(format!("{}: {}", p, super::gpu_golden::i9_line(i9)));
    settings.journal(variant, &mut j);
    let mut clock = gpu_sim::clock::SimClock::default();
    let mut ro = gpu_i6::sim_board();
    let mut cfg = gpu_sim::observed::observed_config();
    let cfg_before = cfg.writes;
    let bm = BusMasterPermit::new("C-55 gpu lant --sim (P6-42)");
    let mut old: Option<u16> = None;
    let mut frts = (0u64, 0u64);
    let mut i6c: Option<nvidia_ga104::i6_check::I6Check> = None;
    let mut g = FakeGsp::new();
    // P6-101: with the FLR trial, the reset from inside at the start, as on the board (WPR2 = 0 in a fresh model).
    let mut flr_cfg = gpu_sim::pci::SimConfig::workstation_rtx3070ti().with_flr();
    let flr = gpu_pci::flr::FlrPermit::new(FLR_REASON);
    if flr_trial {
        let r0 = sim_flr(&mut flr_cfg, &mut g, &mut clock, &flr, &mut j);
        lines.push(format!("{}: {}", p, r0.row()));
    }
    let mut i7p: Option<gpu_i7::I7Prep> = None;
    let mut sim_peer: Option<gpu_i8::SimPeer> = None;
    let mut gs: Option<gpu_golden::GpuState> = None;
    let mut rm = gpu_sim::fake_rm::FakeRm::new();
    // P6-106 (SIMULARE): in the cycle the RM model has a performance state (P8 idle, as os-313 read) and accepts the boost.
    let boost_wanted = BOOST_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst);
    if boost_wanted {
        rm.perf = Some(gpu_sim::fake_rm::SimPerf::new(SIM_BOOST_RULE.lock().unwrap_or_default()));
    }
    let mut boost: Option<nvidia_ga104::gsp::perf::BoostStart> = None;
    // P6-105 (SIMULARE): `--tablou` gives the RM model its RUSD page model (accepts, or refuses the start).
    let rusd_wanted = RUSD_ARMED.load(core::sync::atomic::Ordering::SeqCst);
    if rusd_wanted {
        rm.rusd = Some(gpu_sim::fake_rm::SimRusd::new(SIM_RUSD_RULE.lock().unwrap_or_default()));
    }
    // P6-142 (SIMULARE): the early dashboard, as on the board.
    let rusd_early = rusd_wanted && RUSD_EARLY.load(core::sync::atomic::Ordering::SeqCst);
    let mut early: Option<gpu_golden::EarlyDashboard<SimRusdPage>> = None;
    let mut vram = gpu_sim::pramin::SimBar0Vram::new(gpu_sim::vram::SimVram::new(8 << 30), 0);
    let mut sim_gpu: Option<gpu_i10::SimGpu> = None;
    let mut launched: Option<gpu_i10::Launched> = None;
    let report = run_chain_ready(WINDOW_CHAIN_C83, READY_IN_OS, writes_allowed, &mut j, |s, j| match s.id {
        "pregatire-i6" => match gpu_i6::prepare(p, &mut ro, j, &mut clock, &mut lines) {
            Some((_, c)) if c.passes() => {
                frts = c.frts;
                i6c = Some(c);
                Ok(())
            }
            _ => Err(step_failed("pregatirea I6 nu trece")),
        },
        "bus-master-pornit" => {
            let o = set_bus_master(&mut cfg, &bm, j, true)?;
            old = Some(o);
            lines.push(format!("{}: Bus Master pornit (COMMAND {:#06x} -> {:#06x})", p, o, cfg.read32(gpu_i6::PCI_COMMAND) as u16));
            Ok(())
        }
        "i6-fwsec-frts" => sim_i6_once(p, &mut g, &mut clock, j, frts, fault, &mut lines),
        "i6-wpr2-citit" => wpr2_step(p, &mut g, j, frts, &mut lines),
        "i7-booter-gsp" => {
            let Some(c) = i6c.as_ref() else { return Err(step_failed("lipseste pregatirea I6")) };
            let fw = gpu_i7::sim_firmware(j)?;
            lines.push(format!("{}: I7: fisierele de proba gpu-sim (NU fisierele NVIDIA) verificate", p));
            let prep = gpu_i7::prepare(p, &mut ro, j, &fw, c, settings, &sim_system_info(), &mut lines)?;
            gpu_i7::sim_expect(&mut g, &prep);
            let r = gpu_i7::booter(p, &mut g, &mut clock, j, &prep, None, &mut lines);
            // C-75: the model drops GSP mailbox writes after FWSEC (as the board did on 5 nov) until GSP is reset;
            // GSP-RM finds its queues only if the LibOS address was there when Booter started it.
            let seen = g.libos_at_booter;
            lines.push(format!(
                "{}: modelul: adresa LibOS vazuta de GSP-RM la pornire = {} (C-75)",
                p,
                seen.map_or_else(|| String::from("NIMIC"), |a| format!("{:#x}", a))
            ));
            let r = r.and_then(|()| {
                if seen == Some(prep.libos) {
                    Ok(())
                } else {
                    Err(step_failed("GSP-RM a pornit fara adresa LibOS (C-75)"))
                }
            });
            // P6-59 (SIM): the model's Booter widened WPR2 itself from our `GspFwWprMeta` (board, 5 nov: LO 0x1f4100000).
            if r.is_ok() {
                let lo = u64::from(g.read32_raw(0x1f_a824) >> 4) << 12;
                lines.push(format!("{}: modelul: WPR2 largit de Booter din GspFwWprMeta (inceput {:#x})", p, lo));
            }
            i7p = Some(prep);
            r
        }
        "i7-citit" => gpu_i7::after(p, &mut g, j, &mut lines),
        "i8-gsp-init" => {
            let Some(prep) = i7p.as_mut() else { return Err(step_failed("lipseste pregatirea I7")) };
            let sys = sim_system_info();
            let mut peer = gpu_i8::SimPeer::new(prep);
            let r = gpu_i8::init(p, &mut g, &mut clock, j, prep, &sys, None, &mut lines, |m| peer.poll(m));
            lines.push(format!("{}: modelul GSP-RM: mesaje primite {:?} (72 = SetSystemInfo, 73 = SetRegistry)", p, peer.seen));
            sim_peer = Some(peer);
            r
        }
        "i8e-golden" => {
            let (Some(prep), Some(c), Some(peer)) = (i7p.as_mut(), i6c.as_ref(), sim_peer.as_mut()) else {
                return Err(step_failed("lipseste I7/I8"));
            };
            // The workstation's screen as read on the board (os-195 `hw`): framebuffer at the start of BAR1.
            let screen = nvidia_ga104::vram_window::screen_vram_range(0xe000_0000, 8192 * 1080, 0xe000_0000, gpu_golden::BAR1_LEN);
            let wpr2 = gpu_golden::read_wpr2_after_i7(&mut g, j)?;
            lines.push(format!("{}: golden: WPR2 dupa I7 {:#x}..{:#x}", p, wpr2.0, wpr2.1));
            let zone = gpu_golden::zone(&gpu_i7::layout(c)?, wpr2, c.fb_size, screen, gpu_golden::BAR1_LEN)?;
            lines.push(format!(
                "{}: golden: WPR2 = cel cerut de GspFwWprMeta; zona driverului {:#x}-{:#x} (restul protejat)",
                p, zone.start, zone.end
            ));
            let st = gs.insert(gpu_golden::GpuState::new(zone)?);
            let r = gpu_golden::golden_step(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, |m| peer.serve(m, &mut rm));
            lines.push(format!(
                "{}: modelul RM: {} mesaje servite; VRAM model: {} cuvinte prin fereastra, {} scrieri in alte registre",
                p, peer.served, vram.words, vram.stray_writes.len()
            ));
            r
        }
        "i9-canal-lucru" => {
            let (Some(prep), Some(peer), Some(st)) = (i7p.as_mut(), sim_peer.as_mut(), gs.as_mut()) else {
                return Err(step_failed("lipseste golden"));
            };
            let r = gpu_golden::channel_step(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, i9, |m| peer.serve(m, &mut rm));
            lines.push(format!("{}: modelul RM: {} mesaje servite in total", p, peer.served));
            // P6-106 (SIMULARE): as on the board, only in the cycle; the RM model answers the performance controls.
            if r.is_ok() && boost_wanted {
                // P6-142 (SIMULARE): the dashboard before the request; the RM model renders the page before each read.
                if rusd_early {
                    let plan = nvidia_ga104::gsp::rusd::RusdPlan { enabled: true, ..Default::default() };
                    let t0 = clock.now_us();
                    match gpu_golden::dashboard_begin(&mut g, &mut vram, &mut clock, j, prep, st, &plan, SIM_RUSD_PHYS, gpu_golden::DASHBOARD_EARLY, None, |m| peer.serve(m, &mut rm)) {
                        Err(e) => lines.push(format!("{}: kd tabloul de bord (devreme): eroare la pornire ({}); pagina ramane rezervata", p, e)),
                        Ok(start) => {
                            early = gpu_golden::EarlyDashboard::new(start, plan, SimRusdPage(vec![0u8; 4096]), SIM_RUSD_PHYS, t0);
                            if let Some(e) = early.as_mut() {
                                if e.start.polling() {
                                    clock.delay_us(plan.poll_ms.saturating_mul(3).saturating_mul(1000));
                                    sim_render(&rm, &mut clock, &mut e.page);
                                    e.before = e.read(p, "inainte de cererea de viteza maxima", &mut clock, j, &mut lines).map(|(d, _)| d);
                                } else {
                                    lines.push(format!("{}: kd tabloul de bord (devreme): placa nu a acceptat toate comenzile; fara citiri", p));
                                }
                            }
                        }
                    }
                }
                if early.as_ref().is_some_and(|e| e.says_stop()) {
                    lines.push(format!("{}: ks: cererea de viteza maxima NU se trimite: ceasul termic spune oprire", p));
                } else {
                    match gpu_golden::boost_begin(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, |m| peer.serve(m, &mut rm)) {
                        Ok(b) => boost = Some(b),
                        Err(e) => lines.push(format!("{}: ks: cererea de viteza maxima nu s-a putut trimite: {}", p, e)),
                    }
                    // P6-142 (SIMULARE, model DEDUS): an accepted request puts the dashboard's model in P0 with the core
                    // clock raised, as NVIDIA's driver shows a boosted board.
                    if boost.as_ref().is_some_and(|b| b.answer == nvidia_ga104::gsp::perf::BoostAnswer::Accepted) {
                        if let Some(m) = rm.rusd.as_mut() {
                            m.pstate_mask = 1;
                            m.clk_graphics_mhz = SIM_BOOST_CLK_MHZ;
                        }
                    }
                    if let (Some(e), true) = (early.as_mut(), boost.is_some()) {
                        clock.delay_us((nvidia_ga104::gsp::rusd_watch::after_wait_ms(e.plan.poll_ms) * 1000) as u32);
                        sim_render(&rm, &mut clock, &mut e.page);
                        if let (Some(b), Some((a, _))) = (e.before, e.read(p, "dupa cererea de viteza maxima (1 s)", &mut clock, j, &mut lines)) {
                            lines.push(gpu_golden::boost_seen_line(p, &b, &a, j));
                        }
                    }
                }
            }
            // P6-161 (SIMULARE): as on the board; the RM model accepts the copy object on the work channel.
            if r.is_ok() && RAM_VRAM_IN_CYCLE.load(core::sync::atomic::Ordering::SeqCst) {
                if let Err(e) = gpu_golden::copy_object_step(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, |m| peer.serve(m, &mut rm)) {
                    lines.push(format!("{}: motorul de copiere: obiectul nu s-a putut aloca ({}); proba RAM <-> VRAM nu ruleaza", p, e));
                }
            }
            r
        }
        "i10-add64" => {
            let Some(st) = gs.as_mut() else { return Err(step_failed("lipseste I9")) };
            let Some(token) = st.space.as_ref().map(|(_, w)| w.doorbell_token) else { return Err(step_failed("lipseste I9")) };
            let gpu = sim_gpu.insert(gpu_i10::SimGpu::new(token));
            let stray = vram.stray_writes.len();
            let r = gpu_i10::add64_step(p, &mut vram, &mut clock, j, st, None, &mut lines, &mut |v, j, off, val, ring| {
                gpu.ring(v, off, val, ring)?;
                sim_journal_doorbell(j, off, val);
                Ok(())
            });
            lines.push(format!(
                "{}: modelul GPU: {} sonerie(i) cu jetonul {:#x}, {} lansari, {} invalidari TLB, {} scrieri in alte registre",
                p, gpu.rung, gpu.token, gpu.launches, vram.invalidates.len(), vram.stray_writes.len() - stray
            ));
            launched = Some(r?);
            Ok(())
        }
        "i10-cost-lansare" => {
            let (Some(st), Some(l), Some(gpu)) = (gs.as_mut(), launched.as_mut(), sim_gpu.as_mut()) else {
                return Err(step_failed("lipseste add64"));
            };
            let r = gpu_i10::cost_step(p, &mut vram, &mut clock, j, st, l, None, &mut lines, &mut |v, j, off, val, ring| {
                gpu.ring(v, off, val, ring)?;
                sim_journal_doorbell(j, off, val);
                Ok(())
            });
            lines.push(format!("{}: modelul GPU: {} sonerii in total, {} lansari (ceasul modelului: 4000 ns pe lansare)", p, gpu.rung, gpu.launches));
            r.map(|_| ())
        }
        "i11-copiere" | "i12-q8-matvec" | "i13-strat" | "i14-simbol" => {
            let (Some(st), Some(l), Some(gpu)) = (gs.as_mut(), launched.as_mut(), sim_gpu.as_mut()) else {
                return Err(step_failed("lipseste I10"));
            };
            let stray = vram.stray_writes.len();
            let mut bell = |v: &mut gpu_sim::pramin::SimBar0Vram, j: &mut Journal, off: u64, val: u32, ring: &gpu_i10::Ring<'_>| {
                gpu.ring(v, off, val, ring)?;
                sim_journal_doorbell(j, off, val);
                Ok(())
            };
            let r = match s.id {
                "i11-copiere" => {
                    lines.push(format!("{}: I11 in SIMULARE: {} octeti (pe placa {}; acelasi cod)", p, gpu_i11::I11_SIM_BYTES, gpu_i11::I11_BOARD_BYTES));
                    gpu_i11::copy_step(p, &mut vram, &mut clock, j, st, l, gpu_i11::I11_SIM_BYTES, None, &mut lines, &mut bell)
                }
                "i12-q8-matvec" => gpu_i11::matvec_step(p, &mut vram, &mut clock, j, st, l, None, &mut lines, &mut bell),
                "i14-simbol" => {
                    lines.push(format!("{}: I14 in SIMULARE: modelul de control de pe discul de test (pe placa: {}; acelasi cod)", p, gpu_i14::MIC_BOARD));
                    gpu_i14::token_step(p, &mut vram, &mut clock, j, st, l, gpu_i14::MIC_SIM, None, &mut lines, &mut bell)
                }
                _ => {
                    lines.push(format!(
                        "{}: I13 in SIMULARE: strat E {} / {} capete (pe placa E {} / {}; acelasi cod)",
                        p, gpu_i11::I13_SIM.0, gpu_i11::I13_SIM.1, gpu_i11::I13_BOARD.0, gpu_i11::I13_BOARD.1
                    ));
                    gpu_i11::layer_step(p, &mut vram, &mut clock, j, st, l, gpu_i11::I13_SIM, None, &mut lines, &mut bell)
                }
            };
            lines.push(format!(
                "{}: modelul GPU: {} sonerii in total, {} lansari, {} scrieri in alte registre",
                p, gpu.rung, gpu.launches, vram.stray_writes.len() - stray
            ));
            // P6-98 (C-148 point 3): the performance state at the end of the chain, as on the board (the RM model answers).
            if s.id == "i14-simbol" && r.is_ok() {
                if let (Some(prep), Some(peer)) = (i7p.as_mut(), sim_peer.as_mut()) {
                    // P6-142 (SIMULARE): the early dashboard's last read while the request still holds, as on the board.
                    if let Some(e) = early.as_mut() {
                        sim_render(&rm, &mut clock, &mut e.page);
                        let _ = e.read(p, "la final (dupa texte, inainte de stergerea cererii)", &mut clock, j, &mut lines);
                    }
                    let cleared = boost.is_some();
                    let r = match boost.take() {
                        Some(b) => gpu_golden::boost_end(p, &mut g, &mut vram, &mut clock, j, prep, st, &b, None, &mut lines, |m| peer.serve(m, &mut rm)),
                        None => gpu_golden::pstate_step(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, |m| peer.serve(m, &mut rm)),
                    };
                    if let Err(e) = r {
                        lines.push(format!("{}: ks: starea de performanta necitita: {}", p, e));
                    }
                    // P6-142 (SIMULARE): the early dashboard stopped (its model back to idle once the request is cleared).
                    if let Some(mut e) = early.take() {
                        if cleared {
                            if let Some(m) = rm.rusd.as_mut() {
                                m.pstate_mask = 1 << 8;
                                m.clk_graphics_mhz = 210;
                            }
                        }
                        sim_render(&rm, &mut clock, &mut e.page);
                        lines.push(gpu_golden::watch_summary_line(p, &e.watch, j));
                        match gpu_golden::dashboard_finish(&mut g, &mut vram, &mut clock, j, prep, st, &e.start, &mut e.page, None, |m| peer.serve(m, &mut rm)) {
                            Ok((dash, stop)) => {
                                let l = gpu_golden::dashboard_lines(p, &e.start, dash.as_ref(), &stop);
                                *LAST_DASHBOARD.lock() = Some(l.clone());
                                lines.extend(l);
                            }
                            Err(err) => lines.push(format!("{}: kd tabloul de bord: eroare la oprire ({}); pagina ramane rezervata", p, err)),
                        }
                    } else if rusd_wanted {
                    // P6-105 (SIMULARE): the dashboard as on the board; the RM model renders the page during the wait.
                        let plan = nvidia_ga104::gsp::rusd::RusdPlan { enabled: true, ..Default::default() };
                        let phys = SIM_RUSD_PHYS;
                        match gpu_golden::dashboard_begin(&mut g, &mut vram, &mut clock, j, prep, st, &plan, phys, gpu_golden::DASHBOARD_BEGIN, None, |m| peer.serve(m, &mut rm)) {
                            Err(e) => lines.push(format!("{}: kd tabloul de bord: eroare la pornire ({}); pagina ramane rezervata", p, e)),
                            Ok(start) => {
                                let mut page = SimRusdPage(vec![0u8; 4096]);
                                if start.polling() {
                                    clock.delay_us(plan.poll_ms.saturating_mul(3).saturating_mul(1000));
                                    if let Some(m) = rm.rusd.as_ref() {
                                        let now = clock.now_us().saturating_mul(1000).max(1);
                                        let b = m.render(now);
                                        page.0[..b.len()].copy_from_slice(&b);
                                    }
                                }
                                match gpu_golden::dashboard_finish(&mut g, &mut vram, &mut clock, j, prep, st, &start, &mut page, None, |m| peer.serve(m, &mut rm)) {
                                    Ok((dash, stop)) => {
                                        let l = gpu_golden::dashboard_lines(p, &start, dash.as_ref(), &stop);
                                        *LAST_DASHBOARD.lock() = Some(l.clone());
                                        lines.extend(l);
                                        if rm.rusd.as_ref().is_some_and(|m| m.phys.is_none()) {
                                            lines.push(format!("{}: kd modelul: zona comuna oprita (adresa stearsa)", p));
                                        } else if rm.rusd.as_ref().is_some_and(|m| m.phys.is_some()) {
                                            // P6-115: no `INIT(0)` sent: the model still holds the page (as on the board).
                                            lines.push(format!("{}: kd modelul: zona comuna lasata pornita (adresa pastrata, ca driverul NVIDIA)", p));
                                        }
                                    }
                                    Err(e) => lines.push(format!("{}: kd tabloul de bord: eroare la oprire ({}); pagina ramane rezervata", p, e)),
                                }
                            }
                        }
                    }
                }
            }
            r
        }
        "bus-master-oprit" => match old {
            None => Ok(()),
            Some(o) => {
                restore_command(&mut cfg, &bm, j, o)?;
                lines.push(format!("{}: Bus Master oprit la loc (COMMAND {:#06x})", p, cfg.read32(gpu_i6::PCI_COMMAND) as u16));
                Ok(())
            }
        },
        "tur-dupa" => {
            let mut bar0 = gpu_sim::observed::observed_bar0();
            tour_on(p, &mut cfg, None, &mut bar0, &mut clock, j, &mut lines)
        }
        _ => Err(GpuError::NotPermitted(NOT_IN_OS)),
    });
    report_lines(p, &report, &mut lines);
    // P6-101 (C-152 point 1, SIMULARE): the FLR trial — WPR2 is up after the chain; the reset from inside must lower
    // it, then I6 runs a second time on the same model (as Cyber-GPU's T138).
    if flr_trial && report.stopped_at.is_none() {
        let r = sim_flr(&mut flr_cfg, &mut g, &mut clock, &flr, &mut j);
        lines.push(format!("{}: proba FLR: {}", p, r.row()));
        lines.push(format!("{}: proba FLR: {}", p, r.verdict.meaning()));
        if r.verdict.chain_may_run() {
            let again = sim_i6_once(p, &mut g, &mut clock, &mut j, frts, false, &mut lines)
                .and_then(|()| wpr2_step(p, &mut g, &mut j, frts, &mut lines));
            lines.push(format!(
                "{}: proba FLR: I6 a doua oara in aceeasi pornire: {}",
                p,
                match again {
                    Ok(()) => String::from("TRECE (WPR2 ridicat din nou)"),
                    Err(e) => format!("NU TRECE ({})", e),
                }
            ));
        }
    }
    let outside = g.writes.iter().filter(|&&(o, _)| !(gpu_i6::GSP_LO..=gpu_i6::GSP_HI).contains(&u64::from(o))).count();
    lines.push(format!(
        "{}: modelul: {} blocuri DMA copiate din RAM; {} scrieri falcon, {} in afara ferestrei GSP; configuratie: {} scrieri; COMMAND la final {:#06x}",
        p, g.dma_blocks, g.writes.len(), outside, cfg.writes - cfg_before, cfg.read32(gpu_i6::PCI_COMMAND) as u16
    ));
    gpu_i6::close_and_check(p, false, &mut lines);
    lines.push(format!(
        "{}: rezultat lant: {} ({} scrieri)",
        p,
        match &report.stopped_at {
            None => String::from("toti pasii au trecut"),
            Some((id, _)) => format!("oprit la {}", id),
        },
        report.writes
    ));
    gpu_i6::sim_lines(lines, "gpu lant")
}

/// SIMULATION: step I6 (FWSEC-FRTS) on the model `g`; also the second I6 of the FLR trial (P6-101).
#[cfg(feature = "test-gpu-sim")]
fn sim_i6_once(
    p: &str,
    g: &mut gpu_sim::fake_gsp::FakeGsp,
    clock: &mut gpu_sim::clock::SimClock,
    j: &mut Journal,
    frts: (u64, u64),
    fault: bool,
    lines: &mut Vec<String>,
) -> GpuResult<()> {
    use gpu_sim::fake_gsp::{SimFault, SimHsImage, SimUcode, GSP_BASE};
    use nvidia_ga104::gsp::hs_load::{self, HsUcode};
        let (desc, ucode, sig) = gpu_i6::sim_fwsec();
        let (u, img) = hs_load::fwsec_frts_image(&desc, &ucode, &sig, frts.0)
            .and_then(|img| Ok((HsUcode::from_fwsec_v3(&desc)?, img)))?;
        let image = match gpu_i6::dma_put(&img) {
            Ok(i) => i,
            Err(why) => {
                lines.push(format!("{}: zona DMA: {}", p, why));
                return Err(step_failed("zona DMA"));
            }
        };
        lines.push(format!("{}: zona DMA: {:#x} octeti la adresa fizica {:#x} (recitita)", p, image.len, image.phys));
        g.frts = (0, 0);
        g.fwsec_mapper_at = Some(0x340);
        if fault {
            g.fault = SimFault::FrtsError(0x00e1);
        }
        g.sim_expect(SimHsImage {
            falcon: GSP_BASE,
            ucode: SimUcode::FwsecFrts,
            imem_pa: 0,
            code: img[..0x1000].to_vec(),
            dmem_pa: 0,
            data: img[0x1000..0x1600].to_vec(),
            sig_dmem_addr: 0x100,
            ucode_id: 5,
            engine_id_mask: 0x400,
            bootvec: 0,
        });
        g.sysmem = Some((image.phys, gpu_i6::dma_bytes(&image)));
        if gpu_i6::i6_falcon(p, g, clock, j, &u, &image, frts, None, lines) {
            Ok(())
        } else {
            Err(step_failed("FWSEC-FRTS a esuat (vezi randul pasului I6)"))
        }
}

/// SIMULATION: the board for `flr_reset` (the FLR seen by the configuration model is passed on to the GSP model
/// before BAR0 is read again, as in Cyber-GPU's T138).
#[cfg(feature = "test-gpu-sim")]
struct SimFlrBoard<'a> {
    cfg: &'a mut gpu_sim::pci::SimConfig,
    gsp: &'a mut gpu_sim::fake_gsp::FakeGsp,
    seen: u32,
}

#[cfg(feature = "test-gpu-sim")]
impl<'a> nvidia_ga104::flr_reset::FlrBoard for SimFlrBoard<'a> {
    type Bar0 = gpu_sim::fake_gsp::FakeGsp;
    type Cfg = gpu_sim::pci::SimConfig;
    fn bar0(&mut self) -> &mut Self::Bar0 {
        while self.seen < self.cfg.flr_count {
            self.seen += 1;
            self.gsp.sim_flr();
        }
        self.gsp
    }
    fn cfg(&mut self) -> &mut Self::Cfg {
        self.cfg
    }
}

#[cfg(feature = "test-gpu-sim")]
fn sim_flr(
    cfg: &mut gpu_sim::pci::SimConfig,
    g: &mut gpu_sim::fake_gsp::FakeGsp,
    clock: &mut gpu_sim::clock::SimClock,
    permit: &gpu_pci::flr::FlrPermit,
    j: &mut Journal,
) -> nvidia_ga104::flr_reset::FlrReport {
    let seen = cfg.flr_count;
    let mut b = SimFlrBoard { cfg, gsp: g, seen };
    nvidia_ga104::flr_reset::reset_if_wpr2_up(&mut b, clock, permit, None, j)
}

/// P6-106 (SIMULARE): how the RM model answers the boost (`gpu lant --sim --ciclu-viteza [refuz]`); `None` = accept.
#[cfg(feature = "test-gpu-sim")]
pub static SIM_BOOST_RULE: crate::sched::Mutex<Option<gpu_sim::fake_rm::BoostRule>> = crate::sched::Mutex::new(None);

/// SIMULATION: the doorbell journaled as on the board (`ring_work_doorbell`), so the journal check sees it.
#[cfg(feature = "test-gpu-sim")]
fn sim_journal_doorbell(j: &mut Journal, off: u64, v: u32) {
    j.push(gpu_core::diag::Event::Note("soneria canalului de lucru (I10): scriere fara recitire"));
    j.push(gpu_core::diag::Event::MmioWrite { bar: 0, offset: off, before: 0, written: v, after: 0 });
}

/// SIMULATION: the PCI facts of the workstation model (gpu-sim), as `gpu_i8::system_info` reads them on the board.
#[cfg(feature = "test-gpu-sim")]
fn sim_system_info() -> nvidia_ga104::gsp::sysinfo::SystemInfoInput {
    use gpu_sim::workstation as w;
    nvidia_ga104::gsp::sysinfo::SystemInfoInput {
        bar0: w::BAR0_BASE,
        bar1: w::BAR1_BASE,
        bar3: w::BAR3_BASE,
        domain: 0,
        bus: w::BUS,
        slot: 0,
        vendor: w::VENDOR,
        device: w::DEVICE,
        subsys_vendor: w::SUBSYS_VENDOR,
        subsys_device: w::SUBSYS_DEVICE,
        revision: w::REVISION,
        is_primary: true,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// P6-62 step 3: `kronos --gpu`.
// ---------------------------------------------------------------------------------------------------------------

/// Set by the shell only after Adrian typed `DA` for `kronos --gpu` (the same question as `gpu lant`); taken by the
/// first `kronos_gpu` call. Without it the board is never touched and `/bin/kronos` stays on the processor.
pub static KRONOS_ARMED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// P6-95 (C-141): the I9 of the chain `kronos --gpu` runs. `None` = `c135`, the only I9 that passed on the real board
/// (os-305/os-306); the test cycle (P6-96) may set another one for one run. `gpu lant` without `--i9` is unchanged.
pub static KRONOS_I9: crate::sched::Mutex<Option<super::gpu_golden::I9Choice>> = crate::sched::Mutex::new(None);

/// The I9 of the next `kronos --gpu` chain (taken: it never outlives one run).
pub fn kronos_i9() -> super::gpu_golden::I9Choice {
    KRONOS_I9.lock().take().unwrap_or(super::gpu_golden::I9Choice::C135)
}

/// Kernel stack of the thread that runs the chain for `kronos --gpu`.
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
const KRONOS_STACK: usize = 256 * 1024;
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
static KRONOS_SIM: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
static KRONOS_LINES: crate::sched::Mutex<Option<Vec<String>>> = crate::sched::Mutex::new(None);

/// The chain thread of `kronos --gpu`: runs the chain once, hands its lines to the waiting system call.
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
fn kronos_chain_thread() {
    let sim = KRONOS_SIM.load(core::sync::atomic::Ordering::SeqCst);
    let i9 = kronos_i9();
    #[cfg(feature = "test-gpu-sim")]
    let lines = if sim { chain_sim(false, true, None, i9, false) } else { chain(None, i9) };
    #[cfg(not(feature = "test-gpu-sim"))]
    let lines = {
        let _ = sim;
        chain(None, i9)
    };
    if let Some((used, size)) = crate::sched::stack_used() {
        crate::info!("gpu: kronos --gpu: stiva firului lantului: {} KiB folositi din {} KiB", used.div_ceil(1024), size / 1024);
    }
    *KRONOS_LINES.lock() = Some(lines);
}

/// What a `kronos --gpu` call gives back to `/bin/kronos` (P6-99): a question of the check after the board (the CPU
/// logits of `token` at `pos`, from its fast engine), or the end: the board's greedy tokens (or why not) and the chain's lines.
pub enum KronosStep {
    Verifica { token: usize, pos: usize },
    Gata(Result<Vec<usize>, String>, Vec<String>),
}

/// A `kronos --gpu` request (call 43 from `/bin/kronos`): the whole window chain with the request queued for its
/// `i14-simbol` step (the board is set up only inside the chain; every token judged against the processor). Returns the
/// board's greedy tokens, or why not (then `/bin/kronos` generates on the processor), and the chain's lines; with
/// `os_check` (P6-99, C-148 point 2) first the questions of the check after the board, one per call (answered by
/// [`kronos_answer`]).
pub fn kronos_gpu(path: &str, prompt: Vec<usize>, n: usize, sim: bool, os_check: bool) -> KronosStep {
    use core::sync::atomic::Ordering;
    if !KRONOS_ARMED.swap(false, Ordering::SeqCst) {
        return KronosStep::Gata(Err(String::from("fara DA de la shell (kronos --gpu se porneste doar din shell, cu DA)")), Vec::new());
    }
    if sim && !cfg!(feature = "test-gpu-sim") {
        return KronosStep::Gata(Err(String::from("--gpu-sim exista doar in build-ul de test")), Vec::new());
    }
    if !sim && !cfg!(feature = "gpu-scrie") {
        return KronosStep::Gata(Err(String::from("acest sistem nu poate scrie in placa (doar stick-ul de test, gpu-scrie)")), Vec::new());
    }
    #[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
    {
        *gpu_i14::KRONOS_RESULT.lock() = None;
        *gpu_i14::KRONOS_JOB.lock() = Some(gpu_i14::KronosJob { path: String::from(path), prompt, n, os_check });
        {
            let mut c = gpu_i14::OS_CHECK.lock();
            c.owner = if os_check { crate::sched::current_id() } else { 0 };
            c.want = None;
            c.handed = false;
            c.answer = None;
        }
        crate::info!(
            "gpu: kronos --gpu: lantul porneste{} cu cererea la i14-simbol{}",
            if sim { " (SIMULARE)" } else { "" },
            if os_check { "; verificarea dupa placa: /bin/kronos (motorul rapid)" } else { "" }
        );
        // The chain is deep (the window steps, the falcon/GSP models in SIMULATION, the model at I14): it runs on its
        // own kernel thread with a big stack, not on this program's 16 KiB kernel stack (P6-60: that overflowed and
        // corrupted the heap). This thread waits for it.
        KRONOS_SIM.store(sim, Ordering::SeqCst);
        *KRONOS_LINES.lock() = None;
        crate::sched::spawn_with_stack(kronos_chain_thread, KRONOS_STACK);
        kronos_wait()
    }
    #[cfg(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")))]
    {
        let _ = (path, prompt, n, os_check);
        KronosStep::Gata(Err(String::from("acest sistem nu poate scrie in placa")), Vec::new())
    }
}

/// P6-99: `/bin/kronos`'s answer to the question at `pos` (its fast engine's logits, or why it could not compute them);
/// then waits for the next question or the end. `None` = this thread made no `kronos --gpu` request with the check. An
/// answer to a question the chain no longer waits for (it gave up) is dropped.
pub fn kronos_answer(pos: usize, logits: Result<Vec<f32>, String>) -> Option<KronosStep> {
    #[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
    {
        {
            let mut c = gpu_i14::OS_CHECK.lock();
            if c.owner == 0 || c.owner != crate::sched::current_id() {
                return None;
            }
            if c.handed && c.want.map(|w| w.1) == Some(pos) {
                c.answer = Some(logits);
            }
        }
        Some(kronos_wait())
    }
    #[cfg(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")))]
    {
        let _ = (pos, logits);
        None
    }
}

/// The calling thread waits for the chain's end or (with the check after the board) its next question.
#[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
fn kronos_wait() -> KronosStep {
    let me = crate::sched::current_id();
    let got = gpu_i14::wait_for(u64::MAX, || {
        if let Some(l) = KRONOS_LINES.lock().take() {
            return Some(Err(l));
        }
        let mut c = gpu_i14::OS_CHECK.lock();
        match c.want {
            Some((token, pos)) if c.owner == me && !c.handed => {
                c.handed = true;
                Some(Ok((token, pos)))
            }
            _ => None,
        }
    });
    let lines = match got {
        Some(Ok((token, pos))) => return KronosStep::Verifica { token, pos },
        Some(Err(lines)) => lines,
        None => return KronosStep::Gata(Err(String::from("programul a fost oprit inainte de sfarsitul lantului")), Vec::new()),
    };
    gpu_i14::OS_CHECK.lock().owner = 0;
    // A chain that stopped before I14 never took the request.
    let left = gpu_i14::KRONOS_JOB.lock().take().is_some();
    let res = gpu_i14::KRONOS_RESULT.lock().take().unwrap_or_else(|| {
        Err(String::from(if left { "lantul s-a oprit inainte de i14-simbol" } else { "niciun raspuns de la i14-simbol" }))
    });
    KronosStep::Gata(res, lines)
}
