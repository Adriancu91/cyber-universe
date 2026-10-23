//! Driver logic for Intel gigabit Ethernet cards: the 8254x (e1000), the 82574 (e1000e) and the PCH family
//! (I217, I218, I219 — a MAC inside the chipset with a separate PHY). Written from Intel's public documents: the
//! PCI/PCI-X Family of Gigabit Ethernet Controllers Software Developer's Manual (8254x), the 82574 datasheet and
//! the Ethernet Connection I217/I218/I219 datasheets. Ideas only from other drivers (docs/CLEAN_ROOM.md): no code copied.
//!
//! The hardware is reached only through [`Hw`] (registers + DMA memory), so the same code runs in the kernel and in
//! host tests against [`model::Model`]. Polling only, legacy descriptors.
#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std;
#[cfg(test)]
pub mod model;
#[cfg(test)]
mod tests;

/// Register offsets (memory-mapped register block, BAR 0).
pub mod reg {
    pub const CTRL: u32 = 0x0000;
    pub const STATUS: u32 = 0x0008;
    pub const CTRL_EXT: u32 = 0x0018;
    pub const MDIC: u32 = 0x0020;
    pub const FEXTNVM4: u32 = 0x0024; // PCH: NVM-loaded settings (read only for diagnostics)
    pub const FEXTNVM: u32 = 0x0028; // PCH: as above
    pub const IMC: u32 = 0x00D8;
    pub const RCTL: u32 = 0x0100;
    pub const TCTL: u32 = 0x0400;
    pub const TIPG: u32 = 0x0410;
    pub const EXTCNF_CTRL: u32 = 0x0F00; // PCH: bit 5 = software owns the PHY / NVM (semaphore)
    pub const PHY_CTRL: u32 = 0x0F10; // PCH: PHY power settings (read only for diagnostics)
    pub const RDBAL: u32 = 0x2800;
    pub const RDBAH: u32 = 0x2804;
    pub const RDLEN: u32 = 0x2808;
    pub const RDH: u32 = 0x2810;
    pub const RDT: u32 = 0x2818;
    pub const TDBAL: u32 = 0x3800;
    pub const TDBAH: u32 = 0x3804;
    pub const TDLEN: u32 = 0x3808;
    pub const TDH: u32 = 0x3810;
    pub const TDT: u32 = 0x3818;
    pub const MTA: u32 = 0x5200;
    pub const RAL0: u32 = 0x5400;
    pub const RAH0: u32 = 0x5404;
    pub const FWSM: u32 = 0x5B54; // PCH: firmware (Management Engine) status; bit 15 = firmware present
}

pub const CTRL_SLU: u32 = 1 << 6; // set link up
pub const CTRL_ASDE: u32 = 1 << 5; // auto speed detection (8254x)
pub const CTRL_LANPHYPC_OVERRIDE: u32 = 1 << 16; // PCH: software drives the PHY power control pin
pub const CTRL_LANPHYPC_VALUE: u32 = 1 << 17;
pub const CTRL_RST: u32 = 1 << 26; // MAC reset, clears by itself
pub const STATUS_FD: u32 = 1 << 0;
pub const STATUS_LU: u32 = 1 << 1;
pub const STATUS_LAN_INIT_DONE: u32 = 1 << 9; // PCH: the MAC finished loading its NVM settings after reset
pub const CTRL_EXT_LPCD: u32 = 1 << 2; // PCH: the PHY power cycle (LANPHYPC) is done
pub const EXTCNF_SWFLAG: u32 = 1 << 5;
pub const FWSM_FW_VALID: u32 = 1 << 15;
pub const MDIC_READY: u32 = 1 << 28;
pub const MDIC_ERROR: u32 = 1 << 30;
pub const RAH_AV: u32 = 1 << 31; // receive address valid
pub const RCTL_EN: u32 = 1 << 1;
pub const TCTL_EN: u32 = 1 << 1;
/// PHY registers (IEEE 802.3 clause 22): 1 = status, 2/3 = identifier.
pub const PHY_BMSR: u8 = 1;
pub const PHY_ID1: u8 = 2;
pub const PHY_ID2: u8 = 3;

