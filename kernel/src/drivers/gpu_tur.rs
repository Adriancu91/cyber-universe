//! P6-42 (command C-55): `gpu tur` and `gpu lant`, the OS side of Cyber-GPU's C-54
//! (`integration/cyber-universe/C-54_gpu-tur-si-lant.md`, hub proposal `idei/2026-11-01_cyber-gpu-tur-si-lant.md`).
//!
//! - `gpu tur`: READ ONLY. `nvidia_ga104::read_tour::run_tour` through the read-only configuration adapter and the
//!   read-only 16 MiB BAR0 window: PCI identity, BOOT_0/42, GFW, the VBIOS layout, FWSEC, the I6 preparation. One line
//!   `fapt <key> = <value>` per fact (the format `compara_raport` of Cyber-GPU reads back), then the write count, which
//!   must be 0. On the allow-list of the automatic loop (C-53).
//! - `gpu lant`: the window steps one after the other. P6-55 (C-71): the whole window chain of Cyber-GPU
//!   (`run_chain_ready(WINDOW_CHAIN_FULL, READY_FOR_WINDOW, ..)`, I6 to the first computation). The OS has the I6
//!   preparation, Bus Master on, FWSEC-FRTS, the WPR2 read after I6 (`i6-wpr2-citit`, reads only), Bus Master back and
//!   the tour after; P6-56 (C-72): I7 (Booter on SEC2, `gpu_i7.rs`) and `i7-citit`; I8-I10 are not in the OS yet, so the chain stops honestly at `i8-gsp-init`. The driver owns the order and the stop rules; this file only does each step. Writes are allowed
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
use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, ConfigSpace, Mmio};
use gpu_core::PciAddress;
use nvidia_ga104::chain::{run_chain_ready, ChainReport, READY_FOR_WINDOW, WINDOW_CHAIN_FULL};
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
    let t = run_tour(cfg, addr, bar0, clock, journal)?;
    for f in &t.facts {
        lines.push(format!("{}", f));
    }
    lines.push(format!("{}: tur de citire (C-54): {} fapte, {} citiri jurnalizate", p, t.facts.len(), t.reads));
    Ok(())
}

/// The chain report, one line per step, with the board name in front.
fn report_lines(p: &str, r: &ChainReport, lines: &mut Vec<String>) {
    lines.push(format!("{}: lantul pasilor (C-54):", p));
    for l in format!("{}", r).lines() {
        lines.push(format!("{}: {}", p, l.trim_start()));
    }
}

/// P6-55: the error of a step of the full chain that the OS does not have yet (I7-I10).
const NOT_IN_OS: &str = "pas fara implementare in OS (I8-I10 exista doar in SIMULAREA cyber-gpu, C-71; P6-56 pas 2-3)";

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

/// `gpu lant` on the real board (the shell asked for `DA` first). Writes only in the `gpu-scrie` build.
pub fn chain() -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
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
    let mut refused = 0usize;
    let report = run_chain_ready(WINDOW_CHAIN_FULL, READY_FOR_WINDOW, writes_allowed, &mut j, |s, j| match s.id {
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
            let prep = {
                let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
                let r = gpu_i7::prepare(p, &mut win, j, &fw, c, &mut lines);
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
        // Writing steps in a build that cannot write never get here (the chain stops before them). I8-I10 (C-71) are
        // written and tested only in Cyber-GPU's simulation: not in the OS yet (P6-56 steps 2-3), so the chain stops
        // here honestly.
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
pub fn chain_sim(fault: bool, writes_allowed: bool) -> Vec<String> {
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
    let mut clock = gpu_sim::clock::SimClock::default();
    let mut ro = gpu_i6::sim_board();
    let mut cfg = gpu_sim::observed::observed_config();
    let cfg_before = cfg.writes;
    let bm = BusMasterPermit::new("C-55 gpu lant --sim (P6-42)");
    let mut old: Option<u16> = None;
    let mut frts = (0u64, 0u64);
    let mut i6c: Option<nvidia_ga104::i6_check::I6Check> = None;
    let mut g = FakeGsp::new();
    let report = run_chain_ready(WINDOW_CHAIN_FULL, READY_FOR_WINDOW, writes_allowed, &mut j, |s, j| match s.id {
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
            let prep = gpu_i7::prepare(p, &mut ro, j, &fw, c, &mut lines)?;
            gpu_i7::sim_expect(&mut g, &prep);
            gpu_i7::booter(p, &mut g, &mut clock, j, &prep, None, &mut lines)
        }
        "i7-citit" => gpu_i7::after(p, &mut g, j, &mut lines),
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
