//! AHCI (SATA) disks, read only, one command at a time with polling.
//! Written from the Serial ATA AHCI 1.3.1 specification (HBA and port registers, command list,
//! command table, PRDT) and the ATA command set (IDENTIFY DEVICE, READ DMA EXT).

use crate::block::{BlockDevice, BlockError, SECTOR};
use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};

// HBA registers
const CAP: u64 = 0x00;
const GHC: u64 = 0x04;
const PI: u64 = 0x0C;
const CAP2: u64 = 0x24;
const BOHC: u64 = 0x28;
// Port registers (offset from the port's base)
const P_CLB: u64 = 0x00;
const P_CLBU: u64 = 0x04;
const P_FB: u64 = 0x08;
const P_FBU: u64 = 0x0C;
const P_IS: u64 = 0x10;
const P_CMD: u64 = 0x18;
const P_TFD: u64 = 0x20;
const P_SIG: u64 = 0x24;
const P_SSTS: u64 = 0x28;
const P_SERR: u64 = 0x30;
const P_CI: u64 = 0x38;

const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const SATA_DISK: u32 = 0x0000_0101;
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
/// Sectors per command: one 4 KiB DMA buffer.
const MAX_SECTORS: u64 = PAGE_SIZE / SECTOR as u64;
const SPIN_LIMIT: u32 = 10_000_000;

struct Port {
    regs: u64,
    /// Command list (1 KiB) at offset 0 and received FIS (256 B) at offset 0x400 of this frame.
    lists: u64,
    /// Command table (header + one PRDT entry).
    table: u64,
    /// DMA buffer for one command.
    buffer: u64,
}

/// One SATA disk on an AHCI controller.
pub struct AhciDisk {
    port: crate::sched::Mutex<Port>,
    sectors: u64,
    pub model: alloc::string::String,
}

fn rd(addr: u64) -> u32 {
    // SAFETY: `addr` is a register of the mapped (uncached) AHCI register block.
    unsafe { (addr as *const u32).read_volatile() }
}

fn wr(addr: u64, v: u32) {
    // SAFETY: as in `rd`.
    unsafe { (addr as *mut u32).write_volatile(v) }
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

/// A zeroed frame for DMA; the controller must be able to reach it (below 4 GiB unless it has 64-bit DMA).
fn dma_frame(s64a: bool) -> Option<u64> {
    let f = frame::alloc_frame()?;
    if !s64a && f >= 1 << 32 {
        frame::free_frame(f);
        return None;
    }
    // SAFETY: fresh frame, reachable through the HHDM.
    unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, PAGE_SIZE as usize) };
    Some(f)
}

impl Port {
    /// Runs one ATA command that moves `count` sectors (<= MAX_SECTORS) between the disk and the DMA
    /// buffer (to the disk if `write`).
    fn command(&mut self, ata: u8, lba: u64, count: u64, write: bool) -> Result<(), BlockError> {
        let p = self.regs;
        if !wait(|| rd(p + P_TFD) & 0x88 == 0) {
            return Err(BlockError::OutOfRange); // still busy from before
        }
        let bytes = count * SECTOR as u64;
        // Command header 0: FIS of 5 dwords, W (bit 6) = direction, one PRDT entry.
        let header = phys_to_virt(self.lists) as *mut u32;
        // Command table: command FIS at 0, PRDT at 0x80.
        let table = phys_to_virt(self.table);
        let mut fis = [0u8; 20];
        fis[0] = 0x27; // register FIS, host to device
        fis[1] = 0x80; // this is a command
        fis[2] = ata;
        fis[4] = lba as u8;
        fis[5] = (lba >> 8) as u8;
        fis[6] = (lba >> 16) as u8;
        fis[7] = 1 << 6; // LBA mode
        fis[8] = (lba >> 24) as u8;
        fis[9] = (lba >> 32) as u8;
        fis[10] = (lba >> 40) as u8;
        fis[12] = count as u8;
        fis[13] = (count >> 8) as u8;
        // SAFETY: the command list and table frames belong to this port and are not in use by the
        // controller (CI is clear); offsets are inside the frames.
        unsafe {
            header.write_volatile(5 | (write as u32) << 6 | (1 << 16));
            header.add(1).write_volatile(0); // bytes transferred
            header.add(2).write_volatile(self.table as u32);
            header.add(3).write_volatile((self.table >> 32) as u32);
            core::ptr::copy_nonoverlapping(fis.as_ptr(), table, fis.len());
            let prdt = table.add(0x80) as *mut u32;
            prdt.write_volatile(self.buffer as u32);
            prdt.add(1).write_volatile((self.buffer >> 32) as u32);
            prdt.add(2).write_volatile(0);
            prdt.add(3).write_volatile((bytes - 1) as u32);
        }
        wr(p + P_IS, u32::MAX);
        wr(p + P_CI, 1);
        let finished = wait(|| rd(p + P_CI) & 1 == 0 || rd(p + P_IS) & (1 << 30) != 0);
        if !finished || rd(p + P_IS) & (1 << 30) != 0 || rd(p + P_TFD) & 1 != 0 {
            crate::warn!("ahci: comanda {:#x} la LBA {} a esuat (TFD={:#x})", ata, lba, rd(p + P_TFD));
            return Err(BlockError::OutOfRange);
        }
        Ok(())
    }

