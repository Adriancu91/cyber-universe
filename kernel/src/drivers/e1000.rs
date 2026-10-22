//! Intel 8254x (e1000) and 82574 (e1000e) network cards: sends and receives raw Ethernet frames with
//! the legacy descriptor rings, by polling. Written from the Intel PCI/PCI-X Family of Gigabit Ethernet
//! Controllers Software Developer's Manual (8254x) and the 82574 datasheet (public documents).

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;

const CTRL: u64 = 0x0000;
const IMC: u64 = 0x00D8;
const RCTL: u64 = 0x0100;
const TCTL: u64 = 0x0400;
const TIPG: u64 = 0x0410;
const RDBAL: u64 = 0x2800;
const RDBAH: u64 = 0x2804;
const RDLEN: u64 = 0x2808;
const RDH: u64 = 0x2810;
const RDT: u64 = 0x2818;
const TDBAL: u64 = 0x3800;
const TDBAH: u64 = 0x3804;
const TDLEN: u64 = 0x3808;
const TDH: u64 = 0x3810;
const TDT: u64 = 0x3818;
const MTA: u64 = 0x5200;
const RAL0: u64 = 0x5400;
const RAH0: u64 = 0x5404;

/// Descriptors per ring (16 bytes each: 32 fit in 512 bytes) and buffer size (2 per frame).
const RING: usize = 32;
const BUF: usize = 2048;

/// Cards this driver knows: (device id, name). Vendor is always Intel (0x8086).
const SUPPORTED: [(u16, &str); 3] = [(0x100E, "82540EM (e1000)"), (0x100F, "82545EM (e1000)"), (0x10D3, "82574L (e1000e)")];

/// True if this driver knows the card.
pub fn supports(vendor: u16, device: u16) -> bool {
    vendor == 0x8086 && SUPPORTED.iter().any(|(id, _)| *id == device)
}

pub struct Nic {
    regs: u64,
    pub mac: [u8; 6],
    pub name: &'static str,
    rx_ring: u64,
    tx_ring: u64,
    rx_bufs: [u64; RING],
    tx_bufs: [u64; RING],
    rx_next: usize,
    tx_next: usize,
    pub sent: u64,
    pub received: u64,
}

fn rd(a: u64) -> u32 {
    // SAFETY: `a` is a register of the mapped (uncached) card register block.
    unsafe { (a as *const u32).read_volatile() }
}
fn wr(a: u64, v: u32) {
    // SAFETY: as in `rd`.
    unsafe { (a as *mut u32).write_volatile(v) }
}

fn zeroed_frame() -> Option<u64> {
    let f = frame::alloc_frame()?;
    // SAFETY: fresh frame, reachable through the HHDM.
    unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, PAGE_SIZE as usize) };
    Some(f)
}

/// One descriptor (16 bytes) of a ring, as two 64-bit words.
fn desc(ring: u64, i: usize) -> *mut u64 {
    // SAFETY (for users): `ring` is a descriptor frame owned by the driver; i < RING.
    unsafe { (phys_to_virt(ring) as *mut u64).add(i * 2) }
}

/// Buffers for `RING` packets: two 2 KiB buffers per frame.
fn buffers() -> Option<[u64; RING]> {
    let mut bufs = [0u64; RING];
    for pair in bufs.chunks_mut(2) {
        let f = zeroed_frame()?;
        pair[0] = f;
        pair[1] = f + BUF as u64;
    }
    Some(bufs)
}

impl Nic {
    /// Sends one Ethernet frame (destination, source, type, payload; at most 1514 bytes).
    pub fn send(&mut self, packet: &[u8]) -> bool {
        if packet.len() > 1514 {
            return false;
        }
        let i = self.tx_next;
        let len = packet.len().max(60); // shorter frames are padded with zeros
        // SAFETY: the buffer and descriptor belong to this driver; the card finished with slot i
        // (checked below before reuse: its "descriptor done" bit is set or it was never used).
        unsafe {
            let buf = phys_to_virt(self.tx_bufs[i]);
            core::ptr::write_bytes(buf, 0, 60);
            core::ptr::copy_nonoverlapping(packet.as_ptr(), buf, packet.len());
            let d = desc(self.tx_ring, i);
            d.write_volatile(self.tx_bufs[i]);
            // length | CMD: EOP (end of packet), IFCS (add the checksum), RS (report status)
            d.add(1).write_volatile(len as u64 | (0b1011u64 << 24));
        }
        self.tx_next = (i + 1) % RING;
        wr(self.regs + TDT, self.tx_next as u32);
        // SAFETY: as above; the status byte is at offset 12 of the descriptor.
        let done = (0..1_000_000).any(|_| unsafe { (desc(self.tx_ring, i).add(1).read_volatile() >> 32) & 1 != 0 });
        if done {
            self.sent += 1;
        }
        done
    }

