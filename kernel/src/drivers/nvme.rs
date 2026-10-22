//! NVMe SSDs, read only: admin queue, one I/O queue pair, one command at a time with polling.
//! Written from the NVM Express Base Specification (controller registers, queues, Identify,
//! Create I/O Queue, Read).

use crate::block::{BlockDevice, BlockError, SECTOR};
use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};

const REG_CAP: u64 = 0x00;
const REG_CC: u64 = 0x14;
const REG_CSTS: u64 = 0x1C;
const REG_AQA: u64 = 0x24;
const REG_ASQ: u64 = 0x28;
const REG_ACQ: u64 = 0x30;

const QUEUE_SIZE: u16 = 16;
const MAX_SECTORS: u64 = PAGE_SIZE / SECTOR as u64;
const SPIN_LIMIT: u32 = 20_000_000;

const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const IO_WRITE: u8 = 0x01;
const IO_READ: u8 = 0x02;

/// A submission queue and its completion queue.
struct Queue {
    id: u16,
    sq: u64, // physical
    cq: u64, // physical
    tail: u16,
    head: u16,
    phase: bool,
}

struct Controller {
    regs: u64,
    stride: u64,
    admin: Queue,
    io: Queue,
    buffer: u64,
    next_cid: u16,
}

pub struct NvmeDisk {
    ctrl: crate::sched::Mutex<Controller>,
    sectors: u64,
}

fn rd32(a: u64) -> u32 {
    // SAFETY: `a` is a register of the mapped (uncached) NVMe register block.
    unsafe { (a as *const u32).read_volatile() }
}
fn wr32(a: u64, v: u32) {
    // SAFETY: as in `rd32`.
    unsafe { (a as *mut u32).write_volatile(v) }
}
fn rd64(a: u64) -> u64 {
    rd32(a) as u64 | (rd32(a + 4) as u64) << 32
}
fn wr64(a: u64, v: u64) {
    wr32(a, v as u32);
    wr32(a + 4, (v >> 32) as u32);
}

fn zeroed_frame() -> Option<u64> {
    let f = frame::alloc_frame()?;
    frame::pin(f, 1, "nvme"); // the card writes here by DMA: never given to anyone else
    // SAFETY: fresh frame, reachable through the HHDM.
    unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, PAGE_SIZE as usize) };
    Some(f)
}

fn wait(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..SPIN_LIMIT {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

impl Queue {
    fn new(id: u16) -> Option<Queue> {
        Some(Queue { id, sq: zeroed_frame()?, cq: zeroed_frame()?, tail: 0, head: 0, phase: true })
    }
}

impl Controller {
    fn doorbell(&self, queue: u16, completion: bool) -> u64 {
        self.regs + 0x1000 + (2 * queue as u64 + completion as u64) * self.stride
    }

    /// Submits a 64-byte command on the admin queue (`io` = false) or the I/O queue and waits for it.
    /// `cdw` are command dwords 10..15. Returns the status field (0 = success).
    fn run(&mut self, io: bool, opcode: u8, nsid: u32, prp1: u64, cdw: [u32; 6]) -> Result<(), u16> {
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1);
        let mut cmd = [0u32; 16];
        cmd[0] = opcode as u32 | (cid as u32) << 16;
        cmd[1] = nsid;
        cmd[6] = prp1 as u32;
        cmd[7] = (prp1 >> 32) as u32;
        cmd[10..16].copy_from_slice(&cdw);
        let q = if io { &mut self.io } else { &mut self.admin };
        // SAFETY: the queue frames belong to this controller; slot `tail` is free (one command at a time).
        unsafe {
            let slot = (phys_to_virt(q.sq) as *mut u32).add(q.tail as usize * 16);
            for (i, w) in cmd.iter().enumerate() {
                slot.add(i).write_volatile(*w);
            }
        }
        q.tail = (q.tail + 1) % QUEUE_SIZE;
        let (id, tail) = (q.id, q.tail);
        wr32(self.doorbell(id, false), tail as u32);

        let q = if io { &mut self.io } else { &mut self.admin };
        // SAFETY: completion entries are 16 bytes; slot `head` is inside the completion-queue frame.
        let entry = unsafe { (phys_to_virt(q.cq) as *const u32).add(q.head as usize * 4) };
        let phase = q.phase;
        // SAFETY: as above.
        let ok = wait(|| unsafe { (entry.add(3).read_volatile() >> 16) & 1 == phase as u32 });
        if !ok {
            return Err(0xFFFF);
        }
        // SAFETY: as above.
        let status = unsafe { (entry.add(3).read_volatile() >> 17) as u16 };
        q.head = (q.head + 1) % QUEUE_SIZE;
        if q.head == 0 {
            q.phase = !q.phase;
        }
        let (id, head) = (q.id, q.head);
        wr32(self.doorbell(id, true), head as u32);
        if status == 0 { Ok(()) } else { Err(status) }
    }

    fn buffer(&self) -> &[u8] {
        // SAFETY: the buffer frame belongs to this controller; the last command finished writing it.
        unsafe { core::slice::from_raw_parts(phys_to_virt(self.buffer), PAGE_SIZE as usize) }
    }

    fn buffer_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above; no command is running, so the controller does not read it now.
        unsafe { core::slice::from_raw_parts_mut(phys_to_virt(self.buffer), PAGE_SIZE as usize) }
    }
}

