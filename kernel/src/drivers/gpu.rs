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
const WINDOW: u64 = 0xFFFF_C100_0000_0000;
/// Page-level write-through + cache disable (PWT, PCD): device registers must not be cached.
const UNCACHED: u64 = (1 << 3) | (1 << 4);

const NV_PMC_BOOT_0: u64 = 0x0;
const NV_PMC_BOOT_42: u64 = 0xA00;

static WINDOW_LOCK: crate::sched::Mutex<()> = crate::sched::Mutex::new(());

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

fn read_reg(window: u64, offset: u64) -> u32 {
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
pub const CYBER_GPU_REV: &str = "41e44ba";

/// Configuration space of one function for the Cyber-GPU libraries: reads only.
struct ReadOnlyConfig<'a> {
    dev: &'a Device,
    refused: usize,
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

/// The steps of the level-0 window on the workstation (`gpu plan`).
pub const PLAN: &[&str] = &[
    "Fereastra nivel 0 pe workstation (doar citire, ~30 min, Kronos oprit):",
    " 1. pornire de pe stick, Esc -> cyber>",
    " 2. lspci                -> poza (placa NVIDIA 10de:xxxx si placa de retea)",
    " 3. gpu identifica       -> poza (BAR-uri, Resizable BAR, BOOT_0/BOOT_42, cipul)",
    " 4. log gpu 40           -> poza",
    " 5. hw                   -> poza",
    " 6. net                  -> poza (placa de retea a workstation-ului e necunoscuta: notam ce apare)",
    "Nu se scrie nimic in placa video; dupa poze: oprire si inapoi in Windows pentru Kronos.",
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
