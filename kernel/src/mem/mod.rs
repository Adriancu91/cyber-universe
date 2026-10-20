//! Memory management: physical memory map, frame allocator, paging and the kernel heap.

pub mod frame;
pub mod heap;
pub mod map;
pub mod paging;

/// Initialises everything memory-related. Returns `false` if the kernel cannot manage memory.
pub fn init() -> bool {
    if !(map::init() && frame::init()) {
        return false;
    }
    paging::init();
    heap::init()
}
