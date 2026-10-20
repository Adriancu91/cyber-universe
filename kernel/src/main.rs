//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]

mod arch;
mod boot;

use arch::x86_64::halt_forever;

/// Kernel entry point, called by Limine in 64-bit long mode.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    if !boot::BASE_REVISION.is_supported() {
        halt_forever();
    }
    halt_forever();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt_forever();
}
