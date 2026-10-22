//! A software model of the RTL8168, only for host tests. It follows the behaviour described in the public
//! datasheets for what the driver uses: reset, the configuration lock, the descriptor rings, transmit polling,
//! receive filtering (broadcast + own MAC) and the link bit.

use crate::{desc, reg, Hw, CMD_RESET, CMD_RX, CMD_TX, POLL_NORMAL};
use std::{boxed::Box, vec, vec::Vec};

pub const MAC: [u8; 6] = [0x00, 0xE0, 0x4C, 0x68, 0x01, 0x02];
const PHYS_BASE: u64 = 0x1_2340_0000; // above 4 GiB: checks that the driver writes the high halves too

pub struct Model {
    regs: [u8; 256],
    pages: Vec<Box<[u8; 4096]>>,
    /// Reads of CHIPCMD before a reset finishes (u32::MAX: never).
    pub reset_reads: u32,
    reset_left: u32,
    pub absent: bool,
    pub page_limit: usize,
    /// Frames the card put on the wire.
    pub wire: Vec<Vec<u8>>,
    /// Frames lost because no receive descriptor was free.
    pub missed: u32,
    /// Writes to locked configuration registers (the driver must unlock first).
    pub locked_writes: u32,
    tx_idx: usize,
    rx_idx: usize,
    pub delays_us: u64,
    /// PHY register 0 (BMCR); with the power-down bit (11) set the link stays down.
    pub bmcr: u16,
    /// Whether a cable is plugged in (the link comes up only with the PHY powered and auto-negotiation on).
    pub cable: bool,
    /// Receive stalls while the FIFO-overflow bit (6) of ISR is set, as some revisions do.
    pub stall_on_overflow: bool,
    /// After running out of descriptors (RDU), the card takes no frame until the receiver is switched off and on
    /// again: what the laptop's RTL8168H showed at test 3 (16 frames, then nothing).
    pub stall_after_rdu: bool,
    /// The card keeps its receive index when the receiver is switched off and on (instead of restarting at the
    /// ring start), so the driver has to find where it writes.
    pub keep_index_on_restart: bool,
    rx_stalled: bool,
    /// Frame data is written with PCIe "No Snoop": the CPU keeps seeing its old (cached) bytes until the driver drops
    /// them with `dma_sync`, as on the test laptop (test 4: frames 2+ read as zeros). Descriptors stay coherent.
    pub no_snoop: bool,
    /// Bytes in memory that the CPU does not see yet (physical address, data), when `no_snoop` is on.
    hidden: Vec<(u64, Vec<u8>)>,
    /// `dma_sync` calls and the bytes they covered.
    pub syncs: u64,
    pub synced_bytes: u64,
}

impl Model {
    pub fn new() -> Model {
        let mut m = Model {
            regs: [0; 256], pages: Vec::new(), reset_reads: 3, reset_left: 0, absent: false, page_limit: 160,
            wire: Vec::new(), missed: 0, locked_writes: 0, tx_idx: 0, rx_idx: 0, delays_us: 0,
            bmcr: 1 << 12, cable: true, stall_on_overflow: true, stall_after_rdu: false,
            keep_index_on_restart: false, rx_stalled: false, no_snoop: false, hidden: Vec::new(), syncs: 0,
            synced_bytes: 0,
        };
        m.regs[..6].copy_from_slice(&MAC);
        m.set32(reg::TCR, 0x5480_0000); // a revision number in bits 23 and 26-30, like a real chip
        m.update_link();
        m
    }

    pub fn set_link(&mut self, up: bool) {
        self.cable = up;
        self.update_link();
    }

    /// A PHY left powered down by the firmware (the link cannot come up until the driver powers it).
    pub fn power_down_phy(&mut self) {
        self.bmcr |= 1 << 11;
        self.update_link();
    }

    fn update_link(&mut self) {
        let up = self.cable && self.bmcr & (1 << 11) == 0 && self.bmcr & (1 << 12) != 0;
        // 1000 Mb/s full duplex when up.
        self.regs[reg::PHYSTATUS as usize] = if up { 0b1_0011 } else { 0 };
    }

    /// Sets ISR bits as the card would (e.g. 1 << 6 for a FIFO overflow).
    pub fn raise(&mut self, bits: u16) {
        let old = u16::from_le_bytes([self.regs[0x3E], self.regs[0x3F]]);
        self.regs[0x3E..0x40].copy_from_slice(&(old | bits).to_le_bytes());
    }

