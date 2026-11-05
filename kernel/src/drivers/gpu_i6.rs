//! P6-40 (command C-52): the first writes to the graphics card from the OS — step I6 of Cyber-GPU (FWSEC-FRTS on the
//! GSP falcon), plan `docs/FEREASTRA_C-49.md` of Cyber-GPU, proposal `idei/2026-11-01_cyber-gpu-i6-in-os.md` of the hub.
//!
//! Three pieces:
//! - `gpu pregatire-i6`: READ ONLY. Through the read-only BAR0 window (now 16 MiB long, still mapped one page at a
//!   time without write permission): where FWSEC is in the VBIOS, its fuse and the signature it picks, the VRAM size,
//!   whether WPR2 is already up, the FRTS region (`nvidia_ga104::i6_check`).
//! - the live journal: every register access the Cyber-GPU libraries make is written, with the time, to
//!   `/usb/raport/gpu-jurnal-NNN.txt` the moment it happens (`Journal::with_sink`), so a hang leaves the accesses
//!   before it on the stick. After the command the file is read back and checked with `nvidia_ga104::i6_log`.
//! - `gpu i6 --scrie`: the writes. Bus Master on (one write to the PCI Command register, bit 2 only), FWSEC copied
//!   from the VBIOS into contiguous RAM below 4 GiB and loaded into the GSP falcon over DMA (18 registers, all in the
//!   GSP window 0x110000..0x113FFF, through `falcon::GuardedBar0`), then Bus Master back to its old value — also when
//!   I6 fails. The adapters that can reach the card exist ONLY in builds with the `gpu-scrie` option (the test stick;
//!   it also turns on `nvidia-ga104/hw-write`); the normal and the test builds cannot write to a real card at all.
//!   They add their own fence under the library's: configuration writes only to Command and only bit 2; BAR0 writes
//!   only inside the GSP window. On the real board it runs only with Adrian present, after he types `DA`.
//!
//! The same sequence (`run_writes`) runs on the gpu-sim models in test builds (`--sim`): the falcon model reads the
//! image back from the real DMA area of the OS, so the test proves the copy, the addresses, the order and the
//! Bus-Master restore, not only the library.
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml); PCI Local Bus 3.0 §6.2.2 (Command register).

#![cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell

use super::gpu::{self, ReadOnlyBar0Window, ReadOnlyConfig, TscClock, CYBER_GPU_REV, NVIDIA, WINDOW_LOCK};
use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::{Event, Journal};
use gpu_core::hal::{Clock, ConfigSpace, Mmio};
use nvidia_ga104::fwsec::FwsecLocation;
use nvidia_ga104::gsp::falcon::{self, GuardedBar0};
use nvidia_ga104::gsp::hs_load::{self, DmaImage, HsUcode};
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::i6_check::I6Check;

/// The GSP falcon window, the only BAR0 range I6 writes (`i6_log::GSP_LO..=GSP_HI`).
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
pub const GSP_LO: u64 = 0x11_0000;
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
pub const GSP_HI: u64 = 0x11_3FFF;
/// PCI Command register and its Bus Master bit.
pub(super) const PCI_COMMAND: u16 = 0x04;
#[cfg_attr(not(feature = "gpu-scrie"), allow(dead_code))]
const CMD_BUS_MASTER: u16 = 1 << 2;
/// The DMA area for the FWSEC image: 16 pages (64 KiB) between 16 MiB and 4 GiB; the real image is 0xEA00 bytes.
const DMA_PAGES: u64 = 16;
/// Journal capacity in memory (the stick file has everything).
const JOURNAL_CAP: usize = 4096;

// ---------------------------------------------------------------------------------------------------------------
// The live journal on the stick.
// ---------------------------------------------------------------------------------------------------------------

struct Live {
    path: String,
    off: u64,
    lines: usize,
    failed: Option<String>,
}

static LIVE: crate::sched::Mutex<Option<Live>> = crate::sched::Mutex::new(None);

/// Where the journal goes: the stick; test builds also the test disk (QEMU has no writable stick in the scenario).
fn destinations() -> &'static [&'static str] {
    #[cfg(feature = "qemu-test")]
    return &["/usb", "/disk"];
    #[cfg(not(feature = "qemu-test"))]
    return &["/usb"];
}

/// Creates `<mount>/raport/gpu-jurnal-NNN.txt` and makes it the destination of `sink`.
pub(super) fn live_open(what: &str) -> Result<String, String> {
    let mut last_err = String::from("stick-ul nu e montat (/usb)");
    for mount in destinations() {
        if fs::list(mount).is_err() {
            continue;
        }
        let dir = format!("{}/raport", mount);
        match fs::create(&dir, Kind::Dir) {
            Ok(()) | Err(FsError::AlreadyExists) => {}
            Err(FsError::ReadOnly) => {
                last_err = format!("{} e doar pentru citire (lipseste cyber.rw)", mount);
                continue;
            }
            Err(e) => {
                last_err = format!("{}: {}", dir, e.message());
                continue;
            }
        }
        let existing: Vec<String> =
            fs::list(&dir).map_err(|e| e.message())?.into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
        let Some(name) = (1..1000).map(|n| format!("gpu-jurnal-{:03}.txt", n)).find(|n| !existing.contains(n)) else {
            return Err(format!("{}: prea multe jurnale gpu (999)", dir));
        };
        let path = format!("{}/{}", dir, name);
        fs::create(&path, Kind::File).map_err(|e| format!("{}: {}", path, e.message()))?;
        // The header has no "bar"/"cfg " words: `i6_log::parse_line` must skip it.
        let head = format!("# jurnal gpu P6-40, cyber-gpu {}, comanda: {}\n", CYBER_GPU_REV, what);
        fs::write(&path, 0, head.as_bytes()).map_err(|e| format!("{}: {}", path, e.message()))?;
        fs::flush(&path).map_err(|e| format!("{}: {}", path, e.message()))?;
        *LIVE.lock() = Some(Live { path: path.clone(), off: head.len() as u64, lines: 0, failed: None });
        crate::info!("gpu: jurnalul pe loc: {}", path);
        return Ok(path);
    }
    Err(last_err)
}

