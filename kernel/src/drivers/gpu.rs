//! Level-0 identification of display controllers (shell command `gpu`, task P6-11, command C-12).
//!
//! Read only, by design: this module has no function that writes to a card. It never writes the PCI
//! configuration space (no BAR sizing, no Command bits, no power state), and the one register page it reads on
//! NVIDIA cards is mapped at a private virtual address WITHOUT write permission, so a stray write would raise a
//! page fault instead of reaching the card (scenario `gpu-ro`; `tools/after-gpu.sh` checks the source).
//!
//! Sources: PCI Local Bus and PCI Express Base specifications (configuration header, BARs, Resizable BAR
//! extended capability 0x15); NVIDIA's public register documentation (open-gpu-doc) and the MIT-licensed
//! open-gpu-kernel-modules headers (`dev_boot.h`), read only for the offsets and fields of NV_PMC_BOOT_0 (0x0)
//! and NV_PMC_BOOT_42 (0xA00). No code copied.

#![cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell

use crate::mem::paging;
use crate::pci::Device;
use alloc::{format, string::String, vec::Vec};

pub const NVIDIA: u16 = 0x10DE;
const AMD: u16 = 0x1002;
const INTEL: u16 = 0x8086;

/// Private virtual page for the register window (unused by anything else; see mem/README.md).
pub(super) const WINDOW: u64 = 0xFFFF_C100_0000_0000;
/// Page-level write-through + cache disable (PWT, PCD): device registers must not be cached.
pub(super) const UNCACHED: u64 = (1 << 3) | (1 << 4);

const NV_PMC_BOOT_0: u64 = 0x0;
const NV_PMC_BOOT_42: u64 = 0xA00;

pub(super) static WINDOW_LOCK: crate::sched::Mutex<()> = crate::sched::Mutex::new(());

/// One base address register, as read from the configuration space (size not measured: that needs a write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub index: u16,
    pub phys: u64,
    pub io: bool,
    pub is64: bool,
    pub prefetch: bool,
}

/// What `NV_PMC_BOOT_0` and `NV_PMC_BOOT_42` say about the chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NvIdentity {
    pub boot0: u32,
    pub boot42: u32,
    /// Architecture << 4 | implementation, from BOOT_42 (e.g. 0x174 = GA104).
    pub chip: u16,
    pub revision: u8,
}

/// Decodes the two identity registers. BOOT_42: architecture 28:24 (plus bit 29 on newer chips), implementation
/// 23:20, revision major 19:16 / minor 15:12. Older chips leave BOOT_42 at zero: then BOOT_0 (architecture 28:24,
/// implementation 23:20, revision 7:0) is used.
pub fn decode_nv(boot0: u32, boot42: u32) -> NvIdentity {
    let (chip, revision) = if boot42 != 0 && boot42 != u32::MAX {
        (((boot42 >> 20) & 0x3FF) as u16, ((boot42 >> 12) & 0xFF) as u8)
    } else {
        (((boot0 >> 20) & 0x1FF) as u16, (boot0 & 0xFF) as u8)
    };
    NvIdentity { boot0, boot42, chip, revision }
}

/// Architecture name for a chip id (only the families that matter here; anything else is "necunoscuta").
pub fn nv_architecture(chip: u16) -> &'static str {
    match chip >> 4 {
        0x16 => "Turing",
        0x17 => "Ampere",
        0x18 => "Hopper",
        0x19 => "Ada Lovelace",
        0x1A | 0x1B => "Blackwell",
        _ => "necunoscuta",
    }
}

/// Chip name for the known Turing / Ampere / Ada ids.
pub fn nv_chip_name(chip: u16) -> Option<&'static str> {
    Some(match chip {
        0x162 => "TU102",
        0x164 => "TU104",
        0x166 => "TU106",
        0x167 => "TU117",
        0x168 => "TU116",
        0x172 => "GA102",
        0x173 => "GA103",
        0x174 => "GA104",
        0x176 => "GA106",
        0x177 => "GA107",
        0x192 => "AD102",
        0x193 => "AD103",
        0x194 => "AD104",
        0x196 => "AD106",
        0x197 => "AD107",
        _ => return None,
    })
}

fn vendor_name(v: u16) -> &'static str {
    match v {
        NVIDIA => "NVIDIA",
        AMD => "AMD",
        INTEL => "Intel",
        0x1234 => "QEMU (VGA standard)",
        0x1AF4 => "virtio",
        _ => "necunoscut",
    }
}

/// The memory and I/O BARs of a function (type 0 header: 6 BARs), read only.
pub fn bars(d: &Device) -> Vec<Bar> {
    let mut out = Vec::new();
    let mut i = 0u16;
    while i < 6 {
        let low = d.read32(0x10 + i * 4);
        let io = low & 1 != 0;
        let is64 = !io && (low >> 1) & 3 == 2;
        let phys = if io {
            (low & !0x3) as u64
        } else {
            let mut a = (low & !0xF) as u64;
            if is64 && i < 5 {
                a |= (d.read32(0x14 + i * 4) as u64) << 32;
            }
            a
        };
        if phys != 0 {
            out.push(Bar { index: i, phys, io, is64, prefetch: !io && low & 8 != 0 });
        }
        i += if is64 { 2 } else { 1 };
    }
    out
}

/// Resizable BAR (PCIe extended capability 0x15), read only: for each BAR it covers, (BAR index, current size in
/// MiB). Control register bits 2:0 = BAR index, 13:8 = size as 2^(n) MiB.
pub fn resizable_bars(d: &Device) -> Vec<(u16, u64)> {
    let mut out = Vec::new();
    let mut ptr: u16 = 0x100;
    for _ in 0..64 {
        let header = d.read32(ptr);
        if header == 0 || header == u32::MAX {
            break;
        }
        if header & 0xFFFF == 0x15 {
            let first_ctrl = d.read32(ptr + 8);
            let count = ((first_ctrl >> 5) & 7).clamp(1, 6) as u16;
            for k in 0..count {
                let ctrl = d.read32(ptr + 8 + k * 8);
                out.push(((ctrl & 7) as u16, 1u64 << ((ctrl >> 8) & 0x3F)));
            }
            break;
        }
        let next = (header >> 20) as u16 & 0xFFC;
        if next < 0x100 {
            break;
        }
        ptr = next;
    }
    out
}

