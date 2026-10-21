//! Cyber Universe OS kernel entry point.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod acpi;
mod block;
mod arch;
mod boot;
mod drivers;
mod fs;
mod gfx;
mod input;
mod log;
mod mem;
mod net;
mod modmgr;
mod pci;
mod proc;
mod sched;
mod shell;
mod syscall;
mod term;
#[cfg(feature = "qemu-test")]
mod qemu;
mod random;
#[cfg(feature = "qemu-test")]
mod tests;
mod time;
mod ui;

use arch::x86_64::halt_forever;

/// Kernel version shown at boot.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Size of the kernel's own boot stack (Limine's stack is in bootloader memory, which is reclaimed).
const BOOT_STACK_SIZE: usize = 128 * 1024;

#[repr(C, align(16))]
struct BootStack([u8; BOOT_STACK_SIZE]);

static mut BOOT_STACK: BootStack = BootStack([0; BOOT_STACK_SIZE]);

/// Kernel entry point, called by Limine in 64-bit long mode. Moves to the kernel's own stack at once.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    // SAFETY: BOOT_STACK is used only from here on, as the stack of `kernel_main`, which never returns;
    // its top is 16-byte aligned, so after `call` the stack has the alignment the System V ABI expects.
    unsafe {
        core::arch::asm!(
            "mov rsp, {top}",
            "xor ebp, ebp",
            "call {main}",
            top = in(reg) (core::ptr::addr_of_mut!(BOOT_STACK) as *mut u8).add(BOOT_STACK_SIZE),
            main = sym kernel_main,
            options(noreturn)
        )
    }
}

extern "C" fn kernel_main() -> ! {
    time::mark_boot();
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
    sched::init();

    let has_acpi = acpi::init();
    if has_acpi && acpi::madt::init() && arch::x86_64::apic::init() {
        time::start();
        if drivers::keyboard::init() {
            drivers::mouse::init();
        }
    } else {
        warn!("sistem: fara ceas (APIC indisponibil); continui fara intreruperi");
    }
    if has_acpi {
        pci::init();
    }
    match drivers::rtc::now() {
        Some(t) => info!("rtc: {:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second),
        None => warn!("rtc: ceasul calculatorului nu raspunde"),
    }
    boot::collect_files();
    fs::init();
    if time::uptime_ms().is_some() {
        drivers::usb::init(); // needs the clock (waits for USB 3 links)
    }
    // A real disk (NVMe first, then SATA) gives /disk, unless the test disk image already did.
    let nvme = drivers::nvme::init().map(|d| alloc::boxed::Box::new(d) as alloc::boxed::Box<dyn block::BlockDevice>);
    let sata = drivers::ahci::init().map(|d| alloc::boxed::Box::new(d) as alloc::boxed::Box<dyn block::BlockDevice>);
    for disk in [nvme, sata].into_iter().flatten() {
        if fs::kind("/disk").is_err() {
            fs::mount_disk(disk, "/disk");
        }
    }
    drivers::nic::init_global();
    net::init();
    if let Some(stick) = drivers::usb::take_disk() {
        fs::mount_disk(alloc::boxed::Box::new(stick), "/usb");
    }
    apply_settings();
    // Every Limine response has been read (memory map, HHDM, framebuffer, RSDP, modules): its memory can be reused.
    mem::reclaim_bootloader();

    report_boot_metrics();

    #[cfg(feature = "qemu-test")]
    run_tests();

    #[cfg(not(feature = "qemu-test"))]
    {
        run_desktop(); // the desktop first; when it closes (Escape), the text shell
        loop {
            shell::run();
        }
    }
}

/// Starts the desktop if there is a screen and a `/bin/desktop`, and waits until it closes. Returns
/// whether it ran.
fn run_desktop() -> bool {
    if ui::screen_size().is_none() {
        return false;
    }
    match proc::spawn_path("/bin/desktop", &["desktop"]) {
        Ok(id) => {
            proc::wait(id);
            true
        }
        Err(e) => {
            warn!("desktop: nu porneste ({})", e.message());
            false
        }
    }
}

/// Settings saved by the Settings app that the kernel applies at start-up (the keyboard layout).
fn apply_settings() {
    let Ok(text) = fs::read_all("/disk/config/settings.toml") else { return };
    let text = alloc::string::String::from_utf8_lossy(&text);
    let ro = text.lines().any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim() == "tastatura" && v.trim() == "\"ro\""));
    if ro {
        drivers::keyboard::set_layout(drivers::keyboard::Layout::Ro);
        info!("tastatura: aranjament romanesc (din setari)");
    }
}

/// Logs how long the kernel took to start and how much memory it uses (read by tools/metrics.sh).
fn report_boot_metrics() {
    unsafe extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    }
    // Only the addresses of the linker symbols are taken, never their contents.
    let image = core::ptr::addr_of!(__kernel_end) as u64 - core::ptr::addr_of!(__kernel_start) as u64;
    let used = mem::frame::used_frames() * mem::map::PAGE_SIZE;
    let (heap_used, heap_mapped) = mem::heap::stats();
    let boot = match time::since_boot_us() {
        Some(us) => alloc::format!("{}.{:03} ms", us / 1000, us % 1000),
        None => alloc::string::String::from("necunoscut"),
    };
    info!(
        "metrici: pornire {}; memorie nucleu {} KiB (imagine {} KiB + cadre {} KiB, heap {} / {} KiB); libera {} MiB",
        boot,
        (image + used) / 1024,
        image / 1024,
        used / 1024,
        heap_used / 1024,
        heap_mapped / 1024,
        mem::frame::free_frames() * mem::map::PAGE_SIZE / (1024 * 1024)
    );
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
    test_all_frames();
    test_paging();
    test_heap();
    test_timer();
    test_threads();
    test_preemption();
    test_sleep_and_mutex();
    test_address_spaces();
    test_user_mode();
    test_files();
    test_canvas();
    #[cfg(feature = "test-net")]
    test_network();
    #[cfg(feature = "test-dns")]
    test_dns();
    #[cfg(feature = "test-tcp")]
    test_tcp();
    #[cfg(feature = "test-appnet")]
    test_appnet();
    #[cfg(feature = "test-crypto")]
    test_crypto();
    #[cfg(feature = "test-https")]
    test_https();
    #[cfg(feature = "test-usb")]
    assert!(boot::medium() == boot::Medium::Disk, "test: pornit de pe {:?}, nu de pe stick", boot::medium());
    #[cfg(any(feature = "test-disk", feature = "test-ahci", feature = "test-nvme"))]
    test_disk_files("/disk");
    #[cfg(feature = "test-usbdisk")]
    test_disk_files("/usb");
    #[cfg(feature = "test-disk")]
    test_fat32_mbr();
    test_elf();
    test_user_files();

    #[cfg(feature = "test-keyboard")]
    test_keyboard();

    #[cfg(feature = "test-keyboard-ro")]
    test_keyboard_romanian();

    #[cfg(feature = "test-mouse")]
    test_mouse();

    #[cfg(feature = "test-windows")]
    test_windows();

    #[cfg(feature = "test-appwindow")]
    test_appwindow();

    #[cfg(feature = "test-desktop")]
    test_desktop();

    #[cfg(feature = "test-fatwrite")]
    test_fat_write();

    #[cfg(feature = "test-modules")]
    test_modules();

    #[cfg(feature = "test-sysapps")]
    test_system_apps();

    #[cfg(feature = "test-editor")]
    test_editor_and_settings();

    #[cfg(feature = "test-phase4")]
    test_phase4();

    #[cfg(feature = "test-terminal")]
    test_terminal();

    #[cfg(feature = "test-shell")]
    {
        serial_println!("TASTATURA: astept taste");
        shell::run();
    }

    #[cfg(feature = "test-stress")]
    tests::stress::run();

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