/// Descriptors per ring (16 bytes each) and buffer size.
pub const RING: usize = 32;
pub const BUF: usize = 2048;
/// Pages of the DMA block: 1 RX ring + 1 TX ring + 2 × 16 pages of buffers (two 2 KiB buffers per page).
pub const BLOCK_PAGES: usize = 2 + RING;
/// How many start-up steps are kept for `log e1000`.
pub const MAX_STEPS: usize = 24;

/// Which generation of card: decides the PHY address and the extra PCH steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// 8254x (e1000): PHY at address 1, no semaphore.
    E1000,
    /// 82574 (e1000e): PHY at address 1, no semaphore.
    E82574,
    /// I217/I218/I219: MAC in the chipset (PCH), PHY at address 2 behind the software semaphore,
    /// PHY power controlled by the LANPHYPC pin.
    Pch,
}

impl Family {
    pub fn phy_address(self) -> u8 {
        match self {
            Family::Pch => 2,
            _ => 1,
        }
    }
}

/// The cards this driver knows: (PCI device id, family, name). Vendor is always Intel (0x8086).
pub const CARDS: &[(u16, Family, &str)] = &[
    (0x100E, Family::E1000, "82540EM (e1000)"),
    (0x100F, Family::E1000, "82545EM (e1000)"),
    (0x10D3, Family::E82574, "82574L (e1000e)"),
    (0x153A, Family::Pch, "I217-LM"),
    (0x153B, Family::Pch, "I217-V"),
    (0x155A, Family::Pch, "I218-LM"),
    (0x1559, Family::Pch, "I218-V"),
    (0x15A0, Family::Pch, "I218-LM"),
    (0x15A1, Family::Pch, "I218-V"),
    (0x15A2, Family::Pch, "I218-LM"),
    (0x15A3, Family::Pch, "I218-V"),
    (0x15B7, Family::Pch, "I219-LM"),
    (0x15B8, Family::Pch, "I219-V"),
    (0x15BB, Family::Pch, "I219-LM"),
    (0x15BC, Family::Pch, "I219-V"),
    (0x15BD, Family::Pch, "I219-LM"),
    (0x15BE, Family::Pch, "I219-V"),
    (0x15D6, Family::Pch, "I219-V"),
    (0x15D7, Family::Pch, "I219-LM"),
    (0x15D8, Family::Pch, "I219-V"),
    (0x15DF, Family::Pch, "I219-LM"),
    (0x15E0, Family::Pch, "I219-V"),
    (0x15E1, Family::Pch, "I219-LM"),
    (0x15E2, Family::Pch, "I219-V"),
    (0x15E3, Family::Pch, "I219-LM"),
    (0x0D4C, Family::Pch, "I219-LM"),
    (0x0D4D, Family::Pch, "I219-V"),
    (0x0D4E, Family::Pch, "I219-LM"),
    (0x0D4F, Family::Pch, "I219-V"),
    (0x0D53, Family::Pch, "I219-LM"),
    (0x0D55, Family::Pch, "I219-V"),
];

/// The family and name of an Intel card this driver knows.
pub fn identify(vendor: u16, device: u16) -> Option<(Family, &'static str)> {
    if vendor != 0x8086 {
        return None;
    }
    CARDS.iter().find(|(id, _, _)| *id == device).map(|(_, f, n)| (*f, *n))
}

/// What the driver needs from the machine.
pub trait Hw {
    fn read32(&mut self, reg: u32) -> u32;
    fn write32(&mut self, reg: u32, v: u32);
    /// `pages` zeroed, physically contiguous 4 KiB pages below 4 GiB, kept for the card only:
    /// (physical address for the card, pointer for the CPU).
    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)>;
    /// Busy-waits about `us` microseconds.
    fn delay_us(&mut self, us: u32);
    /// Makes the CPU see what the card wrote by DMA to `len` bytes at `ptr` (the kernel: CLFLUSH).
    fn dma_sync(&mut self, ptr: *mut u8, len: usize);
}

/// Start-up variants (one behaviour change per Release, D-013): the default is the safest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// PCH: when the PHY does not answer, power-cycle it through the LANPHYPC pin (the usual way out of the
    /// PHY's low-power mode).
    pub lanphypc: bool,
    /// Stop receive and transmit before the reset (in case the firmware left the rings running).
    pub stop_before_reset: bool,
    /// Drop the CPU's cached copy of every received buffer before reading it.
    pub dma_sync: bool,
}