/// Maps the page at `phys` read-only and uncached at `WINDOW`, calls `f` with its virtual address, unmaps it.
fn with_readonly_window<T>(phys: u64, f: impl FnOnce(u64) -> T) -> Result<T, &'static str> {
    let _guard = WINDOW_LOCK.lock();
    // Deliberately no WRITABLE flag: the card's registers cannot be written through this mapping.
    paging::map(WINDOW, phys & !0xFFF, paging::NO_EXECUTE | UNCACHED).map_err(|_| "maparea ferestrei a esuat")?;
    let r = f(WINDOW + (phys & 0xFFF));
    paging::unmap(WINDOW);
    Ok(r)
}

pub(super) fn read_reg(window: u64, offset: u64) -> u32 {
    // SAFETY: `window` is the read-only, uncached mapping of BAR0's first page made by `with_readonly_window`;
    // offset < 4096 and 4-byte aligned. Reading NV_PMC_BOOT_0/42 has no side effects (public documentation).
    unsafe { ((window + offset) as *const u32).read_volatile() }
}

/// Reads NV_PMC_BOOT_0 and NV_PMC_BOOT_42. Refuses (with the reason) when the firmware has not enabled memory
/// decoding: we never turn it on ourselves.
pub fn read_nv_identity(d: &Device) -> Result<NvIdentity, &'static str> {
    if d.vendor != NVIDIA {
        return Err("nu e placa NVIDIA");
    }
    if d.command() & 0b10 == 0 {
        return Err("Memory Space oprit de firmware (nu il pornim: ar fi o scriere)");
    }
    let bar0 = bars(d).into_iter().find(|b| b.index == 0 && !b.io).ok_or("BAR0 lipseste sau nu e de memorie")?;
    let (boot0, boot42) = with_readonly_window(bar0.phys, |w| (read_reg(w, NV_PMC_BOOT_0), read_reg(w, NV_PMC_BOOT_42)))?;
    if boot0 == u32::MAX {
        return Err("placa nu raspunde (FFFFFFFF: probabil in D3)");
    }
    Ok(decode_nv(boot0, boot42))
}

/// Every display controller (PCI class 03).
pub fn display_devices() -> Vec<Device> {
    crate::pci::devices().into_iter().filter(|d| d.class == 0x03).collect()
}

fn power_state(d: &Device) -> Option<u32> {
    let ptr = d.capability(0x01)?;
    Some(d.read32(ptr + 4) & 3)
}

/// The report of `gpu identifica`, one line per entry; every line also goes to the log (prefix "gpu:").
pub fn identify() -> Vec<String> {
    let mut lines = Vec::new();
    let mut refused = 0usize;
    let devs = display_devices();
    if devs.is_empty() {
        lines.push(String::from("nicio placa video gasita in PCI"));
    }
    for (n, d) in devs.iter().enumerate() {
        lines.push(format!(
            "gpu{}: {} {:04x}:{:04x} {} (clasa {:02x}:{:02x}:{:02x}) subsistem {:08x}",
            n, d.address(), d.vendor, d.device, vendor_name(d.vendor), d.class, d.subclass, d.prog_if, d.read32(0x2C)
        ));
        let cmd = d.command();
        lines.push(format!(
            "gpu{}: Command {:04x} (memorie {}, bus master {}), stare {}",
            n, cmd, if cmd & 2 != 0 { "da" } else { "nu" }, if cmd & 4 != 0 { "da" } else { "nu" },
            power_state(d).map(|s| format!("D{}", s)).unwrap_or_else(|| String::from("necunoscuta"))
        ));
        for b in bars(d) {
            lines.push(format!(
                "gpu{}: BAR{} {:#x} {}{}{}",
                n, b.index, b.phys,
                if b.io { "I/O" } else { "memorie" },
                if b.is64 { ", 64 biti" } else { "" },
                if b.prefetch { ", prefetch" } else { "" }
            ));
        }
        let rebar = resizable_bars(d);
        if rebar.is_empty() {
            lines.push(format!("gpu{}: Resizable BAR: absent (marimea BAR-urilor nu se masoara: ar cere scriere)", n));
        }
        for (i, mib) in rebar {
            lines.push(format!("gpu{}: Resizable BAR: BAR{} = {} MiB acum", n, i, mib));
        }
        let lib = probe_with_cyber_gpu(n, d, &mut lines);
        refused += lib.refused;
        if lib.identified {
            // The Cyber-GPU probe read and decoded BOOT_0/BOOT_42 itself (lines added above).
        } else if d.vendor == NVIDIA {
            match read_nv_identity(d) {
                Ok(id) => {
                    lines.push(format!("gpu{}: NV_PMC_BOOT_0 = {:08x}, NV_PMC_BOOT_42 = {:08x}", n, id.boot0, id.boot42));
                    lines.push(format!(
                        "gpu{}: cip {:#x} {} (arhitectura {}), revizia {:02x}",
                        n, id.chip, nv_chip_name(id.chip).unwrap_or("?"), nv_architecture(id.chip), id.revision
                    ));
                }
                Err(why) => lines.push(format!("gpu{}: registrele nu s-au citit: {}", n, why)),
            }
        } else {
            lines.push(format!("gpu{}: registre necitite (nivelul 0 citeste registre doar la NVIDIA)", n));
        }
    }
    if !devs.iter().any(|d| d.vendor == NVIDIA) {
        lines.push(String::from("nicio placa NVIDIA"));
    }
    if refused == 0 {
        lines.push(String::from(
            "zero scrieri in placa (configuratie PCI si registre: doar citire; adaptorul cyber-gpu a refuzat 0 scrieri)",
        ));
    } else {
        lines.push(format!(
            "zero scrieri in placa: biblioteca cyber-gpu a cerut {} scrieri, toate REFUZATE de adaptor",
            refused
        ));
    }
    for l in &lines {
        crate::info!("gpu: {}", l);
    }
    lines
}

