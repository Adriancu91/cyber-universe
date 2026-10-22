//! Realtek RTL8168/8111 gigabit Ethernet (PCI `10ec:8168`), e.g. the wired card of the Lenovo AMD test laptop.
//! The driver logic lives in the `rtl8168/` crate (tested on the host against a model of the chip); this file
//! only gives it the hardware: the memory-mapped registers, DMA pages from the frame allocator, a delay.

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;
use cyber_rtl8168::{Hw, InitError, Options, Rtl8168};

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
    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)> {
        let f = frame::alloc_contiguous_below(pages as u64, 1 << 32)?;
        let p = phys_to_virt(f);
        // SAFETY: `pages` fresh contiguous frames, reachable through the HHDM (one linear range); kept by the driver.
        unsafe { core::ptr::write_bytes(p, 0, pages * PAGE_SIZE as usize) };
        Some((f, p))
    }
    fn dma_sync(&mut self, _phys: u64, ptr: *mut u8, len: usize) {
        const LINE: usize = 64; // cache line size of every x86-64 CPU we target (CPUID could say; all use 64)
        let start = ptr as usize & !(LINE - 1);
        let end = ptr as usize + len;
        // SAFETY: CLFLUSH writes a cached line back if dirty and drops it; the lines belong to a DMA buffer owned by
        // this driver and mapped through the HHDM. MFENCE orders it against the reads and writes around it.
        unsafe {
            core::arch::asm!("mfence", options(nostack, preserves_flags));
            for a in (start..end).step_by(LINE) {
                core::arch::asm!("clflush [{}]", in(reg) a, options(nostack, preserves_flags));
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
    dev: crate::pci::Device,
    /// How many of the first frames were already written to the log (`log rx`).
    logged: usize,
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
        let have = (self.inner.received as usize).min(cyber_rtl8168::FIRST_FRAMES);
        while self.logged < have {
            log_frame(self.logged, &self.inner.detail[self.logged]);
            self.logged += 1;
        }
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
             cadre cu tip 0000: {}; reparate de golirea cache-ului: {}; Bus Master: {}; MISC {:08x} (poarta RXDV {}); varianta: {} (detalii: log rx, log rtl)",
            link_text(d.phy), r.dropped, d.mpc, r.isr_seen, d.rx_next, d.rx_opts1, r.polls,
            cyber_rtl8168::RX_RING, d.rx_next, d.rx_owned, r.rdu_count, r.restarts, r.resyncs, r.dhcp_replies,
            if first.is_empty() { alloc::string::String::from("-") } else { first.join(" ") },
            r.zero_type, r.fixed_by_sync, if self.dev.command() & 1 << 2 != 0 { "da" } else { "NU" },
            d.misc, if d.misc & cyber_rtl8168::MISC_RXDV_GATE != 0 { "INCHISA" } else { "deschisa" },
            variant_text(r.options)
        ))
    }
}

/// `log rx`: one received frame: the full 64-bit address in the descriptor (high_low), the virtual address the driver
/// copies from and the physical address the page tables give for it, and the first 16 bytes.
fn log_frame(k: usize, d: &cyber_rtl8168::RxDetail) {
    let hex = |b: &[u8; 16]| b.iter().map(|x| alloc::format!("{:02x}", x)).collect::<Vec<_>>().join("");
    let walked = paging::translate(d.read_ptr);
    let same = walked == Some(d.desc_addr) && d.read_addr == d.desc_addr;
    crate::info!(
        "net: rx cadru {}: index {}, lungime {}, descriptor {:08x}_{:08x}, citit din virtual {:x} = fizic {}{}, octeti {}{}",
        k + 1, d.index, d.len, d.desc_addr >> 32, d.desc_addr as u32, d.read_ptr,
        walked.map_or(alloc::string::String::from("?"), |a| alloc::format!("{:x}", a)),
        if same { "" } else { " (DIFERITE)" },
        hex(&d.before),
        if d.after == d.before { alloc::string::String::new() } else { alloc::format!(", dupa golire {}", hex(&d.after)) }
    );
}

fn below_4g(a: u64) -> &'static str {
    if a < 1 << 32 { "sub 4 GB" } else { "PESTE 4 GB" }
}

