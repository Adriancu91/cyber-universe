//! Global Descriptor Table and Task State Segment.
//!
//! Layout (selectors):
//!   0x00 null · 0x08 kernel code · 0x10 kernel data · 0x18 user data · 0x20 user code · 0x28 TSS (16 bytes)
//! User data comes before user code so `sysret` can be used later (Phase 2).
//! Written from Intel SDM Vol. 3A (ch. 3 and 7) and the OSDev Wiki descriptions.

use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
#[allow(dead_code)] // used from Phase 2 (user mode)
pub const USER_DATA: u16 = 0x18 | 3;
#[allow(dead_code)] // used from Phase 2 (user mode)
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS_SELECTOR: u16 = 0x28;

/// IST slot (1-based, as stored in the IDT) used by the double-fault handler.
pub const DOUBLE_FAULT_IST: u8 = 1;
const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed(4))]
struct TaskStateSegment {
    _reserved0: u32,
    privilege_stacks: [u64; 3],
    _reserved1: u64,
    interrupt_stacks: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

#[repr(C, align(16))]
struct Stack([u8; IST_STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: Stack = Stack([0; IST_STACK_SIZE]);

static mut TSS: TaskStateSegment = TaskStateSegment {
    _reserved0: 0,
    privilege_stacks: [0; 3],
    _reserved1: 0,
    interrupt_stacks: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: size_of::<TaskStateSegment>() as u16, // no I/O permission bitmap
};

/// Flat 64-bit segments; base and limit are ignored in long mode except for the flags.
static mut GDT: [u64; 7] = [
    0,
    0x00AF_9A00_0000_FFFF, // kernel code: present, ring 0, executable, long mode
    0x00CF_9200_0000_FFFF, // kernel data: present, ring 0, writable
    0x00CF_F200_0000_FFFF, // user data:   present, ring 3, writable
    0x00AF_FA00_0000_FFFF, // user code:   present, ring 3, executable, long mode
    0,                     // TSS low  (filled in by `init`)
    0,                     // TSS high (filled in by `init`)
];

#[repr(C, packed)]
struct DescriptorPointer {
    limit: u16,
    base: u64,
}

/// Builds the TSS descriptor, loads the GDT, reloads every segment register and the task register.
pub fn init() {
    // SAFETY: called once on the boot CPU before interrupts are enabled; nothing else accesses
    // GDT, TSS or the IST stack concurrently.
    unsafe {
        let stack_top = core::ptr::addr_of!(DOUBLE_FAULT_STACK) as u64 + IST_STACK_SIZE as u64;
        let tss = &mut *core::ptr::addr_of_mut!(TSS);
        tss.interrupt_stacks[(DOUBLE_FAULT_IST - 1) as usize] = stack_top;

        let base = core::ptr::addr_of!(TSS) as u64;
        let limit = (size_of::<TaskStateSegment>() - 1) as u64;
        let low = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89 << 40) // present, 64-bit available TSS
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        let gdt = &mut *core::ptr::addr_of_mut!(GDT);
        gdt[5] = low;
        gdt[6] = base >> 32;

        let ptr = DescriptorPointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };
        asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));

        // Reload CS with a far return, then the data segments.
        asm!(
            "push {code}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov ss, {data:x}",
            "xor {tmp:e}, {tmp:e}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            code = in(reg) KERNEL_CODE as u64,
            data = in(reg) KERNEL_DATA as u64,
            tmp = lateout(reg) _,
        );

        asm!("ltr {:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
    }
}

/// Current code segment selector.
pub fn current_cs() -> u16 {
    let cs: u16;
    // SAFETY: reading CS has no side effects.
    unsafe { asm!("mov {:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags)) }
    cs
}

/// Current task register selector.
pub fn current_tr() -> u16 {
    let tr: u16;
    // SAFETY: `str` only reads the task register.
    unsafe { asm!("str {:x}", out(reg) tr, options(nomem, nostack, preserves_flags)) }
    tr
}
