//! Partition tables: GPT (UEFI specification, ch. 5) and the classic MBR. Finds the first partition
//! that can hold a FAT file system.

use crate::block::{BlockDevice, SECTOR};
use alloc::{format, string::String, vec::Vec};

/// GUID of a "Microsoft basic data" partition (FAT/NTFS), as stored on disk (mixed-endian).
const BASIC_DATA: [u8; 16] = [0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7];
/// GUID of the EFI system partition (also FAT).
const EFI_SYSTEM: [u8; 16] = [0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B];

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Found {
    pub start: u64,
    pub sectors: u64,
    pub table: &'static str,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// True if `s` is a FAT boot sector (jump instruction, 512-byte sectors, "FAT" type text, signature 55 AA) —
/// Microsoft FAT specification ("BPB"). A disk formatted without a partition table ("superfloppy") starts with one.
pub fn is_fat_boot_sector(s: &[u8]) -> bool {
    let jump = s[0] == 0xEB && s[2] == 0x90 || s[0] == 0xE9;
    let bytes_per_sector = u16::from_le_bytes([s[11], s[12]]);
    let fat_text = &s[82..87] == b"FAT32" || &s[54..57] == b"FAT";
    jump && bytes_per_sector == 512 && fat_text && s[510] == 0x55 && s[511] == 0xAA
}

/// The first FAT-capable partition of `dev`, or `None` if there is no partition table we understand.
pub fn find_fat(dev: &dyn BlockDevice) -> Option<Found> {
    let mut mbr = [0u8; SECTOR];
    dev.read(0, &mut mbr).ok()?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return None;
    }
    // A boot sector at sector 0 also ends in 55 AA, so it is checked before reading the bytes as an MBR.
    if is_fat_boot_sector(&mbr) {
        return Some(Found { start: 0, sectors: dev.sectors(), table: "fara tabela" });
    }
    let entry = |i: usize| &mbr[446 + i * 16..462 + i * 16];
    if (0..4).any(|i| entry(i)[4] == 0xEE) {
        return find_gpt(dev);
    }
    // MBR types for FAT32 (0x0B, 0x0C) and FAT16 (0x06, 0x0E); the FAT driver checks the real format.
    let typed = (0..4).map(entry).find(|e| matches!(e[4], 0x0B | 0x0C | 0x06 | 0x0E));
    // Otherwise any used entry whose first sector really is a FAT boot sector (tools sometimes write another type, e.g. 0x07 or 0xEF).
    let by_content = || {
        (0..4).map(entry).filter(|e| e[4] != 0 && u32_at(e, 8) != 0).find(|e| {
            let mut s = [0u8; SECTOR];
            dev.read(u32_at(e, 8) as u64, &mut s).is_ok() && is_fat_boot_sector(&s)
        })
    };
    typed.or_else(by_content).map(|e| Found { start: u32_at(e, 8) as u64, sectors: u32_at(e, 12) as u64, table: "MBR" })
}

fn find_gpt(dev: &dyn BlockDevice) -> Option<Found> {
    let mut header = [0u8; SECTOR];
    dev.read(1, &mut header).ok()?;
    if &header[0..8] != b"EFI PART" {
        return None;
    }
    let entries_lba = u64_at(&header, 72);
    let count = u32_at(&header, 80).min(256) as u64;
    let size = u32_at(&header, 84) as u64;
    if !(128..=SECTOR as u64).contains(&size) || SECTOR as u64 % size != 0 {
        return None;
    }
    let per_sector = SECTOR as u64 / size;
    let mut sector = [0u8; SECTOR];
    for i in 0..count {
        if i % per_sector == 0 {
            dev.read(entries_lba + i / per_sector, &mut sector).ok()?;
        }
        let e = &sector[((i % per_sector) * size) as usize..][..size as usize];
        let kind: [u8; 16] = e[0..16].try_into().unwrap();
        if kind == BASIC_DATA || kind == EFI_SYSTEM {
            let (first, last) = (u64_at(e, 32), u64_at(e, 40));
            if last >= first {
                return Some(Found { start: first, sectors: last - first + 1, table: "GPT" });
            }
        }
    }
    None
}

/// P6-100: one used entry of the partition table (for the disk inventory; nothing is written).
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    /// 1, 2, ... in the order of the table.
    pub number: usize,
    pub start: u64,
    pub sectors: u64,
    pub table: &'static str,
    /// What the table says the partition is (in Romanian, for the report).
    pub kind: String,
}