/// `Journal::with_sink` target: the log line, then the same line appended to the stick file and flushed — before the
/// library makes its next access, everything up to this one is on the stick.
fn sink(e: &Event) {
    crate::info!("gpu: jurnal: {}", e);
    let us = crate::time::since_boot_us().unwrap_or(0);
    let line = format!("[{:5}.{:06}] gpu: {}\n", us / 1_000_000, us % 1_000_000, e);
    let mut live = LIVE.lock();
    let Some(l) = live.as_mut() else { return };
    if l.failed.is_some() {
        return;
    }
    let r = fs::write(&l.path, l.off, line.as_bytes()).and_then(|n| fs::flush(&l.path).map(|_| n));
    match r {
        Ok(n) => {
            l.off += n as u64;
            l.lines += 1;
        }
        Err(err) => l.failed = Some(format!("randul {}: {}", l.lines + 1, err.message())),
    }
}

/// Stops the live journal: (path, lines written, first failure).
fn live_close() -> Option<(String, usize, Option<String>)> {
    LIVE.lock().take().map(|l| (l.path, l.lines, l.failed))
}

/// The journal of one command: live on the stick when `path` is set, in memory only otherwise.
pub(super) fn journal(live: bool) -> Journal {
    if live {
        Journal::with_sink(JOURNAL_CAP, sink)
    } else {
        Journal::new(JOURNAL_CAP)
    }
}

