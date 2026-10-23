//! Data that belongs to one processor (K6-02b): its index, the kernel stack `syscall` switches to, its TSS.
//! The block of each processor is found through the MSR IA32_KERNEL_GS_BASE, which nothing else uses:
//! `syscall_entry` reaches it with `swapgs` (and swaps back at once), the rest of the kernel reads the MSR.
//! GS itself is never used by the kernel, so a program that changes its GS selector cannot fool the kernel.
//! Written from the Intel SDM vol. 2 (SWAPGS, RDMSR) and vol. 3 §2.2.1 (FS/GS bases in 64-bit mode).

use core::sync::atomic::{AtomicU64, Ordering};

/// Most processors the kernel uses (the workstation has 36 logical processors).
pub const MAX_CPUS: usize = 64;

const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// One processor's block. `kernel_rsp` and `user_rsp` are at fixed offsets used by `syscall_entry`.
#[repr(C, align(64))]
pub struct PerCpu {
    /// Offset 0: top of the running thread's kernel stack (where `syscall` switches to).
    pub kernel_rsp: AtomicU64,
    /// Offset 8: the user stack pointer, kept here for the few instructions until it is on the kernel stack.
    pub user_rsp: AtomicU64,
    /// Offset 16: this processor's index (0 = the boot processor).
    pub index: AtomicU64,
    /// Offset 24: address of this processor's TSS (for TSS.RSP0).
    pub tss: AtomicU64,
    /// Offset 32: local APIC id.
    pub apic_id: AtomicU64,
}

pub const KERNEL_RSP_OFFSET: usize = 0;
pub const USER_RSP_OFFSET: usize = 8;

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY: PerCpu = PerCpu {
    kernel_rsp: AtomicU64::new(0),
    user_rsp: AtomicU64::new(0),
    index: AtomicU64::new(0),
    tss: AtomicU64::new(0),
    apic_id: AtomicU64::new(0),
};

static CPUS: [PerCpu; MAX_CPUS] = [EMPTY; MAX_CPUS];

fn wrmsr(msr: u32, value: u64) {
    // SAFETY: only IA32_KERNEL_GS_BASE, an architectural MSR present on every x86_64 CPU.
    unsafe { core::arch::asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nomem, nostack)) }
}

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: as in `wrmsr`.
    unsafe { core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack)) }
    ((hi as u64) << 32) | lo as u64
}

/// Points this processor's IA32_KERNEL_GS_BASE at block `index`. Called once per processor, before it runs threads.
pub fn init(index: usize, tss: u64, apic_id: u8) {
    let c = &CPUS[index];
    c.index.store(index as u64, Ordering::Relaxed);
    c.tss.store(tss, Ordering::Relaxed);
    c.apic_id.store(apic_id as u64, Ordering::Relaxed);
    wrmsr(IA32_KERNEL_GS_BASE, c as *const PerCpu as u64);
}

/// This processor's block. Call with interrupts disabled if the answer must stay true (a thread can move).
pub fn this() -> &'static PerCpu {
    let p = rdmsr(IA32_KERNEL_GS_BASE);
    if p == 0 {
        return &CPUS[0]; // before `init` on the boot processor
    }
    // SAFETY: the MSR only ever holds the address of an element of CPUS (set by `init`; the kernel never runs with
    // the user's GS base swapped in, see `syscall_entry`).
    unsafe { &*(p as *const PerCpu) }
}

/// Index of the processor running this code (0 = boot processor).
pub fn index() -> usize {
    this().index.load(Ordering::Relaxed) as usize
}

/// Block of processor `index`.
pub fn get(index: usize) -> &'static PerCpu {
    &CPUS[index]
}
