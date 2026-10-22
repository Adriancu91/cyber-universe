//! Driver logic for Realtek RTL8168/8111 gigabit Ethernet cards (PCI `10ec:8168`), written from the public
//! Realtek RTL8169/RTL8111 datasheets and the OSDev Wiki description (no code copied: docs/CLEAN_ROOM.md).
//!
//! The hardware is reached only through the [`Hw`] trait (registers + DMA pages), so the same code runs in the
//! kernel (memory-mapped registers) and in host tests against [`model::Model`], a software model of the chip.
//! Polling only: no interrupts.
#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std;
#[cfg(test)]
pub mod model;

/// Register offsets (memory-mapped register block).
pub mod reg {
    pub const IDR0: u16 = 0x00; // MAC address, 6 bytes
    pub const MAR0: u16 = 0x08; // multicast filter, 8 bytes
    pub const TNPDS: u16 = 0x20; // transmit (normal priority) descriptor ring start, 64-bit
    pub const CHIPCMD: u16 = 0x37; // bit 4 reset, bit 3 receiver enable, bit 2 transmitter enable
    pub const TPPOLL: u16 = 0x38; // bit 6: normal-priority transmit queue has work
    pub const IMR: u16 = 0x3C; // interrupt mask (16-bit)
    pub const ISR: u16 = 0x3E; // interrupt status (16-bit, write 1 to clear)
    pub const TCR: u16 = 0x40; // transmit configuration; bits 23 and 26-30 also identify the chip revision
    pub const RCR: u16 = 0x44; // receive configuration
    pub const MPC: u16 = 0x4C; // missed packet counter (RTL8169 datasheet; read only for diagnostics)
    pub const CR9346: u16 = 0x50; // 0xC0 unlocks the configuration registers, 0x00 locks them
    pub const PHYAR: u16 = 0x60; // PHY access: bit 31 flag (write: 1 = write, clears when done), 20-16 register, 15-0 data
    pub const PHYSTATUS: u16 = 0x6C; // bit 0 full duplex, 1 link up, 2 10 Mb/s, 3 100 Mb/s, 4 1000 Mb/s
    pub const RMS: u16 = 0xDA; // receive maximum packet size (16-bit)
    pub const RDSAR: u16 = 0xE4; // receive descriptor ring start, 64-bit
    pub const MTPS: u16 = 0xEC; // maximum transmit packet size, in 128-byte units
    pub const MCU: u16 = 0xD3; // 8168G and later: MCU status (read only for diagnostics)
    pub const MISC: u16 = 0xF0; // 8168G and later: bit 19 = RXDV gate (receive/transmit DMA held back while set)
    pub const PMCH: u16 = 0x6F; // power management; bit 7 keeps the PHY powered (FreeBSD re(4): "PHYWAKE_PM")
    pub const OCP_DATA: u16 = 0xB0; // 8168G and later: MAC OCP access: bit 31 write, the (even) address << 15, data 15-0
    pub const CPLUS_CMD: u16 = 0xE0; // C+ Command (16-bit): bit 0 and bit 3 (PCI multiple read/write), bit 7
}

/// TCR bit 11 (8168E-VL and later, read only): the transmit queue is empty.
pub const TCR_TX_EMPTY: u32 = 1 << 11;
/// RCR bit 11 (8168G and later): "early off" version 2, set by FreeBSD re(4) for these revisions.
pub const RCR_EARLY_OFF_V2: u32 = 1 << 11;
/// MCU (0xD3) bits, 8168G and later: 7 the firmware's out-of-band mode, 5/4 transmit/receive FIFO empty, 1 the MCU's
/// internal list is ready.
pub const MCU_NOW_IS_OOB: u8 = 1 << 7;
pub const MCU_TX_EMPTY: u8 = 1 << 5;
pub const MCU_RX_EMPTY: u8 = 1 << 4;
pub const MCU_LINK_LIST_READY: u8 = 1 << 1;
/// The MAC OCP register the hand-over from firmware toggles (bit 14 cleared, then bit 15 set).
pub const OCP_HANDOVER: u16 = 0xE8DE;
/// How many start-up steps are kept for `log rtl`.
pub const MAX_STEPS: usize = 32;

/// One start-up step of the cold-start variants: a register (or PHY register) before and after, for `log rtl`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Step {
    /// What the step does, in words.
    pub what: &'static str,
    /// The register (MMIO offset; MAC OCP address for OCP steps; PHY register number for PHY steps).
    pub reg: u16,
    pub before: u32,
    pub after: u32,
    /// false: a wait ran out of time, or a PHY write never completed (the start-up goes on anyway).
    pub ok: bool,
}

/// The steps taken at start-up (only the cold-start variants add any).
#[derive(Debug, Clone, Copy)]
pub struct Steps {
    pub list: [Step; MAX_STEPS],
    pub len: usize,
}

impl Steps {
    const fn new() -> Steps {
        Steps { list: [Step { what: "", reg: 0, before: 0, after: 0, ok: true }; MAX_STEPS], len: 0 }
    }
    fn push(&mut self, s: Step) {
        if self.len < MAX_STEPS {
            self.list[self.len] = s;
            self.len += 1;
        }
    }
    pub fn as_slice(&self) -> &[Step] {
        &self.list[..self.len]
    }
}

