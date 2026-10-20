//! FAT32. Written from Microsoft's public "FAT32 File System Specification" (fatgen103): boot sector
//! (BPB), the FAT (cluster chains, both copies kept equal), FSInfo, directory entries, 8.3 names and
//! long names (LFN).
//!
//! Writing (P4-07) is allowed only on volumes that have a file named `cyber.rw` in their root folder,
//! so the kernel never changes a disk nobody marked for it. Order of every change: data first, then
//! the FAT, then the directory entry; an interrupted write can only leave lost clusters behind
//! (which `fsck` frees), never a file pointing at someone else's data.

use super::vfs::{FileSystem, FsError, Kind};
use crate::block::{BlockDevice, BlockError, SECTOR};
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

/// A file with this name in the root folder allows the kernel to write to the volume.
pub const WRITE_MARKER: &str = "cyber.rw";

const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_LONG_NAME: u8 = 0x0F;
const ATTR_ARCHIVE: u8 = 0x20;
const END_OF_CHAIN: u32 = 0x0FFF_FFF8;
const EOC_MARK: u32 = 0x0FFF_FFFF;
const FSINFO_UNKNOWN: u32 = 0xFFFF_FFFF;

pub struct Fat32 {
    dev: Box<dyn BlockDevice>,
    sectors_per_cluster: u64,
    fat_start: u64,
    fat_size: u64,
    fats: u64,
    data_start: u64,
    clusters: u32,
    root_cluster: u32,
    /// Sector of the FSInfo structure (free-cluster count and search hint), 0 if there is none.
    fsinfo: u64,
    writable: bool,
    /// Where to start looking for a free cluster.
    next_free: AtomicU32,
}

#[derive(Clone)]
struct DirEntry {
    name: String,
    /// The 8.3 name as stored (11 bytes, space padded).
    short: [u8; 11],
    is_dir: bool,
    cluster: u32,
    size: u32,
    /// Where its 32-byte entries are (cluster, byte offset): long-name entries first, the short one last.
    /// Empty for the root folder.
    slots: Vec<(u32, usize)>,
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
        let fsinfo_sector = u16_at(&bs, 48) as u64;
        let mut fsinfo = 0;
        let mut hint = 2;
        if fsinfo_sector != 0 && fsinfo_sector < reserved {
            let mut fi = [0u8; SECTOR];
            if dev.read(fsinfo_sector, &mut fi).is_ok() && u32_at(&fi, 0) == 0x4161_5252 && u32_at(&fi, 484) == 0x6141_7272 {
                fsinfo = fsinfo_sector;
                hint = u32_at(&fi, 492);
            }
        }
        let mut fat = Self {
            dev,
            sectors_per_cluster: spc,
            fat_start: reserved,
            fat_size,
            fats,
            data_start,
            clusters,
            root_cluster: u32_at(&bs, 44),
            fsinfo,
            writable: false,
            next_free: AtomicU32::new(hint),
        };
        fat.writable = fat.lookup(&[WRITE_MARKER]).is_ok_and(|e| !e.is_dir);
        Ok(fat)
    }

    /// Whether the volume is marked for writing (see `WRITE_MARKER`).
    pub fn writable(&self) -> bool {
        self.writable
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
        let mut long_slots: Vec<(u32, usize)> = Vec::new();
        let mut long_sum = None;
        self.walk(cluster, |c| {
            self.read_cluster(c, &mut buf)?;
            for (i, e) in buf.chunks(32).enumerate() {
                match e[0] {
                    0x00 => return Ok(false), // no more entries
                    0xE5 => {
                        long.clear();
                        long_slots.clear();
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
                        long_slots.clear();
                    }
                    part.extend_from_slice(&long);
                    long = part;
                    long_slots.push((c, i * 32));
                    long_sum = Some(e[13]);
                    continue;
                }
                if attr & ATTR_VOLUME_ID != 0 {
                    long.clear();
                    long_slots.clear();
                    continue;
                }
                let has_long = !long.is_empty() && long_sum == Some(short_checksum(&e[0..11]));
                let name = if has_long {
                    let end = long.iter().position(|&c| c == 0 || c == 0xFFFF).unwrap_or(long.len());
                    char::decode_utf16(long[..end].iter().copied()).map(|r| r.unwrap_or('?')).collect()
                } else {
                    short_name(e)
                };
                let mut slots = if has_long { core::mem::take(&mut long_slots) } else { Vec::new() };
                slots.push((c, i * 32));
                long.clear();
                long_slots.clear();
                if name == "." || name == ".." {
                    continue;
                }
                out.push(DirEntry {
                    name,
                    short: e[0..11].try_into().unwrap(),
                    is_dir: attr & ATTR_DIRECTORY != 0,
                    cluster: ((u16_at(e, 20) as u32) << 16) | u16_at(e, 26) as u32,
                    size: u32_at(e, 28),
                    slots,
                });
            }
            Ok(true)
        })?;
        Ok(out)
    }

    fn root(&self) -> DirEntry {
        DirEntry { name: String::new(), short: [b' '; 11], is_dir: true, cluster: self.root_cluster, size: 0, slots: Vec::new() }
    }

    /// Finds `path` from the root directory (names compared without regard to case, like FAT does).
    fn lookup(&self, path: &[&str]) -> Result<DirEntry, FsError> {
        let mut current = self.root();
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


fn io(e: BlockError) -> FsError {
    if e == BlockError::ReadOnly { FsError::ReadOnly } else { FsError::Io }
}

/// Characters allowed in 8.3 names (besides A-Z and 0-9).
fn short_char_ok(c: u8) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || b"!#$%&'()-@^_`{}~".contains(&c)
}