impl BlockDevice for NvmeDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() % SECTOR != 0 {
            return Err(BlockError::BadBuffer);
        }
        let total = (buf.len() / SECTOR) as u64;
        if lba.checked_add(total).is_none_or(|e| e > self.sectors) {
            return Err(BlockError::OutOfRange);
        }
        let mut c = self.ctrl.lock();
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_SECTORS);
            let start = lba + done;
            let buffer = c.buffer;
            if let Err(status) = c.run(true, IO_READ, 1, buffer, [start as u32, (start >> 32) as u32, (n - 1) as u32, 0, 0, 0]) {
                crate::warn!("nvme: citirea la LBA {} a esuat (stare {:#x})", start, status);
                return Err(BlockError::OutOfRange);
            }
            let bytes = n as usize * SECTOR;
            buf[done as usize * SECTOR..done as usize * SECTOR + bytes].copy_from_slice(&c.buffer()[..bytes]);
            done += n;
        }
        Ok(())
    }

    fn write(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        if buf.len() % SECTOR != 0 {
            return Err(BlockError::BadBuffer);
        }
        let total = (buf.len() / SECTOR) as u64;
        if lba.checked_add(total).is_none_or(|e| e > self.sectors) {
            return Err(BlockError::OutOfRange);
        }
        let mut c = self.ctrl.lock();
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_SECTORS);
            let start = lba + done;
            let bytes = n as usize * SECTOR;
            c.buffer_mut()[..bytes].copy_from_slice(&buf[done as usize * SECTOR..done as usize * SECTOR + bytes]);
            let buffer = c.buffer;
            if let Err(status) = c.run(true, IO_WRITE, 1, buffer, [start as u32, (start >> 32) as u32, (n - 1) as u32, 0, 0, 0]) {
                crate::warn!("nvme: scrierea la LBA {} a esuat (stare {:#x})", start, status);
                return Err(BlockError::OutOfRange);
            }
            done += n;
        }
        Ok(())
    }
}

/// Finds the first NVMe controller, starts it and returns namespace 1 as a disk.
pub fn init() -> Option<NvmeDisk> {
    let dev = crate::pci::find(0x01, 0x08, 0x02)?;
    dev.enable();
    let regs = paging::map_mmio(dev.bar(0)?, 0x2000).ok()?;
    let cap = rd64(regs + REG_CAP);
    let stride = 4u64 << ((cap >> 32) & 0xF);
    if (cap >> 48) & 0xF != 0 {
        crate::warn!("nvme: controlerul nu accepta pagini de 4 KiB");
        return None;
    }
    // Reset, give it the admin queues, start it.
    wr32(regs + REG_CC, rd32(regs + REG_CC) & !1);
    if !wait(|| rd32(regs + REG_CSTS) & 1 == 0) {
        crate::warn!("nvme: controlerul nu se opreste");
        return None;
    }
    let mut ctrl = Controller { regs, stride, admin: Queue::new(0)?, io: Queue::new(1)?, buffer: zeroed_frame()?, next_cid: 0 };
    let size = (QUEUE_SIZE - 1) as u32;
    wr32(regs + REG_AQA, size << 16 | size);
    wr64(regs + REG_ASQ, ctrl.admin.sq);
    wr64(regs + REG_ACQ, ctrl.admin.cq);
    wr32(regs + REG_CC, 1 | (6 << 16) | (4 << 20)); // enable; SQ entries 64 B, CQ entries 16 B; 4 KiB pages
    if !wait(|| rd32(regs + REG_CSTS) & 0b11 == 1) {
        crate::warn!("nvme: controlerul nu porneste (CSTS={:#x})", rd32(regs + REG_CSTS));
        return None;
    }
    let buffer = ctrl.buffer;
    ctrl.run(false, ADMIN_IDENTIFY, 0, buffer, [1, 0, 0, 0, 0, 0]).ok()?; // controller
    let model = alloc::string::String::from(core::str::from_utf8(&ctrl.buffer()[24..64]).unwrap_or("?").trim());
    ctrl.run(false, ADMIN_IDENTIFY, 1, buffer, [0, 0, 0, 0, 0, 0]).ok()?; // namespace 1
    let ns = ctrl.buffer();
    let sectors = u64::from_le_bytes(ns[0..8].try_into().unwrap());
    let format = (ns[26] & 0xF) as usize;
    let lbads = ns[128 + format * 4 + 2];
    if lbads != 9 {
        crate::warn!("nvme: sectoare de {} octeti, doar 512 sunt suportate", 1u64 << lbads);
        return None;
    }
    let (cq, sq) = (ctrl.io.cq, ctrl.io.sq);
    ctrl.run(false, ADMIN_CREATE_CQ, 0, cq, [size << 16 | 1, 1, 0, 0, 0, 0]).ok()?;
    ctrl.run(false, ADMIN_CREATE_SQ, 0, sq, [size << 16 | 1, 1 << 16 | 1, 0, 0, 0, 0]).ok()?;
    crate::info!("nvme: disc {} ({} MiB)", model, sectors / 2048);
    Some(NvmeDisk { ctrl: crate::sched::Mutex::new(ctrl), sectors })
}