/// MISC bit 19 (8168G and later): while set, the card's receive/transmit paths are gated off. Firmware or a previous
/// driver may leave it set; the FreeBSD re(4) driver clears it before enabling the receiver (read for the idea only).
pub const MISC_RXDV_GATE: u32 = 1 << 19;

/// Start-up choices, one per behaviour change (rule of 2026-10-22: one change per Release, verified on the laptop).
/// The default is exactly the start-up of Release os-125 (test 4, the last version that sent and received).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Receive buffers from one physically contiguous block below 4 GiB (`Hw::alloc_block`), buffer i at base + i * BUF.
    pub rx_block: bool,
    /// Clear the RXDV gate (MISC bit 19) if it is set, before the receiver and transmitter are enabled.
    pub rxdv: bool,
    /// Drop the CPU's cached copy of each data buffer: before a received frame is read, before the card reads a frame
    /// to send (`Hw::dma_sync`). Descriptor rings are never touched.
    pub cache: bool,
    /// Cold start (test 6: the card works only after the firmware's boot menu touched it). 8168G and later: before the
    /// reset, take the MAC over from the firmware's MCU: RXDV gate closed, wait for empty queues, receiver and
    /// transmitter off, MCU out-of-band bit cleared, MAC OCP 0xE8DE bit 14 cleared / bit 15 set with a wait for "list
    /// ready" after each (idea from SerenityOS, BSD-2). The gate is opened again before the receiver starts.
    pub mcu: bool,
    /// Cold start: PHY awake: PMCH bit 7, PHY register 0x1F = 0 (page 0) and 0x0E = 0 (idea from FreeBSD re(4), BSD).
    pub phy: bool,
    /// Cold start: C+ Command bits 0, 3 and 7, before every other register (as FreeBSD re(4) does for this revision).
    pub cplus: bool,
    /// Cold start: 8168G order: TCR and RCR (with "early off" v2) written before the receiver and transmitter are
    /// switched on (FreeBSD re(4): "RX/TX MACs should be enabled after RX/TX configuration").
    pub order: bool,
}

impl Options {
    /// Every cold-start step (`rtl porneste rece`): mcu, phy, cplus, order and rxdv.
    pub fn cold() -> Options {
        Options { rxdv: true, mcu: true, phy: true, cplus: true, order: true, ..Options::default() }
    }
}

/// Bits of a descriptor's first word.
pub mod desc {
    pub const OWN: u32 = 1 << 31; // the card owns the descriptor
    pub const EOR: u32 = 1 << 30; // last descriptor of the ring
    pub const FS: u32 = 1 << 29; // first segment of a frame
    pub const LS: u32 = 1 << 28; // last segment of a frame
    pub const RX_ERROR: u32 = 1 << 21; // receive: the frame has an error (RES)
    pub const LEN: u32 = 0x3FFF; // length (transmit: frame; receive: buffer size, then frame + CRC)
}

pub const CMD_RESET: u8 = 1 << 4;
pub const CMD_RX: u8 = 1 << 3;
pub const CMD_TX: u8 = 1 << 2;
pub const POLL_NORMAL: u8 = 1 << 6;
/// Receive: no FIFO threshold (7 << 13), unlimited DMA burst (7 << 8), accept broadcast (bit 3) and our MAC (bit 1).
pub const RCR_VALUE: u32 = 7 << 13 | 7 << 8 | 1 << 3 | 1 << 1;
/// Transmit: standard inter-frame gap (3 << 24), unlimited DMA burst (7 << 8).
pub const TCR_VALUE: u32 = 3 << 24 | 7 << 8;

/// IEEE 802.3 clause 22 PHY register 0 (BMCR): bit 12 auto-negotiation enable, bit 9 restart it, bit 11 power down.
pub const BMCR_AUTONEG_RESTART: u16 = 1 << 12 | 1 << 9;
/// Status bits acknowledged on every receive poll: ROK, RER, TOK, TER, RDU (descriptor unavailable), LinkChg, FOVW
/// (FIFO overflow), TDU, SWInt. Some revisions stop receiving while an overflow or "no descriptor" bit stays set.
pub const ISR_ACK: u16 = 0x01FF;

/// ISR bit 4: the card found no free receive descriptor (RDU).
pub const ISR_RDU: u16 = 1 << 4;

/// Receive descriptors (16 bytes each: the ring fills exactly one 4 KiB page, so it is 256-byte aligned). Big enough
/// that the broadcast traffic arriving while the driver waits for the link at start-up does not fill it (with 16, the
/// laptop's RTL8168GU filled the ring at boot, reported RDU and never received again: test 3, 2026-10-22).
pub const RX_RING: usize = 256;
/// Transmit descriptors (the driver waits for each frame, so a few are enough).
pub const TX_RING: usize = 16;
pub const BUF: usize = 2048;
/// Polls without a frame, after an RDU, before the receiver is restarted.
pub const STALL_POLLS: u32 = 50;
/// How many received frames are summarised for diagnostics (`net`).
pub const FIRST_FRAMES: usize = 8;
/// Largest Ethernet frame we send (without CRC), and the shortest one on the wire (padded with zeros).
pub const MAX_FRAME: usize = 1514;
pub const MIN_FRAME: usize = 60;

