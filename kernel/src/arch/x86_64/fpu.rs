//! The vector unit (x87, SSE, AVX): switched on at boot and saved / restored for every thread at each context switch.
//! The kernel itself never uses vector registers (the `x86_64-unknown-none` target is soft-float), so saving the
//! state when switching threads is enough. Written from the Intel SDM vol. 1 ch. 13 (XSAVE) and vol. 3 §2.5–2.6
//! (CR0, CR4, XCR0).

use core::arch::asm;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// XCR0 bits we enable when the CPU has them: x87, SSE (XMM), AVX (upper halves of YMM).
const X87: u64 = 1 << 0;
const SSE: u64 = 1 << 1;
const AVX: u64 = 1 << 2;
/// Size of the save area when only FXSAVE exists.
const FXSAVE_SIZE: u32 = 512;
/// Largest area we accept (x87 + SSE + AVX = 832 bytes; room for later components).
pub const MAX_AREA: usize = 4096;

/// 0 = not set up, 1 = FXSAVE, 2 = XSAVE.
static MODE: AtomicU32 = AtomicU32::new(0);
static XCR0: AtomicU64 = AtomicU64::new(0);
static AREA_SIZE: AtomicU32 = AtomicU32::new(FXSAVE_SIZE);
static AVX2: AtomicU32 = AtomicU32::new(0);

/// The clean state every new thread starts from (saved right after `fninit` and the default MXCSR).
#[repr(C, align(64))]
struct Template([u8; MAX_AREA]);
static mut TEMPLATE: Template = Template([0; MAX_AREA]);

/// Bytes of a thread's save area.
pub fn area_size() -> usize {
    AREA_SIZE.load(Ordering::Relaxed) as usize
}

/// True if AVX is enabled (YMM registers saved with the thread).
#[allow(dead_code)] // used by the self-tests
pub fn has_avx() -> bool {
    XCR0.load(Ordering::Relaxed) & AVX != 0
}

/// True if AVX2 is usable (CPUID and XCR0 both say so).
pub fn has_avx2() -> bool {
    AVX2.load(Ordering::Relaxed) != 0
}

/// Switches the vector unit on: CR0.MP=1, EM=0, TS=0; CR4.OSFXSR, OSXMMEXCPT and (with XSAVE) OSXSAVE; XCR0.
/// Called once on the boot CPU, before the first thread is created.
pub fn init() {
    // SAFETY: CPUID is available on every x86-64 CPU.
    let leaf1 = unsafe { __cpuid(1) };
    let has_xsave = leaf1.ecx & (1 << 26) != 0;
    let has_avx = leaf1.ecx & (1 << 28) != 0;
    // SAFETY: leaf 7 exists on every CPU new enough to have AVX; on older ones it returns zeros or the last leaf.
    let max_leaf = unsafe { __cpuid(0) }.eax;
    let avx2 = max_leaf >= 7 && unsafe { __cpuid_count(7, 0) }.ebx & (1 << 5) != 0;

    // SAFETY: writing CR0/CR4 with these bits only enables the FPU/SSE instructions and their exceptions;
    // no other bit changes.
    unsafe {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        cr0 = (cr0 | 1 << 1) & !(1 << 2) & !(1 << 3); // MP on, EM off, TS off
        asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));
        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr4 |= 1 << 9 | 1 << 10; // OSFXSR, OSXMMEXCPT
        if has_xsave {
            cr4 |= 1 << 18; // OSXSAVE
        }
        asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
    }

    let mut text = "FXSAVE (fara XSAVE)";
    if has_xsave {
        // SAFETY: leaf 0Dh exists when XSAVE does; EAX/EDX = the XCR0 bits this CPU supports.
        let d = unsafe { __cpuid_count(0xD, 0) };
        let supported = d.eax as u64 | (d.edx as u64) << 32;
        let mut want = X87 | SSE;
        if has_avx && supported & AVX != 0 {
            want |= AVX;
        }
        let want = want & supported;
        // SAFETY: CR4.OSXSAVE is set; `want` only has bits CPUID reports, with x87 and SSE as XSETBV requires.
        unsafe { asm!("xsetbv", in("ecx") 0u32, in("eax") want as u32, in("edx") (want >> 32) as u32, options(nomem, nostack)) };
        // SAFETY: after XSETBV, CPUID 0Dh.0 EBX = the area size for the bits now enabled.
        let size = unsafe { __cpuid_count(0xD, 0) }.ebx;
        if size as usize <= MAX_AREA {
            XCR0.store(want, Ordering::Relaxed);
            AREA_SIZE.store(size, Ordering::Relaxed);
            MODE.store(2, Ordering::Relaxed);
            text = if want & AVX != 0 { "XSAVE: x87, SSE, AVX" } else { "XSAVE: x87, SSE" };
            if want & AVX != 0 && avx2 {
                AVX2.store(1, Ordering::Relaxed);
            }
        }
    }
    if MODE.load(Ordering::Relaxed) == 0 {
        MODE.store(1, Ordering::Relaxed);
        AREA_SIZE.store(FXSAVE_SIZE, Ordering::Relaxed);
    }
    // The clean state: empty x87 stack, default MXCSR (all exceptions masked), zero vector registers.
    // SAFETY: the unit is on; TEMPLATE is 64-byte aligned and large enough; single-threaded boot.
    unsafe {
        asm!("fninit", options(nomem, nostack));
        let mxcsr: u32 = 0x1F80;
        asm!("ldmxcsr [{}]", in(reg) &mxcsr, options(nostack, readonly));
        asm!("pxor xmm0, xmm0", options(nomem, nostack));
        save(core::ptr::addr_of_mut!(TEMPLATE) as *mut u8);
    }
    crate::info!(
        "fpu: {}; zona per fir {} octeti; AVX2 {}",
        text,
        area_size(),
        if has_avx2() { "da" } else { "nu" }
    );
}

