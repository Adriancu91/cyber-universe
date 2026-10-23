//! A heap for the programs that want one (P6-04, P6-05): `cyber_user::heap!(1024 * 1024);` gives the program a global
//! allocator over a static area of that size, so it can use `alloc` (Vec, String, the JSON reader).
//!
//! First fit over a list of blocks laid one after the other in the area: each block starts with a 16-byte header
//! (its size and whether it is free). Allocating splits a large enough free block; freeing joins neighbouring free
//! blocks again, so a long-running program (the Claude chat) can allocate and free in any order. Alignment up to 16
//! bytes (enough for every type our programs use); larger alignments are refused.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

const HEADER: usize = 16;
const MIN_BLOCK: usize = 2 * HEADER;

#[repr(C, align(16))]
struct Area<const N: usize>([u8; N]);

pub struct Heap<const N: usize> {
    area: UnsafeCell<Area<N>>,
    ready: UnsafeCell<bool>,
    /// Held while a thread walks or changes the blocks (programs may have several threads, K6-04).
    lock: AtomicBool,
}

// SAFETY: every access to `area` and `ready` happens with `lock` held (see `locked`).
unsafe impl<const N: usize> Sync for Heap<N> {}

impl<const N: usize> Default for Heap<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Heap<N> {
    pub const fn new() -> Heap<N> {
        Heap { area: UnsafeCell::new(Area([0; N])), ready: UnsafeCell::new(false), lock: AtomicBool::new(false) }
    }

    /// Runs `f` with the heap lock held. A thread that finds it taken gives up its processor after a short spin (the
    /// holder may have been preempted).
    fn locked<R>(&self, f: impl FnOnce() -> R) -> R {
        let mut spins = 0u32;
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins % 64 == 0 {
                crate::yield_now();
            } else {
                core::hint::spin_loop();
            }
        }
        let r = f();
        self.lock.store(false, Ordering::Release);
        r
    }

    /// Usable size of the area (a whole number of 16-byte units).
    const SIZE: usize = N / HEADER * HEADER;

    fn base(&self) -> *mut u8 {
        self.area.get() as *mut u8
    }

    /// (size, free) of the block at `off`.
    fn header(&self, off: usize) -> (usize, bool) {
        // SAFETY: `off` is the start of a block inside the area (the callers walk the block list); headers are
        // 16-byte aligned and inside the area.
        unsafe {
            let p = self.base().add(off) as *const usize;
            (p.read(), p.add(1).read() != 0)
        }
    }

    fn set_header(&self, off: usize, size: usize, free: bool) {
        // SAFETY: as in `header`.
        unsafe {
            let p = self.base().add(off) as *mut usize;
            p.write(size);
            p.add(1).write(usize::from(free));
        }
    }

    fn init(&self) {
        // SAFETY: called with the lock held (see Sync above).
        let ready = unsafe { &mut *self.ready.get() };
        if !*ready {
            self.set_header(0, Self::SIZE, true);
            *ready = true;
        }
    }

    /// Joins every run of neighbouring free blocks.
    fn merge(&self) {
        let mut off = 0;
        while off < Self::SIZE {
            let (size, free) = self.header(off);
            if free {
                let mut end = off + size;
                while end < Self::SIZE {
                    let (next, next_free) = self.header(end);
                    if !next_free {
                        break;
                    }
                    end += next;
                }
                self.set_header(off, end - off, true);
                off = end;
            } else {
                off += size;
            }
        }
    }

    /// Bytes in free blocks (for tests and the program's own information).
    pub fn free(&self) -> usize {
        self.locked(|| self.free_locked())
    }

    fn free_locked(&self) -> usize {
        self.init();
        let (mut off, mut total) = (0, 0);
        while off < Self::SIZE {
            let (size, free) = self.header(off);
            if free {
                total += size - HEADER;
            }
            off += size;
        }
        total
    }
}

impl<const N: usize> Heap<N> {
    fn alloc_locked(&self, layout: Layout) -> *mut u8 {
        if layout.align() > HEADER {
            return core::ptr::null_mut();
        }
        self.init();
        let need = HEADER + layout.size().max(1).next_multiple_of(HEADER);
        let mut off = 0;
        while off < Self::SIZE {
            let (size, free) = self.header(off);
            if free && size >= need {
                if size - need >= MIN_BLOCK {
                    self.set_header(off + need, size - need, true);
                    self.set_header(off, need, false);
                } else {
                    self.set_header(off, size, false);
                }
                // SAFETY: off + HEADER is inside the block just taken.
                return unsafe { self.base().add(off + HEADER) };
            }
            off += size;
        }
        core::ptr::null_mut()
    }

    fn dealloc_locked(&self, ptr: *mut u8) {
        let off = ptr as usize - self.base() as usize - HEADER;
        let (size, _) = self.header(off);
        self.set_header(off, size, true);
        self.merge();
    }
}

unsafe impl<const N: usize> GlobalAlloc for Heap<N> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.locked(|| self.alloc_locked(layout))
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        self.locked(|| self.dealloc_locked(ptr))
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.locked(|| {
            let off = ptr as usize - self.base() as usize - HEADER;
            let (size, _) = self.header(off);
            if HEADER + new_size <= size {
                return ptr; // still fits
            }
            // SAFETY: the layout with the new size is valid (GlobalAlloc's contract for realloc).
            let new = self.alloc_locked(unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) });
            if !new.is_null() {
                // SAFETY: both blocks are ours, distinct, and at least layout.size() long.
                unsafe { core::ptr::copy_nonoverlapping(ptr, new, layout.size()) };
                self.dealloc_locked(ptr);
            }
            new
        })
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
