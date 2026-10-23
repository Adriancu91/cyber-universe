//! PCI / PCI Express: finds every device through the memory-mapped configuration space (ECAM)
//! described by the ACPI MCFG table: PCI-to-PCI bridges from the first bus, then every other root bus of the range.
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

    /// Physical address of the first memory BAR (I/O BARs and the upper halves of 64-bit BARs are skipped).
    pub fn first_memory_bar(&self) -> Option<u64> {
        let mut i = 0;
        while i < 6 {
            let low = self.read32(0x10 + i * 4);
            if low & 1 == 0 {
                if let Some(a) = self.bar(i) {
                    return Some(a);
                }
            }
            i += if low & 1 == 0 && (low >> 1) & 3 == 2 { 2 } else { 1 };
        }
        None
    }

    /// Offset of the capability with ID `id` in the configuration space (PCI Local Bus spec: capability list).
    pub fn capability(&self, id: u8) -> Option<u16> {
        if self.read32(0x04) >> 16 & 1 << 4 == 0 {
            return None; // no capability list
        }
        let mut ptr = (self.read32(0x34) & 0xFC) as u16;
        for _ in 0..48 {
            if ptr == 0 {
                return None;
            }
            let cap = self.read32(ptr);
            if cap & 0xFF == id as u32 {
                return Some(ptr);
            }
            ptr = ((cap >> 8) & 0xFC) as u16;
        }
        None
    }

    /// Puts the device in the full-power state D0 if it has the PCI power-management capability (firmware may
    /// leave unused devices in D3, where their registers read as all ones). PCI Bus Power Management spec 1.2.
    /// Read, change only the power-state field (bits 1:0), write; before and after go to the log.
    pub fn power_on(&self) {
        let Some(ptr) = self.capability(0x01) else { return };
        let pmcsr = self.read32(ptr + 4);
        if pmcsr & 3 != 0 {
            self.write32(ptr + 4, pmcsr & !3);
            // The device needs up to 10 ms to leave D3 (spec: D3hot -> D0 recovery time).
            let start = crate::time::since_boot_us().unwrap_or(0);
            while crate::time::since_boot_us().is_some_and(|t| t < start + 10_000) {
                core::hint::spin_loop();
            }
            crate::info!(
                "pci: {} PMCSR stare D{} -> D0 ({:04x} -> {:04x})",
                self.address(), pmcsr & 3, pmcsr as u16, self.read32(ptr + 4) as u16
            );
        }
    }

    /// PCI Express Device Control, read only (`log rtl`): the OS no longer writes it (test 5, 2026-10-22).
    pub fn pcie_devctl(&self) -> Option<u16> {
        let ptr = self.capability(0x10)?;
        Some(self.read32(ptr + 8) as u16) // Device Control is the low half of the dword at +8
    }

    /// The Command register (low half of offset 4).
    pub fn command(&self) -> u16 {
        self.read32(0x04) as u16
    }

    /// Sets one bit of the Command register: read, change that bit, write (only if it is not set yet); before and
    /// after go to the log, tagged with `who` (so `log rtl` shows the Realtek's). Writing 0 to the Status half leaves
    /// its write-1-to-clear bits alone.
    fn set_command_bit(&self, bit: u16, name: &str, who: &str) {
        let before = self.command();
        if before & 1 << bit != 0 {
            return;
        }
        self.write32(0x04, (before | 1 << bit) as u32);
        let after = self.command();
        crate::info!("pci: {} {} Command bit {} ({}) 0 -> {} ({:04x} -> {:04x})",
            self.address(), who, bit, name, after >> bit & 1, before, after);
    }

    /// Lets the device answer memory accesses (bit 1) and act as bus master (bit 2, needed for DMA): one bit at a time.
    #[allow(dead_code)] // used by the disk, USB and network drivers (P3-02 and later)
    pub fn enable(&self) {
        self.enable_as("");
    }

    /// `enable`, with a word for the log lines (e.g. "rtl8168").
    pub fn enable_as(&self, who: &str) {
        self.set_command_bit(1, "Memory Space", who);
        self.set_command_bit(2, "Bus Master", who);
    }

    /// bus:device.function, as `lspci` shows it.
    pub fn address(&self) -> alloc::string::String {
        alloc::format!("{:02x}:{:02x}.{}", self.bus, self.dev, self.func)
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

/// Maps the configuration page of bus:dev.func and returns its virtual address and whether this call created the
/// mapping (so an empty slot can be unmapped again: only what is read stays mapped).
fn config_page(ecam: Ecam, bus: u8, dev: u8, func: u8) -> Option<(u64, bool)> {
    let phys = ecam.base + (((bus - ecam.start_bus) as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12);
    let was_mapped = paging::translate(crate::mem::map::phys_to_virt(phys) as u64).is_some();
    paging::map_mmio(phys, 4096).ok().map(|v| (v, !was_mapped))
}

/// Scans one bus and, through PCI-to-PCI bridges, the buses behind it. `visited` marks every bus scanned, so a bus
/// is never listed twice (bridge walk and root-bus sweep below).
fn scan_bus(ecam: Ecam, bus: u8, out: &mut Vec<Device>, depth: u8, visited: &mut [bool; 256]) {
    if bus < ecam.start_bus || bus > ecam.end_bus || depth > 8 || visited[bus as usize] {
        return;
    }
    visited[bus as usize] = true;
    for dev in 0..32 {
        for func in 0..8 {
            let Some((config, fresh)) = config_page(ecam, bus, dev, func) else { return };
            // SAFETY: just mapped; offset 0 holds vendor and device ids.
            let id = unsafe { (config as *const u32).read_volatile() };
            if id & 0xFFFF == 0xFFFF {
                if fresh {
                    paging::unmap(config); // nothing here: do not keep the page mapped
                }
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
                scan_bus(ecam, (d.read32(0x18) >> 8) as u8, out, depth + 1, visited); // bridge: secondary bus
            }
            if func == 0 && header & 0x80 == 0 {
                break; // single-function device
            }
        }
    }
}

/// Finds every PCI device and logs the list.
///
/// First the buses reachable from `start_bus` through bridges, then every other bus of the MCFG range: server
/// processors (e.g. Cascade Lake-X, one root bus per IIO stack) and PCI expander bridges have several root buses
/// that no bridge on bus 0 leads to (workstation test, 2026-10-23: the GPU at B3:00.0 was invisible). The sweep
/// reads only the vendor id of function 0 of each slot (an absent device reads all ones, PCI Express Base spec:
/// Unsupported Request on configuration reads) and writes nothing.
pub fn init() {
    let Some(ecam) = find_ecam() else {
        crate::warn!("pci: tabela ACPI MCFG lipseste, nu pot gasi dispozitivele");
        return;
    };
    let t0 = crate::time::since_boot_us().unwrap_or(0);
    let mut found = Vec::new();
    let mut visited = [false; 256];
    scan_bus(ecam, ecam.start_bus, &mut found, 0, &mut visited);
    let mut roots = 1;
    for bus in ecam.start_bus..=ecam.end_bus {
        if visited[bus as usize] {
            continue;
        }
        let before = found.len();
        scan_bus(ecam, bus, &mut found, 0, &mut visited);
        if found.len() > before {
            roots += 1;
            crate::info!("pci: radacina in plus: magistrala {:02x} ({} dispozitive, cu puntile ei)", bus, found.len() - before);
        }
    }
    found.sort_by_key(|d| (d.bus, d.dev, d.func));
    let ms = crate::time::since_boot_us().unwrap_or(0).saturating_sub(t0) / 1000;
    for d in &found {
        crate::info!(
            "pci: {:02x}:{:02x}.{} {:04x}:{:04x} clasa {:02x}:{:02x}:{:02x} {}",
            d.bus, d.dev, d.func, d.vendor, d.device, d.class, d.subclass, d.prog_if, d.kind()
        );
    }
    crate::info!(
        "pci: {} dispozitive pe {} radacini (ECAM la {:#x}, magistralele {:02x}-{:02x}, cautare {} ms)",
        found.len(), roots, ecam.base, ecam.start_bus, ecam.end_bus, ms
    );
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