/// Allocates every free frame (including the reclaimed bootloader memory), writes to each one and frees
/// them all: proves nothing still in use (page tables, stack, Limine data) was handed to the allocator.
#[cfg(feature = "qemu-test")]
fn test_all_frames() {
    let before = mem::frame::free_frames();
    // Reserve the list first, so the heap does not need to grow while every frame is taken.
    let mut frames = alloc::vec::Vec::with_capacity(before as usize);
    while let Some(f) = mem::frame::alloc_frame() {
        // SAFETY: the frame was just allocated to this test and is reachable through the HHDM.
        unsafe { core::ptr::write_bytes(mem::map::phys_to_virt(f), 0xA5, mem::map::PAGE_SIZE as usize) };
        frames.push(f);
    }
    let n = frames.len() as u64;
    for f in frames.drain(..) {
        mem::frame::free_frame(f);
    }
    drop(frames);
    assert!(mem::frame::free_frames() == before, "cadre pierdute dupa testul complet");
    info!("test: toate cele {} cadre libere scrise si eliberate", n);

    #[cfg(feature = "test-rodata")]
    {
        static READ_ONLY: u64 = 0xC1BE;
        // SAFETY: deliberate write to a read-only kernel page; must raise a page fault.
        unsafe { (core::ptr::addr_of!(READ_ONLY) as *mut u64).write_volatile(0) };
    }
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

/// Three threads take turns with `yield_now`; the order of their steps proves round-robin switching,
/// and the heap returns to its starting size once their stacks are freed.
#[cfg(feature = "qemu-test")]
fn test_threads() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    static TRACE: [core::sync::atomic::AtomicU8; 32] = [const { core::sync::atomic::AtomicU8::new(0) }; 32];
    static LEN: AtomicUsize = AtomicUsize::new(0);
    fn record(c: u8) {
        TRACE[LEN.fetch_add(1, Ordering::Relaxed)].store(c, Ordering::Relaxed);
    }
    fn worker(name: u8) {
        for step in 1..=3u8 {
            record(name);
            record(b'0' + step);
            sched::yield_now();
        }
    }
    // The exact order needs cooperative switching only: no timer switches during this test.
    sched::preempt_disable();
    let (heap_before, _) = mem::heap::stats();
    sched::spawn(|| worker(b'A'));
    sched::spawn(|| worker(b'B'));
    sched::spawn(|| worker(b'C'));
    while sched::alive() > 1 {
        sched::yield_now();
    }
    sched::yield_now(); // nothing else to run: must return at once
    let n = LEN.load(Ordering::Relaxed);
    let mut text = alloc::string::String::new();
    for pair in TRACE[..n].chunks(2) {
        text.push(pair[0].load(Ordering::Relaxed) as char);
        text.push(pair[1].load(Ordering::Relaxed) as char);
        text.push(' ');
    }
    assert!(text.trim_end() == "A1 B1 C1 A2 B2 C2 A3 B3 C3", "ordine gresita a firelor: {}", text);
    drop(text);
    let (heap_after, _) = mem::heap::stats();
    assert!(heap_after == heap_before, "scurgere dupa fire: {} B inainte, {} B dupa", heap_before, heap_after);
    assert!(sched::current_id() == 0, "testul nu ruleaza pe firul 0");
    sched::preempt_enable();
    info!("test: fire A1 B1 C1 A2 B2 C2 A3 B3 C3, stive eliberate");
}

/// Two threads spin without ever yielding; only the timer can switch between them. After one second
/// both must have made progress, in similar amounts.
#[cfg(feature = "qemu-test")]
fn test_preemption() {
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    static COUNT: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    static STOP: AtomicBool = AtomicBool::new(false);
    fn spin(i: usize) {
        while !STOP.load(Ordering::Relaxed) {
            COUNT[i].fetch_add(1, Ordering::Relaxed);
        }
    }
    let switches_before = sched::switches();
    let ids = [sched::spawn(|| spin(0)), sched::spawn(|| spin(1))];
    let end = arch::x86_64::apic::ticks() + arch::x86_64::apic::HZ;
    while arch::x86_64::apic::ticks() < end {
        arch::x86_64::hlt();
    }
    // Processor time as the scheduler counts it (timer ticks), read before the threads end.
    let t = ids.map(|id| sched::cpu_ticks(id).unwrap_or(0));
    STOP.store(true, Ordering::Relaxed);
    while sched::alive() > 1 {
        arch::x86_64::hlt();
    }
    let (a, b) = (COUNT[0].load(Ordering::Relaxed), COUNT[1].load(Ordering::Relaxed));
    let switches = sched::switches() - switches_before;
    assert!(a > 0 && b > 0, "un fir nu a rulat deloc: {} / {}", a, b);
    // Fairness is judged on scheduler time: loop counts also depend on the emulator, which sometimes
    // pauses the guest for a moment (then the thread running at that time counts less).
    assert!(t[0] <= 3 * t[1] && t[1] <= 3 * t[0], "impartire inegala a procesorului: {:?} tick-uri ({} / {} pasi)", t, a, b);
    info!(
        "test: preemptiune OK ({} comutari intr-o secunda, raport {}:{}, tick-uri {}:{}, pasi {}:{})",
        switches,
        t[0] * 10 / (t[0] + t[1]).max(1),
        t[1] * 10 / (t[0] + t[1]).max(1),
        t[0],
        t[1],
        a / 1000,
        b / 1000
    );
}

/// `sleep_ms` must sleep the requested time (measured in timer ticks); four threads incrementing a counter
/// under a `Mutex` must not lose any increment.
#[cfg(feature = "qemu-test")]
fn test_sleep_and_mutex() {
    use arch::x86_64::apic;
    let start = apic::ticks();
    sched::sleep_ms(500);
    let slept = apic::ticks() - start;
    assert!((50..=55).contains(&slept), "sleep_ms(500) a durat {} ticks (asteptat 50)", slept);

    static COUNTER: sched::Mutex<u64> = sched::Mutex::new(0);
    fn add() {
        for i in 0..10_000u32 {
            let mut c = COUNTER.lock();
            let v = *c;
            if i % 1000 == 0 {
                sched::yield_now(); // give the others a chance to find the lock taken
            }
            *c = v + 1;
        }
    }
    for _ in 0..4 {
        sched::spawn(add);
    }
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    let total = *COUNTER.lock();
    assert!(total == 40_000, "mutex: {} in loc de 40000", total);
    info!("test: sleep_ms(500) = {} ticks; mutex: 4 fire x 10000 = {}", slept, total);
}

/// Two address spaces map the same user address to different frames: each one sees only its own data,
/// threads running in them see the right one, and destroying them gives every frame back.
#[cfg(feature = "qemu-test")]
fn test_address_spaces() {
    use core::sync::atomic::{AtomicU64, Ordering};
    use mem::{address_space::AddressSpace, paging};
    const VIRT: u64 = 0x0000_0040_0000_0000;
    static SEEN: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    let free_before = mem::frame::free_frames();
    {
        let mut a = AddressSpace::new().expect("test: spatiu A");
        let mut b = AddressSpace::new().expect("test: spatiu B");
        let fa = a.map_new(VIRT, paging::WRITABLE).expect("test: mapare A");
        let fb = b.map_new(VIRT, paging::WRITABLE).expect("test: mapare B");
        assert!(fa != fb && a.translate(VIRT) == Some(fa) && b.translate(VIRT) == Some(fb), "translate gresit");
        assert!(paging::translate(VIRT).is_none(), "adresa utilizator vizibila in tabelele nucleului");
        for (space, value) in [(&a, 0xAAAA_u64), (&b, 0xBBBB)] {
            // Interrupts off: a thread switch in between would load this thread's own tables again.
            let flags = sched::irq_save();
            // SAFETY: VIRT is mapped, writable, in `space`, which shares the kernel half with the current tables.
            unsafe {
                paging::load(space.root());
                (VIRT as *mut u64).write_volatile(value);
                paging::load(paging::kernel_root());
            }
            sched::irq_restore(flags);
        }
        // Threads read VIRT in their own space; the scheduler switches CR3 for them.
        fn read_user(i: usize) {
            // SAFETY: the thread runs in a space where VIRT is mapped (spawned with `spawn_in`).
            let v = unsafe { (VIRT as *const u64).read_volatile() };
            SEEN[i].store(v, Ordering::Relaxed);
        }
        sched::spawn_in(a.root(), || read_user(0));
        sched::spawn_in(b.root(), || read_user(1));
        while sched::alive() > 1 {
            sched::yield_now();
        }
        assert!(paging::cr3() == paging::kernel_root(), "firul 0 nu a revenit la tabelele nucleului");
        let seen = (SEEN[0].load(Ordering::Relaxed), SEEN[1].load(Ordering::Relaxed));
        assert!(seen == (0xAAAA, 0xBBBB), "firele au vazut {:#x} / {:#x}", seen.0, seen.1);
    }
    let free_after = mem::frame::free_frames();
    assert!(free_after == free_before, "cadre pierdute: {} inainte, {} dupa", free_before, free_after);
    info!("test: spatii de adrese separate OK (A vede 0xaaaa, B vede 0xbbbb, cadre eliberate)");
}

// Two tiny ring-3 test programs (position-independent machine code, copied into a process).
#[cfg(feature = "qemu-test")]
core::arch::global_asm!(
    ".pushsection .rodata.usertest, \"a\"",
    ".global user_hello_start", ".global user_hello_end",
    ".global user_fault_start", ".global user_fault_end",
    // write(1, msg, 17); exit(7)
    "user_hello_start:",
    "mov eax, 1", "mov edi, 1", "lea rsi, [rip + 2f]", "mov edx, 17", "syscall",
    "mov eax, 0", "mov edi, 7", "syscall",
    "ud2",
    "2: .ascii \"salut din ring 3\\n\"",
    "user_hello_end:",
    // a bad pointer to write (kernel memory), then a read of kernel memory: must stop only this program
    "user_fault_start:",
    "mov eax, 1", "mov edi, 1", "movabs rsi, 0xffffffff80000000", "mov edx, 8", "syscall",
    "cmp rax, -2", "je 3f", "mov eax, 0", "mov edi, 99", "syscall", // not refused with E_FAULT: exit(99)
    "3: movabs rax, 0xffffffff80000000", "mov rax, [rax]",
    "ud2",
    "user_fault_end:",
    ".popsection",
);