/// Closes the live journal and reports it; for I6 also reads it back and checks the write order (`i6_log::check_i6`,
/// configuration lines left out: Bus Master is not part of I6). Returns whether the check passed (true when not asked).
pub(super) fn close_and_check(p: &str, check_i6: bool, lines: &mut Vec<String>) -> bool {
    let Some((path, n, failed)) = live_close() else { return !check_i6 };
    match failed {
        None => lines.push(format!("{}: jurnalul pe loc: {} ({} randuri, fiecare scris pe loc)", p, path, n)),
        Some(why) => lines.push(format!("{}: jurnalul pe loc: {}: scrierea a ESUAT la {}", p, path, why)),
    }
    if !check_i6 {
        return true;
    }
    let text = match fs::read_all(&path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            lines.push(format!("{}: verificarea jurnalului: nu se poate citi {}: {}", p, path, e.message()));
            return false;
        }
    };
    let events: Vec<Event> = text
        .lines()
        .filter_map(nvidia_ga104::i6_log::parse_line)
        .filter(|e| matches!(e, Event::MmioRead { .. } | Event::MmioWrite { .. }))
        .collect();
    match nvidia_ga104::i6_log::check_i6(&events) {
        Ok(s) => {
            let secs: Vec<String> =
                s.sections.iter().map(|(imem, b)| format!("{} {} blocuri", if *imem { "IMEM" } else { "DMEM" }, b)).collect();
            lines.push(format!(
                "{}: verificarea jurnalului de pe disc (i6_log): TRECE ({} scrieri I6; {})",
                p, s.writes, secs.join(", ")
            ));
            true
        }
        Err(e) => {
            lines.push(format!(
                "{}: verificarea jurnalului de pe disc (i6_log): NU TRECE la scrierea {} ({:#x}): {}",
                p, e.write_index, e.offset, e.what
            ));
            false
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The DMA area (contiguous RAM below 4 GiB), allocated once and pinned: the card may read it by DMA.
// ---------------------------------------------------------------------------------------------------------------

#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
static DMA: crate::sched::Mutex<Option<u64>> = crate::sched::Mutex::new(None);

/// Copies `img` into the DMA area (zeros after it), reads it back, and describes it for the falcon loader.
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
pub(super) fn dma_put(img: &[u8]) -> Result<DmaImage, String> {
    let cap = (DMA_PAGES * 4096) as usize;
    if img.is_empty() || img.len() > cap || img.len() % hs_load::BLOCK as usize != 0 {
        return Err(format!("imaginea are {} octeti (trebuie multiplu de 256, cel mult {})", img.len(), cap));
    }
    let mut slot = DMA.lock();
    let phys = match *slot {
        Some(p) => p,
        None => {
            let p = crate::mem::frame::alloc_contiguous_between(DMA_PAGES, 16 << 20, 1 << 32)
                .ok_or_else(|| String::from("nu exista 64 KiB contigui sub 4 GiB"))?;
            crate::mem::frame::pin(p, DMA_PAGES, "gpu-i6"); // the card reads here by DMA: never given to anyone else
            *slot = Some(p);
            p
        }
    };
    let va = crate::mem::map::phys_to_virt(phys);
    // SAFETY: [phys, phys + 64 KiB) are DMA_PAGES frames allocated above for this area only (pinned, never freed),
    // reached through the HHDM; `img.len() <= cap` was checked.
    let area = unsafe { core::slice::from_raw_parts_mut(va, cap) };
    area[..img.len()].copy_from_slice(img);
    area[img.len()..].fill(0);
    if area[..img.len()] != *img {
        return Err(String::from("recitirea din RAM difera"));
    }
    Ok(DmaImage { phys, len: img.len() as u32 })
}

/// What the falcon reads at `image` (for the model in `--sim`): the DMA area itself.
#[cfg(feature = "test-gpu-sim")]
pub(super) fn dma_bytes(image: &DmaImage) -> Vec<u8> {
    let va = crate::mem::map::phys_to_virt(image.phys);
    // SAFETY: `image` was returned by `dma_put` (inside the pinned DMA area).
    unsafe { core::slice::from_raw_parts(va, image.len as usize) }.to_vec()
}

// ---------------------------------------------------------------------------------------------------------------
// The sequence, shared by the real board and the models.
// ---------------------------------------------------------------------------------------------------------------

/// Steps 1 of the window, read only: FWSEC in the VBIOS, then `i6_check` (fuse, VRAM, WPR2, FRTS).
pub(super) fn prepare<M: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    win: &mut M,
    journal: &mut Journal,
    clock: &mut K,
    lines: &mut Vec<String>,
) -> Option<(FwsecLocation, I6Check)> {
    use nvidia_ga104::firmware::{GspRole, GSP_570_144};
    let loc = match gpu::locate_fwsec(win, journal, clock) {
        Ok(l) => l,
        Err(e) => {
            lines.push(format!("{}: FWSEC in VBIOS: {}", p, e));
            return None;
        }
    };
    lines.push(format!(
        "{}: FWSEC in VBIOS: descriptor v{} la PROM {:#x}, ucode_id {}, {} semnaturi (masca {:#06x}), {} octeti",
        p, loc.desc.version(), loc.desc_prom, loc.desc.ucode_id, loc.desc.signature_count, loc.desc.signature_versions,
        loc.desc.stored_size
    ));
    let size = |r: GspRole| GSP_570_144.iter().find(|f| f.role == r).map(|f| f.size).unwrap_or(0);
    let c = match nvidia_ga104::i6_check::i6_check(win, journal, &loc.desc, size(GspRole::Bootloader), size(GspRole::GspRm)) {
        Ok(c) => c,
        Err(e) => {
            lines.push(format!("{}: I6-citire: {}", p, e));
            return None;
        }
    };
    lines.push(format!(
        "{}: siguranta FWSEC {:#x}: versiunea {} -> semnatura {} din VBIOS",
        p, c.signature.fuse_offset, c.signature.fuse_version, c.signature.index
    ));
    lines.push(format!("{}: VRAM {} MiB; FRTS {:#x}..{:#x}", p, c.fb_size >> 20, c.frts.0, c.frts.1));
    lines.push(match c.wpr2 {
        None => format!("{}: WPR2 coborat (pornire rece): bun", p),
        Some((s, e)) => format!("{}: WPR2 DEJA RIDICAT {:#x}..{:#x}: trebuie oprire completa (cablul scos 30 s)", p, s, e),
    });
    lines.push(format!("{}: criteriul I6-citire: {}", p, if c.passes() { "TRECE" } else { "NU TRECE" }));
    Some((loc, c))
}

/// Step 2, nothing written to the card: FWSEC and the chosen signature copied from the VBIOS, the FRTS command patched
/// in (`hs_load::fwsec_frts_image`).
#[cfg_attr(not(feature = "gpu-scrie"), allow(dead_code))]
pub(super) fn build_image<M: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    win: &mut M,
    journal: &mut Journal,
    clock: &mut K,
    loc: &FwsecLocation,
    c: &I6Check,
    lines: &mut Vec<String>,
) -> Option<(HsUcode, Vec<u8>)> {
    use nvidia_ga104::{fwsec, vbios};
    let r = fwsec::read_ucode_and_signature(win, journal, clock, vbios::VBIOS_TIMEOUT_US, loc, c.signature.index)
        .and_then(|(ucode, sig)| {
            let img = hs_load::fwsec_frts_image(&loc.desc, &ucode, &sig, c.frts.0)?;
            Ok((HsUcode::from_fwsec_v3(&loc.desc)?, img))
        });
    match r {
        Ok((u, img)) => {
            lines.push(format!(
                "{}: pasul 2: FWSEC copiat din VBIOS in RAM cu semnatura {} si comanda FRTS: {:#x} octeti ({} blocuri de 256 B); nimic scris in placa",
                p, c.signature.index, img.len(), img.len() / 256
            ));
            Some((u, img))
        }
        Err(e) => {
            lines.push(format!("{}: pasul 2 (FWSEC in RAM): {}; STOP, nimic scris", p, e));
            None
        }
    }
}

