//! USB sticks and card readers: Mass Storage class, Bulk-Only Transport, SCSI commands (INQUIRY,
//! READ CAPACITY(10), READ(10), WRITE(10)). Written from the USB Mass Storage Class Bulk-Only Transport
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
    /// Runs one SCSI command with an optional data stage of `len` bytes: into `self.data`, or from it
    /// to the device if `out`.
    fn scsi(&mut self, x: &mut Xhci, cb: &[u8], len: u32, out: bool) -> Result<(), BlockError> {
        self.tag = self.tag.wrapping_add(1);
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        cbw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&len.to_le_bytes());
        cbw[12] = if out { 0x00 } else { 0x80 }; // direction of the data stage
        cbw[14] = cb.len() as u8;
        cbw[15..15 + cb.len()].copy_from_slice(cb);
        // SAFETY: the command frame belongs to this driver and is not in use by the controller.
        unsafe { core::ptr::copy_nonoverlapping(cbw.as_ptr(), phys_to_virt(self.cmd), cbw.len()) };
        x.queue(self.slot, self.dci_out, &mut self.ring_out, self.cmd, 31);
        x.wait_transfer(self.slot, self.dci_out).ok_or(BlockError::OutOfRange)?;
        if len > 0 && out {
            x.queue(self.slot, self.dci_out, &mut self.ring_out, self.data, len);
            x.wait_transfer(self.slot, self.dci_out).ok_or(BlockError::OutOfRange)?;
        } else if len > 0 {
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

    fn data_mut(&mut self, len: usize) -> &mut [u8] {
        // SAFETY: as above; no transfer is running, so the controller does not read it now.
        unsafe { core::slice::from_raw_parts_mut(phys_to_virt(self.data), len) }
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
        bot.scsi(x, &[0x12, 0, 0, 0, 36, 0], 36, false).ok()?; // INQUIRY
        let inquiry = bot.data(36);
        let name = alloc::format!(
            "{} {}",
            core::str::from_utf8(&inquiry[8..16]).unwrap_or("?").trim(),
            core::str::from_utf8(&inquiry[16..32]).unwrap_or("?").trim()
        );
        // Real sticks often answer the first commands with "unit attention" or "becoming ready": each failure is
        // followed by REQUEST SENSE (which clears that state, SCSI SPC), then TEST UNIT READY / READ CAPACITY are
        // retried for up to ~3 s.
        let mut capacity = None;
        let mut sense = [0u8; 3];
        for attempt in 0..30 {
            let ready = bot.scsi(x, &[0x00, 0, 0, 0, 0, 0], 0, false).is_ok(); // TEST UNIT READY
            if ready && bot.scsi(x, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, false).is_ok() {
                capacity = Some(bot.data(8).to_vec());
                break;
            }
            if bot.scsi(x, &[0x03, 0, 0, 0, 18, 0], 18, false).is_ok() {
                let d = bot.data(18);
                sense = [d[2] & 0x0F, d[12], d[13]]; // sense key, ASC, ASCQ
            }
            crate::sched::sleep_ms(if attempt < 5 { 20 } else { 100 });
        }
        let Some(c) = capacity else {
            let why = alloc::format!("stick-ul {} nu e gata (sense {:x}/{:02x}/{:02x})", name, sense[0], sense[1], sense[2]);
            crate::warn!("usb: {}", why);
            crate::fs::note_failure("/usb", &why);
            return None;
        };
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
            super::with_controller(|x| bot.scsi(x, &cmd, n * SECTOR as u32, false)).ok_or(BlockError::OutOfRange)??;
            let bytes = n as usize * SECTOR;
            buf[done as usize * SECTOR..done as usize * SECTOR + bytes].copy_from_slice(bot.data(bytes));
            done += n as u64;
        }
        Ok(())
    }

    fn write(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
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
            let bytes = n as usize * SECTOR;
            bot.data_mut(bytes).copy_from_slice(&buf[done as usize * SECTOR..done as usize * SECTOR + bytes]);
            let s = ((lba + done) as u32).to_be_bytes();
            let cmd = [0x2A, 0, s[0], s[1], s[2], s[3], 0, (n >> 8) as u8, n as u8, 0]; // WRITE(10)
            super::with_controller(|x| bot.scsi(x, &cmd, n * SECTOR as u32, true)).ok_or(BlockError::OutOfRange)??;
            done += n as u64;
        }
        Ok(())
    }

    /// SYNCHRONIZE CACHE(10) for the whole stick (SBC-3): writes held in the stick's memory go to its flash.
    fn flush(&self) -> Result<(), BlockError> {
        let mut bot = self.bot.lock();
        let cmd = [0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        super::with_controller(|x| bot.scsi(x, &cmd, 0, false)).ok_or(BlockError::OutOfRange)?
    }
}