/// Runs the ring-3 test programs: one prints through `write` and exits with 7; the other passes a kernel
/// pointer to `write` (must be refused with E_FAULT, else it exits with 99) and then reads kernel memory (stopped by a page fault, kernel unharmed).
#[cfg(feature = "qemu-test")]
fn test_user_mode() {
    unsafe extern "C" {
        static user_hello_start: u8;
        static user_hello_end: u8;
        static user_fault_start: u8;
        static user_fault_end: u8;
    }
    fn blob(start: *const u8, end: *const u8) -> &'static [u8] {
        // SAFETY: both symbols delimit one block of bytes in .rodata, defined above.
        unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
    }
    let hello = blob(core::ptr::addr_of!(user_hello_start), core::ptr::addr_of!(user_hello_end));
    let fault = blob(core::ptr::addr_of!(user_fault_start), core::ptr::addr_of!(user_fault_end));
    let free_before = mem::frame::free_frames();

    let id = proc::spawn_flat(hello).expect("test: nu pot porni programul");
    let code = proc::wait(id);
    assert!(code == 7, "programul a iesit cu {} in loc de 7", code);

    let id = proc::spawn_flat(fault).expect("test: nu pot porni programul");
    let code = proc::wait(id);
    assert!(code == -1, "programul cu eroare a iesit cu {} in loc de -1", code);

    // A program that never makes a system call (jmp $) is stopped from the timer interrupt.
    let id = proc::spawn_flat(&[0xEB, 0xFE]).expect("test: nu pot porni programul");
    sched::sleep_ms(30);
    assert!(proc::kill(id), "kill: programul nu exista");
    let code = proc::wait(id);
    assert!(code == syscall::EXIT_KILLED, "programul oprit a iesit cu {}", code);

    while sched::alive() > 1 {
        sched::sleep_ms(10); // let the exited threads be reaped
    }
    let free_after = mem::frame::free_frames();
    assert!(free_after == free_before, "cadre pierdute dupa procese: {} inainte, {} dupa", free_before, free_after);
    info!("test: mod utilizator OK (cod 7; programul cu eroare oprit, nucleul continua; bucla infinita oprita cu kill)");
}

/// The initrd is in the root ramfs; files can be created, written, read and removed; bad paths fail cleanly.
#[cfg(feature = "qemu-test")]
fn test_files() {
    use fs::{FsError, Kind};
    let names = |dir: &str| -> alloc::string::String {
        let list = fs::list(dir).unwrap_or_default();
        list.iter().map(|(n, k)| if *k == Kind::Dir { alloc::format!("{}/", n) } else { n.clone() }).collect::<alloc::vec::Vec<_>>().join(" ")
    };
    info!("test: / = {}; /bin = {}", names("/"), names("/bin"));
    let motd = fs::read_all("/etc/motd").expect("test: /etc/motd");
    assert!(motd == b"Bun venit in Cyber Universe OS!\n", "continut /etc/motd gresit");
    let (heap_before, _) = mem::heap::stats();
    fs::create("/tmp", Kind::Dir).expect("test: mkdir /tmp");
    fs::create("/tmp/nota.txt", Kind::File).expect("test: creare fisier");
    fs::write("/tmp/nota.txt", 0, b"salut").unwrap();
    fs::write("/tmp/nota.txt", 5, b", lume").unwrap();
    assert!(fs::read_all("/tmp/nota.txt").unwrap() == b"salut, lume", "citire gresita");
    assert!(fs::remove("/tmp") == Err(FsError::NotEmpty), "director plin sters");
    fs::remove("/tmp/nota.txt").unwrap();
    fs::remove("/tmp").unwrap();
    let (heap_after, _) = mem::heap::stats();
    assert!(heap_after == heap_before, "scurgere in ramfs: {} B", heap_after as i64 - heap_before as i64);
    assert!(fs::read_all("/nu/exista") == Err(FsError::NotFound), "cale inexistenta");
    assert!(fs::read_all("/etc") == Err(FsError::IsADirectory), "director citit ca fisier");
    assert!(fs::create("/etc/motd/x", Kind::File) == Err(FsError::NotADirectory), "fisier folosit ca director");
    assert!(fs::kind("relativ") == Err(FsError::InvalidPath), "cale relativa acceptata");
    info!("test: ramfs OK (/etc/motd citit; creare, scriere, citire, stergere; erori: {})", FsError::NotFound.message());
}

