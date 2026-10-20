//! 4-level paging (x86_64): map, unmap and translate virtual addresses.
//!
//! For now the kernel keeps the page tables Limine built (kernel image + HHDM) and adds its own mappings
//! on top. Table frames come from the frame allocator and are accessed through the HHDM.
//! Written from Intel SDM Vol. 3A ch. 4 and the OSDev Wiki "Paging" description.

use super::frame;
use super::map::{PAGE_SIZE, phys_to_virt};
use core::arch::asm;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
#[allow(dead_code)] // used from Phase 2 (user mode)
pub const USER: u64 = 1 << 2;
const HUGE: u64 = 1 << 7;
pub const NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

#[derive(Debug, PartialEq, Eq)]
pub enum MapError {
    NoMemory,
    AlreadyMapped,
    HugePageInTheWay,
    NotAligned,
}

fn cr3() -> u64 {
    let v: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v & ADDR_MASK
}

fn table(phys: u64) -> &'static mut [u64; 512] {
    // SAFETY: `phys` is the physical address of a page table (from CR3 or a present table entry);
    // every physical page is reachable through the HHDM.
    unsafe { &mut *(phys_to_virt(phys) as *mut [u64; 512]) }
}

fn indices(virt: u64) -> [usize; 4] {
    [(virt >> 39) as usize & 511, (virt >> 30) as usize & 511, (virt >> 21) as usize & 511, (virt >> 12) as usize & 511]
}

fn invlpg(virt: u64) {
    // SAFETY: invalidating a TLB entry is always safe.
    unsafe { asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags)) }
}

/// Turns on the no-execute bit support (EFER.NXE), so `NO_EXECUTE` can be used in mappings.
pub fn init() {
    const IA32_EFER: u32 = 0xC000_0080;
    // SAFETY: EFER exists on every x86_64 CPU; setting NXE only enables the NX bit in page tables.
    unsafe {
        let (lo, hi): (u32, u32);
        asm!("rdmsr", in("ecx") IA32_EFER, out("eax") lo, out("edx") hi, options(nomem, nostack));
        asm!("wrmsr", in("ecx") IA32_EFER, in("eax") lo | (1 << 11), in("edx") hi, options(nomem, nostack));
    }
}

/// Maps one 4 KiB page `virt` -> `phys` with `flags` (PRESENT is added automatically).
pub fn map(virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
    if virt % PAGE_SIZE != 0 || phys % PAGE_SIZE != 0 {
        return Err(MapError::NotAligned);
    }
    let idx = indices(virt);
    let mut t = table(cr3());
    for &i in &idx[..3] {
        let entry = t[i];
        if entry & PRESENT == 0 {
            let new = frame::alloc_frame().ok_or(MapError::NoMemory)?;
            // SAFETY: freshly allocated frame, reachable through the HHDM.
            unsafe { core::ptr::write_bytes(phys_to_virt(new), 0, PAGE_SIZE as usize) };
            // Intermediate entries are permissive; the final entry decides the real rights.
            t[i] = new | PRESENT | WRITABLE | (flags & USER);
        } else if entry & HUGE != 0 {
            return Err(MapError::HugePageInTheWay);
        }
        t = table(t[i] & ADDR_MASK);
    }
    if t[idx[3]] & PRESENT != 0 {
        return Err(MapError::AlreadyMapped);
    }
    t[idx[3]] = phys | flags | PRESENT;
    invlpg(virt);
    Ok(())
}

#[allow(dead_code)] // API used by tests now, by process memory in Phase 2
/// Removes the mapping of `virt` and returns the physical frame it pointed to (the frame is NOT freed).
pub fn unmap(virt: u64) -> Option<u64> {
    let idx = indices(virt);
    let mut t = table(cr3());
    for &i in &idx[..3] {
        let entry = t[i];
        if entry & PRESENT == 0 || entry & HUGE != 0 {
            return None;
        }
        t = table(entry & ADDR_MASK);
    }
    let entry = t[idx[3]];
    if entry & PRESENT == 0 {
        return None;
    }
    t[idx[3]] = 0;
    invlpg(virt);
    Some(entry & ADDR_MASK)
}

#[allow(dead_code)] // API used by tests now, by process memory in Phase 2
/// Physical address that `virt` currently maps to, following huge pages too.
pub fn translate(virt: u64) -> Option<u64> {
    let idx = indices(virt);
    let mut t = table(cr3());
    for (level, &i) in idx.iter().enumerate() {
        let entry = t[i];
        if entry & PRESENT == 0 {
            return None;
        }
        // Level 1 (PDPT) => 1 GiB page, level 2 (PD) => 2 MiB page, level 3 (PT) => 4 KiB page.
        let page_bits = match level {
            1 if entry & HUGE != 0 => 30,
            2 if entry & HUGE != 0 => 21,
            3 => 12,
            _ => {
                t = table(entry & ADDR_MASK);
                continue;
            }
        };
        let offset_mask = (1u64 << page_bits) - 1;
        return Some((entry & ADDR_MASK & !offset_mask) | (virt & offset_mask));
    }
    None
}