/// Steps 3–5: Bus Master on, FWSEC-FRTS through the guarded window, Bus Master back to its old value (also after an
/// I6 failure). Returns whether I6 passed and Bus Master is back.
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn run_writes<C: ConfigSpace + ?Sized, W: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    cfg: &mut C,
    rw: &mut W,
    clock: &mut K,
    journal: &mut Journal,
    ucode: &HsUcode,
    image: &DmaImage,
    frts: (u64, u64),
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
) -> bool {
    use gpu_pci::bus_master::{restore_command, set_bus_master, BusMasterPermit};
    let bm = BusMasterPermit::new("C-52 I6 (P6-40)");
    let old = match set_bus_master(cfg, &bm, journal, true) {
        Ok(o) => o,
        Err(e) => {
            lines.push(format!("{}: pasul 3 (Bus Master pornit): {}; STOP, nimic scris in registre", p, e));
            return false;
        }
    };
    lines.push(format!(
        "{}: pasul 3: Bus Master pornit (COMMAND {:#06x} -> {:#06x})",
        p, old, cfg.read32(PCI_COMMAND) as u16
    ));
    let i6_ok = i6_falcon(p, rw, clock, journal, ucode, image, frts, permit, lines);
    let back = match restore_command(cfg, &bm, journal, old) {
        Ok(()) if cfg.read32(PCI_COMMAND) as u16 == old => {
            lines.push(format!("{}: pasul 5: Bus Master oprit la loc (COMMAND {:#06x}, ca inainte)", p, old));
            true
        }
        Ok(()) => {
            lines.push(format!("{}: pasul 5: COMMAND {:#06x}, nu {:#06x} -> oprire completa", p, cfg.read32(PCI_COMMAND) as u16, old));
            false
        }
        Err(e) => {
            lines.push(format!("{}: pasul 5: Bus Master NU s-a pus la loc: {} -> oprire completa (cablul scos 30 s)", p, e));
            false
        }
    };
    i6_ok && back
}

/// Step 4 alone (no Bus Master): FWSEC-FRTS through the guarded GSP window. Shared by `gpu i6 --scrie` and the
/// `i6-fwsec-frts` step of `gpu lant`. Returns whether I6 passed.
#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(super) fn i6_falcon<W: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    rw: &mut W,
    clock: &mut K,
    journal: &mut Journal,
    ucode: &HsUcode,
    image: &DmaImage,
    frts: (u64, u64),
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
) -> bool {
    let i6 = GuardedBar0::new(rw, journal, permit).and_then(|mut b| falcon::fwsec_frts_dma(&mut b, clock, ucode, image, frts));
    let ok = match &i6 {
        Ok(w) => {
            lines.push(format!("{}: pas I6 FWSEC-FRTS: ok; WPR2 {:#x}..{:#x} (= regiunea FRTS)", p, w.start, w.end));
            true
        }
        Err(e) => {
            lines.push(format!("{}: pas I6 FWSEC-FRTS: ESEC: {} (oprit la prima eroare)", p, e));
            false
        }
    };
    lines.push(wpr2_detail(p, rw, journal, frts));
    ok
}

/// P6-44 (C-61), P6-46 (C-66): after I6, success or not, the raw WPR2 registers and the form of the end the board
/// wrote (`falcon::read_frts_after_i6`: 3 status reads, journalled, no permit, no write path). On 4 nov the board's HI
/// was the start of the last 128 KiB block of the FRTS region; this line names that form.
fn wpr2_detail<W: Mmio + ?Sized>(p: &str, rw: &mut W, journal: &mut Journal, frts: (u64, u64)) -> String {
    match falcon::read_frts_after_i6(rw, journal, frts) {
        Ok(c) => format!(
            "{}: WPR2 citit dupa I6: LO {:#010x} HI {:#010x} (capat: {}) = {:#x}..{:#x}",
            p,
            c.lo_raw,
            c.hi_raw,
            c.form.label(),
            c.wpr2.start,
            c.wpr2.end
        ),
        Err(e) => format!("{}: WPR2 citit dupa I6: {}", p, e),
    }
}

#[cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]
pub(super) fn writes_summary(p: &str, journal: &Journal) -> String {
    let (mut cfg, mut bar0) = (0usize, 0usize);
    for e in journal.events() {
        match e {
            Event::ConfigWrite { .. } => cfg += 1,
            Event::MmioWrite { .. } => bar0 += 1,
            _ => {}
        }
    }
    format!(
        "{}: scrieri in placa: {} in configuratie (Bus Master), {} in registrele GSP{}",
        p, cfg, bar0,
        if journal.dropped() > 0 { " (jurnalul din memorie a pierdut cele mai vechi; fisierul le are pe toate)" } else { "" }
    )
}

// ---------------------------------------------------------------------------------------------------------------
// The real board.
// ---------------------------------------------------------------------------------------------------------------

/// The first NVIDIA board the Cyber-GPU probe accepts, with BAR0, or the reason there is none.
pub(super) fn board(lines: &mut Vec<String>) -> Option<(crate::pci::Device, u64)> {
    let nv: Vec<crate::pci::Device> = gpu::display_devices().into_iter().filter(|d| d.vendor == NVIDIA).collect();
    let Some(d) = nv.into_iter().next() else {
        lines.push(String::from("nicio placa NVIDIA"));
        return None;
    };
    let mut cfg = ReadOnlyConfig { dev: &d, refused: 0 };
    let addr = gpu_core::PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
    let c = match nvidia_ga104::probe::probe_config(&mut cfg, Some(addr)) {
        Ok(c) => c,
        Err(e) => {
            lines.push(format!("gpu0: {} cyber-gpu {}: {}", d.address(), CYBER_GPU_REV, e));
            return None;
        }
    };
    lines.push(format!("gpu0: {} {} (BAR0 {:#x}), cyber-gpu {}", d.address(), c.model, c.bar0, CYBER_GPU_REV));
    if d.command() & 0b10 == 0 {
        lines.push(String::from("gpu0: Memory Space oprit de firmware (nu il pornim); nimic citit"));
        return None;
    }
    if crate::time::since_boot_us().is_none() {
        lines.push(String::from("gpu0: ceasul (TSC) nu e calibrat: limitele de timp nu se pot tine; nimic citit"));
        return None;
    }
    let bar0 = c.bar0 & !0xFFF;
    Some((d, bar0))
}