impl Options {
    pub const DEFAULT: Options = Options { lanphypc: true, stop_before_reset: true, dma_sync: true };
}

/// One start-up step, for `log e1000`: a register before and after (or a PHY register read).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Step {
    pub what: &'static str,
    pub reg: u32,
    pub before: u32,
    pub after: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum InitError {
    /// Every register reads as all ones: the card is switched off or not answering.
    NotAnswering,
    /// PCH: the semaphore for the PHY was never granted (firmware holds it).
    SemaphoreBusy,
    /// The reset bit did not clear.
    ResetStuck,
    /// No valid MAC address after reset.
    NoMac,
    /// No memory for the rings.
    NoMemory,
}

/// A started card.
pub struct E1000e<H: Hw> {
    pub hw: H,
    pub family: Family,
    pub mac: [u8; 6],
    pub options: Options,
    block_phys: u64,
    block: *mut u8,
    rx_next: usize,
    tx_next: usize,
    pub sent: u64,
    pub received: u64,
    pub steps: [Step; MAX_STEPS],
    pub step_count: usize,
    /// PHY identifier (ID1 << 16 | ID2) read at start-up; 0 if the PHY did not answer.
    pub phy_id: u32,
}

// SAFETY: the DMA block belongs to this driver only; the kernel keeps the card behind a lock.
unsafe impl<H: Hw + Send> Send for E1000e<H> {}

/// Records a step (silently drops it when the list is full).
fn note(steps: &mut [Step; MAX_STEPS], count: &mut usize, s: Step) {
    if *count < MAX_STEPS {
        steps[*count] = s;
        *count += 1;
    }
}

/// Polls `reg` until `(value & mask) == want`, at most `tries` times `every_us` apart. Returns the last value and success.
fn poll<H: Hw>(hw: &mut H, reg: u32, mask: u32, want: u32, tries: u32, every_us: u32) -> (u32, bool) {
    let mut v = hw.read32(reg);
    for _ in 0..tries {
        if v & mask == want {
            return (v, true);
        }
        hw.delay_us(every_us);
        v = hw.read32(reg);
    }
    (v, v & mask == want)
}

/// Reads PHY register `r` at address `phy` through MDIC. `None` on timeout or error.
pub fn phy_read<H: Hw>(hw: &mut H, phy: u8, r: u8) -> Option<u16> {
    let cmd = ((r as u32 & 0x1F) << 16) | ((phy as u32 & 0x1F) << 21) | (0b10 << 26);
    hw.write32(reg::MDIC, cmd);
    let (v, ok) = poll(hw, reg::MDIC, MDIC_READY, MDIC_READY, 1000, 50);
    if !ok || v & MDIC_ERROR != 0 {
        return None;
    }
    Some(v as u16)
}

/// Takes the PCH software semaphore (EXTCNF_CTRL bit 5): set it and wait until it reads back as set.
fn take_semaphore<H: Hw>(hw: &mut H) -> bool {
    let v = hw.read32(reg::EXTCNF_CTRL);
    hw.write32(reg::EXTCNF_CTRL, v | EXTCNF_SWFLAG);
    poll(hw, reg::EXTCNF_CTRL, EXTCNF_SWFLAG, EXTCNF_SWFLAG, 1000, 1000).1
}

fn give_semaphore<H: Hw>(hw: &mut H) {
    let v = hw.read32(reg::EXTCNF_CTRL);
    hw.write32(reg::EXTCNF_CTRL, v & !EXTCNF_SWFLAG);
}

/// PHY identifier (ID1 << 16 | ID2), through the semaphore on PCH; 0 if the PHY does not answer.
fn read_phy_id<H: Hw>(hw: &mut H, family: Family) -> u32 {
    let pch = family == Family::Pch;
    if pch && !take_semaphore(hw) {
        return 0;
    }
    let phy = family.phy_address();
    let id1 = phy_read(hw, phy, PHY_ID1);
    let id2 = phy_read(hw, phy, PHY_ID2);
    if pch {
        give_semaphore(hw);
    }
    match (id1, id2) {
        (Some(a), Some(b)) if phy_answers(Some(a)) => (a as u32) << 16 | b as u32,
        _ => 0,
    }
}

