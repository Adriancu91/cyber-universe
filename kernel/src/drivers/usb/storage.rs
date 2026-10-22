//! USB sticks and card readers: Mass Storage class, Bulk-Only Transport, SCSI commands (INQUIRY,
//! READ CAPACITY(10), READ(10), WRITE(10)). Written from the USB Mass Storage Class Bulk-Only Transport
//! 1.0 specification and the public SCSI Primary/Block Commands descriptions.

use super::xhci::{BULK, CC_SHORT_PACKET, CC_STALL, CC_SUCCESS, CC_TIMEOUT, Ring, UsbDevice, Xhci, cc_name};
use crate::block::{BlockDevice, BlockError, SECTOR};
use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}};

const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC"
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS"
const MAX_SECTORS: u32 = (PAGE_SIZE / SECTOR as u64) as u32;

struct Bot {
    slot: u8,
    port: u8,
    /// Endpoint addresses (for CLEAR_FEATURE after a stall) and their DCIs.
    ep_in: u8,
    ep_out: u8,
    dci_in: u8,
    dci_out: u8,
    ring_in: Ring,
    ring_out: Ring,
    /// Command/status block (first 64 bytes) and data buffer (the rest is unused).
    cmd: u64,
    data: u64,
    tag: u32,
    /// Status byte of the last command's CSW (0 passed, 1 failed, 2 phase error); None if no valid CSW came.
    last_csw: Option<u8>,
}

pub struct UsbDisk {
    bot: crate::sched::Mutex<Bot>,
    sectors: u64,
}

