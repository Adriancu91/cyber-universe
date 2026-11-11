//! P6-74 (command C-103, Adrian's decision of 11 nov, variant 1): `gpu recupereaza` — the board brought back to the
//! state I6 expects WITHOUT restarting the computer.
//!
//! After a chain stopped at I8, WPR2 stays raised and the next `gpu lant` stops at the I6 preparation ("WPR2 deja
//! ridicat: oprire completa"). The public way the NVIDIA driver (570.144, MIT) and nouveau tear WPR2 down is the signed
//! `booter_unload-570.144.bin` (already on the stick, the 4th I4 file) run on SEC2 with mailboxes (0xFF, 0xFF); then
//! `WPR2_ADDR_HI` must read 0. Cyber-GPU (`falcon::booter_unload_dma`, SIMULATION only so far) does the order: GSP reset,
//! SEC2 reset, Booter Unload, mailbox 0, WPR2 read back. What the OS does here:
//! - [`prepare`] — nothing written to the card: the Booter Unload image with the signature chosen by the SEC2 fuse (one
//!   read), in its own pinned DMA area between 16 MiB and 4 GiB (read back);
//! - [`unload`] — `booter_unload_dma` through the caller's guarded window (GSP + SEC2 only), then the I6 read-only
//!   pre-check again, so the journal and the screen say whether I6 may run again in the same power-on.
//!
//! On the board: only in the test-stick build (`gpu-scrie`), after `DA`, with Adrian there; Bus Master on only around
//! the program (SEC2 reads the image by DMA) and put back as it was. Never tried on the board: a first try.
//! The NVIDIA files are never in the repo, the tests or the stick archive (the tests use `gpu_sim::fw_stick`).
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml), notes `C-103_gpu-recupereaza.md` and
//! `docs/C-103_RESET_PLACA.md` of Cyber-GPU. Our own code.

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use super::gpu_i6;
use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::firmware::GspFirmware;
use nvidia_ga104::fw_hdr::HsV2;
use nvidia_ga104::gsp::falcon::{self, GuardedBar0, Unloaded};
use nvidia_ga104::gsp::hs_load::{self, BooterMeta, DmaImage, HsUcode};
use nvidia_ga104::gsp::HardwareWritePermit;

const PAGE: u64 = 4096;

/// The Booter Unload program, ready in RAM.
pub(super) struct UnloadPrep {
    pub ucode: HsUcode,
    pub image: DmaImage,
}

/// Its own pinned area (never given back: SEC2 reads it by DMA; reused by the next `gpu recupereaza`).
static AREA: crate::sched::Mutex<Option<(u64, u64)>> = crate::sched::Mutex::new(None);

fn area(pages: u64) -> Result<u64, String> {
    let mut slot = AREA.lock();
    if let Some((p, n)) = *slot {
        if n >= pages {
            return Ok(p);
        }
    }
    let p = crate::mem::frame::alloc_contiguous_between(pages, 16 << 20, 1 << 32)
        .ok_or_else(|| format!("nu exista {} KiB contigui intre 16 MiB si 4 GiB", pages * 4))?;
    crate::mem::frame::pin(p, pages, "gpu-recupereaza"); // SEC2 reads here by DMA: never given to anyone else
    *slot = Some((p, pages));
    Ok(p)
}

fn fail(what: &'static str) -> GpuError {
    GpuError::InvalidArgument(what)
}