    /// The next received frame, if one has arrived.
    pub fn receive(&mut self) -> Option<Vec<u8>> {
        let i = self.rx_next;
        // SAFETY: descriptor of this driver's ring; word 1 = length (bits 0-15), status (bits 32-39).
        let w = unsafe { desc(self.rx_ring, i).add(1).read_volatile() };
        if (w >> 32) & 1 == 0 {
            return None; // descriptor not done
        }
        let len = (w & 0xFFFF) as usize;
        // SAFETY: the card wrote `len` <= 2048 bytes into this buffer.
        let data = unsafe { core::slice::from_raw_parts(phys_to_virt(self.rx_bufs[i]), len.min(BUF)) }.to_vec();
        // SAFETY: give the descriptor back: clear its status, keep the buffer address.
        unsafe { desc(self.rx_ring, i).add(1).write_volatile(0) };
        wr(self.regs + RDT, i as u32);
        self.rx_next = (i + 1) % RING;
        self.received += 1;
        Some(data)
    }
}

impl super::nic::Card for Nic {
    fn send(&mut self, frame: &[u8]) -> bool {
        Nic::send(self, frame)
    }
    fn receive(&mut self) -> Option<Vec<u8>> {
        Nic::receive(self)
    }
    fn mac(&self) -> [u8; 6] {
        self.mac
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn counters(&self) -> (u64, u64) {
        (self.sent, self.received)
    }
}

/// Finds a supported Intel card and starts it (link up, receive and transmit rings).
pub fn init() -> Option<Nic> {
    let (dev, name) = crate::pci::devices().into_iter().find_map(|d| {
        if d.vendor != 0x8086 {
            return None;
        }
        SUPPORTED.iter().find(|(id, _)| *id == d.device).map(|(_, n)| (d, *n))
    })?;
    dev.enable();
    let regs = paging::map_mmio(dev.bar(0)?, 0x20000).ok()?;

    wr(regs + IMC, u32::MAX); // no interrupts (polling)
    wr(regs + CTRL, rd(regs + CTRL) | 1 << 26); // reset
    for _ in 0..1_000_000 {
        if rd(regs + CTRL) & (1 << 26) == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    wr(regs + IMC, u32::MAX);
    wr(regs + CTRL, rd(regs + CTRL) | 1 << 6 | 1 << 5); // set link up, auto speed detection

    let (ral, rah) = (rd(regs + RAL0), rd(regs + RAH0));
    if rah & (1 << 31) == 0 {
        crate::warn!("net: placa {} nu are adresa MAC", name);
        return None;
    }
    let mac = [ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8, rah as u8, (rah >> 8) as u8];
    for i in 0..128 {
        wr(regs + MTA + i * 4, 0);
    }

    let (rx_ring, tx_ring) = (zeroed_frame()?, zeroed_frame()?);
    let (rx_bufs, tx_bufs) = (buffers()?, buffers()?);
    for (i, &b) in rx_bufs.iter().enumerate() {
        // SAFETY: descriptor frame owned by the driver; word 0 = buffer address.
        unsafe { desc(rx_ring, i).write_volatile(b) };
    }
    let ring_bytes = (RING * 16) as u32;
    wr(regs + RDBAL, rx_ring as u32);
    wr(regs + RDBAH, (rx_ring >> 32) as u32);
    wr(regs + RDLEN, ring_bytes);
    wr(regs + RDH, 0);
    wr(regs + RDT, (RING - 1) as u32);
    wr(regs + RCTL, 1 << 1 | 1 << 15 | 1 << 26); // enable, accept broadcast, strip CRC; 2 KiB buffers

    wr(regs + TDBAL, tx_ring as u32);
    wr(regs + TDBAH, (tx_ring >> 32) as u32);
    wr(regs + TDLEN, ring_bytes);
    wr(regs + TDH, 0);
    wr(regs + TDT, 0);
    wr(regs + TCTL, 1 << 1 | 1 << 3 | 0x10 << 4 | 0x40 << 12); // enable, pad short packets, collision settings
    wr(regs + TIPG, 0x0060_200A);

    crate::info!(
        "net: placa {} pornita, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        name, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    crate::info!("net: e1000 inele RX {:#x} TX {:#x}, primul buffer RX {:#x}", rx_ring, tx_ring, rx_bufs[0]);
    Some(Nic { regs, mac, name, rx_ring, tx_ring, rx_bufs, tx_bufs, rx_next: 0, tx_next: 0, sent: 0, received: 0 })
}
