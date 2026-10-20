//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]

mod arch;
mod boot;
mod drivers;
mod gfx;
#[cfg(feature = "qemu-test")]
mod qemu;

use arch::x86_64::halt_forever;

/// Kernel version shown at boot.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Kernel entry point, called by Limine in 64-bit long mode.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    drivers::serial::init();

    if !boot::BASE_REVISION.is_supported() {
        serial_println!("EROARE: revizia protocolului Limine nu e suportata");
        halt_forever();
    }

    serial_println!("CYBER UNIVERSE OS: BOOT OK");
    serial_println!("versiune nucleu: {}", VERSION);

    if gfx::console::init() {
        gfx::console::set_color(gfx::console::ACCENT);
        println!("Cyber Universe OS");
        gfx::console::set_color(gfx::console::FOREGROUND);
        println!("versiune nucleu {}", VERSION);
        println!();
        println!("Sistem minim, modular, rapid.");
        serial_println!("ECRAN OK");
    } else {
        serial_println!("AVERTISMENT: niciun ecran utilizabil");
    }

    #[cfg(feature = "qemu-test")]
    qemu::finish_test();

    #[cfg(not(feature = "qemu-test"))]
    halt_forever();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("PANIC: {}", info);
    #[cfg(feature = "qemu-test")]
    qemu::exit(qemu::ExitCode::Failed);
    #[cfg(not(feature = "qemu-test"))]
    halt_forever();
}
