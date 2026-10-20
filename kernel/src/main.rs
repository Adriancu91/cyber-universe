//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod acpi;
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

    if acpi::init() && acpi::madt::init() && arch::x86_64::apic::init() {
        time::start();
        drivers::keyboard::init();
    } else {
        warn!("sistem: fara ceas (APIC indisponibil); continui fara intreruperi");
    }

    #[cfg(feature = "qemu-test")]
    run_tests();

    #[cfg(not(feature = "qemu-test"))]
    {
        let (heap_used, heap_mapped) = mem::heap::stats();
        info!(
            "sistem: pornire completa; memorie libera {} MiB, heap {} / {} KiB",
            mem::frame::free_frames() * mem::map::PAGE_SIZE / (1024 * 1024),
            heap_used / 1024,
            heap_mapped / 1024
        );
        echo_keyboard();
    }
}

/// Until there is a shell (Phase 2): show typed keys on the screen.
#[cfg(not(feature = "qemu-test"))]
fn echo_keyboard() -> ! {
    use drivers::keyboard::{Key, read_key};
    println!();
    print!("> ");
    loop {
        while let Some(key) = read_key() {
            match key {
                Key::Char(c) => print!("{}", c),
                Key::Enter => print!("\n> "),
                _ => {}
            }
        }
        arch::x86_64::hlt();
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
    test_paging();
    test_heap();
    test_timer();

    #[cfg(feature = "test-keyboard")]
    test_keyboard();

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

/// Maps a fresh frame at an unused address, checks writes are visible through the HHDM, then unmaps it.
#[cfg(feature = "qemu-test")]
fn test_paging() {
    use mem::paging;
    const VIRT: u64 = 0xFFFF_C000_0000_0000;
    let f = mem::frame::alloc_frame().expect("test: niciun cadru liber");
    paging::map(VIRT, f, paging::WRITABLE | paging::NO_EXECUTE).expect("test: maparea a esuat");
    assert!(paging::translate(VIRT + 0x123) == Some(f + 0x123), "translate gresit");
    // SAFETY: VIRT was just mapped to frame `f`, which belongs to this test.
    unsafe { (VIRT as *mut u64).write_volatile(0xC1BE_2026) };
    // SAFETY: frame `f` is reachable through the HHDM.
    let seen = unsafe { (mem::map::phys_to_virt(f) as *const u64).read_volatile() };
    assert!(seen == 0xC1BE_2026, "valoarea scrisa nu apare la adresa fizica");
    assert!(paging::map(VIRT, f, paging::WRITABLE) == Err(paging::MapError::AlreadyMapped), "mapare dubla acceptata");
    assert!(paging::unmap(VIRT) == Some(f), "demapare gresita");
    assert!(paging::translate(VIRT).is_none(), "pagina inca mapata dupa demapare");
    mem::frame::free_frame(f);
    info!("test: paginare OK (mapare, scriere, translate, demapare)");

    #[cfg(feature = "test-unmapped")]
    // SAFETY: deliberate access to the page unmapped above; must raise a page fault.
    unsafe {
        (VIRT as *const u64).read_volatile();
    }
}

/// Exercises the heap with a large Vec and many small boxes freed in mixed order.
#[cfg(feature = "qemu-test")]
fn test_heap() {
    use alloc::{boxed::Box, vec::Vec};
    let (used_before, _) = mem::heap::stats();
    {
        let v: Vec<u64> = (0..1_000_000u64).collect();
        let sum: u64 = v.iter().sum();
        assert!(sum == 999_999 * 1_000_000 / 2, "suma gresita in Vec");
    }
    {
        let mut boxes: Vec<Box<[u8; 48]>> = Vec::new();
        for i in 0..10_000u32 {
            boxes.push(Box::new([(i % 251) as u8; 48]));
        }
        for i in (0..boxes.len()).rev().step_by(2) {
            let b = boxes.swap_remove(i);
            assert!(b[0] == (b[47]), "continut Box stricat");
        }
        let s = alloc::format!("{} cutii ramase", boxes.len());
        assert!(s == "5000 cutii ramase", "String gresit");
    }
    let (used_after, mapped) = mem::heap::stats();
    assert!(used_after == used_before, "scurgere in heap: {} inainte, {} dupa", used_before, used_after);
    info!("test: heap OK (Vec de 1.000.000, 10.000 Box, String); heap mapat {} KiB", mapped / 1024);
}

/// Checks the APIC timer against the PIT: in 1 second (measured by the PIT) there must be ~100 ticks.
#[cfg(feature = "qemu-test")]
fn test_timer() {
    use arch::x86_64::apic;
    let start = apic::ticks();
    apic::pit_sleep_ms(1000);
    let n = apic::ticks() - start;
    assert!((90..=110).contains(&n), "temporizator: {} intreruperi intr-o secunda (asteptat 100)", n);
    // `hlt` must wake up on the next timer interrupt.
    let before = apic::ticks();
    arch::x86_64::hlt();
    assert!(apic::ticks() > before, "hlt nu a fost trezit de temporizator");
    info!("test: temporizator OK ({} intreruperi intr-o secunda)", n);
}

/// Waits for the keys the test harness types (h, i, Enter) and logs them.
#[cfg(feature = "test-keyboard")]
fn test_keyboard() {
    use drivers::keyboard::{Key, read_key};
    serial_println!("TASTATURA: astept taste");
    let mut got = alloc::vec::Vec::new();
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    while got.len() < 3 && arch::x86_64::apic::ticks() < deadline {
        match read_key() {
            Some(k) => {
                match k {
                    Key::Char(c) => info!("KEY: {}", c),
                    other => info!("KEY: {:?}", other),
                }
                got.push(k);
            }
            None => arch::x86_64::hlt(),
        }
    }
    assert!(got == [Key::Char('h'), Key::Char('i'), Key::Enter], "taste primite gresit: {:?}", got);
    info!("test: tastatura OK");
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
