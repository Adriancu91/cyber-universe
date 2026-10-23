//! Kernel heap: makes `Box`, `Vec`, `String` from the `alloc` crate work inside the kernel.
//!
//! A first-fit free list kept sorted by address, with neighbouring free blocks merged on release.
//! The heap lives in its own virtual area and grows on demand by mapping new frames.
//! Design notes: OSDev Wiki "Memory Allocation" (description only); written for this project.

use super::map::PAGE_SIZE;
use super::{frame, paging};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::AtomicBool;

/// Start of the heap's virtual area (well above the HHDM, unused by Limine).
pub const HEAP_START: u64 = 0xFFFF_9000_0000_0000;
/// Upper limit for the heap's growth.
pub const HEAP_MAX: u64 = 256 * 1024 * 1024;
const INITIAL_SIZE: u64 = 1024 * 1024;
/// Every block is a multiple of this, which is also the minimum alignment and the size of a free-list node.
const UNIT: usize = 16;

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct Heap {
    head: *mut FreeBlock,
    mapped: u64,
    used: usize,
}

static mut HEAP: Heap = Heap { head: null_mut(), mapped: 0, used: 0 };
static LOCK: AtomicBool = AtomicBool::new(false);

fn with_heap<R>(f: impl FnOnce(&mut Heap) -> R) -> R {
    let irq = crate::sched::spin::lock(&LOCK);
    // SAFETY: LOCK grants exclusive access until released below.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(HEAP) });
    crate::sched::spin::unlock(&LOCK, irq);
    r
}

impl Heap {
    /// Inserts a free region into the sorted list and merges it with its neighbours.
    ///
    /// # Safety
    /// `[addr, addr + size)` must be mapped, unused heap memory; `addr` and `size` multiples of UNIT.
    unsafe fn insert(&mut self, addr: usize, size: usize) {
        let mut prev: *mut FreeBlock = null_mut();
        let mut cur = self.head;
        // SAFETY (whole block): every node in the list is a valid FreeBlock inside mapped heap memory.
        unsafe {
            while !cur.is_null() && (cur as usize) < addr {
                prev = cur;
                cur = (*cur).next;
            }
            let node = addr as *mut FreeBlock;
            node.write(FreeBlock { size, next: cur });
            if !cur.is_null() && addr + size == cur as usize {
                (*node).size += (*cur).size;
                (*node).next = (*cur).next;
            }
            if prev.is_null() {
                self.head = node;
            } else if prev as usize + (*prev).size == addr {
                (*prev).size += (*node).size;
                (*prev).next = (*node).next;
            } else {
                (*prev).next = node;
            }
        }
    }

    /// Maps at least `bytes` more memory at the end of the heap and adds it to the free list.
    fn grow(&mut self, bytes: u64) -> bool {
        let bytes = bytes.next_multiple_of(PAGE_SIZE);
        if self.mapped + bytes > HEAP_MAX {
            return false;
        }
        let start = HEAP_START + self.mapped;
        for off in (0..bytes).step_by(PAGE_SIZE as usize) {
            let Some(f) = frame::alloc_frame() else { return false };
            if paging::map(start + off, f, paging::WRITABLE | paging::NO_EXECUTE).is_err() {
                frame::free_frame(f);
                return false;
            }
            self.mapped += PAGE_SIZE;
        }
        // SAFETY: the range was just mapped and nothing uses it.
        unsafe { self.insert(start as usize, bytes as usize) };
        true
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(UNIT).next_multiple_of(UNIT);
        let align = layout.align().max(UNIT);
        loop {
            let mut prev: *mut FreeBlock = null_mut();
            let mut cur = self.head;
            // SAFETY: list nodes are valid FreeBlocks in mapped heap memory.
            unsafe {
                while !cur.is_null() {
                    let start = cur as usize;
                    let end = start + (*cur).size;
                    let mut alloc_start = start.next_multiple_of(align);
                    // A front gap must be able to hold a free-list node.
                    if alloc_start != start && alloc_start - start < UNIT {
                        alloc_start = (start + UNIT).next_multiple_of(align);
                    }
                    let alloc_end = alloc_start + size;
                    if alloc_end <= end {
                        let next = (*cur).next;
                        if prev.is_null() { self.head = next } else { (*prev).next = next }
                        if alloc_start > start {
                            self.insert(start, alloc_start - start);
                        }
                        if end > alloc_end {
                            self.insert(alloc_end, end - alloc_end);
                        }
                        self.used += size;
                        return alloc_start as *mut u8;
                    }
                    prev = cur;
                    cur = (*cur).next;
                }
            }
            let want = (size + align) as u64;
            if !self.grow(want.max(INITIAL_SIZE)) {
                return null_mut();
            }
        }
    }

    fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(UNIT).next_multiple_of(UNIT);
        self.used -= size;
        // SAFETY: `ptr` came from `alloc` with the same layout, so the block is ours to return.
        unsafe { self.insert(ptr as usize, size) };
    }
}

pub struct KernelAllocator;

// SAFETY: allocations never overlap (each block leaves the free list while in use) and are aligned as requested.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        with_heap(|h| h.alloc(layout))
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        with_heap(|h| h.dealloc(ptr, layout))
    }
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

/// Maps the initial heap area. Must run after paging and the frame allocator are ready.
pub fn init() -> bool {
    let ok = with_heap(|h| h.grow(INITIAL_SIZE));
    if ok {
        crate::info!("mem: heap gata la {:#x}, {} KiB initial", HEAP_START, INITIAL_SIZE / 1024);
    } else {
        crate::error!("mem: heap-ul nu a putut fi creat");
    }
    ok
}

/// (bytes in use, bytes mapped)
pub fn stats() -> (usize, u64) {
    with_heap(|h| (h.used, h.mapped))
}