    fn get32(&self, r: u16) -> u32 {
        u32::from_le_bytes(self.regs[r as usize..r as usize + 4].try_into().unwrap())
    }
    fn set32(&mut self, r: u16, v: u32) {
        self.regs[r as usize..r as usize + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn get64(&self, r: u16) -> u64 {
        self.get32(r) as u64 | (self.get32(r + 4) as u64) << 32
    }
    pub fn reg32(&self, r: u16) -> u32 {
        self.get32(r)
    }
    pub fn reg8(&self, r: u16) -> u8 {
        self.regs[r as usize]
    }

    /// Bytes at a physical address (inside one allocated page).
    fn mem(&mut self, phys: u64, len: usize) -> &mut [u8] {
        let off = phys.checked_sub(PHYS_BASE).expect("DMA below the pages") as usize;
        let (page, at) = (off / 4096, off % 4096);
        assert!(at + len <= 4096, "DMA crosses a page");
        &mut self.pages[page][at..at + len]
    }

    fn desc(&mut self, ring: u64, i: usize) -> (u32, u64) {
        let d = self.mem(ring + i as u64 * 16, 16);
        let opts1 = u32::from_le_bytes(d[0..4].try_into().unwrap());
        let addr = u64::from_le_bytes(d[8..16].try_into().unwrap());
        (opts1, addr)
    }
    fn set_opts1(&mut self, ring: u64, i: usize, v: u32) {
        self.mem(ring + i as u64 * 16, 4).copy_from_slice(&v.to_le_bytes());
    }

    fn unlocked(&self) -> bool {
        self.regs[reg::CR9346 as usize] & 0xC0 == 0xC0
    }

    fn write(&mut self, r: u16, bytes: &[u8]) {
        let locked_reg = matches!(r, reg::TNPDS..=0x27 | reg::RDSAR..=0xEB | reg::RMS | 0xDB | reg::MTPS);
        if locked_reg && !self.unlocked() {
            self.locked_writes += 1;
            return;
        }
        match r {
            reg::CHIPCMD => {
                let v = bytes[0];
                if v & CMD_RESET != 0 {
                    // Reset: rings back to the start, receiver/transmitter off, configuration cleared (MAC kept).
                    self.reset_left = self.reset_reads;
                    self.regs[reg::CHIPCMD as usize] = CMD_RESET;
                    self.set32(reg::RCR, 0);
                    self.tx_idx = 0;
                    self.rx_idx = 0;
                } else {
                    let was_on = self.regs[reg::CHIPCMD as usize] & CMD_RX != 0;
                    if !was_on && v & CMD_RX != 0 {
                        // Receiver switched on again: a stall after RDU ends; the ring is read from its start.
                        self.rx_stalled = false;
                        if !self.keep_index_on_restart {
                            self.rx_idx = 0;
                        }
                    }
                    self.regs[reg::CHIPCMD as usize] = v & (CMD_RX | CMD_TX);
                }
                return;
            }
            reg::TPPOLL => {
                if bytes[0] & POLL_NORMAL != 0 {
                    self.transmit();
                }
                return;
            }
            reg::ISR => {
                let v = u16::from_le_bytes([bytes[0], bytes[1]]);
                let old = u16::from_le_bytes([self.regs[0x3E], self.regs[0x3F]]);
                self.regs[0x3E..0x40].copy_from_slice(&(old & !v).to_le_bytes());
                return;
            }
            reg::PHYAR => {
                let v = u32::from_le_bytes(bytes.try_into().unwrap());
                if v & 1 << 31 != 0 && (v >> 16) & 0x1F == 0 {
                    self.bmcr = v as u16 & !(1 << 9); // the restart bit clears itself
                    self.update_link();
                }
                self.set32(reg::PHYAR, v & !(1 << 31)); // write done at once
                return;
            }
            reg::TCR => {
                // Bits 23 and 26-30 (revision) are read-only.
                let v = u32::from_le_bytes(bytes.try_into().unwrap());
                let keep = self.get32(reg::TCR) & 0x7C80_0000;
                self.set32(reg::TCR, keep | (v & !0x7C80_0000));
                return;
            }
            _ => {}
        }
        self.regs[r as usize..r as usize + bytes.len()].copy_from_slice(bytes);
    }

    fn transmit(&mut self) {
        if self.regs[reg::CHIPCMD as usize] & CMD_TX == 0 {
            return;
        }
        let ring = self.get64(reg::TNPDS);
        assert_eq!(ring % 256, 0, "transmit ring must be 256-byte aligned");
        loop {
            let (opts1, addr) = self.desc(ring, self.tx_idx);
            if opts1 & desc::OWN == 0 {
                break;
            }
            assert_eq!(opts1 & (desc::FS | desc::LS), desc::FS | desc::LS, "one frame per descriptor");
            let len = (opts1 & desc::LEN) as usize;
            let frame = self.mem(addr, len).to_vec();
            self.wire.push(frame);
            self.set_opts1(ring, self.tx_idx, opts1 & !desc::OWN);
            self.tx_idx = if opts1 & desc::EOR != 0 { 0 } else { self.tx_idx + 1 };
            assert!(self.tx_idx < 1024, "transmit ring without EOR");
        }
    }

    /// A frame arrives from the cable. Returns false if the card did not take it (off, filtered or no free descriptor).
    pub fn deliver(&mut self, frame: &[u8]) -> bool {
        if self.regs[reg::CHIPCMD as usize] & CMD_RX == 0 {
            return false;
        }
        let rcr = self.get32(reg::RCR);
        let broadcast = frame[..6] == [0xFF; 6];
        let ours = frame[..6] == self.regs[..6];
        let isr = u16::from_le_bytes([self.regs[0x3E], self.regs[0x3F]]);
        if self.stall_on_overflow && isr & 1 << 6 != 0 || self.regs[reg::PHYSTATUS as usize] & 0b10 == 0 {
            self.missed += 1;
            return false;
        }
        if !(broadcast && rcr & 1 << 3 != 0 || ours && rcr & 1 << 1 != 0 || rcr & 1 != 0) {
            return false;
        }
        let ring = self.get64(reg::RDSAR);
        assert_eq!(ring % 256, 0, "receive ring must be 256-byte aligned");
        let (opts1, addr) = self.desc(ring, self.rx_idx);
        let size = (opts1 & desc::LEN) as usize;
        let max = u16::from_le_bytes([self.regs[0xDA], self.regs[0xDB]]) as usize;
        let len = frame.len() + 4; // plus the CRC
        if self.rx_stalled {
            self.missed += 1;
            return false;
        }
        if opts1 & desc::OWN == 0 {
            self.raise(crate::ISR_RDU);
            self.rx_stalled = self.stall_after_rdu;
            self.missed += 1;
            return false;
        }
        if len > size || len > max {
            self.missed += 1;
            return false;
        }
        let _ = self.mem(addr, len); // checks the buffer lies inside one page
        let mut data = frame.to_vec();
        data.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        if self.no_snoop {
            self.hidden.push((addr, data));
        } else {
            self.mem(addr, len).copy_from_slice(&data);
        }
        let eor = opts1 & desc::EOR;
        self.set_opts1(ring, self.rx_idx, eor | desc::FS | desc::LS | len as u32);
        self.rx_idx = if eor != 0 { 0 } else { self.rx_idx + 1 };
        self.raise(1); // ROK
        true
    }

    /// The index of the receive descriptor the card will use next.
    pub fn rx_index(&self) -> usize {
        self.rx_idx
    }

    /// Marks the receive descriptor the card will use next as holding a frame with an error.
    pub fn deliver_error(&mut self) {
        let ring = self.get64(reg::RDSAR);
        let (opts1, _) = self.desc(ring, self.rx_idx);
        let eor = opts1 & desc::EOR;
        self.set_opts1(ring, self.rx_idx, eor | desc::FS | desc::LS | desc::RX_ERROR | 64);
        self.rx_idx = if eor != 0 { 0 } else { self.rx_idx + 1 };
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Hw for Model {
    fn read8(&mut self, r: u16) -> u8 {
        if self.absent {
            return 0xFF;
        }
        if r == reg::CHIPCMD && self.reset_left > 0 {
            if self.reset_left != u32::MAX {
                self.reset_left -= 1;
            }
            if self.reset_left == 0 {
                self.regs[r as usize] &= !CMD_RESET;
            }
        }
        self.regs[r as usize]
    }
    fn read16(&mut self, r: u16) -> u16 {
        if self.absent { u16::MAX } else { u16::from_le_bytes([self.regs[r as usize], self.regs[r as usize + 1]]) }
    }
    fn read32(&mut self, r: u16) -> u32 {
        if self.absent { u32::MAX } else { self.get32(r) }
    }
    fn write8(&mut self, r: u16, v: u8) {
        self.write(r, &[v]);
    }
    fn write16(&mut self, r: u16, v: u16) {
        self.write(r, &v.to_le_bytes());
    }
    fn write32(&mut self, r: u16, v: u32) {
        self.write(r, &v.to_le_bytes());
    }
    fn alloc_page(&mut self) -> Option<(u64, *mut u8)> {
        if self.pages.len() >= self.page_limit {
            return None;
        }
        self.pages.push(Box::new([0; 4096]));
        let phys = PHYS_BASE + (self.pages.len() as u64 - 1) * 4096;
        Some((phys, self.pages.last_mut().unwrap().as_mut_ptr()))
    }
    fn delay_us(&mut self, us: u32) {
        self.delays_us += us as u64;
    }
    fn dma_sync(&mut self, phys: u64, _ptr: *mut u8, len: usize) {
        self.syncs += 1;
        self.synced_bytes += len as u64;
        let (lo, hi) = (phys, phys + len as u64);
        let hidden = core::mem::take(&mut self.hidden);
        for (at, data) in hidden {
            let end = at + data.len() as u64;
            if at >= lo && end <= hi {
                self.mem(at, data.len()).copy_from_slice(&data);
            } else {
                assert!(end <= lo || at >= hi, "dma_sync covers only part of a frame");
                self.hidden.push((at, data));
            }
        }
    }
}

/// A test frame: destination, our MAC as source, type 0x0800, `payload_len` bytes of a pattern.
pub fn frame(dst: [u8; 6], payload_len: usize, seed: u8) -> Vec<u8> {
    let mut f = vec![0u8; 14 + payload_len];
    f[..6].copy_from_slice(&dst);
    f[6..12].copy_from_slice(&[2, 0, 0, 0, 0, 1]);
    f[12..14].copy_from_slice(&[0x08, 0x00]);
    for (i, b) in f[14..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31).wrapping_add(seed);
    }
    f
}
