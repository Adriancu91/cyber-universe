//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod boot;
mod drivers;
mod gfx;
mod log;
#[cfg(feature = "qemu-test")]
mod qemu;
mod time;

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

    if gfx::console::init() {
        gfx::console::set_color(gfx::console::ACCENT);
        println!("Cyber Universe OS");
        gfx::console::set_color(gfx::console::FOREGROUND);
        println!("versiune nucleu {}", VERSION);
        println!();
        serial_println!("ECRAN OK");
    } else {
        serial_println!("AVERTISMENT: niciun ecran utilizabil");
    }

    info!("jurnal: pornit (nucleu {})", VERSION);

    arch::x86_64::init();
    info!(
        "cpu: GDT si IDT incarcate (CS={:#04x}, TR={:#04x})",
        arch::x86_64::gdt::current_cs(),
        arch::x86_64::gdt::current_tr()
    );

    #[cfg(feature = "qemu-test")]
    run_tests();

    #[cfg(not(feature = "qemu-test"))]
    {
        info!("sistem: pornire completa");
        halt_forever();
    }
}

/// Self-tests compiled only into test builds; each scenario in tests/scenarios.txt checks their output.
#[cfg(feature = "qemu-test")]
fn run_tests() -> ! {
    warn!("jurnal: test avertisment");
    error!("jurnal: test eroare");

    // Breakpoint: the kernel must log it and continue.
    // SAFETY: int3 only raises #BP, whose handler returns.
    unsafe { core::arch::asm!("int3") }
    info!("test: dupa breakpoint, nucleul continua");

    #[cfg(feature = "test-divzero")]
    // SAFETY: deliberate #DE for the test scenario; the handler stops the kernel.
    unsafe {
        core::arch::asm!("xor edx, edx", "xor eax, eax", "xor ecx, ecx", "div ecx", out("eax") _, out("ecx") _, out("edx") _)
    }

    #[cfg(feature = "test-pagefault")]
    // SAFETY: deliberate read from an unmapped address for the test scenario; the handler stops the kernel.
    unsafe {
        core::ptr::read_volatile(0x0000_7000_dead_b000 as *const u64);
    }

    #[cfg(feature = "test-doublefault")]
    // SAFETY: deliberate: the stack pointer is moved to unmapped memory, so the CPU cannot push the
    // page-fault frame and raises #DF, which must run on its own IST stack.
    unsafe {
        core::arch::asm!("mov rsp, {}", "push rax", in(reg) 0x0000_7000_0000_0000u64, options(noreturn))
    }

    #[cfg(feature = "test-panic")]
    panic!("panica intentionata de test");

    #[allow(unreachable_code)]
    qemu::finish_test()
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // The panicking code may hold an output lock; break it so the message always gets out.
    drivers::serial::force_unlock();
    gfx::console::force_unlock();
    error!("PANIC: {}", info.message());
    if let Some(loc) = info.location() {
        error!("PANIC la {}:{}", loc.file(), loc.line());
    }
    #[cfg(feature = "qemu-test")]
    qemu::exit(qemu::ExitCode::Failed);
    #[cfg(not(feature = "qemu-test"))]
    halt_forever();
}
