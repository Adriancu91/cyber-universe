//! Test-only QEMU helpers. Compiled only with the `qemu-test` feature, so real hardware
//! never receives writes to port 0xF4.

use crate::arch::x86_64::outb;
use crate::drivers::serial;

/// Exit status for the `isa-debug-exit` device. QEMU exits with `(code << 1) | 1`.
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum ExitCode {
    /// QEMU process exit code 33.
    Success = 0x10,
    /// QEMU process exit code 35.
    Failed = 0x11,
}

/// Terminates QEMU with the given status.
pub fn exit(code: ExitCode) -> ! {
    // SAFETY: test builds always run with `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
    unsafe { outb(0xF4, code as u8) }
    crate::arch::x86_64::halt_forever();
}

/// Tells the test harness the kernel is ready, waits for its acknowledgement byte on COM1
/// (giving it time to take a screenshot), then exits QEMU successfully.
pub fn finish_test() -> ! {
    crate::serial_println!("TEST READY");
    loop {
        if serial::try_read_byte().is_some() {
            exit(ExitCode::Success);
        }
        core::hint::spin_loop();
    }
}
