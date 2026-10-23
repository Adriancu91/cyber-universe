//! Interrupt Descriptor Table and CPU exception handlers.
//!
//! Every CPU exception (vectors 0–31 that the CPU defines) ends in a clear log message instead of a silent
//! reboot. Breakpoint (#BP) is the only one the kernel continues after. Double fault runs on its own IST
//! stack so it works even when the normal stack is broken.
//! Written from Intel SDM Vol. 3A ch. 6 and the OSDev Wiki "Exceptions" / "IDT" descriptions.

use super::gdt;
use core::arch::asm;
use core::mem::size_of;

/// What the CPU pushes before calling a handler.
#[repr(C)]
#[derive(Debug)]
pub struct InterruptStackFrame {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Entry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _zero: u32,
}

impl Entry {
    const MISSING: Entry =
        Entry { offset_low: 0, selector: 0, ist: 0, type_attr: 0, offset_mid: 0, offset_high: 0, _zero: 0 };

    fn new(handler: u64, ist: u8) -> Entry {
        Entry {
            offset_low: handler as u16,
            selector: gdt::KERNEL_CODE,
            ist,
            type_attr: 0x8E, // present, ring 0, 64-bit interrupt gate
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _zero: 0,
        }
    }
}

static mut IDT: [Entry; 256] = [Entry::MISSING; 256];

#[repr(C, packed)]
struct DescriptorPointer {
    limit: u16,
    base: u64,
}

pub type Handler = extern "x86-interrupt" fn(InterruptStackFrame);
type HandlerWithCode = extern "x86-interrupt" fn(InterruptStackFrame, u64);
type DivergingWithCode = extern "x86-interrupt" fn(InterruptStackFrame, u64) -> !;

fn set(vector: usize, handler: u64, ist: u8) {
    // SAFETY: only called from `init`, on the boot CPU, before the IDT is loaded.
    unsafe { (*core::ptr::addr_of_mut!(IDT))[vector] = Entry::new(handler, ist) }
}

/// Installs the exception handlers and loads the IDT.
pub fn init() {
    set(0, divide_error as Handler as u64, 0);
    set(1, debug as Handler as u64, 0);
    set(2, nmi as Handler as u64, 0);
    set(3, breakpoint as Handler as u64, 0);
    set(4, overflow as Handler as u64, 0);
    set(5, bound_range as Handler as u64, 0);
    set(6, invalid_opcode as Handler as u64, 0);
    set(7, device_not_available as Handler as u64, 0);
    set(8, double_fault as DivergingWithCode as u64, gdt::DOUBLE_FAULT_IST);
    set(10, invalid_tss as HandlerWithCode as u64, 0);
    set(11, segment_not_present as HandlerWithCode as u64, 0);
    set(12, stack_segment as HandlerWithCode as u64, 0);
    set(13, general_protection as HandlerWithCode as u64, 0);
    set(14, page_fault as HandlerWithCode as u64, 0);
    set(16, x87_floating_point as Handler as u64, 0);
    set(17, alignment_check as HandlerWithCode as u64, 0);
    set(18, machine_check as Handler as u64, 0);
    set(19, simd_floating_point as Handler as u64, 0);

    let ptr = DescriptorPointer {
        limit: (size_of::<[Entry; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT) as u64,
    };
    // SAFETY: the IDT is a static that lives forever and every installed entry points at a valid handler.
    unsafe { asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags)) }
}

/// Loads the (shared) IDT on an additional processor (K6-02).
pub fn load_ap() {
    let ptr = DescriptorPointer {
        limit: (size_of::<[Entry; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT) as u64,
    };
    // SAFETY: the IDT is a static filled by the boot CPU before any other processor starts.
    unsafe { asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags)) }
}

/// Installs (or replaces) the handler of one vector after the IDT is loaded (used by device drivers).
pub fn set_handler(vector: u8, handler: u64) {
    // SAFETY: a single aligned 16-byte entry is replaced while interrupts for this vector are not yet enabled.
    unsafe {
        let entry = &raw mut (*core::ptr::addr_of_mut!(IDT))[vector as usize];
        entry.write_volatile(Entry::new(handler, 0));
    }
}

/// Logs a fatal exception and stops the kernel (or reports failure to QEMU in test builds).
fn fatal(name: &str, frame: &InterruptStackFrame, code: Option<u64>) -> ! {
    if frame.cs & 3 == 3 {
        // The fault happened in a user program: only that program stops; the kernel goes on.
        crate::error!("proces oprit: {} la RIP={:#x}", name, frame.rip);
        crate::syscall::exit_current(-1);
    }
    crate::drivers::serial::force_unlock();
    crate::gfx::console::force_unlock();
    crate::error!("EXCEPTIE: {}", name);
    crate::error!("  RIP={:#018x} RSP={:#018x} RFLAGS={:#x} CS={:#x}", frame.rip, frame.rsp, frame.rflags, frame.cs);
    if let Some(code) = code {
        crate::error!("  cod eroare={:#x}", code);
    }
    crate::error!("SISTEM OPRIT");
    #[cfg(feature = "qemu-test")]
    crate::qemu::exit(crate::qemu::ExitCode::Failed);
    #[cfg(not(feature = "qemu-test"))]
    super::halt_forever();
}

extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    crate::info!("exceptie: breakpoint (#BP) la RIP={:#x}, continui", frame.rip);
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, code: u64) {
    let address: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) address, options(nomem, nostack, preserves_flags)) }
    crate::error!(
        "page fault la adresa {:#018x} ({}, {}, {})",
        address,
        if code & 1 != 0 { "pagina prezenta" } else { "pagina lipsa" },
        if code & 2 != 0 { "scriere" } else { "citire" },
        if code & 4 != 0 { "mod utilizator" } else { "mod nucleu" },
    );
    fatal("page fault (#PF)", &frame, Some(code));
}

extern "x86-interrupt" fn double_fault(frame: InterruptStackFrame, code: u64) -> ! {
    fatal("double fault (#DF)", &frame, Some(code));
}

macro_rules! fatal_handler {
    ($fn_name:ident, $text:expr) => {
        extern "x86-interrupt" fn $fn_name(frame: InterruptStackFrame) {
            fatal($text, &frame, None);
        }
    };
    ($fn_name:ident, $text:expr, code) => {
        extern "x86-interrupt" fn $fn_name(frame: InterruptStackFrame, code: u64) {
            fatal($text, &frame, Some(code));
        }
    };
}

fatal_handler!(divide_error, "impartire la zero (#DE)");
fatal_handler!(debug, "debug (#DB)");
fatal_handler!(nmi, "intrerupere nemascabila (NMI)");
fatal_handler!(overflow, "overflow (#OF)");
fatal_handler!(bound_range, "depasire interval (#BR)");
fatal_handler!(invalid_opcode, "instructiune invalida (#UD)");
fatal_handler!(device_not_available, "coprocesor indisponibil (#NM)");
fatal_handler!(invalid_tss, "TSS invalid (#TS)", code);
fatal_handler!(segment_not_present, "segment absent (#NP)", code);
fatal_handler!(stack_segment, "eroare segment stiva (#SS)", code);
fatal_handler!(general_protection, "protectie generala (#GP)", code);
fatal_handler!(x87_floating_point, "eroare virgula mobila x87 (#MF)");
fatal_handler!(alignment_check, "verificare aliniere (#AC)", code);
fatal_handler!(machine_check, "eroare hardware (#MC)");
fatal_handler!(simd_floating_point, "eroare virgula mobila SIMD (#XM)");