/// The test disk (GPT + FAT32, built by tools/build.sh) is mounted at /disk, from memory (`test-disk`) or
/// read through the AHCI driver (`test-ahci`): long names, a sub-directory, a 100 KiB file over many
/// clusters whose checksum matches the one computed at build time, reads from the middle, read-only.
#[cfg(any(feature = "test-disk", feature = "test-ahci", feature = "test-nvme", feature = "test-usbdisk"))]
fn test_disk_files(at: &str) {
    use fs::{FsError, Kind};
    let list = fs::list(at).expect("test: discul lipseste");
    let names: alloc::vec::Vec<_> = list.iter().map(|(n, k)| if *k == Kind::Dir { alloc::format!("{}/", n) } else { n.clone() }).collect();
    info!("test: {} = {}", at, names.join(", "));
    assert!(fs::read_all(&alloc::format!("{}/salut.txt", at)).unwrap() == b"salut de pe disc\n", "salut.txt gresit");
    assert!(fs::read_all(&alloc::format!("{}/Un nume lung de fisier.txt", at)).is_ok(), "nume lung negasit");
    assert!(fs::read_all(&alloc::format!("{}/SALUT.TXT", at)).is_ok(), "numele nu sunt comparate fara majuscule");
    let data = fs::read_all(&alloc::format!("{}/docs/mare.bin", at)).expect("test: mare.bin");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in &data {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let expected = fs::read_all(&alloc::format!("{}/docs/mare.fnv", at)).unwrap();
    let hex = alloc::format!("{:016x}\n", h);
    assert!(data.len() == 100 * 1024 && hex.as_bytes() == &expected[..], "mare.bin: {} octeti, suma {}", data.len(), hex.trim());
    let mut part = [0u8; 1000];
    assert!(fs::read(&alloc::format!("{}/docs/mare.bin", at), 50_000, &mut part) == Ok(1000), "citire de la mijloc");
    assert!(part.iter().enumerate().all(|(i, &b)| b == (((50_000 + i) * 7) ^ ((50_000 + i) >> 8)) as u8), "continut gresit la mijloc");
    assert!(fs::write(&alloc::format!("{}/salut.txt", at), 0, b"x") == Err(FsError::ReadOnly), "scriere acceptata pe FAT32");

    info!("test: disc OK (nume lungi, 100 KiB cu suma {}, doar citire)", hex.trim());
}

/// Writes on the test disk (a copy marked with `cyber.rw`, through AHCI): new folders and files (short
/// and long names), writes across clusters, overwrite, a gap, deletion, a folder that grows past one
/// cluster. tools/after-fatwrite.sh then checks the image with fsck.fat and mtools.
#[cfg(feature = "test-fatwrite")]
fn test_fat_write() {
    use fs::{FsError, Kind};
    let pattern = |i: usize| ((i * 13) ^ (i >> 7)) as u8;
    fs::create("/disk/scris", Kind::Dir).expect("test: mkdir /disk/scris");
    let long = "/disk/scris/Fisier Nou Lung.txt";
    fs::create(long, Kind::File).expect("test: fisier cu nume lung");
    let mut want: alloc::vec::Vec<u8> = (0..10_000).map(pattern).collect();
    assert!(fs::write(long, 0, &want[..3000]) == Ok(3000) && fs::write(long, 3000, &want[3000..]) == Ok(7000), "scriere");
    assert!(fs::read_all(long).unwrap() == want, "citire dupa scriere");
    fs::write(long, 5000, b"CYBER").unwrap();
    want[5000..5005].copy_from_slice(b"CYBER");
    assert!(fs::read_all(long).unwrap() == want && fs::size(long) == Ok(10_000), "suprascriere la mijloc");

    fs::create("/disk/scris/gol.bin", Kind::File).unwrap();
    fs::write("/disk/scris/gol.bin", 1500, b"X").unwrap();
    let gap = fs::read_all("/disk/scris/gol.bin").unwrap();
    assert!(gap.len() == 1501 && gap[..1500].iter().all(|&b| b == 0) && gap[1500] == b'X', "golul nu e plin cu zerouri");

    fs::create("/disk/SCURT.TXT", Kind::File).unwrap();
    fs::write("/disk/SCURT.TXT", 0, b"salut din Cyber Universe\n").unwrap();
    assert!(fs::create("/disk/scurt.txt", Kind::File) == Err(FsError::AlreadyExists), "nume dublu acceptat");

    fs::create("/disk/scris/sterge.txt", Kind::File).unwrap();
    fs::write("/disk/scris/sterge.txt", 0, &[7u8; 2000]).unwrap();
    fs::remove("/disk/scris/sterge.txt").expect("test: stergere");
    assert!(fs::read_all("/disk/scris/sterge.txt") == Err(FsError::NotFound), "fisier sters inca exista");

    fs::create("/disk/scris/sub", Kind::Dir).unwrap();
    fs::create("/disk/scris/sub/a.txt", Kind::File).unwrap();
    assert!(fs::remove("/disk/scris/sub") == Err(FsError::NotEmpty), "director plin sters");
    fs::remove("/disk/scris/sub/a.txt").unwrap();
    fs::remove("/disk/scris/sub").expect("test: director gol");

    // 512-byte clusters hold 16 entries: 20 files (plus "." and "..") make the folder grow.
    fs::create("/disk/scris/multe", Kind::Dir).unwrap();
    for i in 0..20 {
        let p = alloc::format!("/disk/scris/multe/fisier-{:02}.txt", i);
        fs::create(&p, Kind::File).unwrap();
        fs::write(&p, 0, p.as_bytes()).unwrap();
    }
    let list = fs::list("/disk/scris/multe").unwrap();
    assert!(list.len() == 20, "{} fisiere in /disk/scris/multe", list.len());
    assert!(fs::read_all("/disk/scris/multe/fisier-19.txt").unwrap() == b"/disk/scris/multe/fisier-19.txt", "ultimul fisier");
    assert!(fs::read_all("/disk/salut.txt").unwrap() == b"salut de pe disc\n", "fisierele vechi s-au stricat");
    info!("test: scriere FAT32 OK (nume scurte si lungi, 10000 octeti peste clustere, gol, stergere, director marit)");
}

/// Module manager on a copy of the test disk marked for writing: dependency checks, install, start
/// (the program says hello and keeps running), stop (killed), uninstall. `salut` is installed again at
/// the end for the shell's `mod list`; tools/after-modules.sh checks the disk with fsck.fat.
#[cfg(feature = "test-modules")]
fn test_modules() {
    use modmgr::ModError;
    let names = || modmgr::installed().unwrap().into_iter().map(|m| m.name).collect::<alloc::vec::Vec<_>>();
    assert!(names().is_empty(), "module instalate de la inceput");
    let r = modmgr::install("/disk/pachete/cere-salut.cum");
    assert!(r == Err(ModError::MissingDependency("salut".into())), "dependenta lipsa acceptata: {:?}", r);
    let m = modmgr::install("/disk/pachete/salut.cum").expect("test: instalare salut");
    assert!(m.version == "1.0" && m.start.as_deref() == Some("bin/salut"), "module.toml citit gresit: {:?}", m);
    assert!(modmgr::install("/disk/pachete/salut.cum") == Err(ModError::AlreadyInstalled), "instalat de doua ori");
    modmgr::install("/disk/pachete/cere-salut.cum").expect("test: instalare cere-salut");
    assert!(names() == ["salut", "cere-salut"], "lista: {:?}", names());
    assert!(fs::read_all("/disk/modules/cere-salut/docs/citeste.txt").is_ok(), "fisierele modulului lipsesc");
    assert!(modmgr::remove("salut") == Err(ModError::NeededBy("cere-salut".into())), "sters desi e necesar");

    let id = modmgr::start("salut").expect("test: pornire salut");
    sched::sleep_ms(300);
    assert!(modmgr::is_running("salut") && proc::running(id), "programul modulului nu merge");
    assert!(modmgr::start("salut") == Err(ModError::Running), "pornit de doua ori");
    assert!(modmgr::remove("cere-salut").is_ok(), "stergere cere-salut");
    assert!(modmgr::remove("salut") == Err(ModError::Running), "sters in timp ce merge");
    modmgr::stop("salut").expect("test: oprire salut");
    assert!(!modmgr::is_running("salut") && !proc::running(id), "programul modulului inca merge");
    modmgr::remove("salut").expect("test: stergere salut");
    assert!(fs::kind("/disk/modules/salut") == Err(fs::FsError::NotFound) && names().is_empty(), "directorul modulului a ramas");
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    modmgr::install("/disk/pachete/salut.cum").expect("test: reinstalare salut");
    info!("test: module OK (dependente, instalare, pornire, oprire, stergere)");
}

/// System apps with simulated mouse clicks and keys: the monitor lists the running programs and stops
/// one; the file manager copies a file into another folder, deletes it and goes back up (on a copy of
/// the test disk marked for writing).
#[cfg(feature = "test-sysapps")]
fn test_system_apps() {
    use drivers::keyboard::{Key, push_key};
    use input::{MouseEvent, push_mouse};
    let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
        while !ok() {
            assert!(arch::x86_64::apic::ticks() < deadline, "test aplicatii: {}", what);
            sched::sleep_ms(10);
        }
    };
    // Moves the cursor to (x, y) and clicks the left button.
    let click = |x: usize, y: usize| {
        let (cx, cy) = ui::cursor().expect("test: serverul grafic nu merge");
        push_mouse(MouseEvent { dx: (x as isize - cx as isize) as i16, dy: (y as isize - cy as isize) as i16, buttons: 0 });
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 1 });
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 0 });
        sched::sleep_ms(250);
    };
    let key = |c: Key| {
        push_key(c);
        sched::sleep_ms(250);
    };
    let top_is = |id: u64| ui::top_window().is_some_and(|(o, _)| o == id);

    // Monitor: two programs; stop the first (demo).
    let demo = proc::spawn_path("/bin/demo", &["demo"]).expect("test: demo");
    wait_for("demo nu porneste", &|| top_is(demo));
    let monitor = proc::spawn_path("/bin/monitor", &["monitor"]).expect("test: monitor");
    wait_for("monitorul nu porneste", &|| top_is(monitor));
    sched::sleep_ms(300);
    let (_, (mx, my)) = ui::top_window().unwrap();
    click(mx + 40, my + 60 + 10); // row 0 = demo
    key(Key::Char('o'));
    assert!(proc::wait(demo) == syscall::EXIT_KILLED, "demo nu a fost oprit de monitor");

    // Files, started in /disk: rows .., docs/, pachete/, cyber.rw, salut.txt, Un nume lung de fisier.txt
    let files = proc::spawn_path("/bin/files", &["files", "/disk"]).expect("test: files");
    wait_for("managerul de fisiere nu porneste", &|| top_is(files));
    sched::sleep_ms(300);
    let (_, (fx, fy)) = ui::top_window().unwrap();
    let row = |i: usize| (fx + 100, fy + 48 + i * 20 + 10);
    let (x, y) = row(4);
    click(x, y);
    key(Key::Char('c'));
    let (x, y) = row(1);
    click(x, y);
    key(Key::Enter);
    key(Key::Char('v'));
    wait_for("copierea nu a ajuns pe disc", &|| fs::read_all("/disk/docs/salut.txt").is_ok_and(|d| d == b"salut de pe disc\n"));
    let (x, y) = row(3); // in /disk/docs: .., mare.bin, mare.fnv, salut.txt
    click(x, y);
    key(Key::Char('s'));
    wait_for("stergerea nu a ajuns pe disc", &|| fs::kind("/disk/docs/salut.txt") == Err(fs::FsError::NotFound));
    click(fx + 20, fy + 14); // the "Sus" button
    serial_println!("SYSAPPS: captura");
    sched::sleep_ms(1500); // the harness saves the screen now
    key(Key::Char('q'));
    assert!(proc::wait(files) == 0, "files nu a iesit cu 0");
    wait_for("monitorul nu e deasupra", &|| top_is(monitor));
    key(Key::Char('q'));
    assert!(proc::wait(monitor) == 0, "monitorul nu a iesit cu 0");
    wait_for("serverul grafic nu se opreste", &|| !ui::running());
    info!("test: aplicatii de sistem OK (monitor: lista si oprire; fisiere: copiere, lipire, stergere, navigare)");
}

/// The editor gets real key presses from QEMU (letters, Enter, arrows, Home, End, Backspace, Delete,
/// Shift, Ctrl+S, Ctrl+Q) and saves a new file; the settings app saves a time zone and a layout
/// (simulated clicks); the file manager opens a text file in the editor.
#[cfg(feature = "test-editor")]
fn test_editor_and_settings() {
    use drivers::keyboard::{Key, push_key};
    use input::{MouseEvent, push_mouse};
    let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
        while !ok() {
            assert!(arch::x86_64::apic::ticks() < deadline, "test editor: {}", what);
            sched::sleep_ms(10);
        }
    };
    let click = |x: usize, y: usize| {
        let (cx, cy) = ui::cursor().expect("test: serverul grafic nu merge");
        push_mouse(MouseEvent { dx: (x as isize - cx as isize) as i16, dy: (y as isize - cy as isize) as i16, buttons: 0 });
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 1 });
        push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 0 });
        sched::sleep_ms(250);
    };
    let key = |k: Key| {
        push_key(k);
        sched::sleep_ms(250);
    };
    let top_is = |id: u64| ui::top_window().is_some_and(|(o, _)| o == id);

    let editor = proc::spawn_path("/bin/editor", &["editor", "/disk/nou.txt"]).expect("test: editor");
    wait_for("editorul nu porneste", &|| top_is(editor));
    sched::sleep_ms(200);
    serial_println!("TASTATURA: astept taste");
    assert!(proc::wait(editor) == 0, "editorul nu a iesit cu 0");
    let text = fs::read_all("/disk/nou.txt").expect("test: /disk/nou.txt lipseste");
    assert!(text == b"sal!\nolume", "continut salvat: {:?}", core::str::from_utf8(&text));

    let settings = proc::spawn_path("/bin/settings", &["settings"]).expect("test: settings");
    wait_for("setarile nu pornesc", &|| top_is(settings));
    sched::sleep_ms(200);
    let (_, (sx, sy)) = ui::top_window().unwrap();
    click(sx + 330, sy + 48); // +
    click(sx + 330, sy + 48); // +
    click(sx + 300, sy + 88); // keyboard layout
    click(sx + 60, sy + 160); // save
    wait_for("setarile nu s-au salvat", &|| {
        fs::read_all("/disk/config/settings.toml").is_ok_and(|t| {
            let t = alloc::string::String::from_utf8_lossy(&t).into_owned();
            t.contains("fus_orar = \"+2\"") && t.contains("tastatura = \"ro\"")
        })
    });
    key(Key::Char('q'));
    assert!(proc::wait(settings) == 0, "setarile nu au iesit cu 0");

    // Files: rows .., config/, docs/, pachete/, cyber.rw, nou.txt, ...: open nou.txt in the editor.
    let files = proc::spawn_path("/bin/files", &["files", "/disk"]).expect("test: files");
    wait_for("managerul de fisiere nu porneste", &|| top_is(files));
    sched::sleep_ms(200);
    let (_, (fx, fy)) = ui::top_window().unwrap();
    click(fx + 100, fy + 48 + 5 * 20 + 10);
    key(Key::Enter);
    wait_for("fisierul nu se deschide in editor", &|| ui::top_window().is_some_and(|(o, _)| o != files));
    key(Key::Ctrl('q'));
    wait_for("editorul deschis din fisiere nu se inchide", &|| top_is(files));
    key(Key::Char('q'));
    assert!(proc::wait(files) == 0, "files nu a iesit cu 0");
    wait_for("serverul grafic nu se opreste", &|| !ui::running());
    info!("test: editor si setari OK (taste reale, salvare, setari salvate, deschidere din fisiere)");
}