impl Bot {
    /// One bulk transfer of `len` bytes from/to `buf` (physical). A failure is logged with the SCSI command and
    /// the stage; after a STALL the endpoint is cleared so the next command can run (BOT 1.0 §5.3, §6.7).
    fn bulk(&mut self, x: &mut Xhci, input: bool, buf: u64, len: u32, op: u8, stage: &str) -> Result<(), u32> {
        let (dci, ep) = if input { (self.dci_in, self.ep_in) } else { (self.dci_out, self.ep_out) };
        let stale = x.drop_stale(self.slot, dci);
        if stale > 0 {
            crate::warn!("usb: port {}: {} transferuri vechi ignorate (raspunsuri dupa timp)", self.port, stale);
        }
        let ring = if input { &mut self.ring_in } else { &mut self.ring_out };
        x.queue(self.slot, dci, ring, buf, len);
        let code = x.wait_transfer(self.slot, dci).map_or(CC_TIMEOUT, |t| t.2);
        if code == CC_SUCCESS || code == CC_SHORT_PACKET {
            return Ok(());
        }
        crate::warn!(
            "usb: port {}: SCSI {:02x}, {}: cod {} ({})",
            self.port, op, stage, code, cc_name(code)
        );
        if code == CC_STALL {
            let ring = if input { &self.ring_in } else { &self.ring_out };
            if let Err(c) = x.clear_halt(self.slot, dci, ep, ring) {
                crate::warn!("usb: port {}: stergerea STALL a esuat (cod {}: {})", self.port, c, cc_name(c));
            }
        }
        Err(code)
    }

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
        let op = cb[0];
        self.bulk(x, false, self.cmd, 31, op, "comanda (CBW)").map_err(|_| BlockError::OutOfRange)?;
        if len > 0 {
            // A stalled data stage still ends with a status block (BOT 1.0 §6.7.2, §6.7.3).
            let _ = self.bulk(x, !out, self.data, len, op, if out { "date spre stick" } else { "date de la stick" });
        }
        let mut status = self.bulk(x, true, self.cmd + 64, 13, op, "stare (CSW)");
        if status == Err(CC_STALL) {
            status = self.bulk(x, true, self.cmd + 64, 13, op, "stare (CSW), a doua oara");
        }
        status.map_err(|_| BlockError::OutOfRange)?;
        // SAFETY: as above; the status (CSW) was written 64 bytes into the frame.
        let csw = unsafe { core::slice::from_raw_parts(phys_to_virt(self.cmd + 64), 13) };
        let valid = u32::from_le_bytes(csw[0..4].try_into().unwrap()) == CSW_SIGNATURE
            && u32::from_le_bytes(csw[4..8].try_into().unwrap()) == self.tag;
        if !valid {
            crate::warn!("usb: port {}: SCSI {:02x}: stare (CSW) invalida (semnatura sau eticheta gresita)", self.port, op);
            self.last_csw = None;
            return Err(BlockError::OutOfRange);
        }
        self.last_csw = Some(csw[12]);
        if csw[12] == 0 { Ok(()) } else { Err(BlockError::OutOfRange) }
    }

    /// REQUEST SENSE (SPC): (sense key, ASC, ASCQ) of the last failed command; it also clears "unit attention".
    fn request_sense(&mut self, x: &mut Xhci) -> Option<[u8; 3]> {
        self.scsi(x, &[0x03, 0, 0, 0, 18, 0], 18, false).ok()?;
        let d = self.data(18);
        Some([d[2] & 0x0F, d[12], d[13]])
    }

    /// A CSW status and sense data in words, for the log.
    fn why(csw: Option<u8>, sense: Option<[u8; 3]>) -> alloc::string::String {
        let csw = match csw {
            Some(0) => "CSW 0 (reusit)",
            Some(1) => "CSW 1 (comanda esuata)",
            Some(2) => "CSW 2 (eroare de faza)",
            Some(_) => "CSW necunoscut",
            None => "fara CSW",
        };
        match sense {
            Some(k) => alloc::format!("{}, sense {:x}/{:02x}/{:02x}{}", csw, k[0], k[1], k[2], if k[0] == 6 { " (unit attention)" } else { "" }),
            None => alloc::format!("{}, fara sense", csw),
        }
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
        let port = dev.port;
        // SET_CONFIGURATION
        if let Err(c) = x.control_out(dev.slot, [0x00, 9, dev.config, 0, 0, 0, 0, 0]) {
            crate::warn!("usb: port {}: SET_CONFIGURATION {} a esuat (cod {}: {})", port, dev.config, c, cc_name(c));
            return None;
        }
        crate::info!("usb: port {}: SET_CONFIGURATION {} ok", port, dev.config);
        let (Some(ep_in), Some(ep_out)) = (dev.endpoint(BULK, true), dev.endpoint(BULK, false)) else {
            crate::warn!("usb: port {}: stick-ul nu are endpoint-uri bulk IN si OUT", port);
            return None;
        };
        let mut rings = x.configure_endpoints(dev, &[ep_in, ep_out])?;
        let (dci_out, ring_out) = rings.pop()?;
        let (dci_in, ring_in) = rings.pop()?;
        let mut bot = Bot {
            slot: dev.slot, port, ep_in: ep_in.0, ep_out: ep_out.0, dci_in, dci_out, ring_in, ring_out,
            cmd: frame::alloc_frame()?, data: frame::alloc_frame()?, tag: 0, last_csw: None,
        };
        // INQUIRY; right after power-up a real stick may answer "command failed" (CSW 1, unit attention) until a
        // REQUEST SENSE clears it (SPC): up to 10 tries, 100 ms apart.
        let mut inquired = false;
        for attempt in 0..10 {
            if bot.scsi(x, &[0x12, 0, 0, 0, 36, 0], 36, false).is_ok() {
                inquired = true;
                break;
            }
            let csw = bot.last_csw;
            let sense = if csw == Some(1) { bot.request_sense(x) } else { None };
            crate::warn!("usb: port {}: INQUIRY, incercarea {}: {}", port, attempt + 1, Bot::why(csw, sense));
            crate::sched::sleep_ms(100);
        }
        if !inquired {
            crate::fs::note_failure("/usb", "stick-ul nu raspunde la INQUIRY (vezi log usb)");
            return None;
        }
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
        let mut last = alloc::string::String::new();
        for attempt in 0..30 {
            let ready = bot.scsi(x, &[0x00, 0, 0, 0, 0, 0], 0, false).is_ok(); // TEST UNIT READY
            if ready && bot.scsi(x, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, false).is_ok() {
                capacity = Some(bot.data(8).to_vec());
                break;
            }
            let step = if ready { "READ CAPACITY" } else { "TEST UNIT READY" };
            let csw = bot.last_csw;
            let sense = bot.request_sense(x);
            last = alloc::format!("{}: {}", step, Bot::why(csw, sense));
            if attempt < 3 || attempt % 10 == 9 {
                crate::warn!("usb: port {}: incercarea {}, {}", port, attempt + 1, last);
            }
            crate::sched::sleep_ms(if attempt < 5 { 20 } else { 100 });
        }
        let Some(c) = capacity else {
            let why = alloc::format!("stick-ul {} nu e gata ({})", name, last);
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