/// Switches the vector unit on for an additional processor, exactly as on the boot CPU (CR0/CR4 copied by the
/// caller; here only XCR0, which is per processor).
pub fn init_ap() {
    if MODE.load(Ordering::Relaxed) == 2 {
        let m = XCR0.load(Ordering::Relaxed);
        // SAFETY: CR4.OSXSAVE is set (copied from the boot CPU); `m` was accepted by XSETBV on the boot CPU.
        unsafe { asm!("xsetbv", in("ecx") 0u32, in("eax") m as u32, in("edx") (m >> 32) as u32, options(nomem, nostack)) };
    }
}

/// Fills a new thread's save area with the clean state.
///
/// # Safety
/// `area` must be valid for `area_size()` bytes.
pub unsafe fn init_area(area: *mut u8) {
    // SAFETY: TEMPLATE holds `area_size()` initialised bytes; the caller guarantees `area`.
    unsafe { core::ptr::copy_nonoverlapping(core::ptr::addr_of!(TEMPLATE) as *const u8, area, area_size()) };
}

/// Saves the vector state of the running thread.
///
/// # Safety
/// `area` must be 64-byte aligned and valid for `area_size()` bytes; `init` must have run.
pub unsafe fn save(area: *mut u8) {
    match MODE.load(Ordering::Relaxed) {
        // SAFETY: the caller guarantees the area; XCR0 bits in EDX:EAX select what is saved.
        2 => unsafe {
            let m = XCR0.load(Ordering::Relaxed);
            asm!("xsave64 [{}]", in(reg) area, in("eax") m as u32, in("edx") (m >> 32) as u32, options(nostack))
        },
        // SAFETY: as above; FXSAVE needs 16-byte alignment (we give 64).
        1 => unsafe { asm!("fxsave64 [{}]", in(reg) area, options(nostack)) },
        _ => {}
    }
}

/// Loads the vector state of the thread about to run.
///
/// # Safety
/// `area` must hold a state written by `save` or `init_area`, 64-byte aligned.
pub unsafe fn restore(area: *const u8) {
    match MODE.load(Ordering::Relaxed) {
        // SAFETY: the caller guarantees the area holds a valid XSAVE image.
        2 => unsafe {
            let m = XCR0.load(Ordering::Relaxed);
            asm!("xrstor64 [{}]", in(reg) area, in("eax") m as u32, in("edx") (m >> 32) as u32, options(nostack, readonly))
        },
        // SAFETY: as above, for FXSAVE.
        1 => unsafe { asm!("fxrstor64 [{}]", in(reg) area, options(nostack, readonly)) },
        _ => {}
    }
}

/// A 64-byte aligned save area owned by one thread.
pub struct Area {
    ptr: *mut u8,
    layout: core::alloc::Layout,
}

// SAFETY: the area is plain memory owned by one thread; the scheduler moves it between queues under its lock.
unsafe impl Send for Area {}

impl Area {
    /// A new area with the clean state (`None` before `init` or without memory).
    pub fn new() -> Option<Area> {
        if MODE.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let layout = core::alloc::Layout::from_size_align(area_size(), 64).ok()?;
        // SAFETY: non-zero size.
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: freshly allocated `area_size()` bytes.
        unsafe { init_area(ptr) };
        Some(Area { ptr, layout })
    }
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

impl Drop for Area {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { alloc::alloc::dealloc(self.ptr, self.layout) }
    }
}