pub(super) fn finish(mut lines: Vec<String>) -> Vec<String> {
    for l in &lines {
        crate::info!("gpu: {}", l);
    }
    lines.shrink_to_fit();
    lines
}

/// `gpu pregatire-i6`: read only, on the real board. The journal goes live to the stick when one is writable.
pub fn prepare_i6() -> Vec<String> {
    let mut lines = Vec::new();
    let Some((_d, bar0)) = board(&mut lines) else { return finish(lines) };
    let live = match live_open("gpu pregatire-i6") {
        Ok(_) => true,
        Err(why) => {
            lines.push(format!("gpu0: jurnalul pe loc: nu (stick: {}); doar in log", why));
            false
        }
    };
    let mut j = journal(live);
    let (refused, remaps, map_failed) = {
        let _guard = WINDOW_LOCK.lock();
        let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
        let _ = prepare("gpu0", &mut win, &mut j, &mut TscClock, &mut lines);
        (win.refused + j.writes(), win.remaps, win.map_failed)
    };
    close_and_check("gpu0", false, &mut lines);
    if map_failed {
        lines.push(String::from("gpu0: maparea ferestrei a esuat (citirile au dat FFFFFFFF)"));
    }
    lines.push(format!("gpu0: {} pagini mapate pe rand (doar citire, fereastra de 16 MiB)", remaps));
    lines.push(gpu::writes_line(refused));
    finish(lines)
}

/// `gpu i6 --scrie` in a build without `gpu-scrie`: nothing can reach the card.
#[cfg(not(feature = "gpu-scrie"))]
pub fn i6_write() -> Vec<String> {
    finish(alloc::vec![String::from(
        "gpu i6 --scrie: exista doar pe stick-ul de test (optiunea gpu-scrie); acest sistem nu poate scrie in placa video"
    )])
}

/// `gpu i6 --scrie` on the real board (test stick only; the shell asked for `DA` first).
#[cfg(feature = "gpu-scrie")]
pub fn i6_write() -> Vec<String> {
    let p = "gpu0";
    let mut lines = Vec::new();
    let Some((d, bar0)) = board(&mut lines) else { return finish(lines) };
    let permit = match HardwareWritePermit::new("C-52 I6: fereastra cu Adrian (P6-40)") {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!("{}: {}", p, e));
            return finish(lines);
        }
    };
    // The live journal is a condition: without it nothing is written.
    if let Err(why) = live_open("gpu i6 --scrie") {
        lines.push(format!("{}: jurnalul pe stick nu se poate scrie ({}): STOP, nimic scris in placa", p, why));
        return finish(lines);
    }
    let mut j = journal(true);
    let mut clock = TscClock;
    let _guard = WINDOW_LOCK.lock();
    let prepared = {
        let mut win = ReadOnlyBar0Window::with_len(bar0, nvidia_ga104::fuse::FUSE_WINDOW);
        prepare(p, &mut win, &mut j, &mut clock, &mut lines).and_then(|(loc, c)| {
            if !c.passes() {
                lines.push(format!("{}: STOP: WPR2 deja ridicat; nimic scris", p));
                return None;
            }
            build_image(p, &mut win, &mut j, &mut clock, &loc, &c, &mut lines).map(|(u, img)| (u, img, c.frts))
        })
    };
    let ok = match prepared {
        None => false,
        Some((u, img, frts)) => match dma_put(&img) {
            Err(why) => {
                lines.push(format!("{}: zona DMA: {}; STOP, nimic scris", p, why));
                false
            }
            Ok(image) => {
                lines.push(format!("{}: zona DMA: {:#x} octeti la adresa fizica {:#x} (sub 4 GiB, recitita)", p, image.len, image.phys));
                let mut cfg = hw::BusMasterOnlyConfig { dev: &d, refused: 0 };
                let mut rw = hw::GspWriteWindow::new(bar0);
                let ok = run_writes(p, &mut cfg, &mut rw, &mut clock, &mut j, &u, &image, frts, Some(&permit), &mut lines);
                if cfg.refused + rw.refused > 0 {
                    lines.push(format!("{}: adaptoarele OS au REFUZAT {} scrieri cerute in afara gardului", p, cfg.refused + rw.refused));
                }
                ok
            }
        },
    };
    lines.push(writes_summary(p, &j));
    let checked = close_and_check(p, ok, &mut lines);
    lines.push(format!(
        "{}: rezultat I6: {}",
        p,
        if ok && checked { "TRECE (fa poza, apoi oprire completa: cablul scos 30 s)" } else { "NU TRECE (poza cu toate randurile; oprire completa; nu repeta)" }
    ));
    finish(lines)
}

/// The adapters that can reach the card: only in the test-stick build (`gpu-scrie`), each with its own fence.
#[cfg(feature = "gpu-scrie")]
pub(super) mod hw {
    use super::{gpu, CMD_BUS_MASTER, GSP_HI, GSP_LO, PCI_COMMAND};
    use crate::mem::paging;

