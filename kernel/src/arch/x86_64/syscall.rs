//! `syscall`/`sysret` and the first entry into user mode (ring 3).
//! Written from Intel SDM Vol. 2 (SYSCALL, SYSRET, IRETQ) and Vol. 3A ch. 5.
//!
//! Calling convention for programs: number in rax; arguments in rdi, rsi, rdx, r10, r8; result in rax.
//! rcx and r11 are overwritten (the CPU uses them); every other register is preserved.

use super::gdt;
use core::arch::{asm, naked_asm};
use core::sync::atomic::Ordering;

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;

fn wrmsr(msr: u32, value: u64) {
    // SAFETY: only called with the architectural MSRs above, which exist on every x86_64 CPU.
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nomem, nostack)) }
}

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: as in `wrmsr`.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack)) }
    ((hi as u64) << 32) | lo as u64
}

/// Enables `syscall`: entry point, segments (see the GDT layout in gdt.rs) and the flags cleared on entry.
pub fn init() {
    wrmsr(IA32_EFER, rdmsr(IA32_EFER) | 1); // SCE
    // syscall: CS = 0x08, SS = 0x10. sysret: SS = 0x10 + 8 = user data, CS = 0x10 + 16 = user code (RPL 3).
    wrmsr(IA32_STAR, ((gdt::KERNEL_DATA as u64) << 48) | ((gdt::KERNEL_CODE as u64) << 32));
    wrmsr(IA32_LSTAR, syscall_entry as *const () as u64);
    wrmsr(IA32_FMASK, 0x4_0700); // clear AC, DF, IF, TF on entry
}

/// Sets the kernel stack used when the running thread enters the kernel from ring 3
/// (by `syscall` or by an interrupt/exception), on this processor. Called by the scheduler at every switch,
/// with interrupts disabled.
pub fn set_kernel_stack(top: u64) {
    let cpu = super::percpu::this();
    cpu.kernel_rsp.store(top, Ordering::Relaxed);
    gdt::set_privilege_stack(cpu.tss.load(Ordering::Relaxed), top);
}

#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        // Interrupts are off (FMASK). `swapgs` brings this processor's block (percpu.rs) into GS for three
        // instructions, then the second `swapgs` puts the program's GS base back: the kernel never uses GS.
        "swapgs",
        "mov gs:[{user_rsp}], rsp",
        "mov rsp, gs:[{kernel_rsp}]",
        "push qword ptr gs:[{user_rsp}]",
        "swapgs",
        "push rcx", // user rip
        "push r11", // user rflags
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",
        "sub rsp, 8", // 10 pushes: keep the stack 16-byte aligned for the call
        // dispatch(nr, a1, a2, a3, a4, a5)
        "mov r9, r8",
        "mov r8, r10",
        "mov rcx, rdx",
        "mov rdx, rsi",
        "mov rsi, rdi",
        "mov rdi, rax",
        "call {dispatch}",
        "add rsp, 8",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "cli",
        "pop r11",
        "pop rcx",
        "pop rsp",
        "sysretq",
        user_rsp = const super::percpu::USER_RSP_OFFSET,
        kernel_rsp = const super::percpu::KERNEL_RSP_OFFSET,
        dispatch = sym crate::syscall::dispatch,
    )
}

/// Leaves the kernel for good on this thread: continues in ring 3 at `entry` with the stack `user_rsp`,
/// interrupts enabled, rdi = `arg0`, rsi = `arg1` and every other general register zeroed.
///
/// # Safety
/// `entry` and `user_rsp` must be user addresses mapped (USER) in the current address space, and the
/// kernel stack of this thread must be the one set with `set_kernel_stack`.
pub unsafe fn enter_user(entry: u64, user_rsp: u64, arg0: u64, arg1: u64) -> ! {
    // SAFETY: builds the frame `iretq` expects (SS, RSP, RFLAGS, CS, RIP) with ring-3 selectors.
    unsafe {
        asm!(
            "push {ss}",
            "push {rsp}",
            "push 0x202", // IF set
            "push {cs}",
            "push {rip}",
            "xor eax, eax", "xor ebx, ebx", "xor ecx, ecx", "xor edx, edx", "xor ebp, ebp",
            "xor r8d, r8d", "xor r9d, r9d", "xor r10d, r10d", "xor r11d, r11d",
            "xor r12d, r12d", "xor r13d, r13d", "xor r14d, r14d", "xor r15d, r15d",
            "iretq",
            ss = in(reg) gdt::USER_DATA as u64,
            rsp = in(reg) user_rsp,
            cs = in(reg) gdt::USER_CODE as u64,
            rip = in(reg) entry,
            in("rdi") arg0,
            in("rsi") arg1,
            options(noreturn)
        )
    }
}