/// True if a PHY identifier looks real (not 0 and not all ones).
fn phy_answers(id1: Option<u16>) -> bool {
    matches!(id1, Some(v) if v != 0 && v != 0xFFFF)
}

impl<H: Hw> E1000e<H> {
    /// Resets and starts the card. The order (PCH): semaphore → PHY answers? (else LANPHYPC power cycle) →
    /// stop RX/TX → MAC reset → wait for the NVM load → MAC address → rings → enable.
    pub fn init(mut hw: H, family: Family, options: Options) -> Result<Self, InitError> {
        let mut steps = [Step::default(); MAX_STEPS];
        let mut n = 0;
        if hw.read32(reg::STATUS) == u32::MAX && hw.read32(reg::CTRL) == u32::MAX {
            return Err(InitError::NotAnswering);
        }
        hw.write32(reg::IMC, u32::MAX); // no interrupts (polling)
        let phy = family.phy_address();

        if family == Family::Pch {
            let fwsm = hw.read32(reg::FWSM);
            note(&mut steps, &mut n, Step { what: "firmware ME (FWSM)", reg: reg::FWSM, before: fwsm, after: fwsm });
            let before = hw.read32(reg::EXTCNF_CTRL);
            let ok = take_semaphore(&mut hw);
            let after = hw.read32(reg::EXTCNF_CTRL);
            note(&mut steps, &mut n, Step { what: "semafor PHY luat", reg: reg::EXTCNF_CTRL, before, after });
            if !ok {
                return Err(InitError::SemaphoreBusy);
            }
            let id1 = phy_read(&mut hw, phy, PHY_ID1);
            note(&mut steps, &mut n, Step { what: "PHY raspunde? (ID1)", reg: PHY_ID1 as u32, before: 0, after: id1.map_or(u32::MAX, |v| v as u32) });
            if !phy_answers(id1) && options.lanphypc {
                // Power-cycle the PHY: drive LANPHYPC low for a moment, then give the pin back to the hardware.
                let before = hw.read32(reg::CTRL);
                hw.write32(reg::CTRL, (before | CTRL_LANPHYPC_OVERRIDE) & !CTRL_LANPHYPC_VALUE);
                hw.delay_us(1000);
                hw.write32(reg::CTRL, before & !(CTRL_LANPHYPC_OVERRIDE | CTRL_LANPHYPC_VALUE));
                let (ext, _) = poll(&mut hw, reg::CTRL_EXT, CTRL_EXT_LPCD, CTRL_EXT_LPCD, 20, 1000);
                hw.delay_us(30_000);
                let after = hw.read32(reg::CTRL);
                note(&mut steps, &mut n, Step { what: "LANPHYPC: ciclu de alimentare PHY", reg: reg::CTRL, before, after });
                note(&mut steps, &mut n, Step { what: "LANPHYPC gata? (CTRL_EXT)", reg: reg::CTRL_EXT, before: 0, after: ext });
                let id1 = phy_read(&mut hw, phy, PHY_ID1);
                note(&mut steps, &mut n, Step { what: "PHY raspunde dupa LANPHYPC?", reg: PHY_ID1 as u32, before: 0, after: id1.map_or(u32::MAX, |v| v as u32) });
            }
        }

        if options.stop_before_reset {
            let (r, t) = (hw.read32(reg::RCTL), hw.read32(reg::TCTL));
            if r & RCTL_EN != 0 || t & TCTL_EN != 0 {
                hw.write32(reg::RCTL, r & !RCTL_EN);
                hw.write32(reg::TCTL, t & !TCTL_EN);
                hw.delay_us(10_000);
                note(&mut steps, &mut n, Step { what: "oprit RX/TX lasate pornite de firmware", reg: reg::RCTL, before: r, after: hw.read32(reg::RCTL) });
            }
        }

        let before = hw.read32(reg::CTRL);
        hw.write32(reg::CTRL, before | CTRL_RST);
        hw.delay_us(20_000);
        let (after, ok) = poll(&mut hw, reg::CTRL, CTRL_RST, 0, 1000, 100);
        note(&mut steps, &mut n, Step { what: "reset MAC (CTRL.RST)", reg: reg::CTRL, before, after });
        if !ok {
            if family == Family::Pch {
                give_semaphore(&mut hw);
            }
            return Err(InitError::ResetStuck);
        }
        hw.write32(reg::IMC, u32::MAX);
        if family == Family::Pch {
            let (st, done) = poll(&mut hw, reg::STATUS, STATUS_LAN_INIT_DONE, STATUS_LAN_INIT_DONE, 150, 1000);
            note(&mut steps, &mut n, Step { what: if done { "NVM incarcat (LAN_INIT_DONE)" } else { "NVM: LAN_INIT_DONE nu a venit" }, reg: reg::STATUS, before: 0, after: st });
            if done {
                hw.write32(reg::STATUS, st & !STATUS_LAN_INIT_DONE); // acknowledged, so the next reset can be seen
            }
            give_semaphore(&mut hw); // the reset usually clears it already
        }

        let before = hw.read32(reg::CTRL);
        let ctrl = if family == Family::E1000 { before | CTRL_SLU | CTRL_ASDE } else { before | CTRL_SLU };
        hw.write32(reg::CTRL, ctrl);
        note(&mut steps, &mut n, Step { what: "legatura pornita (CTRL.SLU)", reg: reg::CTRL, before, after: hw.read32(reg::CTRL) });

        let phy_id = read_phy_id(&mut hw, family);

        let (ral, rah) = (hw.read32(reg::RAL0), hw.read32(reg::RAH0));
        if rah & RAH_AV == 0 {
            return Err(InitError::NoMac);
        }
        let mac = [ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8, rah as u8, (rah >> 8) as u8];
        for i in 0..128 {
            hw.write32(reg::MTA + i * 4, 0);
        }

        let (block_phys, block) = hw.alloc_block(BLOCK_PAGES).ok_or(InitError::NoMemory)?;
        let mut card = E1000e {
            hw, family, mac, options, block_phys, block, rx_next: 0, tx_next: 0, sent: 0, received: 0,
            steps, step_count: n, phy_id,
        };
        for i in 0..RING {
            let b = card.rx_buf_phys(i);
            // SAFETY: descriptor i of the RX ring inside this driver's block; word 0 = buffer address.
            unsafe { card.desc(0, i).write_volatile(b) };
        }
        let ring_bytes = (RING * 16) as u32;
        let (rx_ring, tx_ring) = (block_phys, block_phys + 4096);
        let h = &mut card.hw;
        h.write32(reg::RDBAL, rx_ring as u32);
        h.write32(reg::RDBAH, (rx_ring >> 32) as u32);
        h.write32(reg::RDLEN, ring_bytes);
        h.write32(reg::RDH, 0);
        h.write32(reg::RDT, (RING - 1) as u32);
        h.write32(reg::RCTL, RCTL_EN | 1 << 15 | 1 << 26); // enable, accept broadcast, strip CRC; 2 KiB buffers
        h.write32(reg::TDBAL, tx_ring as u32);
        h.write32(reg::TDBAH, (tx_ring >> 32) as u32);
        h.write32(reg::TDLEN, ring_bytes);
        h.write32(reg::TDH, 0);
        h.write32(reg::TDT, 0);
        h.write32(reg::TCTL, TCTL_EN | 1 << 3 | 0x0F << 4 | 0x40 << 12); // enable, pad short packets, collision settings
        h.write32(reg::TIPG, 0x0060_200A);
        Ok(card)
    }