// ---------------------------------------------------------------------------------------------------------------
// P6-19 (C-16): the kernel side of the Cyber-GPU hardware traits (`gpu_core::hal`). The libraries decide what to read;
// this adapter decides what reaches the card: configuration reads go to ECAM, every write is refused and counted,
// and BAR0 is reached only through the read-only, uncached window above.
// ---------------------------------------------------------------------------------------------------------------

/// Pinned Cyber-GPU revision (kernel/Cargo.toml), shown in the report.
pub const CYBER_GPU_REV: &str = "20b4a3b";

/// Configuration space of one function for the Cyber-GPU libraries: reads only.
pub(super) struct ReadOnlyConfig<'a> {
    pub(super) dev: &'a Device,
    pub(super) refused: usize,
}

impl gpu_core::hal::ConfigSpace for ReadOnlyConfig<'_> {
    fn read32(&mut self, offset: u16) -> u32 {
        self.dev.read32(offset & 0xFFC)
    }
    fn write32(&mut self, offset: u16, value: u32) {
        self.refused += 1;
        crate::warn!("gpu: scriere refuzata in configuratia PCI: {:#x} <- {:#010x}", offset, value);
    }
}

/// The first page of BAR0 for the Cyber-GPU libraries: reads only (the page is also mapped without write permission).
struct ReadOnlyWindow {
    va: u64,
    refused: usize,
}

impl gpu_core::hal::Mmio for ReadOnlyWindow {
    fn len(&self) -> u64 {
        0x1000
    }
    fn read32_raw(&mut self, offset: u64) -> u32 {
        // `checked_read32` in gpu-core already bounds and aligns the offset; keep the window safe regardless.
        read_reg(self.va, offset & 0xFFC)
    }
    fn write32_raw(&mut self, offset: u64, value: u32) {
        self.refused += 1;
        crate::warn!("gpu: scriere refuzata in registre: {:#x} <- {:#010x}", offset, value);
    }
}

struct LibProbe {
    identified: bool,
    refused: usize,
}

/// Runs the Cyber-GPU level-0 probe (`nvidia_ga104::probe`) on one display controller. For a known board it reads
/// BOOT_0/BOOT_42 through the library; for anything else it reports why the library does not take the device.
fn probe_with_cyber_gpu(n: usize, d: &Device, lines: &mut Vec<String>) -> LibProbe {
    use nvidia_ga104::probe;
    let mut cfg = ReadOnlyConfig { dev: d, refused: 0 };
    let addr = gpu_core::PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
    let candidate = probe::probe_config(&mut cfg, Some(addr));
    let mut out = LibProbe { identified: false, refused: cfg.refused };
    let c = match candidate {
        Err(e) => {
            lines.push(format!("gpu{}: cyber-gpu {} nvidia-ga104: {}", n, CYBER_GPU_REV, e));
            return out;
        }
        Ok(c) => c,
    };
    lines.push(format!("gpu{}: cyber-gpu {} nvidia-ga104: {} (BAR0 {:#x})", n, CYBER_GPU_REV, c.model, c.bar0));
    if d.command() & 0b10 == 0 {
        lines.push(format!("gpu{}: registrele nu s-au citit: Memory Space oprit de firmware (nu il pornim)", n));
        return out;
    }
    let r = with_readonly_window(c.bar0, |w| {
        let mut win = ReadOnlyWindow { va: w, refused: 0 };
        let mut journal = gpu_core::diag::Journal::new(16);
        let chip = probe::probe_mmio(&mut win, &mut journal);
        (chip, win.refused + journal.writes())
    });
    match r {
        Ok((Ok(chip), w)) => {
            out.refused += w;
            out.identified = true;
            lines.push(format!(
                "gpu{}: NV_PMC_BOOT_0 = {:08x}, NV_PMC_BOOT_42 = {:08x} (cyber-gpu, consecvent {})",
                n, chip.boot0.raw, chip.boot42.raw, chip.consistent
            ));
            lines.push(format!(
                "gpu{}: cip {:#x} {} (arhitectura {})",
                n, chip.boot0.chip_id(), nvidia_ga104::boot::chip_name(chip.boot0.chip_id()),
                nv_architecture(chip.boot0.chip_id())
            ));
        }
        Ok((Err(e), w)) => {
            out.refused += w;
            lines.push(format!("gpu{}: cyber-gpu: registrele: {}", n, e));
        }
        Err(why) => lines.push(format!("gpu{}: cyber-gpu: {}", n, why)),
    }
    out
}