/// Access to one card: its registers and memory it can reach by DMA.
pub trait Hw {
    fn read8(&mut self, reg: u16) -> u8;
    fn read16(&mut self, reg: u16) -> u16;
    fn read32(&mut self, reg: u16) -> u32;
    fn write8(&mut self, reg: u16, v: u8);
    fn write16(&mut self, reg: u16, v: u16);
    fn write32(&mut self, reg: u16, v: u32);
    /// A zeroed, physically contiguous 4 KiB page: (physical address for the card, pointer for the CPU).
    fn alloc_page(&mut self) -> Option<(u64, *mut u8)>;
    /// `pages` zeroed, physically contiguous 4 KiB pages, all below 4 GiB: (physical address, pointer). Used only with
    /// `Options::rx_block`.
    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)>;
    /// Busy-waits about `us` microseconds.
    fn delay_us(&mut self, us: u32);
    /// Makes the CPU see what the card wrote by DMA to `len` bytes at `ptr` (physical `phys`): drops the CPU's cached
    /// copy (the kernel: CLFLUSH on every line). Needed when the card writes with PCIe "No Snoop", which does not
    /// update the CPU caches (test 4: frames 2+ read as zeros).
    fn dma_sync(&mut self, phys: u64, ptr: *mut u8, len: usize);
}

/// What the driver saw for one of the first received frames (`log rx`): to tell an address mix-up from a cache problem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RxDetail {
    /// Receive descriptor index.
    pub index: u16,
    /// Length from the descriptor (frame + CRC).
    pub len: u16,
    /// Buffer address written in the descriptor (read back from it).
    pub desc_addr: u64,
    /// Physical address of the buffer the driver copies from (as the driver recorded it).
    pub read_addr: u64,
    /// Virtual address the driver copies from (the kernel translates it back through the page tables for `log rx`).
    pub read_ptr: u64,
    /// The first 16 bytes before and after dropping the CPU's cached copy (the same read twice without `cache`).
    pub before: [u8; 16],
    pub after: [u8; 16],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum InitError {
    /// Every register reads as all ones: the card is switched off or not answering.
    NotAnswering,
    /// The reset bit never cleared.
    ResetTimeout,
    /// No memory for the descriptor rings or buffers.
    NoMemory,
}

#[derive(Clone, Copy)]
struct Page {
    phys: u64,
    ptr: *mut u8,
}

// The receive ring and the list of its buffers each fit in one 4 KiB page.
const _: () = assert!(RX_RING * 16 <= 4096 && RX_RING * core::mem::size_of::<Page>() <= 4096);

pub struct Rtl8168<H: Hw> {
    hw: H,
    pub mac: [u8; 6],
    /// Bits 23 and 26-30 of TCR: the chip revision ("XID"), shown in the log to tell revisions apart.
    pub revision: u32,
    /// The start-up choices this card was started with.
    pub options: Options,
    /// MISC (before, after) when `Options::rxdv` was asked for.
    pub misc_change: Option<(u32, u32)>,
    /// The cold-start steps taken, with registers before/after (empty for the default start-up).
    pub steps: Steps,
    /// The RCR value the driver writes (with "early off" v2 when `Options::order`).
    rcr_value: u32,
    rx_ring: Page,
    tx_ring: Page,
    /// A page holding the RX_RING receive buffer addresses (`Page`, 16 bytes each): kept out of the struct so the
    /// driver stays small on the kernel stack.
    rx_book: Page,
    tx_bufs: [Page; TX_RING],
    rx_next: usize,
    tx_next: usize,
    pub sent: u64,
    pub received: u64,
    /// Received frames thrown away (errors, or split over several buffers).
    pub dropped: u64,
    /// Every ISR bit seen since start-up (the bits are cleared on each poll, so this keeps the history for diagnostics).
    pub isr_seen: u16,
    /// Receive polls so far.
    pub polls: u64,
    /// Polls that saw the RDU bit (the card ran out of receive descriptors).
    pub rdu_count: u64,
    /// Receiver restarts after a stall (RDU seen, then no frame although every descriptor belongs to the card).
    pub restarts: u64,
    /// Times the driver jumped to another descriptor because the card had written there instead.
    pub resyncs: u64,
    /// UDP frames to port 68 (DHCP answers for a client) received.
    pub dhcp_replies: u64,
    /// The first received frames: (ethertype, UDP destination port or 0).
    pub first: [(u16, u16); FIRST_FRAMES],
    /// Details of the first received frames (addresses, first bytes).
    pub detail: [RxDetail; FIRST_FRAMES],
    /// Good frames whose ethertype was 0000 even after the cache was dropped (impossible on a real network).
    pub zero_type: u64,
    /// Frames that read as zeros before dropping the CPU's cached copy and correctly after it.
    pub fixed_by_sync: u64,
    /// An RDU was seen and the card has not been seen receiving into a drained ring since.
    rdu_pending: bool,
    /// Since the last RDU, the driver found the ring empty (everything handed back to the card).
    drained: bool,
    idle_polls: u32,
}