/// Phase 4 criterion: a module is installed and uninstalled from the interface. The harness moves the
/// real (emulated) mouse and clicks in the Modules app: select the package `salut.cum`, Install,
/// select the module, Start, Stop, Remove; the test then closes the app with `q`.
#[cfg(feature = "test-phase4")]
fn test_phase4() {
    let app = proc::spawn_path("/bin/modules", &["modules"]).expect("test: modules");
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    while !ui::top_window().is_some_and(|(o, _)| o == app) {
        assert!(arch::x86_64::apic::ticks() < deadline, "test: aplicatia Module nu porneste");
        sched::sleep_ms(10);
    }
    let (_, origin) = ui::top_window().unwrap();
    // The harness's clicks are computed for this place (the window centred on a 1280 x 800 screen).
    assert!(origin == (380, 230) && ui::cursor() == Some((640, 400)), "fereastra Module la {:?}", origin);
    sched::sleep_ms(300);
    serial_println!("TASTATURA: astept taste");
    // Installed (the folder appears), then removed (it disappears) by the clicks.
    let deadline = arch::x86_64::apic::ticks() + 20 * arch::x86_64::apic::HZ;
    let mut seen = false;
    loop {
        let present = fs::kind("/disk/modules/salut").is_ok();
        seen |= present;
        if seen && !present && !modmgr::is_running("salut") {
            break;
        }
        assert!(arch::x86_64::apic::ticks() < deadline, "test: modulul nu a fost instalat si sters din interfata");
        sched::sleep_ms(50);
    }
    sched::sleep_ms(300);
    serial_println!("PHASE4: captura");
    sched::sleep_ms(1500); // the harness saves the screen now
    drivers::keyboard::push_key(drivers::keyboard::Key::Char('q'));
    assert!(proc::wait(app) == 0, "aplicatia Module nu a iesit cu 0");
    assert!(modmgr::installed().is_ok_and(|l| l.is_empty()), "au ramas module instalate");
    info!("test: Faza 4 OK (modul instalat, pornit, oprit si sters din interfata, cu mouse-ul)");
}

/// The terminal window runs the shell: QEMU types `ls /bin`, `run hello` (its output must reach the
/// window), `mem` and `exit`. Then the desktop is started the way a normal boot starts it, and closed
/// with Escape.
#[cfg(feature = "test-terminal")]
fn test_terminal() {
    let threads_before = sched::alive();
    let app = proc::spawn_path("/bin/terminal", &["terminal"]).expect("test: terminal");
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    while !ui::top_window().is_some_and(|(o, _)| o == app) {
        assert!(arch::x86_64::apic::ticks() < deadline, "test: terminalul nu porneste");
        sched::sleep_ms(10);
    }
    sched::sleep_ms(300);
    serial_println!("TASTATURA: astept taste");
    assert!(proc::wait(app) == 0, "terminalul nu a iesit cu 0");
    let deadline = arch::x86_64::apic::ticks() + 5 * arch::x86_64::apic::HZ;
    while sched::alive() > threads_before || ui::running() {
        assert!(arch::x86_64::apic::ticks() < deadline, "shell-ul terminalului sau serverul grafic nu s-au oprit");
        sched::sleep_ms(10);
    }
    info!("test: terminal OK");

    // Start-up path of a normal boot: the desktop, closed here with Escape once its bar is drawn.
    sched::spawn(|| {
        let screen = |x: usize, y: usize| gfx::console::with_console(|c| c.as_mut().and_then(|c| c.framebuffer().pixel(x, y)));
        let h = ui::screen_size().map_or(0, |s| s.1);
        while screen(2, h - 2) != Some(0x0D1520) {
            sched::sleep_ms(20);
        }
        drivers::keyboard::push_key(drivers::keyboard::Key::Escape);
    });
    assert!(run_desktop(), "desktopul nu a pornit");
    info!("test: desktop la pornire OK");
}

