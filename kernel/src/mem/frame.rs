//! Physical frame allocator: one bit per 4 KiB frame (1 = used, 0 = free).
//!
//! Only `Usable` regions are handed out at first. Bootloader-reclaimable memory (Limine's page tables,
//! boot stack and responses) is added by `reclaim_bootloader` once the kernel no longer needs any of it.

use super::map::{self, Kind, PAGE_SIZE};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Bitmap {
    bits: *mut u8,
    frames: u64,
    free: u64,
    next: u64,
}

// SAFETY: access is serialised by LOCK.
unsafe impl Send for Bitmap {}

static mut BITMAP: Bitmap = Bitmap { bits: core::ptr::null_mut(), frames: 0, free: 0, next: 0 };
static LOCK: AtomicBool = AtomicBool::new(false);
/// Frames in usable regions at boot (free or not), for the memory-use report.
static USABLE: AtomicU64 = AtomicU64::new(0);

fn with_bitmap<R>(f: impl FnOnce(&mut Bitmap) -> R) -> R {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: LOCK grants exclusive access until released below.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(BITMAP) });
    LOCK.store(false, Ordering::Release);
    r
}

impl Bitmap {
    fn is_used(&self, frame: u64) -> bool {
        // SAFETY: frame < self.frames, so the byte is inside the bitmap.
        unsafe { *self.bits.add((frame / 8) as usize) & (1 << (frame % 8)) != 0 }
    }

    fn set(&mut self, frame: u64, used: bool) {
        // SAFETY: frame < self.frames, so the byte is inside the bitmap.
        unsafe {
            let byte = self.bits.add((frame / 8) as usize);
            if used { *byte |= 1 << (frame % 8) } else { *byte &= !(1 << (frame % 8)) }
        }
    }
}

/// Builds the bitmap from the memory map. Must run after `map::init`.
pub fn init() -> bool {
    let regions = map::regions();
    // Bootloader memory is RAM too: the bitmap covers it so it can be reclaimed later.
    let top = regions
        .iter()
        .filter(|r| matches!(r.kind, Kind::Usable | Kind::BootloaderReclaimable))
        .map(|r| r.end())
        .max()
        .unwrap_or(0);
    let frames = top / PAGE_SIZE;
    let bitmap_bytes = frames.div_ceil(8);
    let bitmap_frames = bitmap_bytes.div_ceil(PAGE_SIZE);

    // Put the bitmap at the start of the first usable region that can hold it.
    let Some(home) = regions
        .iter()
        .filter(|r| r.kind == Kind::Usable)
        .map(|r| (r.base.next_multiple_of(PAGE_SIZE), r.end()))
        .find(|(start, end)| end.saturating_sub(*start) >= bitmap_frames * PAGE_SIZE)
        .map(|(start, _)| start)
    else {
        crate::error!("mem: nu am loc pentru harta cadrelor ({} octeti)", bitmap_bytes);
        return false;
    };

    with_bitmap(|b| {
        b.bits = map::phys_to_virt(home);
        b.frames = frames;
        // SAFETY: the region [home, home + bitmap_frames pages) is usable RAM, reachable through the HHDM.
        unsafe { core::ptr::write_bytes(b.bits, 0xFF, bitmap_bytes as usize) } // everything used...
        for r in regions.iter().filter(|r| r.kind == Kind::Usable) {
            let first = r.base.div_ceil(PAGE_SIZE); // ...then free whole frames of usable regions
            let last = r.end() / PAGE_SIZE;
            for f in first..last {
                b.set(f, false);
                b.free += 1;
            }
        }
        USABLE.store(b.free, Ordering::Relaxed);
        let home_frame = home / PAGE_SIZE;
        for f in home_frame..home_frame + bitmap_frames {
            b.set(f, true); // the bitmap's own frames
            b.free -= 1;
        }
        if !b.is_used(0) {
            b.set(0, true); // never hand out physical address 0
            b.free -= 1;
        }
    });

    crate::info!(
        "mem: alocator de cadre gata: {} cadre libere ({} MiB), harta de {} KiB la {:#x}",
        free_frames(),
        free_frames() * PAGE_SIZE / (1024 * 1024),
        bitmap_bytes / 1024,
        home
    );
    true
}

/// Allocates one 4 KiB physical frame and returns its physical address.
pub fn alloc_frame() -> Option<u64> {
    with_bitmap(|b| {
        if b.free == 0 {
            return None;
        }
        for i in 0..b.frames {
            let f = (b.next + i) % b.frames;
            if !b.is_used(f) {
                b.set(f, true);
                b.free -= 1;
                b.next = f + 1;
                return Some(f * PAGE_SIZE);
            }
        }
        None
    })
}

/// Releases a frame obtained from `alloc_frame`. Double frees and invalid addresses are logged and ignored.
pub fn free_frame(addr: u64) {
    let ok = with_bitmap(|b| {
        let f = addr / PAGE_SIZE;
        if addr % PAGE_SIZE != 0 || f >= b.frames || !b.is_used(f) {
            return false;
        }
        b.set(f, false);
        b.free += 1;
        true
    });
    if !ok {
        crate::warn!("mem: eliberare invalida sau dubla a cadrului {:#x}, ignorata", addr);
    }
}

/// Frees every whole frame of the `BootloaderReclaimable` regions. Returns the number of bytes reclaimed.
/// Call only after the kernel runs on its own page tables and stack and has copied every Limine response.
pub fn reclaim_bootloader() -> u64 {
    let mut reclaimed = 0;
    with_bitmap(|b| {
        for r in map::regions().iter().filter(|r| r.kind == Kind::BootloaderReclaimable) {
            for f in r.base.div_ceil(PAGE_SIZE)..(r.end() / PAGE_SIZE).min(b.frames) {
                if b.is_used(f) {
                    b.set(f, false);
                    b.free += 1;
                    reclaimed += 1;
                }
            }
        }
    });
    USABLE.fetch_add(reclaimed, Ordering::Relaxed);
    reclaimed * PAGE_SIZE
}

/// Frames of usable RAM currently taken (bitmap, page tables, heap, anything allocated).
pub fn used_frames() -> u64 {
    USABLE.load(Ordering::Relaxed) - free_frames()
}

/// Number of free frames.
pub fn free_frames() -> u64 {
    with_bitmap(|b| b.free)
}