/// Booter Unload in RAM: header, fuse-chosen signature, image copied and read back. Nothing written to the card.
pub(super) fn prepare<M: Mmio + ?Sized>(
    p: &str,
    bar0: &mut M,
    journal: &mut Journal,
    fw: &GspFirmware,
    lines: &mut Vec<String>,
) -> GpuResult<UnloadPrep> {
    let h = HsV2::parse(&fw.booter_unload)?;
    let meta = BooterMeta::parse(&fw.booter_unload, &h)?;
    let fuse = nvidia_ga104::fuse::booter_fuse_version(bar0, journal, &meta)?;
    let img = hs_load::booter_image(&h, &fw.booter_unload, u32::from(fuse))?;
    let ucode = HsUcode::from_booter(&h, &fw.booter_unload)?;
    let pages = (img.len() as u64).div_ceil(PAGE).max(1);
    let base = area(pages).map_err(|why| {
        lines.push(format!("{}: C-103 zona DMA: {}; nimic scris in placa", p, why));
        fail("C-103: zona DMA")
    })?;
    let va = crate::mem::map::phys_to_virt(base);
    // SAFETY: [base, base + pages * 4 KiB) are frames allocated by `area` for this area only (pinned, never freed),
    // reached through the HHDM; `img.len() <= pages * 4 KiB`.
    let all = unsafe { core::slice::from_raw_parts_mut(va, (pages * PAGE) as usize) };
    all[..img.len()].copy_from_slice(&img);
    all[img.len()..].fill(0);
    if all[..img.len()] != img[..] {
        lines.push(format!("{}: C-103 pregatire: recitirea din RAM difera; nimic scris in placa", p));
        return Err(fail("C-103: recitirea zonei DMA"));
    }
    let image = DmaImage { phys: base, len: img.len() as u32 };
    lines.push(format!(
        "{}: C-103 pregatire: Booter Unload {} B la {:#x} (recitit; siguranta SEC2 ucode {} = versiunea {}); nimic scris in placa",
        p, img.len(), base, meta.ucode_id, fuse
    ));
    Ok(UnloadPrep { ucode, image })
}

/// `booter_unload_dma` through the guard (GSP + SEC2 windows), and the line for the screen.
pub(super) fn unload<W: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    rw: &mut W,
    clock: &mut K,
    journal: &mut Journal,
    prep: &UnloadPrep,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
) -> GpuResult<Unloaded> {
    let r = GuardedBar0::new(rw, journal, permit).and_then(|mut b| falcon::booter_unload_dma(&mut b, clock, &prep.ucode, &prep.image));
    match &r {
        Ok(Unloaded::Wpr2Coborat) => {
            lines.push(format!("{}: pas C-103 Booter Unload: ok - WPR2 coborat (citit 0 dupa program)", p))
        }
        Ok(Unloaded::Wpr2NuEraRidicat) => lines.push(format!(
            "{}: C-103: WPR2 nu era ridicat: Booter Unload nu s-a rulat (nimic de coborat, ca driverul NVIDIA)",
            p
        )),
        Err(GpuError::HardwareFault { code }) => lines.push(format!(
            "{}: pas C-103 Booter Unload: ESEC: cutia SEC2 0 = {:#x} (codul se noteaza; urmatoarea varianta: FWSEC-SB inainte)",
            p, code
        )),
        Err(e) => lines.push(format!("{}: pas C-103 Booter Unload: ESEC: {}", p, e)),
    }
    r
}

/// After the program: the I6 read-only pre-check (WPR2 down = I6 may run again). `true` = it passes.
fn i6_again<M: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    bar0: &mut M,
    journal: &mut Journal,
    clock: &mut K,
    lines: &mut Vec<String>,
    after: bool,
) -> bool {
    match gpu_i6::prepare(p, bar0, journal, clock, lines) {
        Some((_, c)) if c.passes() => {
            lines.push(format!("{}: C-103: pregatirea I6 trece din nou: `gpu lant` poate rula in aceeasi pornire", p));
            true
        }
        _ if after => {
            lines.push(format!("{}: C-103: pregatirea I6 NU trece: tot pornire rece (cablul scos 30 s)", p));
            false
        }
        _ => {
            lines.push(format!("{}: C-103: pregatirea I6 NU trece inainte (asteptat: WPR2 ridicat de lantul oprit)", p));
            false
        }
    }
}

