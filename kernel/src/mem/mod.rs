//! Memory management: physical memory map, frame allocator (and later paging and the kernel heap).

pub mod frame;
pub mod map;

/// Initialises everything memory-related. Returns `false` if the kernel cannot manage memory.
pub fn init() -> bool {
    map::init() && frame::init()
}