    /// Physical address of the DMA block (rings, then buffers).
    pub fn block_phys(&self) -> u64 {
        self.block_phys
    }

    fn rx_buf_phys(&self, i: usize) -> u64 {
        self.block_phys + 2 * 4096 + (i * BUF) as u64
    }
    fn tx_buf_phys(&self, i: usize) -> u64 {
        self.block_phys + (2 + RING / 2) as u64 * 4096 + (i * BUF) as u64
    }
    fn ptr(&self, phys: u64) -> *mut u8 {
        // SAFETY (for users): `phys` lies inside the block; the block is CPU-visible at `self.block`.
        unsafe { self.block.add((phys - self.block_phys) as usize) }
    }
    /// Descriptor `i` of ring `ring` (0 = RX, 1 = TX), as two 64-bit words.
    fn desc(&self, ring: usize, i: usize) -> *mut u64 {
        // SAFETY (for users): ring pages are the first two pages of the block; i < RING.
        unsafe { (self.block.add(ring * 4096) as *mut u64).add(i * 2) }
    }

    /// Sends one Ethernet frame (at most 1514 bytes). True once the card reports it sent.
    pub fn send(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1514 {
            return false;
        }
        let i = self.tx_next;
        let len = packet.len().max(60);
        let buf = self.ptr(self.tx_buf_phys(i));
        // SAFETY: the buffer and descriptor belong to this driver; slot i is reused only after its "done" bit.
        unsafe {
            core::ptr::write_bytes(buf, 0, 60);
            core::ptr::copy_nonoverlapping(packet.as_ptr(), buf, packet.len());
            let d = self.desc(1, i);
            d.write_volatile(self.tx_buf_phys(i));
            d.add(1).write_volatile(len as u64 | (0b1011u64 << 24)); // EOP, IFCS, RS
        }
        self.tx_next = (i + 1) % RING;
        self.hw.write32(reg::TDT, self.tx_next as u32);
        for _ in 0..100_000 {
            // SAFETY: as above; status byte at offset 12.
            if unsafe { (self.desc(1, i).add(1).read_volatile() >> 32) & 1 } != 0 {
                self.sent += 1;
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    /// The next received frame, if one has arrived (copied into `out`; returns its length).
    pub fn receive_into(&mut self, out: &mut [u8; BUF]) -> Option<usize> {
        let i = self.rx_next;
        // SAFETY: descriptor of this driver's RX ring; word 1 = length (bits 0-15), status (bits 32-39).
        let w = unsafe { self.desc(0, i).add(1).read_volatile() };
        if (w >> 32) & 1 == 0 {
            return None;
        }
        let len = ((w & 0xFFFF) as usize).min(BUF);
        let buf = self.ptr(self.rx_buf_phys(i));
        if self.options.dma_sync {
            self.hw.dma_sync(buf, len);
        }
        // SAFETY: the card wrote `len` <= BUF bytes into this buffer.
        unsafe { core::ptr::copy_nonoverlapping(buf, out.as_mut_ptr(), len) };
        // SAFETY: give the descriptor back: clear its status, keep the buffer address.
        unsafe { self.desc(0, i).add(1).write_volatile(0) };
        self.hw.write32(reg::RDT, i as u32);
        self.rx_next = (i + 1) % RING;
        self.received += 1;
        Some(len)
    }

    /// Link state from STATUS: (link up, speed in Mb/s, full duplex).
    pub fn link(&mut self) -> (bool, u32, bool) {
        let s = self.hw.read32(reg::STATUS);
        let speed = match (s >> 6) & 3 {
            0 => 10,
            1 => 100,
            _ => 1000,
        };
        (s & STATUS_LU != 0, speed, s & STATUS_FD != 0)
    }

    /// The key registers for `log e1000` (name, value), read now.
    pub fn registers(&mut self) -> [(&'static str, u32); 16] {
        let names: [(&'static str, u32); 16] = [
            ("CTRL", reg::CTRL),
            ("STATUS", reg::STATUS),
            ("CTRL_EXT", reg::CTRL_EXT),
            ("EXTCNF_CTRL", reg::EXTCNF_CTRL),
            ("PHY_CTRL", reg::PHY_CTRL),
            ("FWSM", reg::FWSM),
            ("FEXTNVM", reg::FEXTNVM),
            ("FEXTNVM4", reg::FEXTNVM4),
            ("RCTL", reg::RCTL),
            ("TCTL", reg::TCTL),
            ("RDBAL", reg::RDBAL),
            ("RDBAH", reg::RDBAH),
            ("RDH", reg::RDH),
            ("RDT", reg::RDT),
            ("TDH", reg::TDH),
            ("TDT", reg::TDT),
        ];
        let mut out = names;
        for (i, (_, r)) in names.iter().enumerate() {
            out[i].1 = self.hw.read32(*r);
        }
        out
    }

    /// PHY status register (BMSR) now, through the semaphore on PCH; `None` if the PHY does not answer.
    pub fn phy_status(&mut self) -> Option<u16> {
        let pch = self.family == Family::Pch;
        if pch && !take_semaphore(&mut self.hw) {
            return None;
        }
        let v = phy_read(&mut self.hw, self.family.phy_address(), PHY_BMSR);
        if pch {
            give_semaphore(&mut self.hw);
        }
        v
    }
}
