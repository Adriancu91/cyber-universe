//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod boot;
mod drivers;
mod gfx;
mod log;
mod mem;
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

    if !mem::init() {
        error!("mem: nucleul nu poate gestiona memoria, opresc");
        panic!("initializarea memoriei a esuat");
    }

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

    test_frames();

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

/// Allocates 10,000 frames, checks they are distinct by writing a marker into each one through the HHDM,
/// frees them and checks the free count returns to its starting value.
#[cfg(feature = "qemu-test")]
fn test_frames() {
    const N: usize = 10_000;
    static mut FRAMES: [u64; N] = [0; N];
    // SAFETY: test-only static used by this single-threaded function.
    let frames = unsafe { &mut *core::ptr::addr_of_mut!(FRAMES) };
    let before = mem::frame::free_frames();
    for (i, slot) in frames.iter_mut().enumerate() {
        let f = mem::frame::alloc_frame().expect("test: memorie insuficienta pentru 10000 de cadre");
        assert!(f % mem::map::PAGE_SIZE == 0, "cadru nealiniat");
        // SAFETY: the frame was just allocated to us and is reachable through the HHDM.
        unsafe { (mem::map::phys_to_virt(f) as *mut u64).write_volatile(i as u64) };
        *slot = f;
    }
    for (i, &f) in frames.iter().enumerate() {
        // SAFETY: same frames as above, still owned by this test.
        let v = unsafe { (mem::map::phys_to_virt(f) as *const u64).read_volatile() };
        assert!(v == i as u64, "cadrul {:#x} a fost dat de doua ori", f);
    }
    for &f in frames.iter() {
        mem::frame::free_frame(f);
    }
    mem::frame::free_frame(frames[0]); // deliberate double free: must be refused
    let after = mem::frame::free_frames();
    assert!(before == after, "scurgere de cadre: {} inainte, {} dupa", before, after);
    info!("test: {} cadre alocate, unice, eliberate; libere inainte={} dupa={}", N, before, after);
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
