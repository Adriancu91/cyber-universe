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

/// Lowest address of the driver's DMA block (`Options::one_block`, `bloc`).
const DMA_LOW: u64 = 16 << 20;

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
        frame::pin(f, 1, "rtl8168"); // the card writes here by DMA: never given to anyone else
        let p = phys_to_virt(f);
        // SAFETY: a fresh frame, reachable through the HHDM; the driver keeps it for as long as the card runs.
        unsafe { core::ptr::write_bytes(p, 0, PAGE_SIZE as usize) };
        Some((f, p))
    }
    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)> {
        // Between 16 MiB and 4 GiB: away from the low memory every boot-time allocation comes from (test 7: the first
        // start-up, with buffers at 3db000, read zeros; every restart, with buffers higher up, read correctly).
        let f = frame::alloc_contiguous_between(pages as u64, DMA_LOW, 1 << 32)?;
        frame::pin(f, pages as u64, "rtl8168");
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
             cadre cu tip 0000: {}; necitite de placa (model A5): {}; reparate de golirea cache-ului: {}; paza memoriei: {}; Bus Master: {}; MISC {:08x} (poarta RXDV {}); varianta: {} (detalii: log rx, log rtl)",
            link_text(d.phy), r.dropped, d.mpc, r.isr_seen, d.rx_next, d.rx_opts1, r.polls,
            cyber_rtl8168::RX_RING, d.rx_next, d.rx_owned, r.rdu_count, r.restarts, r.resyncs, r.dhcp_replies,
            if first.is_empty() { alloc::string::String::from("-") } else { first.join(" ") },
            r.zero_type, r.unwritten, r.fixed_by_sync, guard_text(), if self.dev.command() & 1 << 2 != 0 { "da" } else { "NU" },
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

/// The DMA guard in words: "ok" or what went wrong (pages of a driver the allocator thinks free, refused frees).
fn guard_text() -> alloc::string::String {
    let (pinned, free, refused, lost) = frame::check_pins();
    if free == 0 && refused == 0 && lost == 0 {
        alloc::format!("ok ({} pagini DMA pazite)", pinned)
    } else {
        alloc::format!("PROBLEMA: {} pagini DMA libere, {} eliberari refuzate, {} nepazite (vezi log paza)", free, refused, lost)
    }
}

/// "3db000 in 100000-9fffff utilizabila" for `log rtl`: the memory-map region a DMA address lies in.
fn region_text(a: u64) -> alloc::string::String {
    match crate::mem::map::region_of(a) {
        Some(r) => alloc::format!("{:x} in {:x}-{:x} {}", a, r.base, r.end() - 1, r.kind.name()),
        None => alloc::format!("{:x} in nicio regiune din harta memoriei", a),
    }
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
    parts.push(if o.one_block { "bloc-dma" } else { "pagini" });
    if o.pattern {
        parts.push("model");
    }
    for (on, name) in [(o.mcu, "mcu"), (o.phy, "phy"), (o.cplus, "cplus"), (o.order, "ordine")] {
        if on {
            parts.push(name);
        }
    }
    if !o.cache {
        parts.push("faracache");
    }
    let text = if parts.is_empty() { alloc::string::String::from("-") } else { parts.join(" ") };
    if o == boot_options() {
        alloc::format!("implicita ({})", text)
    } else if o == Options::default() {
        alloc::format!("ca os-125 ({})", text)
    } else {
        text
    }
}

/// The lines `log rtl` shows after start-up: the card's registers (read back), the PCI side and where the memory is,
/// so one photo can be compared with the previous version's.
fn log_registers(inner: &mut Rtl8168<KernelHw>, dev: &crate::pci::Device) {
    for (i, st) in inner.steps.as_slice().iter().enumerate() {
        crate::info!(
            "net: rtl8168 pas {}: {} [{:04x}] {:x} -> {:x}{}",
            i + 1, st.what, st.reg, st.before, st.after, if st.ok { "" } else { " (EXPIRAT / ESUAT)" }
        );
    }
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
    crate::info!(
        "net: rtl8168 C+ Command {:04x}, PMCH {:02x}, revizia {:08x} = {} (doar citite)",
        d.cplus, d.pmch, inner.revision, chip_name(inner.revision)
    );
    let b: [u64; 3] = inner.rx_buffer_addresses();
    crate::info!(
        "net: rtl8168 buffere RX: 0 la {:x}, 1 la {:x}, 2 la {:x} ({}); RAM {} MiB, cea mai mare adresa {:x} ({})",
        b[0], b[1], b[2], below_4g(b[2].max(b[0]) + 2047),
        frame::usable_bytes() >> 20, frame::top_of_ram(),
        if frame::top_of_ram() > 1 << 32 { "are memorie peste 4 GB" } else { "toata sub 4 GB" }
    );
    crate::info!(
        "net: rtl8168 memorie DMA: {}; inel RX {}; buffer RX 0 {}; paza: {}",
        match inner.block {
            Some((b, n)) => alloc::format!("un bloc {:x}-{:x} ({} pagini)", b, b + n as u64 * 4096 - 1, n),
            None => alloc::string::String::from("pagina cu pagina (ca os-149)"),
        },
        region_text(rx), region_text(b[0]), guard_text()
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

/// The start-up used at boot and as the base of `rtl porneste`: os-149 (os-125 + `cache`) plus, as the one change of
/// this Release (C-10, test 7), the rings and all buffers in one block between 16 MiB and 4 GiB (`one_block`) — on
/// the laptop every restart, with its memory elsewhere, read every frame; only the first start-up (low memory) read
/// zeros. `pattern` is a diagnostic: it changes nothing on the card. `pagini` goes back to os-149's allocation.
pub fn boot_options() -> Options {
    Options { cache: true, one_block: true, pattern: true, ..Options::default() }
}

/// Finds a Realtek RTL8168 and starts it the default way (`boot_options`).
pub fn init() -> Option<Card> {
    init_with(boot_options())
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

/// The chip family from the revision bits of TCR (FreeBSD `if_rlreg.h` lists the same numbers), for the log.
fn chip_name(rev: u32) -> &'static str {
    match rev {
        0x4C00_0000 => "8168G",
        0x5080_0000 => "8168GU/8111GU",
        0x5400_0000 => "8168H",
        0x5C80_0000 => "8411B",
        _ => "alta",
    }
}

/// Parses the words after `rtl porneste`, starting from `boot_options`: `bloc`, `rxdv`, `cache`, `faracache`, `pagini`, the
/// cold-start steps `mcu`, `phy`, `cplus`, `ordine`, and `rece` = all cold-start steps (any order). `Err(word)` for an
/// unknown word.
pub fn parse_options<'a>(words: &[&'a str]) -> Result<Options, &'a str> {
    let mut o = boot_options();
    for &w in words {
        match w {
            "bloc" => o.rx_block = true,
            "rxdv" => o.rxdv = true,
            "cache" => o.cache = true,
            "faracache" => o.cache = false,
            "pagini" => o.one_block = false,
            "mcu" => o.mcu = true,
            "phy" => o.phy = true,
            "cplus" => o.cplus = true,
            "ordine" => o.order = true,
            "rece" => {
                let c = Options::cold();
                (o.rxdv, o.mcu, o.phy, o.cplus, o.order) = (true, c.mcu, c.phy, c.cplus, c.order);
            }
            "implicit" => {}
            _ => return Err(w),
        }
    }
    Ok(o)
}