/// `gpu identifica --sim` (test builds only): the same Cyber-GPU level-0 path, on the `gpu-sim` model of the
/// workstation board (FACTS from Adrian's K0 inventory: 10de:2482 at b3:00.0, BAR0 16 MiB, BAR1 256 MiB, BAR3 32 MiB).
/// Every line says SIMULARE: nothing here touches a real card, and passing proves nothing about the hardware.
#[cfg(feature = "test-gpu-sim")]
pub fn identify_sim() -> Vec<String> {
    use gpu_sim::{mmio::SimMmio, pci::SimConfig, workstation as ws};
    use nvidia_ga104::probe;
    let mut raw = Vec::new();
    let mut cfg = SimConfig::workstation_rtx3070ti();
    let mut bar0 = SimMmio::ga104_bar0();
    // SIM value for BOOT_42 (chip 0x174, as on every GA104).
    bar0.set(0xa00, (0x174 << 20) | (0xa << 16) | (0x1 << 12));
    let addr = gpu_core::PciAddress { segment: 0, bus: ws::BUS, dev: 0, func: 0 };
    let mut journal = gpu_core::diag::Journal::new(64);
    match probe::level0(&mut cfg, Some(addr), &mut bar0, &mut journal) {
        Ok(l0) => {
            let (probe_cfg, probe_regs) = (cfg.writes, bar0.writes.len());
            // Sizes: on the MODEL only, with the library's sizing sequence (on real hardware it stays forbidden).
            let mut sj = gpu_core::diag::Journal::new(64);
            let permit = gpu_pci::SizingPermit::new("SIMULARE: doar modelul gpu-sim");
            let mut l0 = l0;
            match gpu_pci::bar::size_bars(&mut cfg, &permit, &mut sj) {
                Ok(sized) => l0.candidate.report = l0.candidate.report.clone().with_sizes(&sized),
                Err(e) => raw.push(format!("dimensionarea BAR in model a esuat: {}", e)),
            }
            for l in format!("{}", l0).lines() {
                raw.push(String::from(l));
            }
            raw.push(format!(
                "nivel 0: {} scrieri in timpul identificarii (configuratie {}, registre {}); citiri BAR0: {:?}",
                probe_cfg + probe_regs, probe_cfg, probe_regs, bar0.reads
            ));
            raw.push(format!("dimensionarea BAR: {} scrieri, doar in model", cfg.writes - probe_cfg));
        }
        Err(e) => raw.push(format!("nivel 0 a esuat: {}", e)),
    }
    let mut lines: Vec<String> = raw.into_iter().map(|l| format!("SIMULARE: {}", l.trim_end())).collect();
    lines.insert(0, format!("SIMULARE: modelul gpu-sim al placii de pe workstation (cyber-gpu {}), nu placa reala", CYBER_GPU_REV));
    lines.push(String::from("SIMULARE: nicio placa reala atinsa; rezultatul nu dovedeste nimic despre hardware"));
    for l in &lines {
        crate::info!("gpu: {}", l);
    }
    lines
}

// ---------------------------------------------------------------------------------------------------------------
// P6-21 (C-19): steps I2 (`gpu gfw`) and I3 (`gpu vbios`) of Cyber-GPU, read only. Both need the first 4 MiB of BAR0
// (the PROM window ends at 0x400000); they are reached one page at a time through the same read-only, uncached
// window as above, so the libraries still cannot write: every write they ask for is refused and counted.
// ---------------------------------------------------------------------------------------------------------------

/// The first `len` bytes of BAR0 (`INIT_WINDOW` = 4 MiB for I2–I3b; P6-40: `fuse::FUSE_WINDOW` = 16 MiB, the whole BAR0
/// of the GA104, for the I6 pre-check), mapped page by page on demand at `WINDOW`, without write permission.
/// The caller holds `WINDOW_LOCK` for the whole lifetime of this value.
pub(super) struct ReadOnlyBar0Window {
    phys: u64,
    len: u64,
    mapped: Option<u64>,
    pub(super) remaps: usize,
    pub(super) map_failed: bool,
    pub(super) refused: usize,
}

impl ReadOnlyBar0Window {
    fn new(phys: u64) -> Self {
        Self::with_len(phys, nvidia_ga104::regs::INIT_WINDOW)
    }
    pub(super) fn with_len(phys: u64, len: u64) -> Self {
        ReadOnlyBar0Window { phys, len, mapped: None, remaps: 0, map_failed: false, refused: 0 }
    }
}

impl gpu_core::hal::Mmio for ReadOnlyBar0Window {
    fn len(&self) -> u64 {
        self.len
    }
    fn read32_raw(&mut self, offset: u64) -> u32 {
        // `checked_read32` in gpu-core already bounds and aligns the offset; keep the window safe regardless.
        let offset = offset.min(self.len - 4) & !3;
        let page = offset & !0xFFF;
        if self.mapped != Some(page) {
            if self.mapped.take().is_some() {
                paging::unmap(WINDOW);
            }
            // Deliberately no WRITABLE flag (as in `with_readonly_window`).
            if paging::map(WINDOW, self.phys + page, paging::NO_EXECUTE | UNCACHED).is_err() {
                self.map_failed = true;
                return u32::MAX;
            }
            self.mapped = Some(page);
            self.remaps += 1;
        }
        read_reg(WINDOW, offset & 0xFFF)
    }
    fn write32_raw(&mut self, offset: u64, value: u32) {
        self.refused += 1;
        crate::warn!("gpu: scriere refuzata in registre: {:#x} <- {:#010x}", offset, value);
    }
}

impl Drop for ReadOnlyBar0Window {
    fn drop(&mut self) {
        if self.mapped.take().is_some() {
            paging::unmap(WINDOW);
        }
    }
}

/// Time for the Cyber-GPU polling loops: the TSC (rate measured at boot). Waits spin; they are bounded by the library.
pub(super) struct TscClock;

impl gpu_core::hal::Clock for TscClock {
    fn now_us(&mut self) -> u64 {
        crate::time::since_boot_us().unwrap_or(0)
    }
    fn delay_us(&mut self, us: u32) {
        let end = self.now_us() + us as u64;
        while self.now_us() < end {
            core::hint::spin_loop();
        }
    }
}

/// The lines of `gpu gfw` for one result (real card or model).
fn gfw_lines(p: &str, r: &gpu_core::error::GpuResult<nvidia_ga104::gfw::GfwStatus>) -> Vec<String> {
    match r {
        Ok(s) => alloc::vec![format!(
            "{}: GFW gata (progres {:#04x}) dupa {} runde, {} us; PRIV_LEVEL_MASK {:#010x}",
            p, s.progress, s.polls, s.waited_us, s.priv_level_mask
        )],
        Err(e) => alloc::vec![format!("{}: GFW: {}", p, e)],
    }
}

