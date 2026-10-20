//! FAT32, read only. Written from Microsoft's public "FAT32 File System Specification" (fatgen103):
//! boot sector (BPB), the FAT (cluster chains), directory entries, 8.3 names and long names (LFN).

use super::vfs::{FileSystem, FsError, Kind};
use crate::block::{BlockDevice, SECTOR};
use alloc::{boxed::Box, string::String, vec, vec::Vec};

const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_LONG_NAME: u8 = 0x0F;
const END_OF_CHAIN: u32 = 0x0FFF_FFF8;

pub struct Fat32 {
    dev: Box<dyn BlockDevice>,
    sectors_per_cluster: u64,
    fat_start: u64,
    data_start: u64,
    clusters: u32,
    root_cluster: u32,
}

#[derive(Clone)]
struct DirEntry {
    name: String,
    is_dir: bool,
    cluster: u32,
    size: u32,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Checksum of an 8.3 name, stored in each of its long-name entries.
fn short_checksum(name: &[u8]) -> u8 {
    name.iter().fold(0u8, |sum, &c| (sum >> 1 | (sum & 1) << 7).wrapping_add(c))
}

/// "README  TXT" -> "README.TXT". Byte 12 may ask for a lower-case name or extension (bits 3 and 4,
/// as written by Windows NT and later and by mtools) for names that need no long-name entries.
fn short_name(e: &[u8]) -> String {
    let part = |bytes: &[u8], lower: bool| {
        let s = String::from(core::str::from_utf8(bytes).unwrap_or("?").trim_end());
        if lower { s.to_ascii_lowercase() } else { s }
    };
    let base = part(&e[0..8], e[12] & 0x08 != 0);
    let ext = part(&e[8..11], e[12] & 0x10 != 0);
    if ext.is_empty() { base } else { alloc::format!("{}.{}", base, ext) }
}

impl Fat32 {
    /// Reads the boot sector of `dev`; `Err` with a reason if it is not a FAT32 volume we support.
    pub fn new(dev: Box<dyn BlockDevice>) -> Result<Self, &'static str> {
        let mut bs = [0u8; SECTOR];
        dev.read(0, &mut bs).map_err(|_| "nu pot citi sectorul de boot")?;
        if bs[510] != 0x55 || bs[511] != 0xAA {
            return Err("sector de boot fara semnatura");
        }
        if u16_at(&bs, 11) as usize != SECTOR {
            return Err("doar sectoare de 512 octeti sunt suportate");
        }
        let spc = bs[13] as u64;
        let reserved = u16_at(&bs, 14) as u64;
        let fats = bs[16] as u64;
        let fat_size = u32_at(&bs, 36) as u64;
        let total = u32_at(&bs, 32) as u64;
        if u16_at(&bs, 17) != 0 || u16_at(&bs, 22) != 0 || fat_size == 0 {
            return Err("nu e FAT32 (poate FAT12/16)");
        }
        if spc == 0 || !spc.is_power_of_two() || fats == 0 || reserved == 0 || total > dev.sectors() {
            return Err("sector de boot invalid");
        }
        let data_start = reserved + fats * fat_size;
        let clusters = ((total.saturating_sub(data_start)) / spc) as u32;
        if clusters < 65525 {
            return Err("prea putine clustere pentru FAT32");
        }
        Ok(Self { dev, sectors_per_cluster: spc, fat_start: reserved, data_start, clusters, root_cluster: u32_at(&bs, 44) })
    }

    fn cluster_bytes(&self) -> usize {
        self.sectors_per_cluster as usize * SECTOR
    }

    fn valid(&self, cluster: u32) -> bool {
        (2..self.clusters + 2).contains(&cluster)
    }

    fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let lba = self.data_start + (cluster as u64 - 2) * self.sectors_per_cluster;
        self.dev.read(lba, buf).map_err(|_| FsError::NotFound)
    }

    /// The cluster after `cluster` in its chain, or `None` at the end (or on a damaged chain).
    fn next(&self, cluster: u32) -> Option<u32> {
        let offset = cluster as u64 * 4;
        let mut sector = [0u8; SECTOR];
        self.dev.read(self.fat_start + offset / SECTOR as u64, &mut sector).ok()?;
        let v = u32_at(&sector, (offset % SECTOR as u64) as usize) & 0x0FFF_FFFF;
        (v < END_OF_CHAIN && self.valid(v)).then_some(v)
    }

    /// Calls `f` for every cluster of the chain starting at `first` (stops after `clusters` steps,
    /// so a looped chain cannot hang the kernel). `f` returns false to stop early.
    fn walk(&self, first: u32, mut f: impl FnMut(u32) -> Result<bool, FsError>) -> Result<(), FsError> {
        let mut c = first;
        for _ in 0..self.clusters {
            if !self.valid(c) || !f(c)? {
                return Ok(());
            }
            match self.next(c) {
                Some(n) => c = n,
                None => return Ok(()),
            }
        }
        Ok(())
    }

    fn read_dir(&self, cluster: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; self.cluster_bytes()];
        let mut long: Vec<u16> = Vec::new();
        let mut long_sum = None;
        self.walk(cluster, |c| {
            self.read_cluster(c, &mut buf)?;
            for e in buf.chunks(32) {
                match e[0] {
                    0x00 => return Ok(false), // no more entries
                    0xE5 => {
                        long.clear();
                        continue;
                    }
                    _ => {}
                }
                let attr = e[11];
                if attr & 0x3F == ATTR_LONG_NAME {
                    // Long-name pieces come last-first; each holds 13 UCS-2 characters.
                    let mut part: Vec<u16> = Vec::with_capacity(13);
                    for &(o, n) in &[(1usize, 5usize), (14, 6), (28, 2)] {
                        part.extend((0..n).map(|i| u16_at(e, o + i * 2)));
                    }
                    if e[0] & 0x40 != 0 {
                        long.clear();
                    }
                    part.extend_from_slice(&long);
                    long = part;
                    long_sum = Some(e[13]);
                    continue;
                }
                if attr & ATTR_VOLUME_ID != 0 {
                    long.clear();
                    continue;
                }
                let name = if !long.is_empty() && long_sum == Some(short_checksum(&e[0..11])) {
                    let end = long.iter().position(|&c| c == 0 || c == 0xFFFF).unwrap_or(long.len());
                    char::decode_utf16(long[..end].iter().copied()).map(|r| r.unwrap_or('?')).collect()
                } else {
                    short_name(e)
                };
                long.clear();
                if name == "." || name == ".." {
                    continue;
                }
                out.push(DirEntry {
                    name,
                    is_dir: attr & ATTR_DIRECTORY != 0,
                    cluster: ((u16_at(e, 20) as u32) << 16) | u16_at(e, 26) as u32,
                    size: u32_at(e, 28),
                });
            }
            Ok(true)
        })?;
        Ok(out)
    }

    /// Finds `path` from the root directory (names compared without regard to case, like FAT does).
    fn lookup(&self, path: &[&str]) -> Result<DirEntry, FsError> {
        let mut current = DirEntry { name: String::new(), is_dir: true, cluster: self.root_cluster, size: 0 };
        for name in path {
            if !current.is_dir {
                return Err(FsError::NotADirectory);
            }
            current = self
                .read_dir(current.cluster)?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
                .ok_or(FsError::NotFound)?;
        }
        Ok(current)
    }
}

impl FileSystem for Fat32 {
    fn kind(&self, path: &[&str]) -> Result<Kind, FsError> {
        Ok(if self.lookup(path)?.is_dir { Kind::Dir } else { Kind::File })
    }

    fn size(&self, path: &[&str]) -> Result<u64, FsError> {
        Ok(self.lookup(path)?.size as u64)
    }

    fn read(&self, path: &[&str], offset: u64, out: &mut [u8]) -> Result<usize, FsError> {
        let e = self.lookup(path)?;
        if e.is_dir {
            return Err(FsError::IsADirectory);
        }
        let end = (e.size as u64).min(offset + out.len() as u64);
        if offset >= end {
            return Ok(0);
        }
        let cb = self.cluster_bytes() as u64;
        let mut buf = vec![0u8; cb as usize];
        let mut pos = 0u64; // file offset of the current cluster
        let mut written = 0usize;
        self.walk(e.cluster, |c| {
            if pos + cb > offset {
                self.read_cluster(c, &mut buf)?;
                let from = offset.max(pos);
                let to = end.min(pos + cb);
                let n = (to - from) as usize;
                out[written..written + n].copy_from_slice(&buf[(from - pos) as usize..(to - pos) as usize]);
                written += n;
            }
            pos += cb;
            Ok(pos < end)
        })?;
        Ok(written)
    }

    fn write(&self, _: &[&str], _: u64, _: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
    }

    fn create(&self, _: &[&str], _: Kind) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }

    fn list(&self, path: &[&str]) -> Result<Vec<(String, Kind)>, FsError> {
        let e = self.lookup(path)?;
        if !e.is_dir {
            return Err(FsError::NotADirectory);
        }
        Ok(self.read_dir(e.cluster)?.into_iter().map(|d| (d.name, if d.is_dir { Kind::Dir } else { Kind::File })).collect())
    }

    fn remove(&self, _: &[&str]) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
}