/// `gpu recupereaza` on the real board (the shell asked for `DA` first). Writes only in the `gpu-scrie` build.
pub fn recover() -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
    #[cfg(not(feature = "gpu-scrie"))]
    {
        lines.push(format!(
            "{}: gpu recupereaza: exista doar pe stick-ul de test (optiunea gpu-scrie); acest sistem nu poate scrie in placa video",
            p
        ));
        gpu_i6::finish(lines)
    }
    #[cfg(feature = "gpu-scrie")]
    {
        use super::gpu::{ReadOnlyBar0Window, WINDOW_LOCK};
        let Some((d, bar0)) = gpu_i6::board(&mut lines) else { return gpu_i6::finish(lines) };
        let permit = match HardwareWritePermit::new("C-103 gpu recupereaza: fereastra cu Adrian (P6-74)") {
            Ok(x) => x,
            Err(e) => {
                lines.push(format!("{}: {}", p, e));
                return gpu_i6::finish(lines);
            }
        };
        if let Err(why) = gpu_i6::live_open("gpu recupereaza") {
            lines.push(format!("{}: jurnalul pe stick nu se poate scrie ({}): STOP, nimic scris in placa", p, why));
            return gpu_i6::finish(lines);
        }
        let mut j = gpu_i6::journal(true);
        let mut clock = super::gpu::TscClock;
        let _guard = WINDOW_LOCK.lock();
        let mut refused = 0usize;
        // 1. Everything in RAM first; any failure stops before the first write.
        let prep = (|| -> GpuResult<UnloadPrep> {
            let fw = super::gpu_i7::stick_firmware(p, &mut j, &mut lines)?;
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            let r = prepare(p, &mut win, &mut j, &fw, &mut lines);
            refused += win.refused;
            r
        })();
        if let Ok(prep) = prep {
            // 2. Bus Master on only around the program (SEC2 reads the image by DMA), put back as it was.
            let bm = gpu_pci::bus_master::BusMasterPermit::new("C-103 gpu recupereaza (P6-74)");
            let mut cfg = gpu_i6::hw::BusMasterOnlyConfig { dev: &d, refused: 0 };
            match gpu_pci::bus_master::set_bus_master(&mut cfg, &bm, &mut j, true) {
                Ok(old) => {
                    lines.push(format!("{}: Bus Master pornit (COMMAND {:#06x} -> {:#06x})", p, old, d.command()));
                    let mut rw = gpu_i6::hw::GspWriteWindow::with_sec2(bar0);
                    let _ = unload(p, &mut rw, &mut clock, &mut j, &prep, Some(&permit), &mut lines);
                    refused += rw.refused;
                    drop(rw);
                    match gpu_pci::bus_master::restore_command(&mut cfg, &bm, &mut j, old) {
                        Ok(()) => lines.push(format!("{}: Bus Master oprit la loc (COMMAND {:#06x}, ca inainte)", p, d.command())),
                        Err(e) => lines.push(format!("{}: Bus Master NU s-a putut pune la loc: {}", p, e)),
                    }
                }
                Err(e) => lines.push(format!("{}: Bus Master nu s-a pornit: {}; Booter Unload nerulat", p, e)),
            }
            refused += cfg.refused;
            // 3. Read only: may I6 run again?
            let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
            i6_again(p, &mut win, &mut j, &mut clock, &mut lines, true);
            refused += win.refused;
        }
        drop(_guard);
        gpu_i6::close_and_check(p, false, &mut lines);
        lines.push(format!("{}: configuratie: COMMAND la final {:#06x}; {} scrieri refuzate de garduri", p, d.command(), refused));
        gpu_i6::finish(lines)
    }
}