/// A GPT type GUID as text (the first three fields are stored little-endian).
pub fn guid_text(g: &[u8]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u32_at(g, 0),
        u16::from_le_bytes([g[4], g[5]]),
        u16::from_le_bytes([g[6], g[7]]),
        g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

/// The name of a GPT partition type (UEFI specification table 5-7 and Microsoft's documented types).
fn gpt_kind(g: &[u8]) -> String {
    let text = guid_text(g);
    let name = match text.as_str() {
        "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" => "EFI (sistem)",
        "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7" => "date de baza (FAT sau NTFS)",
        "E3C9E316-0B5C-4DB8-817D-F92DF00215AE" => "rezervata Microsoft",
        "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC" => "recuperare Windows",
        "0FC63DAF-8483-4772-8E79-3D69D8477DE4" => "Linux",
        _ => return format!("alt tip ({})", text),
    };
    String::from(name)
}

/// The name of a classic MBR partition type.
fn mbr_kind(t: u8) -> String {
    let name = match t {
        0x0B | 0x0C => "FAT32",
        0x06 | 0x0E => "FAT16",
        0x07 => "NTFS sau exFAT",
        0x27 => "recuperare Windows",
        0xEF => "EFI (sistem)",
        0x83 => "Linux",
        _ => return format!("tip {:#04x}", t),
    };
    String::from(name)
}

/// P6-100: every used partition of `dev` (GPT or MBR); empty for a disk without a table (or one we do not understand).
pub fn list(dev: &dyn BlockDevice) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut mbr = [0u8; SECTOR];
    if dev.read(0, &mut mbr).is_err() || mbr[510] != 0x55 || mbr[511] != 0xAA || is_fat_boot_sector(&mbr) {
        return out;
    }
    let entry = |i: usize| &mbr[446 + i * 16..462 + i * 16];
    if (0..4).any(|i| entry(i)[4] == 0xEE) {
        gpt_each(dev, |i, e| {
            let (first, last) = (u64_at(e, 32), u64_at(e, 40));
            if last >= first {
                out.push(Entry { number: i + 1, start: first, sectors: last - first + 1, table: "GPT", kind: gpt_kind(&e[0..16]) });
            }
        });
        return out;
    }
    for i in 0..4 {
        let e = entry(i);
        if e[4] != 0 && u32_at(e, 12) != 0 {
            out.push(Entry { number: i + 1, start: u32_at(e, 8) as u64, sectors: u32_at(e, 12) as u64, table: "MBR", kind: mbr_kind(e[4]) });
        }
    }
    out
}

/// P6-100: every partition of `dev` that may hold a FAT file system, the one `find_fat` picks first (so a disk with one
/// FAT partition behaves as before); the FAT driver checks the real format.
pub fn fat_candidates(dev: &dyn BlockDevice) -> Vec<Found> {
    let mut out: Vec<Found> = find_fat(dev).into_iter().collect();
    if out.first().is_some_and(|f| f.table == "fara tabela") {
        return out;
    }
    for e in list(dev) {
        if out.iter().any(|f| f.start == e.start) {
            continue;
        }
        let maybe_fat = match e.table {
            "GPT" => e.kind.starts_with("EFI") || e.kind.starts_with("date de baza"),
            _ => matches!(e.kind.as_str(), "FAT32" | "FAT16" | "EFI (sistem)") || {
                let mut s = [0u8; SECTOR];
                dev.read(e.start, &mut s).is_ok() && is_fat_boot_sector(&s)
            },
        };
        if maybe_fat {
            out.push(Found { start: e.start, sectors: e.sectors, table: e.table });
        }
    }
    out
}

/// P6-100: what the first sector of a partition says it holds (for the inventory): FAT32, NTFS, exFAT or unknown.
pub fn content(dev: &dyn BlockDevice, start: u64) -> &'static str {
    let mut s = [0u8; SECTOR];
    if dev.read(start, &mut s).is_err() {
        return "nu se poate citi";
    }
    if &s[3..11] == b"NTFS    " {
        "NTFS"
    } else if &s[3..11] == b"EXFAT   " {
        "exFAT"
    } else if is_fat_boot_sector(&s) {
        "FAT"
    } else {
        "format necunoscut"
    }
}

/// Calls `f(index, entry)` for every used GPT entry (type GUID not zero).
fn gpt_each(dev: &dyn BlockDevice, mut f: impl FnMut(usize, &[u8])) {
    let mut header = [0u8; SECTOR];
    if dev.read(1, &mut header).is_err() || &header[0..8] != b"EFI PART" {
        return;
    }
    let entries_lba = u64_at(&header, 72);
    let count = u32_at(&header, 80).min(256) as u64;
    let size = u32_at(&header, 84) as u64;
    if !(128..=SECTOR as u64).contains(&size) || SECTOR as u64 % size != 0 {
        return;
    }
    let per_sector = SECTOR as u64 / size;
    let mut sector = [0u8; SECTOR];
    for i in 0..count {
        if i % per_sector == 0 && dev.read(entries_lba + i / per_sector, &mut sector).is_err() {
            return;
        }
        let e = &sector[((i % per_sector) * size) as usize..][..size as usize];
        if e[0..16].iter().any(|&b| b != 0) {
            f(i as usize, e);
        }
    }
}
