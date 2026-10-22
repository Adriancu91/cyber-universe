//! Realtek RTL8168/8111 gigabit Ethernet (PCI `10ec:8168`), e.g. the wired card of the Lenovo AMD test laptop.
//! The driver logic lives in the `rtl8168/` crate (tested on the host against a model of the chip); this file
//! only gives it the hardware: the memory-mapped registers, DMA pages from the frame allocator, a delay.

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;
use cyber_rtl8168::{Hw, InitError, Rtl8168};

/// Cards this driver knows (vendor Realtek 0x10EC): the PCIe RTL8111/8168 family.
const SUPPORTED: [u16; 1] = [0x8168];

pub fn supports(vendor: u16, device: u16) -> bool {
    vendor == 0x10EC && SUPPORTED.contains(&device)
}

pub struct KernelHw {
    regs: u64,
}

impl KernelHw {
    fn at(&self, reg: u16) -> u64 {
        self.regs + reg as u64
    }
}

// SAFETY (all register accesses below): `regs` is the mapped, uncached register block of the card (256 bytes,
// mapped as a whole page) and every offset used by the driver is below 0x100, aligned to the access size.
impl Hw for KernelHw {
    fn read8(&mut self, reg: u16) -> u8 {
        unsafe { (self.at(reg) as *const u8).read_volatile() }
    }
    fn read16(&mut self, reg: u16) -> u16 {
        unsafe { (self.at(reg) as *const u16).read_volatile() }
    }
    fn read32(&mut self, reg: u16) -> u32 {
        unsafe { (self.at(reg) as *const u32).read_volatile() }
    }
    fn write8(&mut self, reg: u16, v: u8) {
        unsafe { (self.at(reg) as *mut u8).write_volatile(v) }
    }
    fn write16(&mut self, reg: u16, v: u16) {
        unsafe { (self.at(reg) as *mut u16).write_volatile(v) }
    }
    fn write32(&mut self, reg: u16, v: u32) {
        unsafe { (self.at(reg) as *mut u32).write_volatile(v) }
    }
    fn alloc_page(&mut self) -> Option<(u64, *mut u8)> {
        let f = frame::alloc_frame()?;
        let p = phys_to_virt(f);
        // SAFETY: a fresh frame, reachable through the HHDM; the driver keeps it for as long as the card runs.
        unsafe { core::ptr::write_bytes(p, 0, PAGE_SIZE as usize) };
        Some((f, p))
    }
    fn dma_sync(&mut self, ptr: *mut u8, len: usize) {
        let mut line = ptr as usize & !63;
        let end = ptr as usize + len;
        // SAFETY: `ptr..ptr + len` is a DMA page of this driver, mapped through the HHDM; CLFLUSH writes a cache
        // line back (if dirty) and drops it from every cache level, MFENCE orders it against the reads around it.
        unsafe {
            core::arch::asm!("mfence", options(nostack, preserves_flags));
            while line < end {
                core::arch::asm!("clflush [{}]", in(reg) line, options(nostack, preserves_flags));
                line += 64;
            }
            core::arch::asm!("mfence", options(nostack, preserves_flags));
        }
    }
    fn delay_us(&mut self, us: u32) {
        match crate::time::since_boot_us() {
            Some(start) => while crate::time::since_boot_us().is_some_and(|t| t < start + us as u64) {
                core::hint::spin_loop();
            },
            None => (0..us * 100).for_each(|_| core::hint::spin_loop()),
        }
    }
}

pub struct Card {
    inner: Rtl8168<KernelHw>,
    /// Detailed receive records already written to the log (`log rx`).
    logged: usize,
    /// PCI Express Device Control before/after clearing No Snoop and Relaxed Ordering (`None`: no PCIe capability).
    devctl: Option<(u16, u16)>,
}

// SAFETY: the raw pointers inside point to DMA pages owned only by this driver; the card is used under `nic::NIC`'s lock.
unsafe impl Send for Card {}

impl super::nic::Card for Card {
    fn send(&mut self, frame: &[u8]) -> bool {
        self.inner.send(frame)
    }
    fn receive(&mut self) -> Option<Vec<u8>> {
        let mut buf = [0u8; cyber_rtl8168::BUF];
        let got = self.inner.receive(&mut buf).map(|n| buf[..n].to_vec());
        self.log_new_frames();
        got
    }
    fn mac(&self) -> [u8; 6] {
        self.inner.mac
    }
    fn name(&self) -> &'static str {
        "Realtek RTL8168/8111"
    }
    fn counters(&self) -> (u64, u64) {
        (self.inner.sent, self.inner.received)
    }
    fn link_info(&mut self) -> Option<alloc::string::String> {
        let d = self.inner.diag();
        let r = &self.inner;
        let first: Vec<alloc::string::String> = r.first[..(r.received as usize).min(r.first.len())]
            .iter()
            .map(|&(et, port)| if port != 0 { alloc::format!("{:04x}/udp{}", et, port) } else { alloc::format!("{:04x}", et) })
            .collect();
        Some(alloc::format!(
            "legatura: {}; primire: aruncate {}, pierdute de placa {}, ISR vazut {:04x}, descriptor RX {} = {:08x}, citiri {}\n\
             inel RX {}: index {}, ai placii {}; RDU vazut {} ori, reporniri {}, resincronizari {}; raspunsuri DHCP (udp 68) {}\n\
             primele cadre: {}\n\
             cadre cu tip 0000: {}; reparate de golirea cache-ului: {}; buffere peste 4 GiB: {}; No Snoop: {}",
            link_text(d.phy), r.dropped, d.mpc, r.isr_seen, d.rx_next, d.rx_opts1, r.polls,
            cyber_rtl8168::RX_RING, d.rx_next, d.rx_owned, r.rdu_count, r.restarts, r.resyncs, r.dhcp_replies,
            if first.is_empty() { alloc::string::String::from("-") } else { first.join(" ") },
            r.zero_type, r.fixed_by_sync, r.high_bufs, devctl_text(self.devctl)
        ))
    }
}

