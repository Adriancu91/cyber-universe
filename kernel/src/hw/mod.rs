//! `hw lista`: the complete hardware list (F4 of the independence plan, C-68), by SAFE READS ONLY.
//!
//! Sources: CPUID, the memory map from Limine, the PCI configuration space (already mapped by `pci`), the USB devices
//! the XHCI driver has already enumerated, and the ACPI tables found at boot. Nothing here writes to a device:
//! no Command register changes, no BAR sizing (that needs a write), no power-state changes. Wi-Fi cards are only
//! identified and their configuration registers read; there is no Wi-Fi driver.
//! Written from the PCI Local Bus 3.0 / PCI Express Base specifications (configuration header, capability list,
//! Link Status), ACPI 6.5 chapter 5 (MCFG, HPET, FADT) and the Intel/AMD CPUID documentation.

pub mod names;

use crate::pci::Device;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// One line per fact; `detalii` adds the per-device registers. `driver_of` says which of our drivers runs a function.
pub fn lista(detalii: bool, driver_of: &dyn Fn(&Device) -> &'static str, cpu_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.push(String::from("== hw lista (doar citiri, nicio scriere in dispozitive) =="));
    cpu(&mut out, detalii, cpu_name);
    memory(&mut out, detalii);
    let pci = crate::pci::devices();
    let with_driver = pci.iter().filter(|d| driver_of(d) != "-").count();
    out.push(format!("PCI: {} functii ({} cu driver)", pci.len(), with_driver));
    for d in &pci {
        out.push(format!(
            "  {} {:04x}:{:04x} {}, {} [{}]",
            d.address(), d.vendor, d.device, pci_name(d), names::pci_class(d.class, d.subclass, d.prog_if), driver_of(d)
        ));
        if detalii {
            pci_details(&mut out, d);
        }
    }
    let wifi: Vec<&Device> = pci.iter().filter(|d| names::is_wifi(d.vendor, d.device, d.class, d.subclass)).collect();
    if wifi.is_empty() {
        out.push(String::from("Wi-Fi: niciuna (nicio placa fara fir pe PCI)"));
    }
    for d in &wifi {
        wifi_lines(&mut out, d, detalii);
    }
    let usb = usb(&mut out, &pci, detalii);
    let acpi = acpi(&mut out, detalii);
    out.push(format!(
        "hw lista: {} PCI, {} USB, {} tabele ACPI, Wi-Fi {}; 0 scrieri in dispozitive",
        pci.len(), usb, acpi, wifi.len()
    ));
    out
}

/// "Intel ICH9 SATA AHCI", "Intel 8086:1234" or "furnizor 1d0f:0001".
fn pci_name(d: &Device) -> String {
    match (names::pci_vendor(d.vendor), names::pci_device(d.vendor, d.device)) {
        (Some(v), Some(n)) => format!("{} {}", v, n),
        (Some(v), None) => format!("{} (model necunoscut)", v),
        (None, _) => String::from("(furnizor necunoscut)"),
    }
}

fn cpu(out: &mut Vec<String>, detalii: bool, name: &str) {
    use core::arch::x86_64::__cpuid;
    let l0 = __cpuid(0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&l0.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&l0.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&l0.ecx.to_le_bytes());
    let l1 = __cpuid(1);
    let base_family = (l1.eax >> 8) & 0xF;
    let family = if base_family == 0xF { base_family + ((l1.eax >> 20) & 0xFF) } else { base_family };
    let model = if base_family == 0x6 || base_family == 0xF { ((l1.eax >> 16) & 0xF) << 4 | (l1.eax >> 4) & 0xF } else { (l1.eax >> 4) & 0xF };
    out.push(format!("procesor: {}", name));
    out.push(format!(
        "  {} familia {:#x} model {:#x} pas {}; nuclee in MADT {}, pornite {}",
        core::str::from_utf8(&vendor).unwrap_or("?"),
        family, model, l1.eax & 0xF,
        crate::acpi::madt::get().cpu_count,
        crate::arch::x86_64::smp::online()
    ));
    let mut flags: Vec<&str> = Vec::new();
    for (bit_set, name) in [
        (l1.edx & 1 << 26 != 0, "sse2"),
        (l1.ecx & 1 != 0, "sse3"),
        (l1.ecx & 1 << 19 != 0, "sse4.1"),
        (l1.ecx & 1 << 20 != 0, "sse4.2"),
        (l1.ecx & 1 << 21 != 0, "x2apic"),
        (l1.ecx & 1 << 25 != 0, "aes"),
        (l1.ecx & 1 << 28 != 0, "avx"),
        (l1.ecx & 1 << 31 != 0, "hipervizor"),
    ] {
        if bit_set {
            flags.push(name);
        }
    }
    if l0.eax >= 7 {
        let l7 = __cpuid(7);
        if l7.ebx & 1 << 5 != 0 {
            flags.push("avx2");
        }
        if l7.ebx & 1 << 16 != 0 {
            flags.push("avx512f");
        }
    }
    out.push(format!("  instructiuni: {}", flags.join(" ")));
    if detalii {
        out.push(format!("  CPUID: frunza maxima {:#x}, extinsa {:#x}, semnatura {:#010x}", l0.eax, __cpuid(0x8000_0000).eax, l1.eax));
    }
}