/// The same partition must also be found through an MBR (the disk image seen with a different sector 0).
#[cfg(feature = "test-disk")]
fn test_fat32_mbr() {
    // MBR: the same image, with sector 0 replaced by an MBR that points at the same partition.
    struct WithMbr(block::MemDisk, [u8; 512]);
    impl block::BlockDevice for WithMbr {
        fn sectors(&self) -> u64 {
            self.0.sectors()
        }
        fn read(&self, lba: u64, buf: &mut [u8]) -> Result<(), block::BlockError> {
            self.0.read(lba, buf)?;
            if lba == 0 {
                buf[..512].copy_from_slice(&self.1);
            }
            Ok(())
        }
    }
    let image = boot::file("disk.img").unwrap();
    let mut mbr = [0u8; 512];
    mbr[446 + 4] = 0x0C; // FAT32 (LBA)
    mbr[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
    mbr[446 + 12..446 + 16].copy_from_slice(&(38u32 * 2048).to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    assert!(fs::mount_disk(alloc::boxed::Box::new(WithMbr(block::MemDisk::new(image), mbr)), "/mbr"), "MBR negasit");
    assert!(fs::read_all("/mbr/salut.txt").unwrap() == b"salut de pe disc\n", "citire prin MBR");
    info!("test: FAT32 OK (partitia gasita si prin GPT, si prin MBR)");
}

/// Sends an ARP request for QEMU's virtual gateway (10.0.2.2, user networking; our address 10.0.2.15)
/// and waits for the answer: proves frames go out and come back through the network card.
#[cfg(feature = "test-net")]
fn test_network() {
    let gateway = [10u8, 0, 2, 2];
    let g = net::arp::resolve(gateway, 2000).expect("test: niciun raspuns ARP de la 10.0.2.2");
    info!("net: ARP: 10.0.2.2 este la {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", g[0], g[1], g[2], g[3], g[4], g[5]);
    // Our answer to a ping: an echo request turned into a reply with a valid checksum.
    let request = net::icmp::echo(8, 7, 1, b"abc");
    assert!(net::checksum(&request) == 0, "suma de control ICMP gresita");
    let ms = net::icmp::ping(gateway, 1, 2000).expect("test: niciun raspuns la ping de la 10.0.2.2");
    info!("net: ping 10.0.2.2: raspuns in {} ms", ms);
}

/// Resolves names through the harness's DNS server (tools/dns-test-server.py, reached as 10.0.2.2:15353):
/// a direct answer, a CNAME followed by an A record, a name that does not exist, and an answer from the cache.
#[cfg(feature = "test-dns")]
fn test_dns() {
    use net::dns::{self, Error};
    assert!(net::wait_ready(5000), "test: DHCP nu a dat nicio adresa");
    let server = [10u8, 0, 2, 2];
    let (ip, cache) = dns::resolve_via(server, 15353, "test.cyber").expect("test: test.cyber nu s-a rezolvat");
    assert!(ip == [10, 1, 2, 3] && !cache, "test: test.cyber -> {:?}", ip);
    info!("net: DNS: test.cyber -> 10.1.2.3");
    let (ip, _) = dns::resolve_via(server, 15353, "Alias.Cyber.").expect("test: alias.cyber nu s-a rezolvat");
    assert!(ip == [10, 1, 2, 3], "test: alias.cyber -> {:?}", ip);
    info!("net: DNS: alias.cyber (CNAME) -> 10.1.2.3");
    assert!(dns::resolve_via(server, 15353, "lipsa.cyber") == Err(Error::NotFound), "test: lipsa.cyber trebuia sa lipseasca");
    info!("net: DNS: lipsa.cyber nu exista (corect)");
    assert!(dns::resolve_via(server, 15353, "test.cyber") == Ok(([10, 1, 2, 3], true)), "test: cache-ul nu a raspuns");
    info!("net: DNS: test.cyber din memorie");
    assert!(dns::resolve_via(server, 15353, "rau..nume") == Err(Error::BadName), "test: nume invalid acceptat");
    assert!(dns::resolve("10.0.2.2") == Ok(([10, 0, 2, 2], false)), "test: o adresa trebuie intoarsa direct");
}

/// TCP against the harness's web server (tools/before-tcp.sh: python's http.server, reached as 10.0.2.2:18080):
/// a small page, a 200 000-byte file with one received segment "lost" (out-of-order data kept and reassembled),
/// a request whose first transmission is "lost" (retransmission timer), and a refused connection.
#[cfg(feature = "test-tcp")]
fn test_tcp() {
    use net::tcp;
    assert!(net::wait_ready(5000), "test: DHCP nu a dat nicio adresa");
    let server = [10u8, 0, 2, 2];
    fn get(server: net::Ipv4, path: &str, lose_request: bool) -> (alloc::vec::Vec<u8>, u32) {
        let c = tcp::connect(server, 18080, 5000).expect("test: conexiunea TCP nu s-a deschis");
        if lose_request {
            tcp::DROP_NEXT_SEND.store(true, core::sync::atomic::Ordering::Relaxed);
        }
        let req = alloc::format!("GET {} HTTP/1.0\r\nHost: test\r\n\r\n", path);
        tcp::send(c, req.as_bytes(), 5000).expect("test: trimitere TCP");
        let mut all = alloc::vec::Vec::new();
        loop {
            let d = tcp::recv(c, 8192, 10_000).expect("test: primire TCP");
            if d.is_empty() {
                break;
            }
            all.extend_from_slice(&d);
        }
        let retransmits = tcp::retransmissions(c);
        tcp::close(c, 2000);
        let body = all.windows(4).position(|w| w == b"\r\n\r\n").map(|i| all[i + 4..].to_vec()).expect("test: raspuns HTTP fara antet");
        assert!(all.starts_with(b"HTTP/1.0 200"), "test: raspuns HTTP: {:?}", core::str::from_utf8(&all[..all.len().min(40)]));
        (body, retransmits)
    }
    let (small, _) = get(server, "/mic.txt", false);
    assert!(small == b"Salut de la serverul de test TCP\n", "test: mic.txt gresit");
    info!("tcp: GET /mic.txt -> {} octeti, text corect", small.len());

    tcp::DROP_RECV_AFTER.store(5, core::sync::atomic::Ordering::Relaxed);
    let start = arch::x86_64::apic::ticks();
    let (big, _) = get(server, "/mare.bin", false);
    let ms = (arch::x86_64::apic::ticks() - start) * 1000 / arch::x86_64::apic::HZ;
    assert!(big.len() == 200_000, "test: mare.bin are {} octeti", big.len());
    assert!(big.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8), "test: continutul mare.bin difera");
    info!("tcp: GET /mare.bin -> 200000 octeti corecti, un segment pierdut si recuperat ({} ms)", ms);

    let (again, retransmits) = get(server, "/mic.txt", true);
    assert!(again == small && retransmits >= 1, "test: retransmisia ({} retransmisii)", retransmits);
    info!("tcp: cererea pierduta a fost retrimisa ({} retransmisie)", retransmits);

    assert!(tcp::connect(server, 18081, 3000) == Err(tcp::Error::Refused), "test: portul inchis trebuia refuzat");
    info!("tcp: port inchis -> conexiune refuzata (corect)");
}

/// Programs on the network (P5-05): /bin/fetch downloads the test page over TCP, /bin/udp talks to the UDP
/// echo server (both started by tools/before-appnet.sh); their output goes to the serial port.
#[cfg(feature = "test-appnet")]
fn test_appnet() {
    assert!(net::wait_ready(5000), "test: DHCP nu a dat nicio adresa");
    for (args, what) in [
        (&["fetch", "10.0.2.2", "18080", "/mic.txt"][..], "fetch"),
        (&["fetch", "10.0.2.2", "18080", "/lipsa.txt"][..], "fetch (pagina lipsa)"),
        (&["udp", "10.0.2.2", "15354", "salut"][..], "udp"),
    ] {
        let id = proc::spawn_path(alloc::format!("/bin/{}", args[0]).as_str(), args).expect("test: program de retea");
        let code = proc::wait_child(id);
        info!("test: {} a iesit cu codul {}", what, code);
    }
    // The Network app: clear the address box, type the gateway, Enter = four pings; then Esc.
    use drivers::keyboard::{Key, push_key};
    let app = proc::spawn_path("/bin/network", &["network"]).expect("test: network");
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    while !ui::top_window().is_some_and(|(o, _)| o == app) {
        assert!(arch::x86_64::apic::ticks() < deadline, "test: aplicatia Retea nu porneste");
        sched::sleep_ms(10);
    }
    sched::sleep_ms(300);
    for _ in 0..10 {
        push_key(Key::Backspace);
    }
    for c in "10.0.2.2".chars() {
        push_key(Key::Char(c));
    }
    push_key(Key::Enter);
    sched::sleep_ms(1500); // four pings, 200 ms apart at most in QEMU
    info!("NETWORK: captura");
    sched::sleep_ms(1000);
    push_key(Key::Escape);
    assert!(proc::wait(app) == 0, "test: aplicatia Retea nu s-a inchis");
    info!("test: aplicatia Retea OK");
}

/// HTTPS from a program (P5-16): /bin/fetch talks TLS 1.3 to the OpenSSL test server (tools/before-https.sh, reached
/// as 10.0.2.2:18443). The test root is put in /etc/certs only here: its private key is public (tls/tests/data).
#[cfg(feature = "test-https")]
fn test_https() {
    assert!(net::wait_ready(5000), "test: DHCP nu a dat nicio adresa");
    let _ = fs::create("/etc/certs", fs::Kind::Dir);
    fs::create("/etc/certs/test.der", fs::Kind::File).expect("test: /etc/certs/test.der");
    fs::write("/etc/certs/test.der", 0, include_bytes!("../../tls/tests/data/root.der")).expect("test: radacina de test");
    let mut r = [0u8; 16];
    random::fill(&mut r);
    let mut r2 = [0u8; 16];
    random::fill(&mut r2);
    assert!(r != r2 && r != [0; 16], "test: numerele aleatoare se repeta");
    info!("test: aleator {} (RDRAND: {})", if r != r2 { "diferit la fiecare cerere" } else { "?" }, random::hardware());
    let url = "https://test.cyber.local:18443/pagina";
    for (args, what) in [
        (&["fetch", url, "10.0.2.2"][..], "fetch https"),
        (&["fetch", "https://alt.cyber.local:18443/", "10.0.2.2"][..], "fetch https (nume gresit)"),
    ] {
        let start = arch::x86_64::apic::ticks();
        let id = proc::spawn_path("/bin/fetch", args).expect("test: /bin/fetch");
        let code = proc::wait_child(id);
        let ms = (arch::x86_64::apic::ticks() - start) * 1000 / arch::x86_64::apic::HZ;
        info!("test: {} a iesit cu codul {} in {} ms", what, code, ms);
    }
}

/// Our own cryptography (crypto/, D-011): every official test vector, then the SHA-256 speed.
#[cfg(feature = "test-crypto")]
fn test_crypto() {
    use cyber_crypto::{Digest, sha2::Sha256};
    let ticks = arch::x86_64::apic::ticks();
    match cyber_crypto::vectors::self_test() {
        Ok(n) => info!("crypto: {} vectori oficiali trecuti (SHA-256/384/512, HMAC, HKDF, TLS 1.3, X25519, ChaCha20-Poly1305, RSA, ECDSA P-256, X.509)", n),
        Err(name) => panic!("crypto: vectorul {} a esuat", name),
    }
    let ms = (arch::x86_64::apic::ticks() - ticks) * 1000 / arch::x86_64::apic::HZ;
    let block = alloc::vec![0x5Au8; 1 << 20];
    let start = arch::x86_64::apic::ticks();
    let mut h = Sha256::new();
    for _ in 0..4 {
        h.update(&block);
    }
    core::hint::black_box(h.finish());
    let t = ((arch::x86_64::apic::ticks() - start) * 1000 / arch::x86_64::apic::HZ).max(1);
    // ChaCha20-Poly1305 over 4 MiB, in 16 KiB pieces (the largest TLS record).
    let mut block = block;
    let start = arch::x86_64::apic::ticks();
    for _ in 0..4 {
        for piece in block.chunks_mut(16 * 1024) {
            core::hint::black_box(cyber_crypto::aead::seal(&[7; 32], &[1; 12], &[], piece));
        }
    }
    let t2 = ((arch::x86_64::apic::ticks() - start) * 1000 / arch::x86_64::apic::HZ).max(1);
    info!("crypto: vectori in {} ms; SHA-256: {} MiB/s, ChaCha20-Poly1305: {} MiB/s (in emulare)", ms, 4 * 1000 / t, 4 * 1000 / t2);
}

/// Draws a known scene in a back buffer, checks pixels, the changed area and the copy to the screen,
/// and measures how many full-screen frames per second the CPU can draw and copy.
#[cfg(feature = "qemu-test")]
fn test_canvas() {
    use gfx::canvas::{Canvas, Rect};
    let mut c = Canvas::new(200, 100, 0x000000);
    assert!(c.take_dirty() == Some(Rect::new(0, 0, 200, 100)), "o panza noua trebuie redesenata toata");
    c.fill(Rect::new(10, 10, 50, 20), 0xFF0000);
    c.outline(Rect::new(100, 10, 40, 30), 2, 0x00FF00);
    c.line(0, 99, 199, 0, 0x0000FF);
    let sprite = [0x123456, 0xABCDEF, 0xABCDEF, 0x123456];
    c.blit(180, 80, 2, 2, &sprite, Some(0xABCDEF));
    let w = c.text(20, 60, "Cyber", 0xFFFFFF, 2);
    assert!(c.pixel(10, 10) == Some(0xFF0000) && c.pixel(59, 29) == Some(0xFF0000) && c.pixel(60, 30) == Some(0), "dreptunghi");
    assert!(c.pixel(100, 10) == Some(0x00FF00) && c.pixel(101, 39) == Some(0x00FF00) && c.pixel(120, 25) == Some(0), "contur");
    assert!(c.pixel(0, 99) == Some(0x0000FF) && c.pixel(199, 0) == Some(0x0000FF), "capetele liniei");
    assert!(c.pixel(180, 80) == Some(0x123456) && c.pixel(181, 80) == Some(0), "imagine cu transparenta");
    assert!(w == 5 * 12, "latimea textului");
    assert!(c.take_dirty() == Some(Rect::new(0, 0, 200, 100)), "zona schimbata");
    c.fill(Rect::new(30, 40, 5, 5), 0x808080);
    assert!(c.take_dirty() == Some(Rect::new(30, 40, 5, 5)), "doar zona noua");

    // Romanian letters: the comma below Ș, the breve over ă, and a box for a character the font lacks.
    let mut t = Canvas::new(40, 10, 0);
    t.text(0, 0, "Șă§", 0xFFFFFF, 1);
    let lit = |x, y| t.pixel(x, y) == Some(0xFFFFFF);
    assert!(lit(2, 8) && lit(1, 9) && !lit(2, 0), "Ș: virgula de dedesubt");
    assert!(lit(6, 0) && lit(10, 0) && lit(7, 1) && lit(6, 6), "ă: caciula");
    assert!((12..17).all(|x| lit(x, 1)) && lit(12, 4) && !lit(14, 4), "caracter necunoscut: chenar");
    let mut g = Canvas::new(6, 10, 0);
    g.text(0, 0, "g", 0xFFFFFF, 1);
    let lit_g = |x, y| g.pixel(x, y) == Some(0xFFFFFF);
    assert!(lit_g(1, 9) && lit_g(3, 9) && lit_g(4, 8) && !lit_g(1, 2), "g: coada sub rand");

    // Copy to the screen (bottom-right corner) and read it back.
    let on_screen = gfx::console::with_console(|slot| {
        let fb = slot.as_mut()?.framebuffer();
        let (x, y) = (fb.width - 200, fb.height - 100);
        let mut scene = Canvas::new(200, 100, 0x203040);
        scene.fill(Rect::new(10, 10, 50, 20), 0xFF0000);
        scene.present(fb, x, y);
        Some((fb.pixel(x + 10, y + 10), fb.pixel(x + 5, y + 5)))
    });
    if let Some(p) = on_screen {
        assert!(p == (Some(0xFF0000), Some(0x203040)), "copierea pe ecran: {:x?}", p);
    }

    // Speed: full-screen frames (draw everything + copy everything) in half a second.
    let fps = gfx::console::with_console(|slot| {
        let fb = slot.as_mut()?.framebuffer();
        let (w, h) = (fb.width, fb.height);
        let mut frame = Canvas::new(w, h, 0);
        let start = arch::x86_64::apic::ticks();
        let mut frames = 0u64;
        while arch::x86_64::apic::ticks() < start + arch::x86_64::apic::HZ / 2 {
            frame.fill(Rect::new(0, 0, w, h), 0x0010_1828 + frames as u32 % 16);
            frame.present(fb, 0, 0);
            frames += 1;
        }
        Some(frames * 2)
    });
    info!("test: grafica OK (desen in memorie, zone schimbate, diacritice, copiere pe ecran; {} cadre/s pe tot ecranul)", fps.unwrap_or(0));
    println!("Diacritice: ă â î ș ț  Ă Â Î Ș Ț");
}

/// Runs the `hello` program (an ELF file from the initrd) and checks that damaged ELF files are refused.
#[cfg(feature = "qemu-test")]
fn test_elf() {
    use proc::{SpawnError, elf::ElfError};
    let file = fs::read_all("/bin/hello").expect("test: /bin/hello lipseste din initrd");
    let hello = &file[..];
    let free_before = mem::frame::free_frames();
    let id = proc::spawn_elf(hello, &["hello"]).expect("test: hello nu porneste");
    let code = proc::wait(id);
    assert!(code == 0, "hello a iesit cu {}", code);

    let mut bad = alloc::vec::Vec::from(hello);
    bad[18] = 0x28; // machine: ARM
    let r = proc::spawn_elf(&bad, &[]);
    assert!(matches!(r, Err(SpawnError::BadElf(ElfError::NotX86_64))), "ELF stricat acceptat: {:?}", r);
    info!("test: ELF stricat refuzat ({})", ElfError::NotX86_64.message());
    bad[18] = 0x3E;
    bad[0] = 0;
    assert!(matches!(proc::spawn_elf(&bad, &[]), Err(SpawnError::BadElf(ElfError::NotElf))), "fisier non-ELF acceptat");
    drop(bad);
    assert!(matches!(proc::spawn_elf(&hello[..40], &[]), Err(SpawnError::BadElf(ElfError::TooSmall))), "fisier trunchiat acceptat");

    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    let free_after = mem::frame::free_frames();
    assert!(free_after == free_before, "cadre pierdute dupa ELF: {} inainte, {} dupa", free_before, free_after);
    info!("test: ELF OK (hello a iesit cu 0, fisierele stricate refuzate, memoria eliberata)");
}

/// `/bin/cat` (a separate program) opens and reads files through system calls; a missing file gives exit 1.
#[cfg(feature = "qemu-test")]
fn test_user_files() {
    let free_before = mem::frame::free_frames();
    let id = proc::spawn_path("/bin/cat", &["cat", "/etc/motd"]).expect("test: /bin/cat nu porneste");
    assert!(proc::wait(id) == 0, "cat /etc/motd nu a iesit cu 0");
    let id = proc::spawn_path("/bin/cat", &["cat", "/nu/exista", "/etc"]).expect("test: /bin/cat nu porneste");
    assert!(proc::wait(id) == 1, "cat pe fisiere lipsa nu a iesit cu 1");
    assert!(matches!(proc::spawn_path("/bin/lipsa", &[]), Err(proc::SpawnError::NotFound(_))), "program lipsa pornit");
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    let free_after = mem::frame::free_frames();
    assert!(free_after == free_before, "cadre pierdute dupa cat: {} inainte, {} dupa", free_before, free_after);
    info!("test: fisiere din programe OK (cat /etc/motd; erori: cod 1)");
}

/// Waits for the mouse events the test harness sends (a movement, then a left click) and logs them.
#[cfg(feature = "test-mouse")]
fn test_mouse() {
    serial_println!("TASTATURA: astept taste"); // the harness sends its monitor commands now
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    let mut got = alloc::vec::Vec::new();
    while got.len() < 2 && arch::x86_64::apic::ticks() < deadline {
        match input::read_mouse() {
            Some(e) => {
                info!("MOUSE: dx={} dy={} butoane={}", e.dx, e.dy, e.buttons);
                got.push(e);
            }
            None => sched::sleep_ms(10),
        }
    }
    assert!(got.len() == 2, "test: {} evenimente de mouse primite (asteptat 2)", got.len());
    info!("test: mouse OK");
}

/// Two windows; the harness types two keys (they must reach the focused window) and drags the top window by
/// its title bar with the mouse. The screen is read back to check where each window ended up; then the
/// first window is closed with its close button (synthetic mouse events).
#[cfg(feature = "test-windows")]
fn test_windows() {
    use gfx::canvas::Rect;
    use input::MouseEvent;
    use ui::server::{BORDER, DESKTOP, Server, TITLE_H};
    const GREEN: u32 = 0x30A050;
    const ORANGE: u32 = 0xE0A030;
    let mut s = Server::new().expect("test: niciun ecran pentru ferestre");
    let (cx, cy) = s.cursor();
    let a = s.create("Fereastra 1", cx - 250, cy - 150, 300, 180);
    let b = s.create("Fereastra 2", cx - 30, cy - 10, 300, 180); // the cursor is on its title bar
    s.window_mut(a).unwrap().client.fill(Rect::new(0, 0, 300, 180), GREEN);
    s.window_mut(b).unwrap().client.fill(Rect::new(0, 0, 300, 180), ORANGE);
    s.present();
    let screen = |x: usize, y: usize| gfx::console::with_console(|c| c.as_mut().and_then(|c| c.framebuffer().pixel(x, y)));
    assert!(screen(cx, cy + 50) == Some(ORANGE) && screen(cx - 100, cy - 100) == Some(GREEN), "ferestrele nu apar pe ecran");

    serial_println!("TASTATURA: astept taste"); // the harness types h, i, then presses, moves (100, 50), releases
    let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
    let target = (cx + 70, cy + 40);
    let mut keys = alloc::vec::Vec::new();
    loop {
        s.pump();
        while let Some(ui::server::Event::Key(k)) = s.next_event(b) {
            keys.push(k);
        }
        let r = s.window(b).unwrap().outer();
        if (r.x, r.y) == target && keys.len() >= 2 && input::read_mouse().is_none() {
            break;
        }
        assert!(arch::x86_64::apic::ticks() < deadline, "test: fereastra 2 la ({}, {}), asteptat {:?}; taste {:?}", r.x, r.y, target, keys);
        sched::sleep_ms(10);
    }
    sched::sleep_ms(300); // the button release arrives last
    s.pump();
    use drivers::keyboard::Key;
    assert!(keys == [Key::Char('h'), Key::Char('i')], "tastele nu au ajuns la fereastra activa: {:?}", keys);
    info!("FERESTRE: fereastra 2 mutata la ({}, {}), tastele h i primite", target.0 - cx, target.1 - cy);
    let ok = [
        (cx + 100, cy + 100, ORANGE, "fereastra 2 in locul nou"),
        (cx + 10, cy + 100, DESKTOP, "locul vechi al ferestrei 2 (fundal)"),
        (cx + 10, cy + 20, GREEN, "fereastra 1 descoperita"),
        (cx + 70 + 5, cy + 40 + TITLE_H + 5, ORANGE, "coltul ferestrei 2"),
        (cx + 70 + 5, cy + 40 + 5, ui::server::TITLE_FOCUSED, "bara de titlu activa"),
    ];
    for (x, y, want, what) in ok {
        let got = screen(x, y);
        assert!(got == Some(want), "{}: pixel ({}, {}) = {:x?}, asteptat {:x}", what, x, y, got, want);
    }
    assert!(s.focused() == Some(b), "focus gresit");

    // Click on window 1 (raises it), then on its close button.
    let click = |s: &mut Server, x: usize, y: usize| {
        let (px, py) = s.cursor();
        s.handle_mouse(MouseEvent { dx: (x as isize - px as isize) as i16, dy: (y as isize - py as isize) as i16, buttons: 0 });
        s.handle_mouse(MouseEvent { dx: 0, dy: 0, buttons: 1 });
        s.handle_mouse(MouseEvent { dx: 0, dy: 0, buttons: 0 });
        s.present();
    };
    click(&mut s, cx + 10, cy + 20);
    assert!(s.focused() == Some(a), "clic pe fereastra 1: nu a primit focusul");
    let (title_a, title_b) = (screen(cx - 250 + 5, cy - 150 + 5), screen(cx + 70 + 5, cy + 40 + 5));
    assert!(title_a == Some(ui::server::TITLE_FOCUSED) && title_b == Some(ui::server::TITLE_IDLE), "culorile barelor de titlu");
    let r = s.window(a).unwrap().outer();
    click(&mut s, r.x + r.w - 3 - 8 - BORDER, r.y + TITLE_H / 2);
    assert!(s.window(a).is_none() && s.focused() == Some(b), "butonul de inchidere nu a inchis fereastra 1");
    assert!(screen(cx - 100, cy - 100) == Some(DESKTOP) && screen(cx + 100, cy + 100) == Some(ORANGE), "dupa inchidere");
    info!("test: ferestre OK (mutare cu mouse-ul, focus, taste, inchidere; pixeli verificati)");
}

/// `/bin/demo` opens a window through system calls; the screen must show its picture. The harness clicks
/// in the middle of the screen (inside the window) and types `q`; the program reports both and exits,
/// its window closes and the text console gets the screen back.
#[cfg(feature = "test-appwindow")]
fn test_appwindow() {
    let id = proc::spawn_path("/bin/demo", &["demo"]).expect("test: /bin/demo nu porneste");
    let screen = |x: usize, y: usize| gfx::console::with_console(|c| c.as_mut().and_then(|c| c.framebuffer().pixel(x, y)));
    let deadline = arch::x86_64::apic::ticks() + 5 * arch::x86_64::apic::HZ;
    let (ox, oy) = loop {
        if let Some((x, y)) = ui::window_of(id) {
            if screen(x + 30, y + 30) == Some(0xE03030) {
                break (x, y);
            }
        }
        assert!(arch::x86_64::apic::ticks() < deadline, "test: fereastra programului demo nu apare");
        sched::sleep_ms(10);
    };
    assert!(screen(ox + 200, oy + 150) == Some(0x203050) && screen(ox + 119, oy + 79) == Some(0xE03030)
        && screen(ox + 120, oy + 80) == Some(0x203050), "pixelii ferestrei demo");
    info!("FEREASTRA PROGRAM: continut la ({}, {}), pixeli corecti", ox, oy);
    serial_println!("TASTATURA: astept taste"); // the harness clicks, then types q
    let code = proc::wait(id);
    assert!(code == 0, "demo a iesit cu {}", code);
    let deadline = arch::x86_64::apic::ticks() + arch::x86_64::apic::HZ;
    while ui::running() {
        assert!(arch::x86_64::apic::ticks() < deadline, "serverul grafic nu s-a oprit dupa ultima fereastra");
        sched::sleep_ms(10);
    }
    println!("Consola a revenit dupa fereastra programului.");
    info!("test: ferestre pentru programe OK (fereastra prin apeluri de sistem, clic, tasta, inchidere la iesire)");
}

/// The desktop program: its bar appears; the harness clicks Start with the mouse; the menu opens; a
/// (simulated) click on `demo` starts it; `q` closes it; a screenshot is taken; Escape closes the desktop.
#[cfg(feature = "test-desktop")]
fn test_desktop() {
    use drivers::keyboard::{Key, push_key};
    use input::{MouseEvent, push_mouse};
    const BAR: u32 = 0x0D1520;
    const START: u32 = 0x2070C0;
    const MENU: u32 = 0x24303C;
    let screen = |x: usize, y: usize| gfx::console::with_console(|c| c.as_mut().and_then(|c| c.framebuffer().pixel(x, y)));
    let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = arch::x86_64::apic::ticks() + 10 * arch::x86_64::apic::HZ;
        while !ok() {
            assert!(arch::x86_64::apic::ticks() < deadline, "test desktop: {}", what);
            sched::sleep_ms(10);
        }
    };
    let used_before = mem::frame::used_frames();
    let id = proc::spawn_path("/bin/desktop", &["desktop"]).expect("test: /bin/desktop nu porneste");
    let (_, h) = ui::screen_size().expect("test: fara ecran");
    wait_for("bara de jos nu apare", &|| screen(2, h - 2) == Some(BAR) && screen(10, h - 16) == Some(START));
    let (heap_used, heap_mapped) = mem::heap::stats();
    info!(
        "metrici: desktop pornit: +{} KiB memorie (program + server grafic); heap {} / {} KiB; total folosit {} MiB",
        (mem::frame::used_frames() - used_before) * mem::map::PAGE_SIZE / 1024,
        heap_used / 1024,
        heap_mapped / 1024,
        mem::frame::used_frames() * mem::map::PAGE_SIZE / (1024 * 1024)
    );
    serial_println!("TASTATURA: astept taste"); // the harness moves the mouse onto Start and clicks
    wait_for("meniul nu se deschide", &|| screen(150, h - 32 - 6) == Some(MENU));
    // Menu items from the top: the programs in /bin except the desktop, sorted (cat, demo, ...)
    // (24 pixels each, 4 of margin, the bar is 32 high); the cursor is on Start (46, h - 16); `demo` is
    // the second.
    let items = fs::list("/bin").expect("test: /bin").iter().filter(|(n, _)| n != "desktop").count();
    let demo_y = h - 32 - (items * 24 + 8) + 4 + 24 + 12;
    push_mouse(MouseEvent { dx: 4, dy: demo_y as i16 - (h - 16) as i16, buttons: 0 });
    push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 1 });
    push_mouse(MouseEvent { dx: 0, dy: 0, buttons: 0 });
    let demo_up = || ui::top_window().is_some_and(|(o, (x, y))| o != id && screen(x + 30, y + 30) == Some(0xE03030));
    wait_for("programul demo nu porneste din meniu", &demo_up);
    assert!(screen(150, h - 32 - 6) != Some(MENU), "meniul a ramas deschis");
    push_key(Key::Char('q'));
    wait_for("demo nu se inchide", &|| ui::top_window().is_some_and(|(o, _)| o == id));
    sched::sleep_ms(100);
    serial_println!("DESKTOP: captura");
    sched::sleep_ms(1500); // the harness saves the screen now
    push_key(Key::Escape);
    assert!(proc::wait(id) == 0, "desktopul nu a iesit cu 0");
    wait_for("serverul grafic nu se opreste", &|| !ui::running());
    info!("test: desktop OK (bara, ceas, meniu Start cu mouse-ul, pornire program din meniu, Esc)");
}