/// The 8.3 form of `name` if it needs no long-name entries: base up to 8 characters, extension up to
/// 3, each all upper or all lower case (lower case is recorded in byte 12, like Windows NT does).
fn exact_short(name: &str) -> Option<([u8; 11], u8)> {
    let (base, ext) = match name.rfind('.') {
        Some(i) => (&name[..i], &name[i + 1..]),
        None => (name, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || base.contains('.') || !name.is_ascii() {
        return None;
    }
    let mut out = [b' '; 11];
    let mut flags = 0;
    for (part, at, flag) in [(base, 0, 0x08), (ext, 8, 0x10)] {
        let lower = part.bytes().any(|c| c.is_ascii_lowercase());
        let upper = part.bytes().any(|c| c.is_ascii_uppercase());
        if lower && upper {
            return None;
        }
        if lower {
            flags |= flag;
        }
        for (i, c) in part.bytes().enumerate() {
            let c = c.to_ascii_uppercase();
            if !short_char_ok(c) {
                return None;
            }
            out[at + i] = c;
        }
    }
    Some((out, flags))
}

/// A unique 8.3 alias for a long name: up to 6 characters of the name, `~n`, up to 3 of the extension.
fn alias(name: &str, taken: &[[u8; 11]]) -> Option<[u8; 11]> {
    let clean = |s: &str| -> Vec<u8> {
        s.chars().filter_map(|c| c.is_ascii().then(|| c.to_ascii_uppercase() as u8)).filter(|&c| short_char_ok(c)).collect()
    };
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (clean(&name[..i]), clean(&name[i + 1..])),
        _ => (clean(name), Vec::new()),
    };
    for n in 1..1000u32 {
        let digits = alloc::format!("~{}", n);
        let keep = base.len().min(8 - digits.len());
        let mut out = [b' '; 11];
        out[..keep].copy_from_slice(&base[..keep]);
        if keep == 0 {
            out[0] = b'_';
        }
        let at = keep.max(1);
        out[at..at + digits.len()].copy_from_slice(digits.as_bytes());
        for (i, &c) in ext.iter().take(3).enumerate() {
            out[8 + i] = c;
        }
        if !taken.contains(&out) {
            return Some(out);
        }
    }
    None
}

/// FAT time and date fields from the computer's clock (1 January 2026 if there is none).
fn fat_now() -> (u16, u16) {
    match crate::drivers::rtc::now() {
        Some(t) => (
            (t.hour as u16) << 11 | (t.minute as u16) << 5 | (t.second as u16 / 2),
            (t.year.saturating_sub(1980)) << 9 | (t.month as u16) << 5 | t.day as u16,
        ),
        None => (0, 46 << 9 | 1 << 5 | 1),
    }
}

/// A short (8.3) directory entry.
fn short_entry(name: &[u8; 11], flags: u8, attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0..11].copy_from_slice(name);
    e[11] = attr;
    e[12] = flags;
    let (time, date) = fat_now();
    for (o, v) in [(14, time), (16, date), (18, date), (22, time), (24, date)] {
        e[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    e
}

/// The long-name entries for `name`, in disk order (last piece first), for the short name `short`.
fn long_entries(name: &str, short: &[u8; 11]) -> Vec<[u8; 32]> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let count = units.len().div_ceil(13);
    let sum = short_checksum(short);
    let mut out = Vec::with_capacity(count);
    for seq in (1..=count).rev() {
        let mut e = [0u8; 32];
        e[0] = seq as u8 | if seq == count { 0x40 } else { 0 };
        e[11] = ATTR_LONG_NAME;
        e[13] = sum;
        let mut k = 0;
        for &(o, n) in &[(1usize, 5usize), (14, 6), (28, 2)] {
            for i in 0..n {
                let idx = (seq - 1) * 13 + k;
                let v = match idx.cmp(&units.len()) {
                    core::cmp::Ordering::Less => units[idx],
                    core::cmp::Ordering::Equal => 0,
                    core::cmp::Ordering::Greater => 0xFFFF,
                };
                e[o + i * 2..o + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
                k += 1;
            }
        }
        out.push(e);
    }
    out
}

impl Fat32 {
    fn check_writable(&self) -> Result<(), FsError> {
        if self.writable { Ok(()) } else { Err(FsError::ReadOnly) }
    }

    fn cluster_lba(&self, cluster: u32) -> u64 {
        self.data_start + (cluster as u64 - 2) * self.sectors_per_cluster
    }

    fn write_cluster(&self, cluster: u32, buf: &[u8]) -> Result<(), FsError> {
        self.dev.write(self.cluster_lba(cluster), buf).map_err(io)
    }

    /// Replaces the 32-byte entry at `offset` in directory cluster `cluster` (one sector rewritten).
    fn write_slot(&self, cluster: u32, offset: usize, entry: &[u8]) -> Result<(), FsError> {
        let lba = self.cluster_lba(cluster) + (offset / SECTOR) as u64;
        let mut sector = [0u8; SECTOR];
        self.dev.read(lba, &mut sector).map_err(io)?;
        sector[offset % SECTOR..offset % SECTOR + entry.len()].copy_from_slice(entry);
        self.dev.write(lba, &sector).map_err(io)
    }

    /// Sets the FAT entry of `cluster` in every copy of the FAT (the top 4 bits are kept, as required).
    fn set_fat(&self, cluster: u32, value: u32) -> Result<(), FsError> {
        let offset = cluster as u64 * 4;
        let mut sector = [0u8; SECTOR];
        for copy in 0..self.fats {
            let lba = self.fat_start + copy * self.fat_size + offset / SECTOR as u64;
            self.dev.read(lba, &mut sector).map_err(io)?;
            let at = (offset % SECTOR as u64) as usize;
            let old = u32_at(&sector, at);
            sector[at..at + 4].copy_from_slice(&((old & 0xF000_0000) | (value & 0x0FFF_FFFF)).to_le_bytes());
            self.dev.write(lba, &sector).map_err(io)?;
        }
        Ok(())
    }

    /// Adds `delta` to the free-cluster count in FSInfo and stores the search hint.
    fn update_fsinfo(&self, delta: i64) -> Result<(), FsError> {
        if self.fsinfo == 0 {
            return Ok(());
        }
        let mut fi = [0u8; SECTOR];
        self.dev.read(self.fsinfo, &mut fi).map_err(io)?;
        let free = u32_at(&fi, 488);
        if free != FSINFO_UNKNOWN {
            fi[488..492].copy_from_slice(&((free as i64 + delta).clamp(0, self.clusters as i64) as u32).to_le_bytes());
        }
        fi[492..496].copy_from_slice(&self.next_free.load(Ordering::Relaxed).to_le_bytes());
        self.dev.write(self.fsinfo, &fi).map_err(io)
    }

    /// Takes a free cluster, marks it as the end of a chain and fills it with zeros.
    fn alloc(&self) -> Result<u32, FsError> {
        let per_sector = (SECTOR / 4) as u32;
        let start = self.next_free.load(Ordering::Relaxed).clamp(2, self.clusters + 1);
        let mut sector = [0u8; SECTOR];
        let mut loaded = u64::MAX;
        for i in 0..self.clusters {
            let c = 2 + (start - 2 + i) % self.clusters;
            let lba = self.fat_start + (c / per_sector) as u64;
            if lba != loaded {
                self.dev.read(lba, &mut sector).map_err(io)?;
                loaded = lba;
            }
            if u32_at(&sector, (c % per_sector) as usize * 4) & 0x0FFF_FFFF == 0 {
                self.write_cluster(c, &vec![0u8; self.cluster_bytes()])?;
                self.set_fat(c, EOC_MARK)?;
                self.next_free.store(if c + 1 < self.clusters + 2 { c + 1 } else { 2 }, Ordering::Relaxed);
                self.update_fsinfo(-1)?;
                return Ok(c);
            }
        }
        Err(FsError::NoSpace)
    }

    /// Marks every cluster of the chain starting at `first` as free.
    fn free_chain(&self, first: u32) -> Result<(), FsError> {
        let mut chain = Vec::new();
        self.walk(first, |c| {
            chain.push(c);
            Ok(true)
        })?;
        for &c in &chain {
            self.set_fat(c, 0)?;
        }
        self.update_fsinfo(chain.len() as i64)
    }

    fn chain(&self, first: u32) -> Result<Vec<u32>, FsError> {
        let mut chain = Vec::new();
        if first != 0 {
            self.walk(first, |c| {
                chain.push(c);
                Ok(true)
            })?;
        }
        Ok(chain)
    }

    /// `n` consecutive free entry places in directory `dir`; the directory grows by a cluster if needed.
    fn free_slots(&self, dir: u32, n: usize) -> Result<Vec<(u32, usize)>, FsError> {
        let mut run: Vec<(u32, usize)> = Vec::new();
        let mut buf = vec![0u8; self.cluster_bytes()];
        let chain = self.chain(dir)?;
        for &c in &chain {
            self.read_cluster(c, &mut buf)?;
            for (i, e) in buf.chunks(32).enumerate() {
                if e[0] == 0x00 || e[0] == 0xE5 {
                    run.push((c, i * 32));
                    if run.len() == n {
                        return Ok(run);
                    }
                } else {
                    run.clear();
                }
            }
        }
        let last = *chain.last().ok_or(FsError::NotFound)?;
        let mut prev = last;
        while run.len() < n {
            let c = self.alloc()?; // already zeroed: every entry is free
            self.set_fat(prev, c)?;
            prev = c;
            for i in 0..self.cluster_bytes() / 32 {
                run.push((c, i * 32));
                if run.len() == n {
                    break;
                }
            }
        }
        Ok(run)
    }

    /// Adds an entry called `name` to directory `dir`.
    fn add_entry(&self, dir: &DirEntry, name: &str, attr: u8, cluster: u32) -> Result<(), FsError> {
        if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || name.ends_with(['.', ' ']) {
            return Err(FsError::InvalidPath);
        }
        let existing = self.read_dir(dir.cluster)?;
        let (short, flags, longs) = match exact_short(name) {
            Some((short, flags)) if !existing.iter().any(|e| e.short == short) => (short, flags, Vec::new()),
            _ => {
                let taken: Vec<[u8; 11]> = existing.iter().map(|e| e.short).collect();
                let short = alias(name, &taken).ok_or(FsError::NoSpace)?;
                (short, 0, long_entries(name, &short))
            }
        };
        let slots = self.free_slots(dir.cluster, longs.len() + 1)?;
        let entry = short_entry(&short, flags, attr, cluster, 0);
        for (slot, e) in slots.iter().zip(longs.iter().chain(core::iter::once(&entry))) {
            self.write_slot(slot.0, slot.1, e)?;
        }
        Ok(())
    }

    /// Rewrites the size, first cluster and change time of an entry's short entry.
    fn update_entry(&self, e: &DirEntry, cluster: u32, size: u32) -> Result<(), FsError> {
        let &(c, off) = e.slots.last().ok_or(FsError::InvalidPath)?;
        let lba = self.cluster_lba(c) + (off / SECTOR) as u64;
        let mut sector = [0u8; SECTOR];
        self.dev.read(lba, &mut sector).map_err(io)?;
        let at = off % SECTOR;
        let (time, date) = fat_now();
        sector[at + 18..at + 20].copy_from_slice(&date.to_le_bytes());
        sector[at + 20..at + 22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        sector[at + 22..at + 24].copy_from_slice(&time.to_le_bytes());
        sector[at + 24..at + 26].copy_from_slice(&date.to_le_bytes());
        sector[at + 26..at + 28].copy_from_slice(&(cluster as u16).to_le_bytes());
        sector[at + 28..at + 32].copy_from_slice(&size.to_le_bytes());
        self.dev.write(lba, &sector).map_err(io)
    }

    fn parent_and_name<'a>(&self, path: &[&'a str]) -> Result<(DirEntry, &'a str), FsError> {
        let (&name, parent) = path.split_last().ok_or(FsError::AlreadyExists)?; // the root always exists
        let dir = self.lookup(parent)?;
        if !dir.is_dir {
            return Err(FsError::NotADirectory);
        }
        Ok((dir, name))
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

    fn write(&self, path: &[&str], offset: u64, data: &[u8]) -> Result<usize, FsError> {
        self.check_writable()?;
        if path.is_empty() {
            return Err(FsError::IsADirectory);
        }
        let e = self.lookup(path)?;
        if e.is_dir {
            return Err(FsError::IsADirectory);
        }
        let end = offset.checked_add(data.len() as u64).filter(|&e| e <= u32::MAX as u64).ok_or(FsError::NoSpace)?;
        let old_size = e.size as u64;
        let new_size = end.max(old_size);
        let cb = self.cluster_bytes() as u64;
        let mut chain = self.chain(e.cluster)?;
        let old_len = chain.len();
        // 1. New clusters (taken, zeroed, each marked as a chain end; not linked yet).
        while (chain.len() as u64) * cb < new_size {
            match self.alloc() {
                Ok(c) => chain.push(c),
                Err(err) => {
                    for &c in &chain[old_len..] {
                        let _ = self.set_fat(c, 0);
                    }
                    return Err(err);
                }
            }
        }
        // 2. Data: every cluster touched by the write, or by the gap between the old end and `offset`
        //    (old bytes after the old end are cleared, so the gap reads as zeros).
        let from = offset.min(old_size);
        let mut buf = vec![0u8; cb as usize];
        for (i, &c) in chain.iter().enumerate() {
            let pos = i as u64 * cb;
            if pos + cb <= from || pos >= end.max(from) {
                continue;
            }
            if i < old_len {
                self.read_cluster(c, &mut buf)?;
                if pos + cb > old_size {
                    buf[old_size.saturating_sub(pos) as usize..].fill(0);
                }
            } else {
                buf.fill(0);
            }
            let (s, t) = (offset.max(pos), end.min(pos + cb));
            if s < t {
                buf[(s - pos) as usize..(t - pos) as usize].copy_from_slice(&data[(s - offset) as usize..(t - offset) as usize]);
            }
            self.write_cluster(c, &buf)?;
        }
        // 3. FAT: link the new clusters after the old ones.
        for i in old_len.max(1)..chain.len() {
            self.set_fat(chain[i - 1], chain[i])?;
        }
        // 4. Directory entry: first cluster and size.
        self.update_entry(&e, chain.first().copied().unwrap_or(0), new_size as u32)?;
        Ok(data.len())
    }

    fn create(&self, path: &[&str], kind: Kind) -> Result<(), FsError> {
        self.check_writable()?;
        let (dir, name) = self.parent_and_name(path)?;
        if self.read_dir(dir.cluster)?.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Err(FsError::AlreadyExists);
        }
        match kind {
            Kind::File => self.add_entry(&dir, name, ATTR_ARCHIVE, 0),
            Kind::Dir => {
                let c = self.alloc()?;
                // "." and ".." (the root is written as cluster 0 in "..", as the specification says).
                let mut first = vec![0u8; self.cluster_bytes()];
                let parent = if dir.cluster == self.root_cluster { 0 } else { dir.cluster };
                first[0..32].copy_from_slice(&short_entry(b".          ", 0, ATTR_DIRECTORY, c, 0));
                first[32..64].copy_from_slice(&short_entry(b"..         ", 0, ATTR_DIRECTORY, parent, 0));
                self.write_cluster(c, &first)?;
                self.add_entry(&dir, name, ATTR_DIRECTORY, c).inspect_err(|_| {
                    let _ = self.free_chain(c);
                })
            }
        }
    }
    fn list(&self, path: &[&str]) -> Result<Vec<(String, Kind)>, FsError> {
        let e = self.lookup(path)?;
        if !e.is_dir {
            return Err(FsError::NotADirectory);
        }
        Ok(self.read_dir(e.cluster)?.into_iter().map(|d| (d.name, if d.is_dir { Kind::Dir } else { Kind::File })).collect())
    }

    fn remove(&self, path: &[&str]) -> Result<(), FsError> {
        self.check_writable()?;
        if path.is_empty() {
            return Err(FsError::InvalidPath);
        }
        let e = self.lookup(path)?;
        if e.is_dir && !self.read_dir(e.cluster)?.is_empty() {
            return Err(FsError::NotEmpty);
        }
        // The directory entry goes first: if the clusters were freed first and the write stopped in
        // between, the entry would point at free clusters.
        for &(c, off) in &e.slots {
            self.write_slot(c, off, &[0xE5])?;
        }
        if e.cluster != 0 {
            self.free_chain(e.cluster)?;
        }
        Ok(())
    }
}
