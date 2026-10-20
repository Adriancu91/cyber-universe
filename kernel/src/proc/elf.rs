//! Loader for static ELF64 executables (x86_64, type EXEC), written from the System V gABI ELF
//! specification and the AMD64 psABI supplement. Every check happens before memory is allocated.

use crate::mem::{address_space::{AddressSpace, USER_END}, map::{PAGE_SIZE, phys_to_virt}, paging};

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
/// Largest memory a program may ask for in its segments.
const MAX_IMAGE: u64 = 64 * 1024 * 1024;
/// Programs may not map the first 64 KiB (keeps null-pointer bugs visible).
const MIN_ADDR: u64 = 0x1_0000;

#[derive(Debug, PartialEq, Eq)]
pub enum ElfError {
    TooSmall,
    NotElf,
    Not64BitLittleEndian,
    NotX86_64,
    NotExecutable,
    BadProgramHeaders,
    BadSegment,
    TooBig,
    NoMemory,
}

impl ElfError {
    #[allow(dead_code)] // used by the self-tests now, by the shell (`run`) later
    pub fn message(&self) -> &'static str {
        match self {
            ElfError::TooSmall => "fisier prea mic",
            ElfError::NotElf => "nu e fisier ELF",
            ElfError::Not64BitLittleEndian => "nu e ELF pe 64 de biti, little-endian",
            ElfError::NotX86_64 => "nu e pentru x86_64",
            ElfError::NotExecutable => "nu e executabil static (tip EXEC)",
            ElfError::BadProgramHeaders => "tabel de segmente invalid",
            ElfError::BadSegment => "segment in afara fisierului sau a memoriei programului",
            ElfError::TooBig => "programul cere prea multa memorie",
            ElfError::NoMemory => "memorie insuficienta",
        }
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

struct Segment {
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    flags: u32,
}

/// Checks `file` and returns its entry point and loadable segments.
fn parse(file: &[u8]) -> Result<(u64, alloc::vec::Vec<Segment>), ElfError> {
    if file.len() < 64 {
        return Err(ElfError::TooSmall);
    }
    if &file[0..4] != b"\x7fELF" {
        return Err(ElfError::NotElf);
    }
    if file[4] != 2 || file[5] != 1 {
        return Err(ElfError::Not64BitLittleEndian);
    }
    if u16_at(file, 18) != 0x3E {
        return Err(ElfError::NotX86_64);
    }
    if u16_at(file, 16) != 2 {
        return Err(ElfError::NotExecutable);
    }
    let entry = u64_at(file, 24);
    let phoff = u64_at(file, 32);
    let phentsize = u16_at(file, 54) as u64;
    let phnum = u16_at(file, 56) as u64;
    if phentsize != 56 || phnum == 0 || phnum > 64 || phoff.checked_add(phnum * 56).is_none_or(|e| e > file.len() as u64) {
        return Err(ElfError::BadProgramHeaders);
    }
    let mut segments = alloc::vec::Vec::new();
    let mut total = 0u64;
    for i in 0..phnum {
        let h = (phoff + i * 56) as usize;
        if u32_at(file, h) != PT_LOAD {
            continue;
        }
        let s = Segment {
            flags: u32_at(file, h + 4),
            offset: u64_at(file, h + 8),
            vaddr: u64_at(file, h + 16),
            filesz: u64_at(file, h + 32),
            memsz: u64_at(file, h + 40),
        };
        let file_end = s.offset.checked_add(s.filesz);
        let mem_end = s.vaddr.checked_add(s.memsz);
        if s.filesz > s.memsz
            || file_end.is_none_or(|e| e > file.len() as u64)
            || mem_end.is_none_or(|e| e > USER_END)
            || s.vaddr < MIN_ADDR
        {
            return Err(ElfError::BadSegment);
        }
        total += s.memsz;
        if total > MAX_IMAGE {
            return Err(ElfError::TooBig);
        }
        segments.push(s);
    }
    let in_code = segments.iter().any(|s| s.flags & PF_X != 0 && (s.vaddr..s.vaddr + s.memsz).contains(&entry));
    if segments.is_empty() || !in_code {
        return Err(ElfError::BadSegment);
    }
    Ok((entry, segments))
}

/// Maps the segments of `file` into `space` (zeroed memory, file bytes copied in, `.bss` left zero)
/// and returns the entry point.
pub fn load(file: &[u8], space: &mut AddressSpace) -> Result<u64, ElfError> {
    let (entry, segments) = parse(file)?;
    for s in &segments {
        let mut flags = 0;
        if s.flags & PF_W != 0 {
            flags |= paging::WRITABLE;
        }
        if s.flags & PF_X == 0 {
            flags |= paging::NO_EXECUTE;
        }
        let first = s.vaddr & !(PAGE_SIZE - 1);
        let mut page = first;
        while page < s.vaddr + s.memsz {
            // Two segments may share a page; the first one's rights stay (user.ld keeps them apart).
            let frame = match space.translate(page) {
                Some(f) => f,
                None => space.map_new(page, flags).map_err(|_| ElfError::NoMemory)?,
            };
            // Copy the part of [vaddr, vaddr + filesz) that falls in this page.
            let from = page.max(s.vaddr);
            let to = (page + PAGE_SIZE).min(s.vaddr + s.filesz);
            if from < to {
                let src = &file[(s.offset + (from - s.vaddr)) as usize..(s.offset + (to - s.vaddr)) as usize];
                // SAFETY: `frame` belongs to `space` and is reachable through the HHDM; the copy stays in the page.
                unsafe {
                    core::ptr::copy_nonoverlapping(src.as_ptr(), phys_to_virt(frame).add((from - page) as usize), src.len())
                };
            }
            page += PAGE_SIZE;
        }
    }
    Ok(entry)
}