/// Registers read back for diagnostics (`log rtl`, `net`): what the card really holds after our writes.
#[derive(Debug, Clone, Copy)]
pub struct Diag {
    pub chipcmd: u8,
    pub rcr: u32,
    pub tcr: u32,
    pub phy: u8,
    pub isr: u16,
    pub imr: u16,
    pub rdsar: u64,
    pub tnpds: u64,
    pub rms: u16,
    pub mpc: u32,
    /// opts1 of the receive descriptor the driver looks at next (OWN set = still waiting for the card).
    pub rx_opts1: u32,
    pub rx_next: usize,
    /// Receive descriptors that belong to the card right now (OWN set); all of them = the ring is free.
    pub rx_owned: usize,
    /// MISC (0xF0) and MCU (0xD3), 8168G and later; read only.
    pub misc: u32,
    pub mcu: u8,
    /// C+ Command (0xE0) and PMCH (0x6F); read only.
    pub cplus: u16,
    pub pmch: u8,
}

fn desc_ptr(ring: Page, i: usize) -> *mut u32 {
    // SAFETY (for users): `ring` is a page owned by the driver and i < RX_RING (256 * 16 bytes = 4096).
    unsafe { ring.ptr.add(i * 16) as *mut u32 }
}

fn read_desc(ring: Page, i: usize) -> u32 {
    // SAFETY: see `desc_ptr`; the card may change the word at any time, hence volatile.
    unsafe { desc_ptr(ring, i).read_volatile() }
}