/// Romanian (Standard) layout: QEMU presses the US keys [ ] \ ; ' (alone and with Shift), AltGr+E, `,
/// y and z; the kernel must get ă î â ș ț Ș Ț € „ y z.
#[cfg(feature = "test-keyboard-ro")]
fn test_keyboard_romanian() {
    use drivers::keyboard::{Key, Layout, set_layout, wait_key};
    set_layout(Layout::Ro);
    serial_println!("TASTATURA: astept taste");
    let want = ['ă', 'î', 'â', 'ș', 'ț', 'Ș', 'Ț', '€', '„', 'y', 'z'];
    let mut got = alloc::string::String::new();
    while got.chars().count() < want.len() {
        match wait_key() {
            Key::Char(c) => {
                info!("KEY: {}", c);
                got.push(c);
            }
            other => info!("KEY: {:?}", other),
        }
    }
    assert!(got.chars().eq(want.iter().copied()), "taste primite: {}", got);
    set_layout(Layout::Us);
    info!("test: tastatura romana OK ({})", got);
}

/// Waits for the keys the test harness types (h, i, Enter) and logs them.
#[cfg(feature = "test-keyboard")]
fn test_keyboard() {
    use drivers::keyboard::{Key, wait_key};
    serial_println!("TASTATURA: astept taste");
    let mut got = alloc::vec::Vec::new();
    // The harness has a time limit, so a missing key fails the test by timeout.
    while got.len() < 3 {
        let k = wait_key();
        match k {
            Key::Char(c) => info!("KEY: {}", c),
            other => info!("KEY: {:?}", other),
        }
        got.push(k);
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
