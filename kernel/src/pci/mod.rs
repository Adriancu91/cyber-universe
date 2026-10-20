//! PCI / PCI Express: finds every device through the memory-mapped configuration space (ECAM)
//! described by the ACPI MCFG table, following PCI-to-PCI bridges from bus 0.
//! Written from the PCI Local Bus and PCI Express Base specifications (configuration header layout)
//! and the ACPI specification (MCFG).

use crate::mem::paging;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug)]
pub struct Device {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    /// Virtual address of the device's 4 KiB configuration space.
    config: u64,
}

#[derive(Clone, Copy)]
struct Ecam {
    base: u64,
    start_bus: u8,
    end_bus: u8,
}

static DEVICES: crate::sched::Mutex<Vec<Device>> = crate::sched::Mutex::new(Vec::new());

impl Device {
    pub fn read32(&self, offset: u16) -> u32 {
        // SAFETY: `config` is the mapped (uncached) configuration page of this function; offset < 4096, aligned.
        unsafe { ((self.config + (offset & 0xFFC) as u64) as *const u32).read_volatile() }
    }

    pub fn write32(&self, offset: u16, value: u32) {
        // SAFETY: as in `read32`.
        unsafe { ((self.config + (offset & 0xFFC) as u64) as *mut u32).write_volatile(value) }
    }

    /// Physical address of memory BAR `i` (0..6), handling 64-bit BARs. `None` for I/O or empty BARs.
    #[allow(dead_code)] // used by the disk, USB and network drivers (P3-02 and later)
    pub fn bar(&self, i: u16) -> Option<u64> {
        let low = self.read32(0x10 + i * 4);
        if low & 1 != 0 {
            return None; // I/O space
        }
        let mut addr = (low & !0xF) as u64;
        if (low >> 1) & 3 == 2 {
            addr |= (self.read32(0x14 + i * 4) as u64) << 32;
        }
        (addr != 0).then_some(addr)
    }

    /// Lets the device answer memory accesses and act as bus master (needed for DMA).
    #[allow(dead_code)] // used by the disk, USB and network drivers (P3-02 and later)
    pub fn enable(&self) {
        let cmd = self.read32(0x04);
        self.write32(0x04, (cmd & 0xFFFF) | 0b110);
    }

    pub fn kind(&self) -> &'static str {
        match (self.class, self.subclass) {
            (0x01, 0x01) => "controler IDE",
            (0x01, 0x06) => "controler SATA (AHCI)",
            (0x01, 0x08) => "controler NVMe",
            (0x01, _) => "controler de stocare",
            (0x02, _) => "placa de retea",
            (0x03, _) => "placa video",
            (0x04, _) => "multimedia",
            (0x06, 0x00) => "punte gazda",
            (0x06, 0x01) => "punte ISA",
            (0x06, 0x04) => "punte PCI",
            (0x06, _) => "punte",
            (0x0C, 0x03) => "controler USB",
            (0x0C, 0x05) => "SMBus",
            _ => "alt dispozitiv",
        }
    }
}

fn find_ecam() -> Option<Ecam> {
    let (table, len) = crate::acpi::find(b"MCFG")?;
    if len < 44 + 16 {
        return None;
    }
    // SAFETY: `find` returned a mapped, checksummed table of `len` bytes; the first entry starts at 44.
    let e = unsafe { core::slice::from_raw_parts(table.add(44), 16) };
    let segment = u16::from_le_bytes([e[8], e[9]]);
    if segment != 0 {
        return None; // only PCI segment 0 for now (all PCs we target)
    }
    Some(Ecam { base: u64::from_le_bytes(e[0..8].try_into().unwrap()), start_bus: e[10], end_bus: e[11] })
}

fn config_page(ecam: Ecam, bus: u8, dev: u8, func: u8) -> Option<u64> {
    let phys = ecam.base + (((bus - ecam.start_bus) as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12);
    paging::map_mmio(phys, 4096).ok()
}

fn scan_bus(ecam: Ecam, bus: u8, out: &mut Vec<Device>, depth: u8) {
    if bus < ecam.start_bus || bus > ecam.end_bus || depth > 8 {
        return;
    }
    for dev in 0..32 {
        for func in 0..8 {
            let Some(config) = config_page(ecam, bus, dev, func) else { return };
            // SAFETY: just mapped; offset 0 holds vendor and device ids.
            let id = unsafe { (config as *const u32).read_volatile() };
            if id & 0xFFFF == 0xFFFF {
                if func == 0 {
                    break; // no device in this slot
                }
                continue;
            }
            let d = Device {
                bus,
                dev,
                func,
                vendor: id as u16,
                device: (id >> 16) as u16,
                class: 0,
                subclass: 0,
                prog_if: 0,
                config,
            };
            let class = d.read32(0x08);
            let d = Device { class: (class >> 24) as u8, subclass: (class >> 16) as u8, prog_if: (class >> 8) as u8, ..d };
            let header = (d.read32(0x0C) >> 16) as u8;
            out.push(d);
            if header & 0x7F == 1 {
                scan_bus(ecam, (d.read32(0x18) >> 8) as u8, out, depth + 1); // bridge: secondary bus
            }
            if func == 0 && header & 0x80 == 0 {
                break; // single-function device
            }
        }
    }
}

/// Finds every PCI device and logs the list.
pub fn init() {
    let Some(ecam) = find_ecam() else {
        crate::warn!("pci: tabela ACPI MCFG lipseste, nu pot gasi dispozitivele");
        return;
    };
    let mut found = Vec::new();
    scan_bus(ecam, ecam.start_bus, &mut found, 0);
    for d in &found {
        crate::info!(
            "pci: {:02x}:{:02x}.{} {:04x}:{:04x} clasa {:02x}:{:02x}:{:02x} {}",
            d.bus, d.dev, d.func, d.vendor, d.device, d.class, d.subclass, d.prog_if, d.kind()
        );
    }
    crate::info!("pci: {} dispozitive (ECAM la {:#x}, magistralele {}-{})", found.len(), ecam.base, ecam.start_bus, ecam.end_bus);
    *DEVICES.lock() = found;
}

/// Every device found by `init`.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell (`lspci`)
pub fn devices() -> Vec<Device> {
    DEVICES.lock().clone()
}

/// The first device of the given class, subclass and programming interface.
#[allow(dead_code)] // used by the disk, USB and network drivers (P3-02 and later)
pub fn find(class: u8, subclass: u8, prog_if: u8) -> Option<Device> {
    DEVICES.lock().iter().copied().find(|d| (d.class, d.subclass, d.prog_if) == (class, subclass, prog_if))
}
