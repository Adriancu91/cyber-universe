//! USB sticks and card readers: Mass Storage class, Bulk-Only Transport, SCSI commands (INQUIRY,
//! READ CAPACITY(10), READ(10)), read only. Written from the USB Mass Storage Class Bulk-Only Transport
//! 1.0 specification and the public SCSI Primary/Block Commands descriptions.

use super::xhci::{BULK, Ring, UsbDevice, Xhci};
use crate::block::{BlockDevice, BlockError, SECTOR};
use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}};

const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC"
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS"
const MAX_SECTORS: u32 = (PAGE_SIZE / SECTOR as u64) as u32;

struct Bot {
    slot: u8,
    dci_in: u8,
    dci_out: u8,
    ring_in: Ring,
    ring_out: Ring,
    /// Command/status block (first 64 bytes) and data buffer (the rest is unused).
    cmd: u64,
    data: u64,
    tag: u32,
}

pub struct UsbDisk {
    bot: crate::sched::Mutex<Bot>,
    sectors: u64,
}

impl Bot {
    /// Runs one SCSI command with an optional IN data stage of `len` bytes (into `self.data`).
    fn scsi(&mut self, x: &mut Xhci, cb: &[u8], len: u32) -> Result<(), BlockError> {
        self.tag = self.tag.wrapping_add(1);
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        cbw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&len.to_le_bytes());
        cbw[12] = 0x80; // data goes to the host
        cbw[14] = cb.len() as u8;
        cbw[15..15 + cb.len()].copy_from_slice(cb);
        // SAFETY: the command frame belongs to this driver and is not in use by the controller.
        unsafe { core::ptr::copy_nonoverlapping(cbw.as_ptr(), phys_to_virt(self.cmd), cbw.len()) };
        x.queue(self.slot, self.dci_out, &mut self.ring_out, self.cmd, 31);
        x.wait_transfer(self.slot, self.dci_out).ok_or(BlockError::OutOfRange)?;
        if len > 0 {
            x.queue(self.slot, self.dci_in, &mut self.ring_in, self.data, len);
            x.wait_transfer(self.slot, self.dci_in).ok_or(BlockError::OutOfRange)?;
        }
        x.queue(self.slot, self.dci_in, &mut self.ring_in, self.cmd + 64, 13);
        x.wait_transfer(self.slot, self.dci_in).ok_or(BlockError::OutOfRange)?;
        // SAFETY: as above; the status (CSW) was written 64 bytes into the frame.
        let csw = unsafe { core::slice::from_raw_parts(phys_to_virt(self.cmd + 64), 13) };
        let ok = u32::from_le_bytes(csw[0..4].try_into().unwrap()) == CSW_SIGNATURE
            && u32::from_le_bytes(csw[4..8].try_into().unwrap()) == self.tag
            && csw[12] == 0;
        if ok { Ok(()) } else { Err(BlockError::OutOfRange) }
    }

    fn data(&self, len: usize) -> &[u8] {
        // SAFETY: the data frame belongs to this driver; the last transfer finished.
        unsafe { core::slice::from_raw_parts(phys_to_virt(self.data), len) }
    }
}

impl UsbDisk {
    /// Configures `dev` (a Bulk-Only mass-storage device) and reads its size.
    pub fn start(x: &mut Xhci, dev: &UsbDevice) -> Option<UsbDisk> {
        x.control_out(dev.slot, [0x00, 9, dev.config, 0, 0, 0, 0, 0]).ok()?; // SET_CONFIGURATION
        let (ep_in, ep_out) = (dev.endpoint(BULK, true)?, dev.endpoint(BULK, false)?);
        let mut rings = x.configure_endpoints(dev, &[ep_in, ep_out])?;
        let (dci_out, ring_out) = rings.pop()?;
        let (dci_in, ring_in) = rings.pop()?;
        let mut bot = Bot { slot: dev.slot, dci_in, dci_out, ring_in, ring_out, cmd: frame::alloc_frame()?, data: frame::alloc_frame()?, tag: 0 };
        bot.scsi(x, &[0x12, 0, 0, 0, 36, 0], 36).ok()?; // INQUIRY
        let inquiry = bot.data(36);
        let name = alloc::format!(
            "{} {}",
            core::str::from_utf8(&inquiry[8..16]).unwrap_or("?").trim(),
            core::str::from_utf8(&inquiry[16..32]).unwrap_or("?").trim()
        );
        // The device may need a moment after power-up: READ CAPACITY is retried a few times.
        let mut capacity = None;
        for _ in 0..5 {
            if bot.scsi(x, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8).is_ok() {
                capacity = Some(bot.data(8).to_vec());
                break;
            }
            crate::sched::sleep_ms(20);
        }
        let c = capacity?;
        let last = u32::from_be_bytes(c[0..4].try_into().unwrap()) as u64;
        let block = u32::from_be_bytes(c[4..8].try_into().unwrap());
        if block as usize != SECTOR {
            crate::warn!("usb: stick cu blocuri de {} octeti, doar 512 sunt suportate", block);
            return None;
        }
        crate::info!("usb: stocare {} ({} MiB)", name, (last + 1) / 2048);
        Some(UsbDisk { bot: crate::sched::Mutex::new(bot), sectors: last + 1 })
    }
}

impl BlockDevice for UsbDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() % SECTOR != 0 {
            return Err(BlockError::BadBuffer);
        }
        let total = (buf.len() / SECTOR) as u64;
        if lba.checked_add(total).is_none_or(|e| e > self.sectors || e > u32::MAX as u64) {
            return Err(BlockError::OutOfRange);
        }
        let mut bot = self.bot.lock();
        let mut done = 0u64;
        while done < total {
            let n = ((total - done) as u32).min(MAX_SECTORS);
            let start = (lba + done) as u32;
            let s = start.to_be_bytes();
            let cmd = [0x28, 0, s[0], s[1], s[2], s[3], 0, (n >> 8) as u8, n as u8, 0]; // READ(10)
            super::with_controller(|x| bot.scsi(x, &cmd, n * SECTOR as u32)).ok_or(BlockError::OutOfRange)??;
            let bytes = n as usize * SECTOR;
            buf[done as usize * SECTOR..done as usize * SECTOR + bytes].copy_from_slice(bot.data(bytes));
            done += n as u64;
        }
        Ok(())
    }
}