    /// Configuration space: reads anywhere; the only write let through is Command with nothing but bit 2 changed.
    pub(crate) struct BusMasterOnlyConfig<'a> {
        pub(crate) dev: &'a crate::pci::Device,
        pub(crate) refused: usize,
    }

    impl gpu_core::hal::ConfigSpace for BusMasterOnlyConfig<'_> {
        fn read32(&mut self, offset: u16) -> u32 {
            self.dev.read32(offset & 0xFFC)
        }
        fn write32(&mut self, offset: u16, value: u32) {
            let now = self.dev.command();
            if offset & 0xFFC != PCI_COMMAND || value >> 16 != 0 || (value as u16 ^ now) & !CMD_BUS_MASTER != 0 {
                self.refused += 1;
                crate::warn!("gpu: scriere refuzata in configuratia PCI (gardul Bus Master): {:#x} <- {:#010x}", offset, value);
                return;
            }
            self.dev.write32(PCI_COMMAND, value);
        }
    }

    /// The whole 16 MiB BAR0 for reads; writes only inside the GSP window. One page mapped at a time (uncached).
    pub(crate) struct GspWriteWindow {
        phys: u64,
        mapped: Option<u64>,
        /// P6-56 (I7): the SEC2 window (0x840000-0x843FFF) is writable too (Booter runs on SEC2).
        sec2: bool,
        pub(crate) refused: usize,
    }

    /// The SEC2 falcon window (P6-56, I7 only).
    pub(crate) const SEC2_LO: u64 = 0x84_0000;
    pub(crate) const SEC2_HI: u64 = 0x84_3FFF;

    impl GspWriteWindow {
        pub(crate) fn new(phys: u64) -> Self {
            GspWriteWindow { phys, mapped: None, sec2: false, refused: 0 }
        }
        /// I7: writes inside the GSP or the SEC2 window, nothing else.
        #[cfg_attr(not(feature = "gpu-scrie"), allow(dead_code))]
        pub(crate) fn with_sec2(phys: u64) -> Self {
            GspWriteWindow { phys, mapped: None, sec2: true, refused: 0 }
        }
        fn page(&mut self, offset: u64) -> Option<u64> {
            let page = offset & !0xFFF;
            if self.mapped != Some(page) {
                if self.mapped.take().is_some() {
                    paging::unmap(gpu::WINDOW);
                }
                let flags = paging::NO_EXECUTE | paging::WRITABLE | gpu::UNCACHED;
                paging::map(gpu::WINDOW, self.phys + page, flags).ok()?;
                self.mapped = Some(page);
            }
            Some(gpu::WINDOW + (offset & 0xFFF))
        }
    }

    impl gpu_core::hal::Mmio for GspWriteWindow {
        fn len(&self) -> u64 {
            nvidia_ga104::fuse::FUSE_WINDOW
        }
        fn read32_raw(&mut self, offset: u64) -> u32 {
            let offset = offset.min(nvidia_ga104::fuse::FUSE_WINDOW - 4) & !3;
            match self.page(offset) {
                // SAFETY: `va` is the uncached mapping of the BAR0 page holding `offset` (4-byte aligned, inside BAR0).
                Some(va) => unsafe { (va as *const u32).read_volatile() },
                None => u32::MAX,
            }
        }
        fn write32_raw(&mut self, offset: u64, value: u32) {
            let in_sec2 = self.sec2 && (SEC2_LO..=SEC2_HI - 3).contains(&offset);
            if !((GSP_LO..=GSP_HI - 3).contains(&offset) || in_sec2) || offset & 3 != 0 {
                self.refused += 1;
                crate::warn!("gpu: scriere refuzata in afara ferestrelor GSP/SEC2: {:#x} <- {:#010x}", offset, value);
                return;
            }
            match self.page(offset) {
                // SAFETY: the uncached, writable mapping of the BAR0 page holding `offset`, which is inside the GSP
                // window, or the SEC2 window when `sec2` (checked above). Reached only in the gpu-scrie build, after `DA`, under a HardwareWritePermit.
                Some(va) => unsafe { (va as *mut u32).write_volatile(value) },
                None => self.refused += 1,
            }
        }
    }

    impl Drop for GspWriteWindow {
        fn drop(&mut self) {
            if self.mapped.take().is_some() {
                paging::unmap(gpu::WINDOW);
            }
        }
    }

    /// P6-56 step 2b (golden, I9): the VRAM-window fence. Reads anywhere in BAR0; writes only to the four registers of
    /// `vram_window::CONTROL_REGISTERS` (the window base, the TLB invalidate) and the PRAMIN range the window shows.
    /// It maps its own page (`VRAM_FENCE_VA`, next to `gpu::WINDOW`), so it can live next to a `GspWriteWindow`.
    pub(crate) struct VramFence {
        phys: u64,
        mapped: Option<u64>,
        pub(crate) refused: usize,
    }

    /// The page this fence maps BAR0 at (one page after the GSP window's page, unused by anything else).
    const VRAM_FENCE_VA: u64 = gpu::WINDOW + 0x1000;
    /// `NV_PRAMIN`: the 1 MiB of BAR0 that shows VRAM at the window base.
    pub(crate) const PRAMIN_LO: u64 = 0x70_0000;
    pub(crate) const PRAMIN_HI: u64 = 0x7F_FFFF;

    impl VramFence {
        pub(crate) fn new(phys: u64) -> Self {
            VramFence { phys, mapped: None, refused: 0 }
        }
        fn page(&mut self, offset: u64) -> Option<u64> {
            let page = offset & !0xFFF;
            if self.mapped != Some(page) {
                if self.mapped.take().is_some() {
                    paging::unmap(VRAM_FENCE_VA);
                }
                let flags = paging::NO_EXECUTE | paging::WRITABLE | gpu::UNCACHED;
                paging::map(VRAM_FENCE_VA, self.phys + page, flags).ok()?;
                self.mapped = Some(page);
            }
            Some(VRAM_FENCE_VA + (offset & 0xFFF))
        }
    }

    impl gpu_core::hal::Mmio for VramFence {
        fn len(&self) -> u64 {
            nvidia_ga104::fuse::FUSE_WINDOW
        }
        fn read32_raw(&mut self, offset: u64) -> u32 {
            let offset = offset.min(nvidia_ga104::fuse::FUSE_WINDOW - 4) & !3;
            match self.page(offset) {
                // SAFETY: `va` is the uncached mapping of the BAR0 page holding `offset` (4-byte aligned, inside BAR0).
                Some(va) => unsafe { (va as *const u32).read_volatile() },
                None => u32::MAX,
            }
        }
        fn write32_raw(&mut self, offset: u64, value: u32) {
            let control = nvidia_ga104::vram_window::CONTROL_REGISTERS.iter().any(|&r| u64::from(r) == offset);
            if !(control || (PRAMIN_LO..=PRAMIN_HI - 3).contains(&offset)) || offset & 3 != 0 {
                self.refused += 1;
                crate::warn!("gpu: scriere refuzata in afara ferestrei VRAM: {:#x} <- {:#010x}", offset, value);
                return;
            }
            match self.page(offset) {
                // SAFETY: the uncached, writable mapping of the BAR0 page holding `offset`, which is one of the VRAM
                // window's control registers or inside PRAMIN (checked above). Reached only in the gpu-scrie build,
                // after `DA`, under a HardwareWritePermit (VramWindow refuses a real BAR without one).
                Some(va) => unsafe { (va as *mut u32).write_volatile(value) },
                None => self.refused += 1,
            }
        }
    }

    impl Drop for VramFence {
        fn drop(&mut self) {
            if self.mapped.take().is_some() {
                paging::unmap(VRAM_FENCE_VA);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SIMULARE (test builds): the same functions on the gpu-sim models.
// ---------------------------------------------------------------------------------------------------------------

/// The read side of the workstation board: BAR0 16 MiB with the ASUS-like VBIOS chain and a GA10x-like FWSEC, the
/// FWSEC fuse at version 1, VRAM 8 GiB, WPR2 down. All values SIM.
#[cfg(feature = "test-gpu-sim")]
pub(super) fn sim_board() -> gpu_sim::mmio::SimMmio {
    use gpu_sim::{mmio::SimMmio, rom};
    let mut m = SimMmio::ga104_bar0();
    m.load_bytes(nvidia_ga104::regs::PROM_BASE, &rom::asus_like_chain_with_fwsec(&rom::SimFwsec::ga10x_like()));
    // SIM: the FWSEC of `SimFwsec` has ucode_id 0x0a; fuse version 1 (one bit), 8 GiB of VRAM.
    let fuse = nvidia_ga104::fuse::ucode_fuse_offset(nvidia_ga104::fuse::FuseEngine::Gsp, 0x0a).unwrap_or(0);
    m.set(fuse, 0b1);
    m.set(nvidia_ga104::i6_check::NV_PGC6_AON_SECURE_SCRATCH_GROUP_42, 8192);
    m
}

#[cfg(feature = "test-gpu-sim")]
pub(super) fn sim_lines(mut lines: Vec<String>, what: &str) -> Vec<String> {
    lines = lines.into_iter().map(|l| format!("SIMULARE: {}", l)).collect();
    lines.insert(0, format!("SIMULARE: {} pe modelele gpu-sim (cyber-gpu {}), nu placa reala", what, CYBER_GPU_REV));
    lines.push(String::from("SIMULARE: nicio placa reala atinsa; rezultatul nu dovedeste nimic despre hardware"));
    finish(lines)
}

/// `gpu pregatire-i6 --sim`.
#[cfg(feature = "test-gpu-sim")]
pub fn prepare_i6_sim() -> Vec<String> {
    let mut lines = Vec::new();
    let live = live_open("gpu pregatire-i6 --sim").is_ok();
    let mut j = journal(live);
    let mut m = sim_board();
    let mut clock = gpu_sim::clock::SimClock::default();
    let _ = prepare("model", &mut m, &mut j, &mut clock, &mut lines);
    close_and_check("model", false, &mut lines);
    lines.push(gpu::writes_line(m.writes.len() + j.writes()));
    sim_lines(lines, "gpu pregatire-i6")
}

/// SIM stand-in for a FWSEC whose data has the interface table (positions as in Cyber-GPU's own test,
/// `hs_load::tests::fwsec_iface_*`): header at DMEM 0x300 (2 entries), DMEM mapper at 0x340, command buffer at 0x380
/// (0x40 bytes), signature at 0x100. Synthetic bytes, never NVIDIA firmware.
#[cfg(feature = "test-gpu-sim")]
pub(super) fn sim_fwsec() -> (nvidia_ga104::fwsec::FalconUcodeDescV3, Vec<u8>, Vec<u8>) {
    let desc = nvidia_ga104::fwsec::FalconUcodeDescV3 {
        stored_size: 0x1600,
        pkc_data_offset: 0x100,
        interface_offset: 0x300,
        imem_load_size: 0x1000,
        dmem_load_size: 0x600,
        engine_id_mask: 0x400,
        ucode_id: 5,
        signature_count: 1,
        signature_versions: 1,
        ..Default::default()
    };
    let pattern = |n: usize, seed: u8| -> Vec<u8> { (0..n).map(|i| (i as u8).wrapping_mul(31) ^ seed).collect() };
    let mut u = pattern(0x1600, 0x22);
    let put = |u: &mut Vec<u8>, o: usize, v: u32| u[o..o + 4].copy_from_slice(&v.to_le_bytes());
    let d = 0x1000;
    u[d + 0x300..d + 0x304].copy_from_slice(&[1, 4, 8, 2]);
    put(&mut u, d + 0x304, 1);
    put(&mut u, d + 0x308, 0);
    put(&mut u, d + 0x30c, 4);
    put(&mut u, d + 0x310, 0x340);
    put(&mut u, d + 0x340 + 8, 0x380);
    put(&mut u, d + 0x340 + 12, 0x40);
    (desc, u, pattern(384, 0x5a))
}

/// `gpu i6 --scrie --sim` (the shell asked for `DA` first): step 1 and the VBIOS copy on the read model, then steps
/// 2–5 with the SIM FWSEC through the REAL DMA area of the OS, a model of the configuration space (Bus Master off at
/// the start, as on the workstation) and the falcon model (`gpu_sim::fake_gsp`), which reads the image back from that
/// RAM and creates WPR2 where FWSEC's command says. `fault`: FWSEC halts with an error (Bus Master must still go back).
#[cfg(feature = "test-gpu-sim")]
pub fn i6_write_sim(fault: bool) -> Vec<String> {
    use gpu_sim::fake_gsp::{FakeGsp, SimFault, SimHsImage, SimUcode, GSP_BASE};
    let p = "model";
    let mut lines = Vec::new();
    if let Err(why) = live_open(if fault { "gpu i6 --scrie --sim --eroare" } else { "gpu i6 --scrie --sim" }) {
        lines.push(format!("{}: jurnalul pe disc nu se poate scrie ({}): STOP", p, why));
        return sim_lines(lines, "gpu i6 --scrie");
    }
    let mut j = journal(true);
    let mut clock = gpu_sim::clock::SimClock::default();
    let mut ro = sim_board();
    let Some((loc, c)) = prepare(p, &mut ro, &mut j, &mut clock, &mut lines) else {
        close_and_check(p, false, &mut lines);
        return sim_lines(lines, "gpu i6 --scrie");
    };
    // The VBIOS copy on the read model (the SIM ucode bytes are zeros, without the interface table FRTS needs).
    match nvidia_ga104::fwsec::read_ucode_and_signature(&mut ro, &mut j, &mut clock, nvidia_ga104::vbios::VBIOS_TIMEOUT_US, &loc, c.signature.index) {
        Ok((u, s)) => lines.push(format!("{}: din VBIOS-ul modelului: ucode {:#x} octeti, semnatura {} octeti (doar citire)", p, u.len(), s.len())),
        Err(e) => lines.push(format!("{}: copierea din VBIOS-ul modelului: {}", p, e)),
    }
    // Steps 2-5 with the SIM FWSEC (it has the DMEM mapper), for the FRTS region the check computed.
    let (desc, ucode, sig) = sim_fwsec();
    let built = hs_load::fwsec_frts_image(&desc, &ucode, &sig, c.frts.0).and_then(|img| Ok((HsUcode::from_fwsec_v3(&desc)?, img)));
    let (u, img) = match built {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!("{}: pasul 2 (FWSEC SIM in RAM): {}", p, e));
            close_and_check(p, false, &mut lines);
            return sim_lines(lines, "gpu i6 --scrie");
        }
    };
    lines.push(format!("{}: pasul 2: FWSEC SIM cu comanda FRTS: {:#x} octeti ({} blocuri de 256 B)", p, img.len(), img.len() / 256));
    let image = match dma_put(&img) {
        Ok(i) => i,
        Err(why) => {
            lines.push(format!("{}: zona DMA: {}", p, why));
            close_and_check(p, false, &mut lines);
            return sim_lines(lines, "gpu i6 --scrie");
        }
    };
    lines.push(format!(
        "{}: zona DMA: {:#x} octeti la adresa fizica {:#x} ({}, recitita)",
        p, image.len, image.phys, if image.phys + image.len as u64 <= 1 << 32 { "sub 4 GiB" } else { "PESTE 4 GiB" }
    ));
    let mut g = FakeGsp::new();
    g.frts = (0, 0); // FWSEC must take the region from the command
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
    // The falcon model reads the OS's DMA area, not a copy of `img`.
    g.sysmem = Some((image.phys, dma_bytes(&image)));
    let mut cfg = gpu_sim::pci::SimConfig::workstation_rtx3070ti();
    cfg.write32(PCI_COMMAND, 0x0003); // SIM: I/O + memory on, Bus Master off
    let cfg_before = cfg.writes;
    let ok = run_writes(p, &mut cfg, &mut g, &mut clock, &mut j, &u, &image, c.frts, None, &mut lines);
    lines.push(writes_summary(p, &j));
    let outside = g.writes.iter().filter(|&&(o, _)| !(GSP_LO..=GSP_HI).contains(&u64::from(o))).count();
    let mut regs: Vec<u32> = Vec::new();
    for &(o, _) in &g.writes {
        if !regs.contains(&o) {
            regs.push(o);
        }
    }
    lines.push(format!(
        "{}: modelul: {} blocuri DMA copiate din RAM, {} refuzate; {} scrieri falcon in {} registre, {} in afara ferestrei GSP; configuratie: {} scrieri; COMMAND la final {:#06x}",
        p, g.dma_blocks, g.dma_faults, g.writes.len(), regs.len(), outside, cfg.writes - cfg_before, cfg.read32(PCI_COMMAND) as u16
    ));
    let checked = close_and_check(p, !fault, &mut lines);
    lines.push(format!("{}: rezultat I6: {}", p, if ok && checked { "TRECE" } else { "NU TRECE" }));
    sim_lines(lines, "gpu i6 --scrie")
}