fn variant_text(o: Options) -> alloc::string::String {
    let mut parts = Vec::new();
    if o.rx_block {
        parts.push("bloc");
    }
    if o.rxdv {
        parts.push("rxdv");
    }
    if o.cache {
        parts.push("cache");
    }
    if parts.is_empty() { alloc::string::String::from("implicita (ca os-125)") } else { parts.join(" ") }
}

/// The lines `log rtl` shows after start-up: the card's registers (read back), the PCI side and where the memory is,
/// so one photo can be compared with the previous version's.
fn log_registers(inner: &mut Rtl8168<KernelHw>, dev: &crate::pci::Device) {
    let d = inner.diag();
    let (rx, tx) = inner.ring_addresses();
    crate::info!(
        "net: rtl8168 registre: CR {:02x} RCR {:08x} TCR {:08x} IMR {:04x} ISR {:04x} RMS {} MISC {:08x} MCU {:02x} PHY {:02x}",
        d.chipcmd, d.rcr, d.tcr, d.imr, d.isr, d.rms, d.misc, d.mcu, d.phy
    );
    crate::info!(
        "net: rtl8168 inele: RDSAR {:08x}_{:08x}{} TNPDS {:08x}_{:08x}{} ({}, {})",
        d.rdsar >> 32, d.rdsar as u32, if d.rdsar == rx { "" } else { " (GRESIT)" },
        d.tnpds >> 32, d.tnpds as u32, if d.tnpds == tx { "" } else { " (GRESIT)" },
        below_4g(rx), below_4g(tx)
    );
    let b: [u64; 3] = inner.rx_buffer_addresses();
    crate::info!(
        "net: rtl8168 buffere RX: 0 la {:x}, 1 la {:x}, 2 la {:x} ({}); RAM {} MiB, cea mai mare adresa {:x} ({})",
        b[0], b[1], b[2], below_4g(b[2].max(b[0]) + 2047),
        frame::usable_bytes() >> 20, frame::top_of_ram(),
        if frame::top_of_ram() > 1 << 32 { "are memorie peste 4 GB" } else { "toata sub 4 GB" }
    );
    let cmd = dev.command();
    crate::info!(
        "net: rtl8168 PCI {}: Command {:04x} (Bus Master {}, Memory {}), PCIe Device Control {} (doar citit); varianta {}{}",
        dev.address(), cmd, if cmd & 1 << 2 != 0 { "da" } else { "NU" }, if cmd & 1 << 1 != 0 { "da" } else { "NU" },
        dev.pcie_devctl().map_or(alloc::string::String::from("-"), |v| alloc::format!("{:04x}", v)),
        variant_text(inner.options),
        match inner.misc_change {
            Some((a, b)) if a != b => alloc::format!("; MISC {:08x} -> {:08x} (poarta RXDV deschisa)", a, b),
            Some((a, _)) => alloc::format!("; MISC {:08x} neschimbat (poarta RXDV era deschisa)", a),
            None => alloc::string::String::new(),
        }
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

/// Finds a Realtek RTL8168 and starts it the default way (exactly as Release os-125).
pub fn init() -> Option<Card> {
    init_with(Options::default())
}

/// Finds a Realtek RTL8168 and starts it with the given start-up choices (`rtl porneste ...` in the shell).
pub fn init_with(options: Options) -> Option<Card> {
    let dev = crate::pci::devices().into_iter().find(|d| supports(d.vendor, d.device))?;
    dev.power_on();
    dev.enable_as("rtl8168");
    let Some(base) = dev.first_memory_bar() else {
        crate::warn!("net: Realtek {:04x}: niciun BAR de memorie", dev.device);
        return None;
    };
    let regs = paging::map_mmio(base, 4096).ok()?;
    match Rtl8168::with_options(KernelHw { regs }, options) {
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
            log_registers(&mut inner, &dev);
            if !up {
                crate::warn!("net: Realtek: fara legatura dupa 4 s (cablu? PHYstatus {:02x}); DHCP reincearca singur", phy);
            }
            Some(Card { inner, dev, logged: 0 })
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

/// Parses the words after `rtl porneste`: `bloc`, `rxdv`, `cache` (any order). `Err(word)` for an unknown word.
pub fn parse_options<'a>(words: &[&'a str]) -> Result<Options, &'a str> {
    let mut o = Options::default();
    for &w in words {
        match w {
            "bloc" => o.rx_block = true,
            "rxdv" => o.rxdv = true,
            "cache" => o.cache = true,
            "implicit" => {}
            _ => return Err(w),
        }
    }
    Ok(o)
}
