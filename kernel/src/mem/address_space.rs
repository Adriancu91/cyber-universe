//! Address spaces for processes: every one has its own PML4. The upper half (entries 256..512, the
//! kernel) is shared with the kernel's tables; the lower half belongs to the process.

use super::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};

/// Highest address (exclusive) a process may use: the end of the lower canonical half.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

pub struct AddressSpace {
    root: u64,
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
        Some(Self { root })
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
