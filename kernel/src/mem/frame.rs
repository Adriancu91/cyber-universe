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
/// The page below 1 MiB kept at start-up for the processor trampoline (0 = none).
static LOW_PAGE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Takes the page below 1 MiB kept at start-up (once).
pub fn take_low_page() -> Option<u64> {
    let p = LOW_PAGE.swap(0, Ordering::Relaxed);
    (p != 0).then_some(p)
}

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
        // One page below 1 MiB for the trampoline of the other processors (K6-02): a start-up IPI can only point
        // there, and every later allocation would otherwise take low memory first.
        if let Some(f) = (1..0x100).find(|&f| f < b.frames && !b.is_used(f)) {
            b.set(f, true);
            b.free -= 1;
            LOW_PAGE.store(f * PAGE_SIZE, Ordering::Relaxed);
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

/// Allocates `count` physically contiguous frames that all end at or below `limit` (the lowest such run).
#[cfg(feature = "test-high-dma")]
pub fn alloc_contiguous_below(count: u64, limit: u64) -> Option<u64> {
    alloc_contiguous_between(count, 0, limit)
}

/// Allocates `count` physically contiguous frames inside [`low`, `high`) (e.g. a DMA block between 16 MiB and
/// 4 GiB, away from the low memory every boot-time allocation comes from). Searches upwards from `low`.
pub fn alloc_contiguous_between(count: u64, low: u64, high: u64) -> Option<u64> {
    if count == 0 {
        return None;
    }
    with_bitmap(|b| {
        let first_ok = (low.div_ceil(PAGE_SIZE)).max(1);
        let last = (high / PAGE_SIZE).min(b.frames);
        let mut run = 0;
        for f in first_ok..last {
            run = if b.is_used(f) { 0 } else { run + 1 };
            if run == count {
                let first = f + 1 - count;
                for g in first..=f {
                    b.set(g, true);
                }
                b.free -= count;
                return Some(first * PAGE_SIZE);
            }
        }
        None
    })
}

/// DMA guard: frames a device driver handed to its card. The card may write there at any moment, so nothing else
/// may ever get them: `free_frame` refuses them (and logs who asked), `reclaim_bootloader` skips them, and
/// `check_pins` verifies they are all still marked used. Adjacent pins of the same owner are merged.
#[derive(Clone, Copy)]
struct Pin {
    start: u64,
    end: u64,
    owner: &'static str,
}

const MAX_PINS: usize = 64;
static mut PINS: [Pin; MAX_PINS] = [Pin { start: 0, end: 0, owner: "" }; MAX_PINS];
static PIN_COUNT: AtomicU64 = AtomicU64::new(0);
static PIN_LOCK: AtomicBool = AtomicBool::new(false);
/// Frees (or reclaims) of pinned frames refused since boot.
static PIN_REFUSED: AtomicU64 = AtomicU64::new(0);
/// Pins that did not fit in the table (the guard then does not cover them; shown by `check_pins`).
static PIN_LOST: AtomicU64 = AtomicU64::new(0);

fn with_pins<R>(f: impl FnOnce(&mut [Pin; MAX_PINS], &mut usize) -> R) -> R {
    while PIN_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    let mut n = PIN_COUNT.load(Ordering::Relaxed) as usize;
    // SAFETY: PIN_LOCK grants exclusive access until released below.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(PINS) }, &mut n);
    PIN_COUNT.store(n as u64, Ordering::Relaxed);
    PIN_LOCK.store(false, Ordering::Release);
    r
}

/// Marks `pages` frames from `addr` as DMA memory of `owner` (a driver name).
pub fn pin(addr: u64, pages: u64, owner: &'static str) {
    let (start, end) = (addr, addr + pages * PAGE_SIZE);
    with_pins(|pins, n| {
        if let Some(p) = pins[..*n].iter_mut().find(|p| p.owner == owner && (p.end == start || p.start == end)) {
            p.start = p.start.min(start);
            p.end = p.end.max(end);
        } else if *n < MAX_PINS {
            pins[*n] = Pin { start, end, owner };
            *n += 1;
        } else {
            PIN_LOST.fetch_add(1, Ordering::Relaxed);
        }
    });
}

/// The driver that pinned the frame holding `addr`, if any.
pub fn pinned_by(addr: u64) -> Option<&'static str> {
    with_pins(|pins, n| pins[..*n].iter().find(|p| (p.start..p.end).contains(&addr)).map(|p| p.owner))
}

/// (pinned frames, pinned frames the allocator thinks are free, refused frees, pins that did not fit).
/// A non-zero second number means a DMA page could be given to someone else.
pub fn check_pins() -> (u64, u64, u64, u64) {
    let ranges: alloc::vec::Vec<(u64, u64)> = with_pins(|pins, n| pins[..*n].iter().map(|p| (p.start, p.end)).collect());
    let (mut total, mut free) = (0, 0);
    with_bitmap(|b| {
        for (s, e) in ranges {
            for f in s / PAGE_SIZE..(e / PAGE_SIZE).min(b.frames) {
                total += 1;
                if !b.is_used(f) {
                    free += 1;
                }
            }
        }
    });
    (total, free, PIN_REFUSED.load(Ordering::Relaxed), PIN_LOST.load(Ordering::Relaxed))
}

/// Test only: from now on `alloc_frame` starts looking at 4 GiB, so ordinary allocations land above it (as on a
/// laptop with 8 GB). Returns false if there is no RAM above 4 GiB.
#[cfg(feature = "test-high-dma")]
pub fn prefer_above_4g() -> bool {
    with_bitmap(|b| {
        let start = (1u64 << 32) / PAGE_SIZE;
        if start >= b.frames {
            return false;
        }
        b.next = start;
        true
    })
}

/// The end of the highest RAM the allocator knows (for `log rtl`: does the machine have memory above 4 GiB?).
pub fn top_of_ram() -> u64 {
    with_bitmap(|b| b.frames * PAGE_SIZE)
}

/// Frames in usable regions at boot, in bytes (total RAM the OS uses).
pub fn usable_bytes() -> u64 {
    USABLE.load(Ordering::Relaxed) * PAGE_SIZE
}

/// Releases a frame obtained from `alloc_frame`. Double frees and invalid addresses are logged and ignored, and so
/// is a frame a device driver pinned for DMA (`pin`): the caller's file and line go to the log.
#[track_caller]
pub fn free_frame(addr: u64) {
    if let Some(owner) = pinned_by(addr) {
        let at = core::panic::Location::caller();
        PIN_REFUSED.fetch_add(1, Ordering::Relaxed);
        crate::error!(
            "mem: paza DMA: cadrul {:#x} e al lui {}, eliberarea ceruta de {}:{} a fost REFUZATA",
            addr, owner, at.file(), at.line()
        );
        return;
    }
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
                if pinned_by(f * PAGE_SIZE).is_some() {
                    // A DMA page inside bootloader memory would mean the memory map overlaps: keep it, count it.
                    PIN_REFUSED.fetch_add(1, Ordering::Relaxed);
                    crate::error!("mem: paza DMA: cadrul {:#x} e in memoria bootloader-ului, nu il recuperez", f * PAGE_SIZE);
                    continue;
                }
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
