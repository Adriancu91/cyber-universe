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
/// laptop's RTL8168H filled the ring at boot, reported RDU and never received again: test 3, 2026-10-22).
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
    /// Busy-waits about `us` microseconds.
    fn delay_us(&mut self, us: u32);
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

impl<H: Hw> Rtl8168<H> {
    /// Resets and starts the card: MAC address, rings, receiver and transmitter on, interrupts off.
    pub fn new(mut hw: H) -> Result<Self, InitError> {
        if hw.read32(reg::TCR) == u32::MAX && hw.read32(reg::IDR0) == u32::MAX {
            return Err(InitError::NotAnswering);
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
        for i in (0..RX_RING).step_by(2) {
            let p = hw_page(&mut hw).ok_or(InitError::NoMemory)?;
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
        hw.write8(reg::CHIPCMD, CMD_RX | CMD_TX);
        // Written after enabling as well: some revisions only keep RCR/TCR once the receiver/transmitter is on.
        hw.write32(reg::RCR, RCR_VALUE);
        hw.write32(reg::TCR, TCR_VALUE);
        hw.write8(reg::CR9346, 0x00); // lock configuration

        Ok(Rtl8168 {
            hw, mac, revision, rx_ring, tx_ring, rx_book, tx_bufs,
            rx_next: 0, tx_next: 0, sent: 0, received: 0, dropped: 0, isr_seen: 0, polls: 0,
            rdu_count: 0, restarts: 0, resyncs: 0, dhcp_replies: 0, first: [(0, 0); FIRST_FRAMES],
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
        self.hw.write32(reg::PHYAR, 1 << 31 | ((reg as u32 & 0x1F) << 16) | value as u32);
        for _ in 0..2000 {
            self.hw.delay_us(25);
            if self.hw.read32(reg::PHYAR) & 1 << 31 == 0 {
                self.hw.delay_us(20); // the datasheets ask for a short pause between PHY accesses
                return true;
            }
        }
        false
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
            if good {
                n = (len - 4).min(out.len());
                // SAFETY: the card wrote `len` <= BUF bytes into this driver-owned buffer and handed it back.
                unsafe { core::ptr::copy_nonoverlapping(self.rx_buf(i).ptr, out.as_mut_ptr(), n) };
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

    /// After an RDU, some revisions (the laptop's RTL8168H) do not take frames again on their own once descriptors
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
        self.hw.write32(reg::RCR, RCR_VALUE);
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
        }
    }

    /// The physical addresses of the rings (to compare with what the card reads back).
    pub fn ring_addresses(&self) -> (u64, u64) {
        (self.rx_ring.phys, self.tx_ring.phys)
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