/// Writes a whole descriptor; the first word (with OWN) last, after a fence, so the card never sees half of it.
fn write_desc(ring: Page, i: usize, opts1: u32, addr: u64) {
    let d = desc_ptr(ring, i);
    // SAFETY: see `desc_ptr`; the driver owns this descriptor (OWN clear) while it writes it.
    unsafe {
        d.add(1).write_volatile(0);
        d.add(2).write_volatile(addr as u32);
        d.add(3).write_volatile((addr >> 32) as u32);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        d.write_volatile(opts1);
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// The buffer address held by descriptor `i` (words 2 and 3).
fn read_desc_addr(ring: Page, i: usize) -> u64 {
    let d = desc_ptr(ring, i);
    // SAFETY: see `desc_ptr`; the driver owns this descriptor (OWN clear).
    unsafe { d.add(2).read_volatile() as u64 | (d.add(3).read_volatile() as u64) << 32 }
}

/// The first 16 bytes of a receive buffer, read straight from memory (volatile: exactly what the CPU sees now).
fn first16(buf: Page) -> [u8; 16] {
    let mut b = [0u8; 16];
    for (k, x) in b.iter_mut().enumerate() {
        // SAFETY: k < 16 < BUF bytes of a driver-owned buffer.
        *x = unsafe { buf.ptr.add(k).read_volatile() };
    }
    b
}

fn eor(i: usize, len: usize) -> u32 {
    if i == len - 1 { desc::EOR } else { 0 }
}

/// (ethertype, UDP destination port or 0) of a frame, for diagnostics.
pub fn summarize(frame: &[u8]) -> (u16, u16) {
    if frame.len() < 14 {
        return (0, 0);
    }
    let et = u16::from_be_bytes([frame[12], frame[13]]);
    let ip = &frame[14..];
    if et == 0x0800 && ip.len() >= 20 && ip[9] == 17 {
        let ihl = (ip[0] & 0x0F) as usize * 4;
        if ip.len() >= ihl + 4 {
            return (et, u16::from_be_bytes([ip[ihl + 2], ip[ihl + 3]]));
        }
    }
    (et, 0)
}

/// Read, change one bit, write (only if it must change), read back; recorded as a step. `width` is 8, 16 or 32.
fn set_bit<H: Hw>(hw: &mut H, steps: &mut Steps, what: &'static str, reg: u16, width: u8, bit: u32, on: bool) {
    let read = |hw: &mut H| match width {
        8 => hw.read8(reg) as u32,
        16 => hw.read16(reg) as u32,
        _ => hw.read32(reg),
    };
    let before = read(hw);
    let want = if on { before | 1 << bit } else { before & !(1 << bit) };
    if want != before {
        match width {
            8 => hw.write8(reg, want as u8),
            16 => hw.write16(reg, want as u16),
            _ => hw.write32(reg, want),
        }
    }
    let after = read(hw);
    steps.push(Step { what, reg, before, after, ok: after & 1 << bit == want & 1 << bit });
}

/// Waits (at most `limit_us`, in 10 us steps) until all `mask` bits of a register are set; recorded as a step with the
/// value at the start and at the end. Never blocks for good: a timeout is only noted.
fn wait_bits<H: Hw>(hw: &mut H, steps: &mut Steps, what: &'static str, reg: u16, wide: bool, mask: u32, limit_us: u32) {
    let read = |hw: &mut H| if wide { hw.read32(reg) } else { hw.read8(reg) as u32 };
    let before = read(hw);
    let mut now = before;
    let mut waited = 0;
    while now & mask != mask && waited < limit_us {
        hw.delay_us(10);
        waited += 10;
        now = read(hw);
    }
    steps.push(Step { what, reg, before, after: now, ok: now & mask == mask });
}

/// Reads a 16-bit MAC OCP register (8168G and later).
fn ocp_read<H: Hw>(hw: &mut H, at: u16) -> u16 {
    hw.write32(reg::OCP_DATA, (at as u32) << 15);
    hw.read32(reg::OCP_DATA) as u16
}

/// One bit of a MAC OCP register: read, change, write, read back; recorded as a step.
fn ocp_set_bit<H: Hw>(hw: &mut H, steps: &mut Steps, what: &'static str, at: u16, bit: u32, on: bool) {
    let before = ocp_read(hw, at);
    let want = if on { before | 1 << bit } else { before & !(1 << bit) };
    if want != before {
        hw.write32(reg::OCP_DATA, 1 << 31 | (at as u32) << 15 | want as u32);
    }
    let after = ocp_read(hw, at);
    steps.push(Step { what, reg: at, before: before as u32, after: after as u32, ok: after == want });
}

/// Writes PHY register `r` (MII) through PHYAR; false if the write never completed.
fn phy_write_raw<H: Hw>(hw: &mut H, r: u8, value: u16) -> bool {
    hw.write32(reg::PHYAR, 1 << 31 | ((r as u32 & 0x1F) << 16) | value as u32);
    for _ in 0..2000 {
        hw.delay_us(25);
        if hw.read32(reg::PHYAR) & 1 << 31 == 0 {
            hw.delay_us(20); // the datasheets ask for a short pause between PHY accesses
            return true;
        }
    }
    false
}

/// The hand-over from the firmware's MCU before the reset (`Options::mcu`), each step recorded.
fn take_over_from_firmware<H: Hw>(hw: &mut H, steps: &mut Steps) {
    set_bit(hw, steps, "poarta RXDV inchisa (MISC bit 19)", reg::MISC, 32, 19, true);
    wait_bits(hw, steps, "astept coada TX goala (TCR bit 11)", reg::TCR, true, TCR_TX_EMPTY, 100_000);
    let fifo = (MCU_TX_EMPTY | MCU_RX_EMPTY) as u32;
    wait_bits(hw, steps, "astept FIFO RX+TX goale (MCU biti 4,5)", reg::MCU, false, fifo, 100_000);
    set_bit(hw, steps, "primire oprita (CR bit 3)", reg::CHIPCMD, 8, 3, false);
    set_bit(hw, steps, "trimitere oprita (CR bit 2)", reg::CHIPCMD, 8, 2, false);
    set_bit(hw, steps, "MCU fara modul OOB (bit 7)", reg::MCU, 8, 7, false);
    ocp_set_bit(hw, steps, "OCP E8DE bit 14 sters", OCP_HANDOVER, 14, false);
    let ready = MCU_LINK_LIST_READY as u32;
    wait_bits(hw, steps, "astept MCU lista gata (bit 1)", reg::MCU, false, ready, 100_000);
    ocp_set_bit(hw, steps, "OCP E8DE bit 15 pus", OCP_HANDOVER, 15, true);
    wait_bits(hw, steps, "astept MCU lista gata (bit 1)", reg::MCU, false, ready, 100_000);
}

impl<H: Hw> Rtl8168<H> {
    /// Resets and starts the card exactly as Release os-125 did (`Options::default()`).
    pub fn new(hw: H) -> Result<Self, InitError> {
        Self::with_options(hw, Options::default())
    }

    /// Resets and starts the card: MAC address, rings, receiver and transmitter on, interrupts off.
    pub fn with_options(mut hw: H, options: Options) -> Result<Self, InitError> {
        if hw.read32(reg::TCR) == u32::MAX && hw.read32(reg::IDR0) == u32::MAX {
            return Err(InitError::NotAnswering);
        }
        let mut steps = Steps::new();
        if options.mcu {
            take_over_from_firmware(&mut hw, &mut steps);
        }
        hw.write8(reg::CHIPCMD, CMD_RESET);
        let mut done = false;
        for _ in 0..100_000 {
            if hw.read8(reg::CHIPCMD) & CMD_RESET == 0 {
                done = true;
                break;
            }
            hw.delay_us(10);
        }
        if !done {
            return Err(InitError::ResetTimeout);
        }
        let revision = hw.read32(reg::TCR) & 0x7C80_0000;
        let mut mac = [0u8; 6];
        for (i, b) in mac.iter_mut().enumerate() {
            *b = hw.read8(reg::IDR0 + i as u16);
        }

        let mut page = || hw_page(&mut hw);
        let (rx_ring, tx_ring) = (page().ok_or(InitError::NoMemory)?, page().ok_or(InitError::NoMemory)?);
        let rx_book = hw_page(&mut hw).ok_or(InitError::NoMemory)?;
        let mut tx_bufs = [tx_ring; TX_RING];
        let block = if options.rx_block {
            let (phys, ptr) = hw.alloc_block(RX_RING * BUF / 4096).ok_or(InitError::NoMemory)?;
            Some(Page { phys, ptr })
        } else {
            None
        };
        for i in (0..RX_RING).step_by(2) {
            let p = match block {
                // SAFETY: i * BUF < RX_RING * BUF, the block's size; physical and virtual addresses move together.
                Some(b) => Page { phys: b.phys + (i * BUF) as u64, ptr: unsafe { b.ptr.add(i * BUF) } },
                None => hw_page(&mut hw).ok_or(InitError::NoMemory)?,
            };
            if options.cache {
                // The page was zeroed by the CPU: push those zeros out of the cache before the card writes there.
                hw.dma_sync(p.phys, p.ptr, 4096);
            }
            for (k, b) in halves(p).into_iter().enumerate() {
                // SAFETY: the book page holds exactly RX_RING `Page`s (4096 bytes) and is page-aligned.
                unsafe { (rx_book.ptr as *mut Page).add(i + k).write(b) };
                write_desc(rx_ring, i + k, desc::OWN | eor(i + k, RX_RING) | BUF as u32, b.phys);
            }
        }
        for pair in tx_bufs.chunks_mut(2) {
            let p = hw_page(&mut hw).ok_or(InitError::NoMemory)?;
            pair.copy_from_slice(&halves(p));
        }
        for i in 0..TX_RING {
            write_desc(tx_ring, i, eor(i, TX_RING), 0);
        }

        hw.write8(reg::CR9346, 0xC0); // unlock configuration
        if options.cplus {
            // FreeBSD re(4): the C+ register is configured before all others.
            set_bit(&mut hw, &mut steps, "C+ Command bit 0", reg::CPLUS_CMD, 16, 0, true);
            set_bit(&mut hw, &mut steps, "C+ Command bit 3 (PCI citire/scriere multipla)", reg::CPLUS_CMD, 16, 3, true);
            set_bit(&mut hw, &mut steps, "C+ Command bit 7", reg::CPLUS_CMD, 16, 7, true);
        }
        if options.phy {
            set_bit(&mut hw, &mut steps, "PHY alimentat (PMCH bit 7)", reg::PMCH, 8, 7, true);
            for (r, what) in [(0x1F, "PHY registrul 1F = 0 (pagina 0)"), (0x0E, "PHY registrul 0E = 0")] {
                let ok = phy_write_raw(&mut hw, r, 0);
                steps.push(Step { what, reg: r as u16, before: 0, after: 0, ok });
            }
        }
        let rcr_value = if options.order { RCR_VALUE | RCR_EARLY_OFF_V2 } else { RCR_VALUE };
        hw.write16(reg::IMR, 0); // polling: no interrupts
        hw.write16(reg::ISR, 0xFFFF);
        hw.write16(reg::RMS, BUF as u16);
        hw.write8(reg::MTPS, 0x3B);
        hw.write32(reg::TNPDS, tx_ring.phys as u32);
        hw.write32(reg::TNPDS + 4, (tx_ring.phys >> 32) as u32);
        hw.write32(reg::RDSAR, rx_ring.phys as u32);
        hw.write32(reg::RDSAR + 4, (rx_ring.phys >> 32) as u32);
        hw.write32(reg::MAR0, 0);
        hw.write32(reg::MAR0 + 4, 0);
        let mut misc_change = None;
        if options.rxdv || options.mcu {
            // Read, change one bit, write; only if it is set.
            let before = hw.read32(reg::MISC);
            if before & MISC_RXDV_GATE != 0 {
                hw.write32(reg::MISC, before & !MISC_RXDV_GATE);
            }
            misc_change = Some((before, hw.read32(reg::MISC)));
        }
        if options.order {
            // 8168G and later: configure first, then switch the receiver and transmitter on.
            hw.write32(reg::TCR, TCR_VALUE);
            hw.write32(reg::RCR, rcr_value);
            steps.push(Step { what: "TCR scris inainte de pornire", reg: reg::TCR, before: 0, after: hw.read32(reg::TCR), ok: true });
            steps.push(Step { what: "RCR scris inainte de pornire", reg: reg::RCR, before: 0, after: hw.read32(reg::RCR), ok: true });
        }
        hw.write8(reg::CHIPCMD, CMD_RX | CMD_TX);
        // Written after enabling as well: some revisions only keep RCR/TCR once the receiver/transmitter is on.
        hw.write32(reg::RCR, rcr_value);
        hw.write32(reg::TCR, TCR_VALUE);
        hw.write8(reg::CR9346, 0x00); // lock configuration

        Ok(Rtl8168 {
            hw, mac, revision, options, misc_change, steps, rcr_value, rx_ring, tx_ring, rx_book, tx_bufs,
            rx_next: 0, tx_next: 0, sent: 0, received: 0, dropped: 0, isr_seen: 0, polls: 0,
            rdu_count: 0, restarts: 0, resyncs: 0, dhcp_replies: 0, first: [(0, 0); FIRST_FRAMES],
            detail: [RxDetail::default(); FIRST_FRAMES], zero_type: 0, fixed_by_sync: 0,
            rdu_pending: false, drained: false, idle_polls: 0,
        })
    }

    /// True if the cable is connected and the link negotiated.
    pub fn link_up(&mut self) -> bool {
        self.hw.read8(reg::PHYSTATUS) & 0b10 != 0
    }

    /// The raw PHYstatus byte (link, speed, duplex) and the interrupt status word, for diagnostics.
    pub fn status(&mut self) -> (u8, u16) {
        (self.hw.read8(reg::PHYSTATUS), self.hw.read16(reg::ISR))
    }

    /// Writes PHY register `reg` (MII) through PHYAR; false if the write never completed.
    pub fn phy_write(&mut self, reg: u8, value: u16) -> bool {
        phy_write_raw(&mut self.hw, reg, value)
    }

    /// Powers the PHY up and restarts auto-negotiation (firmware may leave it powered down when nothing boots
    /// from the network), then waits up to `wait_ms` for the link. Returns whether the link is up.
    pub fn start_link(&mut self, wait_ms: u32) -> bool {
        self.phy_write(0, BMCR_AUTONEG_RESTART);
        for _ in 0..wait_ms / 10 {
            if self.link_up() {
                return true;
            }
            self.hw.delay_us(10_000);
        }
        self.link_up()
    }

    /// Sends one Ethernet frame (destination, source, type, payload; at most 1514 bytes). Waits until the card took it.
    pub fn send(&mut self, frame: &[u8]) -> bool {
        if frame.is_empty() || frame.len() > MAX_FRAME {
            return false;
        }
        let i = self.tx_next;
        if !self.wait_tx_free(i) {
            return false;
        }
        let len = frame.len().max(MIN_FRAME);
        let buf = self.tx_bufs[i];
        // SAFETY: the buffer belongs to the driver and the card is done with it (OWN clear); len <= 1514 < BUF.
        unsafe {
            core::ptr::write_bytes(buf.ptr, 0, MIN_FRAME);
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buf.ptr, frame.len());
        }
        if self.options.cache {
            // The frame goes from the CPU cache to memory before the card reads it.
            self.hw.dma_sync(buf.phys, buf.ptr, len);
        }
        write_desc(self.tx_ring, i, desc::OWN | eor(i, TX_RING) | desc::FS | desc::LS | len as u32, buf.phys);
        self.tx_next = (i + 1) % TX_RING;
        self.hw.write8(reg::TPPOLL, POLL_NORMAL);
        let done = self.wait_tx_free(i);
        if done {
            self.sent += 1;
        }
        done
    }

    fn wait_tx_free(&mut self, i: usize) -> bool {
        for _ in 0..100_000 {
            if read_desc(self.tx_ring, i) & desc::OWN == 0 {
                return true;
            }
            self.hw.delay_us(1);
        }
        false
    }

    /// Copies the next received frame (without CRC) into `out`; returns its length (cut to `out.len()`).
    pub fn receive(&mut self, out: &mut [u8]) -> Option<usize> {
        self.polls += 1;
        let isr = self.hw.read16(reg::ISR);
        self.isr_seen |= isr;
        if isr & ISR_ACK != 0 {
            self.hw.write16(reg::ISR, isr & ISR_ACK);
        }
        if isr & ISR_RDU != 0 {
            self.rdu_count += 1;
            self.rdu_pending = true;
            self.drained = false;
        }
        loop {
            let i = self.rx_next;
            let opts1 = read_desc(self.rx_ring, i);
            if opts1 & desc::OWN != 0 {
                if self.resync() {
                    continue;
                }
                self.check_stall();
                return None; // nothing new
            }
            let whole = opts1 & (desc::FS | desc::LS) == desc::FS | desc::LS;
            let len = (opts1 & desc::LEN) as usize;
            let good = whole && opts1 & desc::RX_ERROR == 0 && (4..=BUF).contains(&len);
            let mut n = 0;
            let mut detail = RxDetail::default();
            if good {
                // The descriptor said "done" before the data may be read (no load moves above the OWN check).
                core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
                let buf = self.rx_buf(i);
                n = (len - 4).min(out.len());
                detail = RxDetail {
                    index: i as u16,
                    len: len as u16,
                    desc_addr: read_desc_addr(self.rx_ring, i),
                    read_addr: buf.phys,
                    read_ptr: buf.ptr as u64,
                    before: first16(buf),
                    after: [0; 16],
                };
                if self.options.cache {
                    self.hw.dma_sync(buf.phys, buf.ptr, len);
                }
                // SAFETY: the card wrote `len` <= BUF bytes into this driver-owned buffer and handed it back.
                unsafe { core::ptr::copy_nonoverlapping(buf.ptr, out.as_mut_ptr(), n) };
                detail.after = first16(buf);
            }
            self.rearm(i);
            self.rx_next = (i + 1) % RX_RING;
            self.idle_polls = 0;
            if self.drained {
                // A frame arrived after the ring had been emptied: the card is receiving again by itself.
                self.rdu_pending = false;
                self.drained = false;
            }
            if good {
                let sum = summarize(&out[..n]);
                if (self.received as usize) < FIRST_FRAMES {
                    self.first[self.received as usize] = sum;
                    self.detail[self.received as usize] = detail;
                }
                if n >= 14 && sum.0 == 0 {
                    self.zero_type += 1;
                }
                if detail.before[12..14] == [0, 0] && detail.after[12..14] != [0, 0] {
                    self.fixed_by_sync += 1;
                }
                if sum == (0x0800, 68) {
                    self.dhcp_replies += 1;
                }
                self.received += 1;
                return Some(n);
            }
            self.dropped += 1;
        }
    }

    fn rx_buf(&self, i: usize) -> Page {
        // SAFETY: i < RX_RING and every entry of the book was written in `new`.
        unsafe { (self.rx_book.ptr as *const Page).add(i).read() }
    }

    /// Hands receive descriptor `i` back to the card (OWN, EOR on the last one, buffer size).
    fn rearm(&mut self, i: usize) {
        write_desc(self.rx_ring, i, desc::OWN | eor(i, RX_RING) | BUF as u32, self.rx_buf(i).phys);
    }

    /// The descriptor the driver expects is still the card's: if the card wrote a frame into another one (it started
    /// from another index, e.g. after a restart), continue from the first such descriptor after `rx_next`.
    fn resync(&mut self) -> bool {
        for k in 1..RX_RING {
            let j = (self.rx_next + k) % RX_RING;
            if read_desc(self.rx_ring, j) & desc::OWN == 0 {
                // The card fills descriptors in order: if the expected one is done by now, it simply finished
                // writing while we looked, and nothing was skipped.
                if read_desc(self.rx_ring, self.rx_next) & desc::OWN == 0 {
                    return true;
                }
                self.rx_next = j;
                self.resyncs += 1;
                return true;
            }
        }
        false
    }

    /// After an RDU, some revisions (the laptop's RTL8168GU) do not take frames again on their own once descriptors
    /// are free. If no frame came for `STALL_POLLS` polls although every descriptor is the card's, restart the
    /// receiver: off, descriptors re-armed, ring start written again, on. (On a quiet network this may restart once
    /// for nothing, which costs nothing.)
    fn check_stall(&mut self) {
        if !self.rdu_pending {
            return;
        }
        self.drained = true;
        self.idle_polls += 1;
        if self.idle_polls < STALL_POLLS {
            return;
        }
        self.idle_polls = 0;
        self.rdu_pending = false;
        self.drained = false;
        self.restart_rx();
    }

    /// Stops the receiver, gives every descriptor back to the card from index 0 and starts it again.
    pub fn restart_rx(&mut self) {
        self.restarts += 1;
        self.hw.write8(reg::CHIPCMD, CMD_TX);
        self.hw.delay_us(100);
        for i in 0..RX_RING {
            self.rearm(i);
        }
        self.rx_next = 0;
        self.hw.write8(reg::CR9346, 0xC0);
        self.hw.write32(reg::RDSAR, self.rx_ring.phys as u32);
        self.hw.write32(reg::RDSAR + 4, (self.rx_ring.phys >> 32) as u32);
        self.hw.write16(reg::ISR, ISR_ACK);
        self.hw.write8(reg::CHIPCMD, CMD_RX | CMD_TX);
        self.hw.write32(reg::RCR, self.rcr_value);
        self.hw.write8(reg::CR9346, 0x00);
    }

    /// Reads back the registers the driver set, plus the state of the next receive descriptor.
    pub fn diag(&mut self) -> Diag {
        let r64 = |hw: &mut H, at: u16| hw.read32(at) as u64 | (hw.read32(at + 4) as u64) << 32;
        Diag {
            chipcmd: self.hw.read8(reg::CHIPCMD),
            rcr: self.hw.read32(reg::RCR),
            tcr: self.hw.read32(reg::TCR),
            phy: self.hw.read8(reg::PHYSTATUS),
            isr: self.hw.read16(reg::ISR),
            imr: self.hw.read16(reg::IMR),
            rdsar: r64(&mut self.hw, reg::RDSAR),
            tnpds: r64(&mut self.hw, reg::TNPDS),
            rms: self.hw.read16(reg::RMS),
            mpc: self.hw.read32(reg::MPC) & 0x00FF_FFFF,
            rx_opts1: read_desc(self.rx_ring, self.rx_next),
            rx_next: self.rx_next,
            rx_owned: (0..RX_RING).filter(|&i| read_desc(self.rx_ring, i) & desc::OWN != 0).count(),
            misc: self.hw.read32(reg::MISC),
            mcu: self.hw.read8(reg::MCU),
            cplus: self.hw.read16(reg::CPLUS_CMD),
            pmch: self.hw.read8(reg::PMCH),
        }
    }

    /// The physical addresses of the rings (to compare with what the card reads back).
    pub fn ring_addresses(&self) -> (u64, u64) {
        (self.rx_ring.phys, self.tx_ring.phys)
    }

    /// Physical addresses of the first `N` receive buffers (`log rtl`).
    pub fn rx_buffer_addresses<const N: usize>(&self) -> [u64; N] {
        core::array::from_fn(|i| self.rx_buf(i.min(RX_RING - 1)).phys)
    }

    /// The hardware access object (tests use it to drive the model).
    pub fn hw(&mut self) -> &mut H {
        &mut self.hw
    }
}

/// The two 2 KiB buffers of a 4 KiB page.
fn halves(p: Page) -> [Page; 2] {
    // SAFETY: the second half of the same 4 KiB page.
    [p, Page { phys: p.phys + BUF as u64, ptr: unsafe { p.ptr.add(BUF) } }]
}

fn hw_page<H: Hw>(hw: &mut H) -> Option<Page> {
    hw.alloc_page().map(|(phys, ptr)| Page { phys, ptr })
}

#[cfg(test)]
mod tests;