fn memory(out: &mut Vec<String>, detalii: bool) {
    use crate::mem::{frame, map, map::PAGE_SIZE};
    let regions = map::regions();
    // RAM the OS can own: usable now, plus what the bootloader and the kernel image occupy (reclaimed or in use).
    let ram: u64 = regions
        .iter()
        .filter(|r| matches!(r.kind, map::Kind::Usable | map::Kind::BootloaderReclaimable | map::Kind::Kernel))
        .map(|r| r.length)
        .sum();
    out.push(format!(
        "memorie: RAM {} MiB, libera acum {} MiB; {} regiuni in harta",
        ram >> 20, frame::free_frames() * PAGE_SIZE >> 20, regions.len()
    ));
    if detalii {
        for r in regions {
            out.push(format!("  {:#014x}-{:#014x} {:>8} KiB {}", r.base, r.end(), r.length >> 10, r.kind.name()));
        }
    }
}

/// Configuration registers of one function: header, BARs as programmed (not sized), capabilities, link and power.
fn pci_details(out: &mut Vec<String>, d: &Device) {
    let cmd_status = d.read32(0x04);
    let (cmd, status) = (cmd_status as u16, (cmd_status >> 16) as u16);
    let header = (d.read32(0x0C) >> 16) as u8;
    let sub = d.read32(0x2C);
    let irq = d.read32(0x3C);
    let yes = |b: bool| if b { "da" } else { "nu" };
    out.push(format!(
        "    rev {:02x}, antet {:02x}, subsistem {:04x}:{:04x}, IRQ linie {} pin {}",
        d.read32(0x08) as u8, header,
        if header & 0x7F == 0 { sub as u16 } else { 0 }, if header & 0x7F == 0 { (sub >> 16) as u16 } else { 0 },
        irq as u8, (irq >> 8) as u8
    ));
    out.push(format!(
        "    Command {:04x} (I/O {}, memorie {}, Bus Master {}), Status {:04x}",
        cmd, yes(cmd & 1 != 0), yes(cmd & 2 != 0), yes(cmd & 4 != 0), status
    ));
    for line in bars(d, header) {
        out.push(format!("    {}", line));
    }
    let caps = capabilities(d);
    if !caps.is_empty() {
        let list: Vec<String> = caps.iter().map(|&(p, id)| format!("{}@{:02x}", names::pci_capability(id), p)).collect();
        out.push(format!("    capabilitati: {}", list.join(" ")));
    }
    if let Some(&(p, _)) = caps.iter().find(|&&(_, id)| id == 0x01) {
        out.push(format!("    alimentare: D{}", d.read32(p + 4) & 3));
    }
    if let Some(&(p, _)) = caps.iter().find(|&&(_, id)| id == 0x10) {
        let link = (d.read32(p + 0x10) >> 16) as u16; // Link Status: upper half of the dword at +0x10
        if link & 0x3F0 != 0 {
            out.push(format!("    PCIe: legatura Gen{} x{}", link & 0xF, (link >> 4) & 0x3F));
        }
    }
}

/// The BARs as the firmware programmed them (address and type). Sizing a BAR needs a write, so sizes are not shown.
fn bars(d: &Device, header: u8) -> Vec<String> {
    let count = match header & 0x7F {
        0 => 6,
        1 => 2,
        _ => 0,
    };
    let mut lines = Vec::new();
    let mut i = 0u16;
    while i < count {
        let low = d.read32(0x10 + i * 4);
        let mut step = 1;
        if low & 1 != 0 {
            if low & !3 != 0 {
                lines.push(format!("BAR{}: I/O {:#x}", i, low & !3));
            }
        } else {
            let is64 = (low >> 1) & 3 == 2;
            let mut addr = (low & !0xF) as u64;
            if is64 && i + 1 < count {
                addr |= (d.read32(0x14 + i * 4) as u64) << 32;
                step = 2;
            }
            if addr != 0 {
                lines.push(format!(
                    "BAR{}: memorie {:#x} ({}{})",
                    i, addr, if is64 { "64 biti" } else { "32 biti" }, if low & 8 != 0 { ", prefetch" } else { "" }
                ));
            }
        }
        i += step;
    }
    lines
}

