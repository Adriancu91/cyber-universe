//! Memory management: physical memory map, frame allocator, paging and the kernel heap.

pub mod address_space;
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
    match paging::adopt_tables() {
        Ok(n) => crate::info!("mem: tabele de pagini proprii active ({} tabele copiate de la Limine)", n),
        Err(e) => {
            crate::error!("mem: nu pot copia tabelele de pagini: {:?}", e);
            return false;
        }
    }
    heap::init()
}

/// Gives the bootloader's memory to the frame allocator. Must run after everything that reads Limine responses.
pub fn reclaim_bootloader() {
    let bytes = frame::reclaim_bootloader();
    crate::info!("mem: recuperat {} KiB din memoria bootloader-ului", bytes / 1024);
}