/// The lines of `gpu vbios` for one result (real card or model).
fn vbios_lines(p: &str, r: &gpu_core::error::GpuResult<nvidia_ga104::vbios::VbiosLayout>) -> Vec<String> {
    use nvidia_ga104::vbios;
    let l = match r {
        Ok(l) => l,
        Err(e) => return alloc::vec![format!("{}: VBIOS: {}", p, e)],
    };
    let mut v = Vec::new();
    v.push(format!(
        "{}: VBIOS: antet {}, prima imagine la {:#x}, {} imagini",
        p,
        l.ifr_version.map(|x| format!("IFR v{}", x)).unwrap_or_else(|| String::from("fara IFR")),
        l.rom_start,
        l.images.len()
    ));
    for (k, i) in l.images.iter().enumerate() {
        v.push(format!(
            "{}:   imaginea {}: offset {:#07x}, tip {:#04x} {}, {} octeti, {:04x}:{:04x}{}{}",
            p, k, i.offset, i.code_type, vbios::code_name(i.code_type), i.size, i.vendor, i.device,
            if i.has_npde { ", NPDE" } else { "" },
            if i.last { ", ultima" } else { "" }
        ));
    }
    match &l.bit {
        Some((at, tokens)) => v.push(format!(
            "{}: BIT la {:#x}: {} jetoane, jetonul 0x70 (date Falcon): {}",
            p, at, tokens.len(), if l.has_falcon_data() { "da" } else { "nu" }
        )),
        None => v.push(format!("{}: BIT: negasita", p)),
    }
    v.push(format!(
        "{}: FWSEC: {} imagini; complet (PCI-AT + BIT 0x70 + FWSEC): {}; {} citiri PROM",
        p, l.fwsec_images(), if l.complete() { "da" } else { "nu" }, l.prom_reads
    ));
    v
}

/// P6-31 (C-31): the lines of `gpu vbios --detalii` — the Cyber-GPU read-only diagnostic listing (every image, PCIR,
/// NPDE, breaks with their first 16 bytes, signatures after the last image, BIT, summary with the I3 criterion).
fn vbios_diag_lines(p: &str, r: &gpu_core::error::GpuResult<nvidia_ga104::vbios_diag::VbiosDiag>) -> Vec<String> {
    match r {
        Ok(d) => {
            let mut v: Vec<String> = d.lines().into_iter().map(|l| format!("{}: {}", p, l)).collect();
            v.push(format!("{}: VBIOS detalii: {} scrieri cerute de diagnostic", p, d.writes));
            v
        }
        Err(e) => alloc::vec![format!("{}: VBIOS diagnostic: {}", p, e)],
    }
}

/// Which read-only step `gpu gfw` / `gpu vbios` / `gpu vbios --detalii` / `gpu fwsec` runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitStep {
    Gfw,
    Vbios,
    VbiosDetails,
    /// P6-35 (C-38): step I3b — where FWSEC sits in the VBIOS (tables of the PROM window only; the ucode is never read).
    Fwsec,
}

/// P6-35: the lines of `gpu fwsec` for one result — the VBIOS layout first, then `fwsec::locate` on the same window.
fn fwsec_lines(p: &str, r: &gpu_core::error::GpuResult<nvidia_ga104::fwsec::FwsecLocation>) -> Vec<String> {
    match r {
        Ok(f) => f.lines().into_iter().map(|l| format!("{}: {}", p, l)).collect(),
        Err(e) => alloc::vec![format!("{}: FWSEC (I3b): {}", p, e)],
    }
}

/// `read_layout` then `locate`, both on the same read-only window.
pub(super) fn locate_fwsec<M: gpu_core::hal::Mmio + ?Sized, K: gpu_core::hal::Clock + ?Sized>(
    win: &mut M,
    journal: &mut gpu_core::diag::Journal,
    clock: &mut K,
) -> gpu_core::error::GpuResult<nvidia_ga104::fwsec::FwsecLocation> {
    use nvidia_ga104::{fwsec, vbios};
    let layout = vbios::read_layout(win, journal, clock, vbios::VBIOS_TIMEOUT_US)?;
    fwsec::locate(win, journal, clock, vbios::VBIOS_TIMEOUT_US, &layout)
}

