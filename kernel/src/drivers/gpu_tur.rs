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
use alloc::{format, string::String, vec::Vec};
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
pub fn chain(variant: Option<u8>) -> Vec<String> {
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
    let mut refused = 0usize;
    let report = run_chain_ready(WINDOW_CHAIN_C83, READY_IN_OS, writes_allowed, &mut j, |s, j| match s.id {
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
            let r = gpu_golden::channel_step(p, &mut rw, &mut vf, &mut clock, j, prep, st, Some(&permit), &mut lines, |_| {});
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
    });
    drop(_guard);
    report_lines(p, &report, &mut lines);
    if refused > 0 {
        lines.push(format!("{}: adaptoarele OS au REFUZAT {} scrieri cerute in afara gardului", p, refused));
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
pub fn chain_sim(fault: bool, writes_allowed: bool, variant: Option<u8>) -> Vec<String> {
    use gpu_pci::bus_master::{restore_command, set_bus_master, BusMasterPermit};
    use gpu_sim::fake_gsp::{FakeGsp, SimFault, SimHsImage, SimUcode, GSP_BASE};
    use nvidia_ga104::gsp::hs_load::{self, HsUcode};
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
    let mut i7p: Option<gpu_i7::I7Prep> = None;
    let mut sim_peer: Option<gpu_i8::SimPeer> = None;
    let mut gs: Option<gpu_golden::GpuState> = None;
    let mut rm = gpu_sim::fake_rm::FakeRm::new();
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
        "i6-fwsec-frts" => {
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
            if gpu_i6::i6_falcon(p, &mut g, &mut clock, j, &u, &image, frts, None, &mut lines) {
                Ok(())
            } else {
                Err(step_failed("FWSEC-FRTS a esuat (vezi randul pasului I6)"))
            }
        }
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
            let r = gpu_golden::channel_step(p, &mut g, &mut vram, &mut clock, j, prep, st, None, &mut lines, |m| peer.serve(m, &mut rm));
            lines.push(format!("{}: modelul RM: {} mesaje servite in total", p, peer.served));
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
    #[cfg(feature = "test-gpu-sim")]
    let lines = if sim { chain_sim(false, true, None) } else { chain(None) };
    #[cfg(not(feature = "test-gpu-sim"))]
    let lines = {
        let _ = sim;
        chain(None)
    };
    if let Some((used, size)) = crate::sched::stack_used() {
        crate::info!("gpu: kronos --gpu: stiva firului lantului: {} KiB folositi din {} KiB", used.div_ceil(1024), size / 1024);
    }
    *KRONOS_LINES.lock() = Some(lines);
}

/// A `kronos --gpu` request (call 43 from `/bin/kronos`): the whole window chain with the request queued for its
/// `i14-simbol` step (the board is set up only inside the chain; every token judged against the processor). Returns the
/// board's greedy tokens, or why not (then `/bin/kronos` generates on the processor), and the chain's lines.
pub fn kronos_gpu(path: &str, prompt: Vec<usize>, n: usize, sim: bool) -> (Result<Vec<usize>, String>, Vec<String>) {
    use core::sync::atomic::Ordering;
    if !KRONOS_ARMED.swap(false, Ordering::SeqCst) {
        return (Err(String::from("fara DA de la shell (kronos --gpu se porneste doar din shell, cu DA)")), Vec::new());
    }
    if sim && !cfg!(feature = "test-gpu-sim") {
        return (Err(String::from("--gpu-sim exista doar in build-ul de test")), Vec::new());
    }
    if !sim && !cfg!(feature = "gpu-scrie") {
        return (Err(String::from("acest sistem nu poate scrie in placa (doar stick-ul de test, gpu-scrie)")), Vec::new());
    }
    #[cfg(any(feature = "gpu-scrie", feature = "test-gpu-sim"))]
    {
        *gpu_i14::KRONOS_RESULT.lock() = None;
        *gpu_i14::KRONOS_JOB.lock() = Some(gpu_i14::KronosJob { path: String::from(path), prompt, n });
        crate::info!("gpu: kronos --gpu: lantul porneste{} cu cererea la i14-simbol", if sim { " (SIMULARE)" } else { "" });
        // The chain is deep (the window steps, the falcon/GSP models in SIMULATION, the model at I14): it runs on its
        // own kernel thread with a big stack, not on this program's 16 KiB kernel stack (P6-60: that overflowed and
        // corrupted the heap). This thread waits for it.
        KRONOS_SIM.store(sim, Ordering::SeqCst);
        *KRONOS_LINES.lock() = None;
        crate::sched::spawn_with_stack(kronos_chain_thread, KRONOS_STACK);
        let lines = loop {
            if let Some(l) = KRONOS_LINES.lock().take() {
                break l;
            }
            crate::sched::sleep_ms(20);
        };
        // A chain that stopped before I14 never took the request.
        let left = gpu_i14::KRONOS_JOB.lock().take().is_some();
        let res = gpu_i14::KRONOS_RESULT.lock().take().unwrap_or_else(|| {
            Err(String::from(if left { "lantul s-a oprit inainte de i14-simbol" } else { "niciun raspuns de la i14-simbol" }))
        });
        (res, lines)
    }
    #[cfg(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")))]
    {
        let _ = (path, prompt, n);
        (Err(String::from("acest sistem nu poate scrie in placa")), Vec::new())
    }
}
