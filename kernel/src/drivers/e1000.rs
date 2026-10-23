//! Intel gigabit Ethernet: 8254x (e1000), 82574 (e1000e) and the I217/I218/I219 family (e.g. the I219-LM
//! `8086:15e3` of the Dell workstation). The driver logic lives in the `e1000e/` crate (tested on the host against a
//! model); this file only gives it the hardware: memory-mapped registers, a guarded DMA block, a delay.

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;
use cyber_e1000e::{E1000e, Family, Hw, Options, BUF};

/// True if this driver knows the card.
pub fn supports(vendor: u16, device: u16) -> bool {
    cyber_e1000e::identify(vendor, device).is_some()
}

/// Lowest address of the DMA block: away from the low memory boot-time allocations come from (lesson of test 7).
const DMA_LOW: u64 = 16 << 20;
/// Size of the register window mapped from BAR 0 (the highest register used, FWSM at 0x5B54, is inside).
const REGS_LEN: u64 = 0x20000;

pub struct KernelHw {
    regs: u64,
}

// SAFETY (all register accesses below): `regs` is the mapped, uncached register block of the card (REGS_LEN bytes)
// and every offset the driver uses is below REGS_LEN and 4-byte aligned.
impl Hw for KernelHw {
    fn read32(&mut self, reg: u32) -> u32 {
        unsafe { ((self.regs + reg as u64) as *const u32).read_volatile() }
    }
    fn write32(&mut self, reg: u32, v: u32) {
        unsafe { ((self.regs + reg as u64) as *mut u32).write_volatile(v) }
    }
    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)> {
        let f = frame::alloc_contiguous_between(pages as u64, DMA_LOW, 1 << 32)?;
        frame::pin(f, pages as u64, "e1000"); // the card writes here by DMA: never given to anyone else
        let p = phys_to_virt(f);
        // SAFETY: `pages` fresh contiguous frames, reachable through the HHDM (one linear range); kept by the driver.
        unsafe { core::ptr::write_bytes(p, 0, pages * PAGE_SIZE as usize) };
        Some((f, p))
    }
    fn delay_us(&mut self, us: u32) {
        match crate::time::since_boot_us() {
            Some(start) => while crate::time::since_boot_us().is_some_and(|t| t < start + us as u64) {
                core::hint::spin_loop();
            },
            None => (0..us * 100).for_each(|_| core::hint::spin_loop()),
        }
    }
    fn dma_sync(&mut self, ptr: *mut u8, len: usize) {
        const LINE: usize = 64;
        let start = ptr as usize & !(LINE - 1);
        let end = ptr as usize + len;
        // SAFETY: CLFLUSH drops cached copies of lines of a DMA buffer owned by this driver (mapped through the HHDM);
        // MFENCE orders it against the reads around it.
        unsafe {
            core::arch::asm!("mfence", options(nostack, preserves_flags));
            for a in (start..end).step_by(LINE) {
                core::arch::asm!("clflush [{}]", in(reg) a, options(nostack, preserves_flags));
            }
            core::arch::asm!("mfence", options(nostack, preserves_flags));
        }
    }
}

pub struct Nic {
    inner: E1000e<KernelHw>,
    pub name: &'static str,
}

impl super::nic::Card for Nic {
    fn send(&mut self, frame: &[u8]) -> bool {
        self.inner.send(frame)
    }
    fn receive(&mut self) -> Option<Vec<u8>> {
        let mut buf = [0u8; BUF];
        let n = self.inner.receive_into(&mut buf)?;
        Some(buf[..n].to_vec())
    }
    fn mac(&self) -> [u8; 6] {
        self.inner.mac
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn counters(&self) -> (u64, u64) {
        (self.inner.sent, self.inner.received)
    }
    fn link_info(&mut self) -> Option<alloc::string::String> {
        let (up, speed, fd) = self.inner.link();
        let bmsr = self.inner.phy_status();
        let regs = self.inner.registers();
        let r: Vec<alloc::string::String> = regs.iter().map(|(n, v)| alloc::format!("{} {:08x}", n, v)).collect();
        Some(alloc::format!(
            "legatura: {}; PHY {:08x} (BMSR {}); bloc DMA {:#x}\nregistre: {} (pasii de pornire: log e1000)",
            if up { alloc::format!("da, {} Mb/s, {}", speed, if fd { "full duplex" } else { "half duplex" }) } else { "NU".into() },
            self.inner.phy_id,
            bmsr.map_or(alloc::string::String::from("nu raspunde"), |b| alloc::format!("{:04x}", b)),
            self.inner.block_phys(),
            r.join(", ")
        ))
    }
}

/// Finds a supported Intel card and starts it.
pub fn init() -> Option<Nic> {
    let (dev, family, name) = crate::pci::devices().into_iter().find_map(|d| {
        cyber_e1000e::identify(d.vendor, d.device).map(|(f, n)| (d, f, n))
    })?;
    dev.enable();
    let bar = dev.bar(0)?;
    let regs = paging::map_mmio(bar, REGS_LEN).ok()?;
    crate::info!("e1000: {} ({:04x}:{:04x}) la {}, familia {:?}, registre la {:#x}", name, dev.vendor, dev.device, dev.address(), family, bar);
    let mut card = match E1000e::init(KernelHw { regs }, family, Options::DEFAULT) {
        Ok(c) => c,
        Err(e) => {
            crate::warn!("net: placa {} nu porneste: {:?}", name, e);
            return None;
        }
    };
    for s in &card.steps[..card.step_count] {
        crate::info!("e1000: {}: reg {:#x} {:08x} -> {:08x}", s.what, s.reg, s.before, s.after);
    }
    let block = card.block_phys();
    crate::info!(
        "e1000: PHY {:08x} (adresa {}), bloc DMA de {} pagini la {:#x} ({}), primul buffer RX {:#x}",
        card.phy_id,
        family.phy_address(),
        cyber_e1000e::BLOCK_PAGES,
        block,
        if block >= DMA_LOW && block + (cyber_e1000e::BLOCK_PAGES as u64) * PAGE_SIZE <= 1 << 32 { "intre 16 MiB si 4 GB" } else { "IN AFARA intervalului" },
        block + 2 * PAGE_SIZE
    );
    let m = card.mac;
    crate::info!("net: placa {} pornita, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", name, m[0], m[1], m[2], m[3], m[4], m[5]);
    if family == Family::Pch {
        crate::info!("e1000: {}: pasii PCH sunt testati doar pe model (e1000e/); pe hardware: workstation", name);
    }
    let (up, speed, _) = card.link();
    crate::info!("e1000: legatura {}", if up { alloc::format!("da ({} Mb/s)", speed) } else { "inca nu".into() });
    Some(Nic { inner: card, name })
}