/// `gpu gfw` / `gpu vbios` on every NVIDIA board the Cyber-GPU probe accepts. Every line also goes to the log.
pub fn init_step(step: InitStep) -> Vec<String> {
    use nvidia_ga104::{gfw, probe, vbios, vbios_diag};
    let mut lines = Vec::new();
    let mut refused = 0usize;
    let nv: Vec<Device> = display_devices().into_iter().filter(|d| d.vendor == NVIDIA).collect();
    if nv.is_empty() {
        lines.push(String::from("nicio placa NVIDIA"));
    }
    for (n, d) in nv.iter().enumerate() {
        let p = format!("gpu{}", n);
        let mut cfg = ReadOnlyConfig { dev: d, refused: 0 };
        let addr = gpu_core::PciAddress { segment: 0, bus: d.bus, dev: d.dev, func: d.func };
        let candidate = probe::probe_config(&mut cfg, Some(addr));
        refused += cfg.refused;
        let c = match candidate {
            Ok(c) => c,
            Err(e) => {
                lines.push(format!("{}: {} cyber-gpu {}: {}", p, d.address(), CYBER_GPU_REV, e));
                continue;
            }
        };
        lines.push(format!("{}: {} {} (BAR0 {:#x}), cyber-gpu {}", p, d.address(), c.model, c.bar0, CYBER_GPU_REV));
        if d.command() & 0b10 == 0 {
            lines.push(format!("{}: registrele nu s-au citit: Memory Space oprit de firmware (nu il pornim)", p));
            continue;
        }
        if crate::time::since_boot_us().is_none() {
            lines.push(format!("{}: ceasul (TSC) nu e calibrat: limita de timp nu se poate tine, nu citesc", p));
            continue;
        }
        let _guard = WINDOW_LOCK.lock();
        let mut win = ReadOnlyBar0Window::new(c.bar0 & !0xFFF);
        let mut journal = gpu_core::diag::Journal::new(64);
        let mut clock = TscClock;
        let out = match step {
            InitStep::Gfw => gfw_lines(&p, &gfw::wait_gfw_boot(&mut win, &mut journal, &mut clock, gfw::GFW_TIMEOUT_US)),
            InitStep::Vbios => {
                vbios_lines(&p, &vbios::read_layout(&mut win, &mut journal, &mut clock, vbios::VBIOS_TIMEOUT_US))
            }
            InitStep::VbiosDetails => vbios_diag_lines(
                &p,
                &vbios_diag::diagnose(&mut win, &mut journal, &mut clock, vbios::VBIOS_TIMEOUT_US),
            ),
            InitStep::Fwsec => fwsec_lines(&p, &locate_fwsec(&mut win, &mut journal, &mut clock)),
        };
        lines.extend(out);
        for e in journal.events() {
            crate::info!("gpu: {}: jurnal cyber-gpu: {}", p, e);
        }
        if win.map_failed {
            lines.push(format!("{}: maparea ferestrei a esuat (citirile au dat FFFFFFFF)", p));
        }
        lines.push(format!("{}: {} pagini mapate pe rand (doar citire)", p, win.remaps));
        refused += win.refused + journal.writes();
    }
    lines.push(writes_line(refused));
    for l in &lines {
        crate::info!("gpu: {}", l);
    }
    lines
}

pub(super) fn writes_line(refused: usize) -> String {
    if refused == 0 {
        String::from("0 scrieri in placa (doar citire; adaptorul cyber-gpu a refuzat 0 scrieri)")
    } else {
        format!("0 scrieri in placa: biblioteca cyber-gpu a cerut {} scrieri, toate REFUZATE de adaptor", refused)
    }
}

/// `gpu gfw --sim` / `gpu vbios --sim` / `gpu vbios --detalii --sim` (test builds only): the same library calls on the
/// gpu-sim model (16 MiB BAR0; GFW finishes after a few rounds; a synthetic GA10x-like ROM chain, or for `--detalii`
/// the ASUS-like chain with the "NV" images at 0x24600, as on the workstation card). Every line says SIMULARE.
#[cfg(feature = "test-gpu-sim")]
pub fn init_step_sim(step: InitStep) -> Vec<String> {
    use gpu_sim::{clock::SimClock, mmio::SimMmio, rom};
    use nvidia_ga104::{gfw, regs, vbios, vbios_diag};
    let mut bar0 = SimMmio::ga104_bar0();
    let mut journal = gpu_core::diag::Journal::new(64);
    let mut clock = SimClock::default();
    let mut lines = match step {
        InitStep::Gfw => {
            let (mask, prog) = (regs::NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK.offset,
                regs::NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT.offset);
            // SIM: protection lowered after 5 rounds, progress 0x40 then 0xff.
            bar0.set(mask, 0);
            bar0.set_after(mask, 5, 1);
            bar0.set(prog, 0x40);
            bar0.set_after(prog, 3, 0xff);
            gfw_lines("model", &gfw::wait_gfw_boot(&mut bar0, &mut journal, &mut clock, gfw::GFW_TIMEOUT_US))
        }
        InitStep::Vbios => {
            bar0.load_bytes(regs::PROM_BASE, &rom::ga10x_like_chain());
            vbios_lines("model", &vbios::read_layout(&mut bar0, &mut journal, &mut clock, vbios::VBIOS_TIMEOUT_US))
        }
        InitStep::VbiosDetails => {
            bar0.load_bytes(regs::PROM_BASE, &rom::asus_like_chain());
            vbios_diag_lines("model", &vbios_diag::diagnose(&mut bar0, &mut journal, &mut clock, vbios::VBIOS_TIMEOUT_US))
        }
        InitStep::Fwsec => {
            // SIM: the ASUS-like chain ("NV" images at 0x24600, as on the workstation card) with a GA10x-like FWSEC.
            bar0.load_bytes(regs::PROM_BASE, &rom::asus_like_chain_with_fwsec(&rom::SimFwsec::ga10x_like()));
            fwsec_lines("model", &locate_fwsec(&mut bar0, &mut journal, &mut clock))
        }
    };
    lines.push(format!("model: {} citiri BAR0, timp simulat {} us", bar0.reads.len(), clock.now));
    lines.push(writes_line(bar0.writes.len() + journal.writes()));
    let mut out: Vec<String> = lines.into_iter().map(|l| format!("SIMULARE: {}", l)).collect();
    out.insert(0, format!("SIMULARE: modelul gpu-sim al placii de pe workstation (cyber-gpu {}), nu placa reala", CYBER_GPU_REV));
    out.push(String::from("SIMULARE: nicio placa reala atinsa; rezultatul nu dovedeste nimic despre hardware"));
    for l in &out {
        crate::info!("gpu: {}", l);
    }
    out
}

// P6-35 (C-38): step I4 of Cyber-GPU — are the 4 GSP 570.144 files on the stick the official ones, and does our parser read
// their headers? Pure file check: no PCI configuration access, no BAR0 access. One file in memory at a time (the largest
// is ~64 MB, dropped as soon as it is checked). The NVIDIA files are never in the repo, the tests or the stick archive.

/// `FirmwareSource` over the kernel VFS (`/usb/firmware/...`).
pub(super) struct StickFirmware;