impl Card {
    /// One log line per new detailed receive record: index, length, the address in the descriptor and the one the
    /// driver reads, the first 16 bytes before and after the cache sync (test 5 tells the hypotheses apart).
    fn log_new_frames(&mut self) {
        let have = (self.inner.received as usize).min(self.inner.rx_log.len());
        while self.logged < have {
            let r = self.inner.rx_log[self.logged];
            crate::info!(
                "net: rx cadru {}: index {}, lungime {}, adresa din descriptor {:x}, buffer {:x} (citit de la {:x}){}, inainte {}, dupa {}",
                self.logged + 1, r.index, r.len, r.desc_addr, r.buf_phys, r.buf_virt,
                if r.desc_addr == r.buf_phys { "" } else { " ADRESE DIFERITE" },
                hex(&r.before), hex(&r.after)
            );
            self.logged += 1;
        }
    }
}

fn hex(b: &[u8]) -> alloc::string::String {
    b.iter().map(|x| alloc::format!("{:02x}", x)).collect()
}

/// "oprit (Device Control 2810 -> 2010)", "PORNIT (...)" if the bit did not clear, "-" without PCIe.
fn devctl_text(d: Option<(u16, u16)>) -> alloc::string::String {
    match d {
        Some((old, new)) => alloc::format!(
            "{} (Device Control {:04x} -> {:04x})",
            if new & DEVCTL_NO_SNOOP == 0 { "oprit" } else { "PORNIT" }, old, new
        ),
        None => alloc::string::String::from("- (fara PCI Express)"),
    }
}

/// PCI Express Device Control: Enable No Snoop (bit 11) and Enable Relaxed Ordering (bit 4).
const DEVCTL_NO_SNOOP: u16 = 1 << 11;
const DEVCTL_RELAXED: u16 = 1 << 4;

/// One log line with what the card holds after start-up (`log rtl` shows it on a laptop without a serial port).
fn log_registers(inner: &mut Rtl8168<KernelHw>) {
    let d = inner.diag();
    let (rx, tx) = inner.ring_addresses();
    crate::info!(
        "net: rtl8168 registre: CR {:02x} RCR {:08x} TCR {:08x} IMR {:04x} ISR {:04x} RMS {} RDSAR {:x}{} TNPDS {:x}{} PHY {:02x}",
        d.chipcmd, d.rcr, d.tcr, d.imr, d.isr, d.rms,
        d.rdsar, if d.rdsar == rx { "" } else { " (GRESIT)" },
        d.tnpds, if d.tnpds == tx { "" } else { " (GRESIT)" },
        d.phy
    );
}

/// The PHYstatus byte in words: "da, 1000 Mbit/s, full duplex" or "nu".
fn link_text(phy: u8) -> alloc::string::String {
    if phy & 0b10 == 0 {
        return alloc::string::String::from("nu");
    }
    let speed = if phy & 1 << 4 != 0 { "1000" } else if phy & 1 << 3 != 0 { "100" } else if phy & 1 << 2 != 0 { "10" } else { "?" };
    alloc::format!("da, {} Mbit/s, {}", speed, if phy & 1 != 0 { "full duplex" } else { "half duplex" })
}

/// Finds a Realtek RTL8168 and starts it.
pub fn init() -> Option<Card> {
    let dev = crate::pci::devices().into_iter().find(|d| supports(d.vendor, d.device))?;
    dev.power_on();
    dev.enable();
    // With No Snoop allowed (the PCIe default), the card may write received frames to memory past the CPU caches,
    // and the CPU then reads old zeros (test 4: frames 2+ read as ethertype 0000). The driver also syncs the caches
    // itself (`dma_sync`), so this only has to help, not be honoured.
    let devctl = dev.pcie_devctl_clear(DEVCTL_NO_SNOOP | DEVCTL_RELAXED);
    crate::info!("net: rtl8168 No Snoop: {}", devctl_text(devctl));
    let Some(base) = dev.first_memory_bar() else {
        crate::warn!("net: Realtek {:04x}: niciun BAR de memorie", dev.device);
        return None;
    };
    let regs = paging::map_mmio(base, 4096).ok()?;
    match Rtl8168::new(KernelHw { regs }) {
        Ok(mut inner) => {
            let m = inner.mac;
            // Power the PHY up and wait for auto-negotiation (2-3 s with a cable), so DHCP's first requests are not
            // sent into a link that is still down.
            let up = inner.start_link(4000);
            let (phy, _) = inner.status();
            crate::info!(
                "net: placa Realtek RTL8168 pornita (revizia {:08x}), MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, legatura: {}",
                inner.revision, m[0], m[1], m[2], m[3], m[4], m[5], link_text(phy)
            );
            log_registers(&mut inner);
            if !up {
                crate::warn!("net: Realtek: fara legatura dupa 4 s (cablu? PHYstatus {:02x}); DHCP reincearca singur", phy);
            }
            Some(Card { inner, logged: 0, devctl })
        }
        Err(e) => {
            let why = match e {
                InitError::NotAnswering => "nu raspunde (registrele citesc FFFFFFFF)",
                InitError::ResetTimeout => "resetarea nu s-a terminat",
                InitError::NoMemory => "nu e memorie pentru inele",
            };
            crate::warn!("net: placa Realtek RTL8168: {}", why);
            None
        }
    }
}
