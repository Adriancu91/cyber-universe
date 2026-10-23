//! Address spaces for processes: every one has its own PML4. The upper half (entries 256..512, the
//! kernel) is shared with the kernel's tables; the lower half belongs to the process.

use super::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;

/// Highest address (exclusive) a process may use: the end of the lower canonical half.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

/// Where memory regions asked with `mem_map` (K6-03) are placed: far above the program and below its stack.
pub const REGIONS_START: u64 = 0x0000_1000_0000_0000;
pub const REGIONS_END: u64 = 0x0000_7000_0000_0000;
/// Frames always left to the kernel (16 MiB): a program cannot take the last of the memory.
const KERNEL_RESERVE: u64 = 4096;

/// A memory region of a program (anonymous, zero-filled, readable and writable, not executable).
struct Region {
    start: u64,
    len: u64,
}

pub struct AddressSpace {
    root: u64,
    regions: Vec<Region>,
    /// Next free address for a region (they never overlap; freed address ranges are not reused).
    next_region: u64,
}

/// What `map_region` did (for the log and the tests).
pub struct Mapped {
    pub addr: u64,
    pub huge_pages: u64,
    pub small_pages: u64,
}

#[allow(dead_code)] // used by the self-tests now, by processes (P2-05, P2-06)
impl AddressSpace {
    /// A new, empty user space that sees the kernel. `None` if memory is exhausted.
    pub fn new() -> Option<Self> {
        let root = frame::alloc_frame()?;
        let kernel = paging::kernel_root();
        // SAFETY: `root` is a fresh frame and `kernel` the kernel's PML4, both reachable through the HHDM.
        unsafe {
            let dst = phys_to_virt(root) as *mut u64;
            let src = phys_to_virt(kernel) as *const u64;
            core::ptr::write_bytes(dst, 0, 256);
            core::ptr::copy_nonoverlapping(src.add(256), dst.add(256), 256);
        }
        Some(Self { root, regions: Vec::new(), next_region: REGIONS_START })
    }

    /// Maps a new region of `len` bytes (rounded up to 4 KiB), zero-filled, readable and writable, at an address
    /// aligned to 2 MiB. Memory is taken at once (not at first use): a program that asks for a model's memory uses
    /// all of it, and a failure is better reported here than as a fault in the middle of a computation. Whole 2 MiB
    /// pieces use large pages when the physical memory has free aligned 2 MiB blocks, the rest 4 KiB pages.
    pub fn map_region(&mut self, len: u64) -> Result<Mapped, paging::MapError> {
        let len = len.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        if len == 0 {
            return Err(paging::MapError::NotAligned);
        }
        let start = self.next_region.next_multiple_of(paging::HUGE_PAGE);
        if start.checked_add(len).is_none_or(|end| end > REGIONS_END) {
            return Err(paging::MapError::NoMemory);
        }
        if len / PAGE_SIZE + KERNEL_RESERVE > frame::free_frames() {
            return Err(paging::MapError::NoMemory);
        }
        let flags = paging::WRITABLE | paging::NO_EXECUTE | paging::USER;
        let (mut huge, mut small) = (0, 0);
        let mut off = 0;
        let mut result = Ok(());
        while off < len {
            let virt = start + off;
            if len - off >= paging::HUGE_PAGE {
                if let Some(f) = frame::alloc_huge() {
                    // SAFETY: 512 fresh contiguous frames, reachable through the HHDM.
                    unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, paging::HUGE_PAGE as usize) };
                    if let Err(e) = paging::map_huge_in(self.root, virt, f, flags) {
                        for k in 0..paging::HUGE_PAGE / PAGE_SIZE {
                            frame::free_frame(f + k * PAGE_SIZE);
                        }
                        result = Err(e);
                        break;
                    }
                    huge += 1;
                    off += paging::HUGE_PAGE;
                    continue;
                }
            }
            if let Err(e) = self.map_new(virt, paging::WRITABLE | paging::NO_EXECUTE) {
                result = Err(e);
                break;
            }
            small += 1;
            off += PAGE_SIZE;
        }
        if let Err(e) = result {
            self.unmap_range(start, off);
            return Err(e);
        }
        self.regions.push(Region { start, len });
        // A free 2 MiB gap after each region: a program writing past its end faults instead of hitting the next one.
        self.next_region = start + len.next_multiple_of(paging::HUGE_PAGE) + paging::HUGE_PAGE;
        Ok(Mapped { addr: start, huge_pages: huge, small_pages: small })
    }

    /// Frees the region that starts at `addr` (`len` must be its length, or 0 for "the whole region").
    /// Returns the number of frames given back, or `None` if there is no such region.
    pub fn unmap_region(&mut self, addr: u64, len: u64) -> Option<u64> {
        let i = self.regions.iter().position(|r| r.start == addr && (len == 0 || len.div_ceil(PAGE_SIZE) * PAGE_SIZE == r.len))?;
        let r = self.regions.swap_remove(i);
        Some(self.unmap_range(r.start, r.len))
    }

    /// Unmaps and frees `[start, start + len)` (large and small pages). Returns the frames freed.
    fn unmap_range(&mut self, start: u64, len: u64) -> u64 {
        let mut freed = 0;
        let mut off = 0;
        while off < len {
            let virt = start + off;
            if virt % paging::HUGE_PAGE == 0 {
                if let Some(f) = paging::unmap_huge_in(self.root, virt) {
                    for k in 0..paging::HUGE_PAGE / PAGE_SIZE {
                        frame::free_frame(f + k * PAGE_SIZE);
                    }
                    freed += paging::HUGE_PAGE / PAGE_SIZE;
                    off += paging::HUGE_PAGE;
                    continue;
                }
            }
            if let Some(f) = paging::unmap_in(self.root, virt) {
                frame::free_frame(f);
                freed += 1;
            }
            off += PAGE_SIZE;
        }
        freed
    }

    /// Bytes in the program's memory regions.
    pub fn region_bytes(&self) -> u64 {
        self.regions.iter().map(|r| r.len).sum()
    }

    /// Physical address of this space's PML4 (the value for CR3).
    pub fn root(&self) -> u64 {
        self.root
    }

    /// Maps a fresh, zeroed frame at the user address `virt`. The frame belongs to this space and is
    /// freed with it.
    pub fn map_new(&mut self, virt: u64, flags: u64) -> Result<u64, paging::MapError> {
        if virt >= USER_END {
            return Err(paging::MapError::NotAligned);
        }
        let f = frame::alloc_frame().ok_or(paging::MapError::NoMemory)?;
        // SAFETY: fresh frame, reachable through the HHDM.
        unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, PAGE_SIZE as usize) };
        if let Err(e) = paging::map_in(self.root, virt, f, flags | paging::USER) {
            frame::free_frame(f);
            return Err(e);
        }
        Ok(f)
    }

    /// Physical address behind the user address `virt` in this space.
    pub fn translate(&self, virt: u64) -> Option<u64> {
        paging::translate_in(self.root, virt)
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // Never free the tables in use.
        if paging::cr3() == self.root {
            // SAFETY: the kernel's tables map the kernel exactly like this space's upper half.
            unsafe { paging::load(paging::kernel_root()) };
        }
        paging::free_lower_half(self.root);
    }
}