impl nvidia_ga104::firmware::FirmwareSource for StickFirmware {
    fn read_file(&mut self, path: &str) -> gpu_core::error::GpuResult<Vec<u8>> {
        use crate::fs::FsError;
        // C-74: a file missing on the OS stick is looked for on the second stick (mounted read-only at /usb2).
        let r = match crate::fs::read_all(path) {
            Err(FsError::NotFound | FsError::NotADirectory | FsError::InvalidPath) if path.starts_with("/usb/") => {
                let other = alloc::format!("/usb2/{}", &path["/usb/".len()..]);
                let r = crate::fs::read_all(&other);
                if r.is_ok() {
                    crate::info!("gpu: firmware: {} citit de pe al doilea stick", other);
                }
                r
            }
            r => r,
        };
        r.map_err(|e| match e {
            // No stick mounted at /usb is the same, for the user, as the file not being there.
            FsError::NotFound | FsError::NotADirectory | FsError::InvalidPath => gpu_core::error::GpuError::NotPresent,
            _ => gpu_core::error::GpuError::InvalidArgument("citirea de pe stick a esuat"),
        })
    }
}

/// `gpu firmware`: read-only check of the GSP 570.144 files on the stick. Every line also goes to the log.
pub fn firmware_check() -> Vec<String> {
    use nvidia_ga104::{firmware, fw_check};
    let lines = fw_check::check_stick(&mut StickFirmware, &firmware::GSP_570_144).lines();
    for l in &lines {
        crate::info!("gpu: {}", l);
    }
    lines
}

/// `gpu firmware --sim` (test builds only): the same check on the gpu-sim stand-in files (NOT NVIDIA files), with a
/// manifest pinned to those stand-ins. Every line says SIMULARE.
#[cfg(feature = "test-gpu-sim")]
pub fn firmware_check_sim() -> Vec<String> {
    use gpu_sim::fw_stick;
    use nvidia_ga104::{firmware::STICK_DIR, fw_check};
    let files = fw_stick::files();
    let man = core::array::from_fn(|i| fw_check::pin_of(fw_check::ROLE_ORDER[i], fw_stick::SIM_PATHS[i], &files[i]));
    let mut src = fw_check::MemSource::new();
    for (p, b) in fw_stick::SIM_PATHS.iter().zip(files) {
        src.insert(format!("{}{}", STICK_DIR, p), b);
    }
    src.insert(format!("{}{}", STICK_DIR, fw_check::LICENCE), fw_stick::licence());
    let mut out: Vec<String> =
        fw_check::check_stick(&mut src, &man).lines().into_iter().map(|l| format!("SIMULARE: {}", l)).collect();
    out.insert(0, format!("SIMULARE: fisierele de proba gpu-sim (cyber-gpu {}), NU fisierele NVIDIA", CYBER_GPU_REV));
    for l in &out {
        crate::info!("gpu: {}", l);
    }
    out
}

/// The steps of the level-0 window on the workstation (`gpu plan`).
pub const PLAN: &[&str] = &[
    "Fereastra nivel 0 pe workstation (doar citire, ~30 min, Kronos oprit):",
    " 1. pornire de pe stick, Esc -> cyber>",
    " 2. lspci                -> poza (placa NVIDIA 10de:xxxx si placa de retea)",
    " 3. gpu identifica       -> poza (BAR-uri, Resizable BAR, BOOT_0/BOOT_42, cipul)",
    " 4. gpu gfw              -> poza (I2: firmware-ul placii a terminat pornirea? asteptat: progres 0xff, < 4 s)",
    " 5. gpu vbios            -> poza (I3: imaginile PCI-AT, EFI, FWSEC; BIT cu jetonul 0x70)",
    " 5b. gpu vbios --detalii -> poze cu toate randurile (structura VBIOS: imagini, PCIR, rupturi; 0 scrieri)",
    " 5c. gpu fwsec           -> poza (I3b: unde e FWSEC; 0 scrieri)",
    " 5d. gpu firmware        -> poza (I4: cele 4 fisiere 570.144 de pe stick; 0 accese la placa)",
    " 6. log gpu 40           -> poza",
    " 7. hw                   -> poza",
    " 8. net                  -> poza (placa de retea a workstation-ului e necunoscuta: notam ce apare)",
    "Nu se scrie nimic in placa video; dupa poze: oprire si inapoi in Windows pentru Kronos.",
    "",
    "Fereastra I6 (P6-40, C-52; DOAR cu Adrian de fata, DOAR stick-ul de test `gpu-scrie`):",
    " 0. inainte: oprire completa (cablul scos 30 s), ca placa sa porneasca rece",
    " 1. gpu pregatire-i6     -> poza (doar citire: semnatura 0/1/2, VRAM 8192 MiB, WPR2 coborat, TRECE, 0 scrieri)",
    "    daca NU TRECE: STOP, poza, nu continua",
    " 2. gpu i6 --scrie       -> citeste avertismentul, scrie DA -> poza cu toate randurile",
    "    (Bus Master pornit, FWSEC-FRTS pe GSP, Bus Master oprit la loc; jurnalul fiecarei scrieri merge pe stick pe loc)",
    " 3. raport               -> /usb/raport/... (impreuna cu /usb/raport/gpu-jurnal-NNN.txt)",
    " 4. oprire completa (cablul scos 30 s) -> Windows: nvidia-smi + Kronos",
    "Ecran stins sau blocat: astepti 1 minut, apoi butonul tinut 10 s si cablul scos 30 s.",
];

