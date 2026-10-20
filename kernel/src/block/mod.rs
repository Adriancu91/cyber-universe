//! Block devices: anything that stores 512-byte sectors (a disk, a partition, a disk image in memory).
//! File systems read through this interface only, so a real disk driver (AHCI/NVMe, Phase 3) can be
//! put underneath without changing them.

use alloc::boxed::Box;

pub const SECTOR: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    OutOfRange,
    BadBuffer,
    /// The device (or its driver) cannot write.
    ReadOnly,
}

pub trait BlockDevice: Send + Sync {
    /// Number of sectors.
    fn sectors(&self) -> u64;
    /// Reads `buf.len() / 512` sectors starting at `lba`. `buf.len()` must be a multiple of 512.
    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError>;
    /// Writes `buf.len() / 512` sectors starting at `lba`. Devices that cannot write keep this default.
    fn write(&self, _lba: u64, _buf: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}

/// A disk image in memory (for now: a file loaded by Limine), read only.
pub struct MemDisk {
    data: &'static [u8],
}

impl MemDisk {
    pub fn new(data: &'static [u8]) -> Self {
        Self { data }
    }
}

impl BlockDevice for MemDisk {
    fn sectors(&self) -> u64 {
        (self.data.len() / SECTOR) as u64
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() % SECTOR != 0 {
            return Err(BlockError::BadBuffer);
        }
        let start = (lba as usize).checked_mul(SECTOR).ok_or(BlockError::OutOfRange)?;
        let end = start.checked_add(buf.len()).filter(|&e| e <= self.data.len()).ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(&self.data[start..end]);
        Ok(())
    }
}

/// A range of sectors of another device, seen as a device of its own.
pub struct Partition {
    dev: Box<dyn BlockDevice>,
    start: u64,
    len: u64,
}

impl Partition {
    pub fn new(dev: Box<dyn BlockDevice>, start: u64, len: u64) -> Self {
        Self { dev, start, len }
    }
}

impl BlockDevice for Partition {
    fn sectors(&self) -> u64 {
        self.len
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let count = (buf.len() / SECTOR) as u64;
        if lba.checked_add(count).is_none_or(|e| e > self.len) {
            return Err(BlockError::OutOfRange);
        }
        self.dev.read(self.start + lba, buf)
    }

    fn write(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        let count = (buf.len() / SECTOR) as u64;
        if lba.checked_add(count).is_none_or(|e| e > self.len) {
            return Err(BlockError::OutOfRange);
        }
        self.dev.write(self.start + lba, buf)
    }
}
