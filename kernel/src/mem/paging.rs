//! 4-level paging (x86_64): map, unmap and translate virtual addresses.
//!
//! At boot the kernel copies the page tables Limine built (kernel image with per-segment rights + HHDM) into
//! frames it owns (`adopt_tables`), so the bootloader's memory can be reclaimed afterwards. Table frames come
//! from the frame allocator and are accessed through the HHDM.
//! Written from Intel SDM Vol. 3A ch. 4 and the OSDev Wiki "Paging" description.

use super::frame;
use super::map::{PAGE_SIZE, phys_to_virt};
use core::arch::asm;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
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

static KERNEL_ROOT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Physical address of the kernel's own PML4 (the one built by `adopt_tables`).
pub fn kernel_root() -> u64 {
    KERNEL_ROOT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Physical address of the PML4 in use.
pub fn cr3() -> u64 {
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

/// Turns on the no-execute bit support (EFER.NXE), so `NO_EXECUTE` can be used in mappings, and CR0.WP,
/// so read-only pages are read-only for the kernel too.
pub fn init() {
    // SAFETY: setting CR0.WP only makes writes to read-only pages fault in ring 0.
    unsafe {
        let cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        asm!("mov cr0, {}", in(reg) cr0 | (1 << 16), options(nostack, preserves_flags));
    }
    const IA32_EFER: u32 = 0xC000_0080;
    // SAFETY: EFER exists on every x86_64 CPU; setting NXE only enables the NX bit in page tables.
    unsafe {
        let (lo, hi): (u32, u32);
        asm!("rdmsr", in("ecx") IA32_EFER, out("eax") lo, out("edx") hi, options(nomem, nostack));
        asm!("wrmsr", in("ecx") IA32_EFER, in("eax") lo | (1 << 11), in("edx") hi, options(nomem, nostack));
    }
}

/// Maps one 4 KiB page `virt` -> `phys` with `flags` (PRESENT is added automatically) in the current tables.
pub fn map(virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
    map_in(cr3(), virt, phys, flags)
}

/// Like `map`, in the tables whose PML4 is at physical address `root`.
pub fn map_in(root: u64, virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
    if virt % PAGE_SIZE != 0 || phys % PAGE_SIZE != 0 {
        return Err(MapError::NotAligned);
    }
    let idx = indices(virt);
    let mut t = table(root);
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
    unmap_in(cr3(), virt)
}

/// Like `unmap`, in the tables whose PML4 is at physical address `root`.
pub fn unmap_in(root: u64, virt: u64) -> Option<u64> {
    let idx = indices(virt);
    let mut t = table(root);
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
    translate_in(cr3(), virt)
}

/// Like `translate`, in the tables whose PML4 is at physical address `root`.
pub fn translate_in(root: u64, virt: u64) -> Option<u64> {
    let idx = indices(virt);
    let mut t = table(root);
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

/// Recursively copies the table at `phys` (paging level `level`: 4 = PML4 ... 1 = PT) into fresh frames.
/// Leaf entries (4 KiB pages and huge pages) are copied as they are. Returns the new table's address.
fn clone_table(phys: u64, level: u8, copied: &mut u64) -> Result<u64, MapError> {
    let new = frame::alloc_frame().ok_or(MapError::NoMemory)?;
    *copied += 1;
    let (src, dst) = (table(phys), table(new));
    // The lower half (user space, entries 0..256 of the PML4) is not copied: the kernel lives in the upper half.
    let first = if level == 4 { 256 } else { 0 };
    for i in 0..512 {
        let e = src[i];
        dst[i] = if i < first || e & PRESENT == 0 {
            0
        } else if level == 1 || e & HUGE != 0 {
            e
        } else {
            (e & !ADDR_MASK) | clone_table(e & ADDR_MASK, level - 1, copied)?
        };
    }
    Ok(new)
}

/// Replaces Limine's page tables by an identical copy in kernel-owned frames and loads it into CR3.
/// Returns the number of table frames copied. After this, no page table lives in bootloader memory.
pub fn adopt_tables() -> Result<u64, MapError> {
    let mut copied = 0;
    let new = clone_table(cr3(), 4, &mut copied)?;
    // Every upper-half PML4 entry gets its table now, so later kernel mappings never add a PML4 entry
    // and all address spaces (which copy these 256 entries) keep seeing the same kernel.
    let pml4 = table(new);
    for e in pml4.iter_mut().skip(256) {
        if *e & PRESENT == 0 {
            let t = frame::alloc_frame().ok_or(MapError::NoMemory)?;
            // SAFETY: freshly allocated frame, reachable through the HHDM.
            unsafe { core::ptr::write_bytes(phys_to_virt(t), 0, PAGE_SIZE as usize) };
            *e = t | PRESENT | WRITABLE;
        }
    }
    KERNEL_ROOT.store(new, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: the new PML4 maps exactly what the current one maps in the upper half (kernel image, HHDM,
    // heap, MMIO); the code, stack and data in use are all in the upper half.
    unsafe { asm!("mov cr3, {}", in(reg) new, options(nostack, preserves_flags)) }
    Ok(copied)
}

/// Switches to the tables whose PML4 is at `root`, if they are not already in use.
///
/// # Safety
/// `root` must be a PML4 whose upper half is the kernel's (from `kernel_root` or an `AddressSpace`).
pub unsafe fn load(root: u64) {
    if cr3() != root {
        // SAFETY: the caller guarantees the kernel (code, stack, data) stays mapped in `root`.
        unsafe { asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags)) }
    }
}

/// Frees every table and every mapped frame of the lower half of `root`, then `root` itself.
/// Huge pages are not used in the lower half. Returns the number of frames freed.
pub fn free_lower_half(root: u64) -> u64 {
    fn free_level(t: u64, level: u8, count: &mut u64) {
        let entries = table(t);
        let end = if level == 4 { 256 } else { 512 };
        for e in entries.iter_mut().take(end) {
            if *e & PRESENT != 0 {
                if level > 1 {
                    free_level(*e & ADDR_MASK, level - 1, count);
                } else {
                    frame::free_frame(*e & ADDR_MASK);
                    *count += 1;
                }
                *e = 0;
            }
        }
        frame::free_frame(t);
        *count += 1;
    }
    let mut count = 0;
    free_level(root, 4, &mut count);
    count
}

/// True if `virt` is a 4 KiB page mapped with USER rights at every level (and WRITABLE, if `write`)
/// in the tables at `root`: the check done before the kernel touches memory a program pointed to.
pub fn user_accessible(root: u64, virt: u64, write: bool) -> bool {
    let need = PRESENT | USER | if write { WRITABLE } else { 0 };
    let idx = indices(virt);
    let mut t = table(root);
    for (level, &i) in idx.iter().enumerate() {
        let e = t[i];
        if e & need != need || (level < 3 && e & HUGE != 0) {
            return false;
        }
        if level == 3 {
            return true;
        }
        t = table(e & ADDR_MASK);
    }
    false
}
