//! x86_64-specific primitives: port I/O and CPU control.

pub mod apic;
pub mod gdt;
pub mod idt;

use core::arch::asm;

/// Sets up the CPU tables (GDT, TSS, IDT). Must run before anything that can fault or take interrupts.
pub fn init() {
    gdt::init();
    idt::init();
}

/// Writes one byte to an I/O port.
///
/// # Safety
/// Writing to an arbitrary port can reconfigure hardware; the caller must know the device behind `port`.
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: the caller guarantees the port write is valid for the device.
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) }
}

/// Reads one byte from an I/O port.
///
/// # Safety
/// Reading some ports has side effects (e.g. acknowledging data); the caller must know the device.
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: the caller guarantees the port read is valid for the device.
    unsafe { asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags)) }
    value
}

/// Halts the CPU until the next interrupt.
#[inline]
pub fn hlt() {
    // SAFETY: `hlt` only pauses the CPU; it does not touch memory.
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) }
}

/// Disables interrupts and halts forever.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `cli` only masks maskable interrupts on this CPU.
        unsafe { asm!("cli", options(nomem, nostack)) }
        hlt();
    }
}