/// (offset, id) of every capability, walking the list read-only (at most 48 entries, against loops).
fn capabilities(d: &Device) -> Vec<(u16, u8)> {
    let mut caps = Vec::new();
    if d.read32(0x04) >> 16 & 1 << 4 == 0 {
        return caps;
    }
    let mut ptr = (d.read32(0x34) & 0xFC) as u16;
    while ptr >= 0x40 && caps.len() < 48 {
        let cap = d.read32(ptr);
        caps.push((ptr, cap as u8));
        ptr = ((cap >> 8) & 0xFC) as u16;
    }
    caps
}

/// A Wi-Fi card: identified and its basic registers read (configuration space only; no driver, no write).
fn wifi_lines(out: &mut Vec<String>, d: &Device, detalii: bool) {
    let cmd = d.read32(0x04) as u16;
    let caps = capabilities(d);
    let power = caps.iter().find(|&&(_, id)| id == 0x01).map(|&(p, _)| d.read32(p + 4) & 3);
    out.push(format!(
        "Wi-Fi: {} {:04x}:{:04x} {} - doar identificata, fara driver",
        d.address(), d.vendor, d.device, pci_name(d)
    ));
    out.push(format!(
        "  registre: Command {:04x}, BAR0 {}, alimentare {}",
        cmd,
        d.bar(0).map_or(String::from("-"), |a| format!("{:#x}", a)),
        power.map_or(String::from("?"), |p| format!("D{}", p))
    ));
    if detalii {
        for row in 0..4u16 {
            let words: Vec<String> = (0..4u16).map(|c| format!("{:08x}", d.read32(row * 16 + c * 4))).collect();
            out.push(format!("  config {:02x}: {}", row * 16, words.join(" ")));
        }
    }
}

/// USB controllers (from PCI) and the devices the XHCI driver has already enumerated. Returns the device count.
fn usb(out: &mut Vec<String>, pci: &[Device], detalii: bool) -> usize {
    let controllers = pci.iter().filter(|d| d.class == 0x0C && d.subclass == 0x03).count();
    let devices = crate::drivers::usb::devices();
    out.push(format!("USB: {} controlere, {} dispozitive enumerate", controllers, devices.len()));
    for d in &devices {
        out.push(format!(
            "  port {} {:04x}:{:04x} {}, {}, {}",
            d.location(), d.vendor, d.product,
            names::usb_vendor(d.vendor).unwrap_or("(furnizor necunoscut)"), d.kind(), names::usb_speed(d.speed)
        ));
        if detalii {
            out.push(format!(
                "    clasa {:02x}/{:02x}/{:02x}, interfata {}, configuratia {}, {} puncte finale, slot {}",
                d.class.0, d.class.1, d.class.2, d.interface, d.config, d.endpoints.iter().flatten().count(), d.slot
            ));
        }
    }
    devices.len()
}

/// The ACPI tables found at boot; with details, the useful fields of the simple ones (MADT, MCFG, HPET, FADT).
fn acpi(out: &mut Vec<String>, detalii: bool) -> usize {
    let tables = crate::acpi::tables();
    out.push(format!("ACPI: {} tabele", tables.len()));
    for (phys, h) in &tables {
        let sig = core::str::from_utf8(&h.signature).unwrap_or("????");
        let oem = core::str::from_utf8(&h.oem_id).unwrap_or("?").trim_end();
        let (len, rev) = (h.length, h.revision);
        out.push(format!("  {} rev {}, {} octeti, OEM {}", sig, rev, len, oem));
        if detalii {
            out.push(format!("    la {:#x}", phys));
            if let Some(extra) = table_details(&h.signature) {
                out.push(format!("    {}", extra));
            }
        }
    }
    tables.len()
}

fn table_details(sig: &[u8; 4]) -> Option<String> {
    let (p, len) = crate::acpi::find(sig)?;
    // SAFETY: `find` returned a mapped, checksummed table of `len` bytes.
    let t = unsafe { core::slice::from_raw_parts(p, len) };
    let u64_at = |o: usize| (o + 8 <= t.len()).then(|| u64::from_le_bytes(t[o..o + 8].try_into().unwrap()));
    match sig {
        b"APIC" => {
            let m = crate::acpi::madt::get();
            Some(format!("procesoare {}, I/O APIC {}, redirectari IRQ {}, APIC local {:#x}", m.cpu_count, m.ioapic_count, m.override_count, m.local_apic))
        }
        b"MCFG" if len >= 60 => Some(format!("ECAM {:#x}, magistralele {}-{}", u64_at(44)?, t[54], t[55])),
        b"HPET" if len >= 56 => Some(format!("HPET la {:#x}", u64_at(44)?)),
        b"FACP" if len >= 116 => Some(format!("profil {}, SCI IRQ {}", t[45], u16::from_le_bytes([t[46], t[47]]))),
        _ => None,
    }
}