/// P6-40: what `gpu i6 --scrie` says before it asks for `DA`.
/// What `gpu lant` (P6-42, C-55) does, shown before it asks for `DA`.
pub const LANT_WARNING: &[&str] = &[
    "ATENTIE: gpu lant face pasii ferestrei unul dupa altul si SCRIE in placa video (doar stick-ul de test):",
    " 1. pregatire-i6 (doar citire)  2. Bus Master pornit  3. I6 FWSEC-FRTS (aceleasi scrieri ca gpu i6 --scrie)",
    " 4. WPR2 citit dupa I6 (doar citire)",
    " 5. I7 (NOU, P6-56): fisierele GSP 570.144 de pe stick in RAM, apoi Booter pe SEC2 porneste GSP-RM",
    "    (~750 de scrieri noi, in fereastra SEC2 0x840000-0x843FFF)  6. I7 citit (doar citire)",
    " 7. I8 (NOU, P6-56): SetSystemInfo + SetRegistry in coada din RAM, o sonerie GSP (0x110C00), asteptare GSP_INIT_DONE",
    "    (cererile secventiatorului GSP-RM doar in ferestrele GSP/SEC2)",
    " 8. golden + I9 (NOU, P6-56): mesaje RM catre GSP-RM, fiecare cu o sonerie GSP (0x110C00); tabelele de pagini",
    "    in VRAM prin fereastra VRAM (0x1700 + 0x700000-0x7FFFFF), doar peste 256 MiB si sub zonele GSP-RM",
    "    (ecranul si GSP-RM protejate)",
    " 9. I10 (NOU, P6-56): add64 pe canalul de lucru: invalidarea TLB (0xB8xxxx), SONERIA CANALULUI 0xBB0090 o data",
    "    (prima data pe placa), rezultatul citit din VRAM si comparat bit cu bit; apoi 16 lansari cu marca de timp",
    "10. Bus Master oprit la loc  11. turul de citire dupa",
    "Se opreste la primul esec; Bus Master se pune la loc si dupa o eroare. Fiecare acces merge pe loc in",
    "/usb/raport/gpu-jurnal-NNN.txt. Totul dispare la oprirea completa (cablul scos 30 s). Doar cu Adrian de fata.",
];

pub const I6_WARNING: &[&str] = &[
    "ATENTIE: gpu i6 --scrie SCRIE in placa video (pasul I6, prima scriere a driverului nostru):",
    " - 1 scriere in configuratia PCI: Bus Master pornit (bitul 2 din COMMAND);",
    " - ~720 de scrieri in 18 registre, toate in fereastra GSP 0x110000-0x113FFF: FWSEC (programul semnat NVIDIA",
    "   din VBIOS) incarcat prin DMA in falcon-ul GSP si pornit; el ridica zona protejata WPR2 in memoria video;",
    " - 1 scriere la final: Bus Master inapoi cum era (si daca I6 da eroare).",
    "VBIOS-ul nu se scrie; sigurantele doar se citesc; totul dispare la oprirea completa (cablul scos 30 s).",
    "Fiecare scriere merge pe loc in /usb/raport/gpu-jurnal-NNN.txt. Doar cu Adrian de fata.",
];

/// Scenario `gpu-ro`: maps the first BAR of the first display controller through the read-only window and writes
/// to it. The write must raise a page fault ("pagina prezenta, scriere") — proof that the window cannot write.
#[cfg(feature = "test-gpu-write")]
pub fn test_write_faults() {
    let d = display_devices().into_iter().next().expect("test: nicio placa video in QEMU");
    let bar = bars(&d).into_iter().find(|b| !b.io).expect("test: placa video fara BAR de memorie");
    crate::info!("test: scriu in fereastra gpu (doar citire) la {:#x}", WINDOW);
    let _ = with_readonly_window(bar.phys, |w| {
        // SAFETY: deliberate write to a read-only mapping; must fault before reaching the device.
        unsafe { (w as *mut u32).write_volatile(0) };
    });
    panic!("test: scrierea in fereastra gpu NU a dat page fault");
}

/// P6-21: the 4 MiB read-only window of `gpu gfw` / `gpu vbios`, on QEMU's display controller (its BAR0 is the
/// framebuffer, 16 MiB): reads at offsets in different pages must equal a direct uncached read of the same bytes,
/// each new page is mapped once, and the window is gone afterwards. Nothing is written to the device.
#[cfg(all(feature = "qemu-test", not(feature = "test-gpu-write")))]
pub fn test_bar0_window() {
    use gpu_core::hal::Mmio;
    let Some(d) = display_devices().into_iter().next() else {
        crate::info!("test: fereastra gpu de 4 MiB sarita (nicio placa video)");
        return;
    };
    let Some(bar) = bars(&d).into_iter().find(|b| b.index == 0 && !b.io) else {
        crate::info!("test: fereastra gpu de 4 MiB sarita (fara BAR0 de memorie)");
        return;
    };
    let direct = paging::map_mmio(bar.phys, nvidia_ga104::regs::INIT_WINDOW).expect("test: map_mmio");
    let offsets = [0u64, 4, 0x1000, 0x11_8128, 0x30_0000, 0x3F_FFFC];
    let _guard = WINDOW_LOCK.lock();
    let mut win = ReadOnlyBar0Window::new(bar.phys);
    for &o in &offsets {
        // SAFETY: `direct` maps [bar.phys, bar.phys + 4 MiB) uncached (map_mmio above); o < 4 MiB, aligned.
        let want = unsafe { ((direct + o) as *const u32).read_volatile() };
        let got = win.read32_raw(o);
        assert!(got == want, "test: fereastra gpu la {:#x}: {:#x}, direct {:#x}", o, got, want);
    }
    assert!(win.remaps == 5 && !win.map_failed && win.refused == 0, "test: fereastra gpu: {} remapari", win.remaps);
    drop(win);
    assert!(paging::translate(WINDOW).is_none(), "test: fereastra gpu a ramas mapata");
    crate::info!("test: fereastra gpu de 4 MiB OK (6 citiri, 5 pagini mapate pe rand, doar citire)");
}