/// `gpu recupereaza --sim [--eroare]` (test builds only): the models after a chain stopped at I8 (WPR2 raised as the
/// board left it on 11 nov, GSP RISC-V on), Booter Unload with the gpu-sim stand-in (NOT the NVIDIA file) through the
/// OS's real DMA area, then the I6 pre-check on the model. `fault`: WPR2 stays raised after the program.
#[cfg(feature = "test-gpu-sim")]
pub fn recover_sim(fault: bool) -> Vec<String> {
    use gpu_sim::fake_gsp::{FakeGsp, SimFault, SimHsImage, SimUcode, SEC2_BASE};
    let p = "model";
    let mut lines = Vec::new();
    let what = if fault { "gpu recupereaza --sim --eroare" } else { "gpu recupereaza --sim" };
    if let Err(why) = gpu_i6::live_open(what) {
        lines.push(format!("{}: jurnalul pe disc nu se poate scrie ({}): STOP", p, why));
        return gpu_i6::sim_lines(lines, "gpu recupereaza");
    }
    let mut j = gpu_i6::journal(true);
    let mut clock = gpu_sim::clock::SimClock::default();
    let mut ro = gpu_i6::sim_board();
    // The state a stopped chain leaves: WPR2 raised (values read on the board after I7, 5 nov), GSP-RM "running".
    const WPR2_LO: u32 = 0x1fa824;
    const WPR2_HI: u32 = 0x1fa828;
    let mut g = FakeGsp::new();
    g.write32_raw(u64::from(WPR2_LO), 0x01f4_1000);
    g.write32_raw(u64::from(WPR2_HI), 0x01ff_ee00);
    ro.set(u64::from(WPR2_LO), 0x01f4_1000);
    ro.set(u64::from(WPR2_HI), 0x01ff_ee00);
    g.writes.clear();
    if fault {
        g.fault = SimFault::Wpr2Stuck;
    }
    lines.push(format!("{}: lant oprit la I8 (WPR2 ridicat 0x1f4100000..0x1ffee0000)", p));
    let before = i6_again(p, &mut ro, &mut j, &mut clock, &mut lines, false);
    let r = (|| -> GpuResult<Unloaded> {
        let fw = super::gpu_i7::sim_firmware(&mut j)?;
        lines.push(format!("{}: C-103: fisierele de proba gpu-sim (NU fisierele NVIDIA) verificate", p));
        let prep = prepare(p, &mut ro, &mut j, &fw, &mut lines)?;
        let img = gpu_i6::dma_bytes(&prep.image);
        let u = &prep.ucode;
        let span = |o: u32, n: u32| img[o as usize..(o + n) as usize].to_vec();
        g.sim_expect(SimHsImage {
            falcon: SEC2_BASE,
            ucode: SimUcode::BooterUnload,
            imem_pa: u.imem_pa,
            code: span(u.code_offset, u.imem_size),
            dmem_pa: u.dmem_pa,
            data: span(u.data_offset, u.dmem_size),
            sig_dmem_addr: u.sig_dmem_addr,
            ucode_id: u32::from(u.ucode_id),
            engine_id_mask: u.engine_id_mask,
            bootvec: u.imem_va,
        });
        g.sysmem = Some((prep.image.phys, img.clone()));
        unload(p, &mut g, &mut clock, &mut j, &prep, None, &mut lines)
    })();
    // The read model follows what the falcon model now says about WPR2.
    let (lo, hi) = (g.read32_raw(u64::from(WPR2_LO)), g.read32_raw(u64::from(WPR2_HI)));
    ro.set(u64::from(WPR2_LO), lo);
    ro.set(u64::from(WPR2_HI), hi);
    let after = i6_again(p, &mut ro, &mut j, &mut clock, &mut lines, true);
    gpu_i6::close_and_check(p, false, &mut lines);
    let wpr2_writes = g.writes.iter().filter(|&&(o, _)| o == WPR2_LO || o == WPR2_HI).count();
    lines.push(format!(
        "{}: {} scrieri falcon (GSP + SEC2), {} in registrele WPR2; I6 inainte: {}, dupa: {}",
        p,
        g.writes.len(),
        wpr2_writes,
        if before { "trece" } else { "NU trece" },
        if after { "trece" } else { "NU trece" }
    ));
    lines.push(format!(
        "{}: rezultat recupereaza: {}",
        p,
        match (&r, after) {
            (Ok(Unloaded::Wpr2Coborat), true) => "TRECE (WPR2 coborat, I6 poate rula din nou)",
            _ => "NU TRECE",
        }
    ));
    gpu_i6::sim_lines(lines, "gpu recupereaza")
}