    fn buffer(&self) -> &[u8] {
        // SAFETY: the DMA buffer frame belongs to this port; the controller finished writing it.
        unsafe { core::slice::from_raw_parts(phys_to_virt(self.buffer), PAGE_SIZE as usize) }
    }

    fn buffer_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above; no command is running, so the controller does not read it now.
        unsafe { core::slice::from_raw_parts_mut(phys_to_virt(self.buffer), PAGE_SIZE as usize) }
    }
}

impl BlockDevice for AhciDisk {
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
        let mut port = self.port.lock();
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_SECTORS);
            port.command(ATA_READ_DMA_EXT, lba + done, n, false)?;
            let bytes = (n as usize) * SECTOR;
            buf[done as usize * SECTOR..done as usize * SECTOR + bytes].copy_from_slice(&port.buffer()[..bytes]);
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
        let mut port = self.port.lock();
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_SECTORS);
            let bytes = (n as usize) * SECTOR;
            port.buffer_mut()[..bytes].copy_from_slice(&buf[done as usize * SECTOR..done as usize * SECTOR + bytes]);
            port.command(ATA_WRITE_DMA_EXT, lba + done, n, true)?;
            done += n;
        }
        Ok(())
    }
}

/// Finds the AHCI controller and returns its first SATA disk, ready to read.
pub fn init() -> Option<AhciDisk> {
    let dev = crate::pci::find(0x01, 0x06, 0x01)?;
    dev.enable();
    let Some(abar) = dev.bar(5) else {
        crate::warn!("ahci: controlerul nu are registre (BAR5)");
        return None;
    };
    let hba = paging::map_mmio(abar, 0x1100).ok()?;
    // Take the controller from the firmware, if it asks for that (BIOS/OS handoff).
    if rd(hba + CAP2) & 1 != 0 {
        wr(hba + BOHC, rd(hba + BOHC) | 1 << 1);
        wait(|| rd(hba + BOHC) & 1 == 0);
    }
    wr(hba + GHC, rd(hba + GHC) | 1 << 31); // AHCI mode
    let s64a = rd(hba + CAP) & (1 << 31) != 0;
    let implemented = rd(hba + PI);
    for i in 0..32 {
        if implemented & (1 << i) == 0 {
            continue;
        }
        let p = hba + 0x100 + i as u64 * 0x80;
        let ssts = rd(p + P_SSTS);
        if ssts & 0xF != 3 || (ssts >> 8) & 0xF != 1 || rd(p + P_SIG) != SATA_DISK {
            continue; // no disk (or not an ATA disk) on this port
        }
        match start_port(p, s64a) {
            Some(disk) => {
                crate::info!("ahci: disc SATA pe portul {}: {} ({} MiB)", i, disk.model, disk.sectors / 2048);
                return Some(disk);
            }
            None => crate::warn!("ahci: portul {} nu porneste", i),
        }
    }
    crate::info!("ahci: niciun disc SATA");
    None
}

fn start_port(p: u64, s64a: bool) -> Option<AhciDisk> {
    // Stop the port before giving it our memory.
    wr(p + P_CMD, rd(p + P_CMD) & !CMD_ST);
    if !wait(|| rd(p + P_CMD) & CMD_CR == 0) {
        return None;
    }
    wr(p + P_CMD, rd(p + P_CMD) & !CMD_FRE);
    if !wait(|| rd(p + P_CMD) & CMD_FR == 0) {
        return None;
    }
    let (lists, table, buffer) = (dma_frame(s64a)?, dma_frame(s64a)?, dma_frame(s64a)?);
    wr(p + P_CLB, lists as u32);
    wr(p + P_CLBU, (lists >> 32) as u32);
    wr(p + P_FB, (lists + 0x400) as u32);
    wr(p + P_FBU, ((lists + 0x400) >> 32) as u32);
    wr(p + P_SERR, u32::MAX);
    wr(p + P_IS, u32::MAX);
    wr(p + P_CMD, rd(p + P_CMD) | CMD_FRE);
    wr(p + P_CMD, rd(p + P_CMD) | CMD_ST);

    let mut port = Port { regs: p, lists, table, buffer };
    port.command(ATA_IDENTIFY, 0, 1, false).ok()?;
    let id = port.buffer();
    let word = |i: usize| u16::from_le_bytes([id[i * 2], id[i * 2 + 1]]);
    let sectors = (0..4).fold(0u64, |acc, k| acc | (word(100 + k) as u64) << (16 * k));
    // Model name: words 27..47, two ASCII characters per word, high byte first.
    let mut model = alloc::string::String::new();
    for w in 27..47 {
        let v = word(w);
        model.push((v >> 8) as u8 as char);
        model.push(v as u8 as char);
    }
    let model = alloc::string::String::from(model.trim());
    Some(AhciDisk { port: crate::sched::Mutex::new(port), sectors, model })
}
