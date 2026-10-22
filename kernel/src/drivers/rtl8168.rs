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
}

// SAFETY: the raw pointers inside point to DMA pages owned only by this driver; the card is used under `nic::NIC`'s lock.
unsafe impl Send for Card {}

impl super::nic::Card for Card {
    fn send(&mut self, frame: &[u8]) -> bool {
        self.inner.send(frame)
    }
    fn receive(&mut self) -> Option<Vec<u8>> {
        let mut buf = [0u8; cyber_rtl8168::BUF];
        self.inner.receive(&mut buf).map(|n| buf[..n].to_vec())
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
             primele cadre: {}",
            link_text(d.phy), r.dropped, d.mpc, r.isr_seen, d.rx_next, d.rx_opts1, r.polls,
            cyber_rtl8168::RX_RING, d.rx_next, d.rx_owned, r.rdu_count, r.restarts, r.resyncs, r.dhcp_replies,
            if first.is_empty() { alloc::string::String::from("-") } else { first.join(" ") }
        ))
    }
}

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
            Some(Card { inner })
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
