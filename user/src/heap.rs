//! A heap for the programs that want one (P6-04): `cyber_user::heap!(512 * 1024);` gives the program a global
//! allocator over a static area of that size, so it can use `alloc` (Vec, String, the JSON reader).
//!
//! The allocator hands out memory from the start of the area upwards. Freeing the last block gives its room back,
//! and when nothing is allocated any more the whole area is free again; other holes are only reused then. Enough
//! for a command that does one job and exits; a long-running program that frees in random order needs more.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

pub struct Heap<const N: usize> {
    area: UnsafeCell<[u8; N]>,
    /// (first free offset, number of live blocks)
    state: UnsafeCell<(usize, usize)>,
}

// SAFETY: programs have one thread, so the allocator is never entered twice at the same time.
unsafe impl<const N: usize> Sync for Heap<N> {}

impl<const N: usize> Heap<N> {
    pub const fn new() -> Heap<N> {
        Heap { area: UnsafeCell::new([0; N]), state: UnsafeCell::new((0, 0)) }
    }

    /// Bytes still free at the top of the area.
    pub fn free(&self) -> usize {
        // SAFETY: single thread (see Sync above); only read.
        N - unsafe { (*self.state.get()).0 }
    }
}

impl<const N: usize> Default for Heap<N> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<const N: usize> GlobalAlloc for Heap<N> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: single thread (see Sync above), so nobody else touches the state now.
        let state = unsafe { &mut *self.state.get() };
        let base = self.area.get() as usize;
        let start = (base + state.0).next_multiple_of(layout.align()) - base;
        let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
        if end > N {
            return core::ptr::null_mut();
        }
        state.0 = end;
        state.1 += 1;
        (base + start) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: single thread (see Sync above).
        let state = unsafe { &mut *self.state.get() };
        let base = self.area.get() as usize;
        state.1 -= 1;
        if state.1 == 0 {
            state.0 = 0;
        } else if ptr as usize + layout.size() == base + state.0 {
            state.0 = ptr as usize - base; // the last block: its room comes back
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: single thread (see Sync above).
        let state = unsafe { &mut *self.state.get() };
        let base = self.area.get() as usize;
        let off = ptr as usize - base;
        // The last block grows or shrinks in place (a Vec or String being filled).
        if off + layout.size() == state.0 && off + new_size <= N {
            state.0 = off + new_size;
            return ptr;
        }
        if new_size <= layout.size() {
            return ptr;
        }
        // SAFETY: the layout with the new size is valid (GlobalAlloc's contract for realloc).
        let new = unsafe { self.alloc(Layout::from_size_align_unchecked(new_size, layout.align())) };
        if !new.is_null() {
            // SAFETY: both blocks are ours, distinct, and at least layout.size() long.
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, new, layout.size());
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

/// Gives the program a heap of `size` bytes (see the module documentation).
#[macro_export]
macro_rules! heap {
    ($size:expr) => {
        #[global_allocator]
        static HEAP: $crate::heap::Heap<{ $size }> = $crate::heap::Heap::new();
    };
}
