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
mod log;
mod mem;
mod pci;
mod proc;
mod sched;
#[cfg(any(not(feature = "qemu-test"), feature = "test-shell"))]
mod shell;
mod syscall;
#[cfg(feature = "qemu-test")]
mod qemu;
#[cfg(feature = "qemu-test")]
mod tests;
mod time;

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
        drivers::keyboard::init();
    } else {
        warn!("sistem: fara ceas (APIC indisponibil); continui fara intreruperi");
    }
    if has_acpi {
        pci::init();
    }
    boot::collect_files();
    fs::init();
    // Every Limine response has been read (memory map, HHDM, framebuffer, RSDP, modules): its memory can be reused.
    mem::reclaim_bootloader();

    report_boot_metrics();

    #[cfg(feature = "qemu-test")]
    run_tests();

    #[cfg(not(feature = "qemu-test"))]
    loop {
        shell::run();
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
    #[cfg(feature = "test-disk")]
    test_fat32();
    test_elf();
    test_user_files();

    #[cfg(feature = "test-keyboard")]
    test_keyboard();

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
    sched::spawn(|| spin(0));
    sched::spawn(|| spin(1));
    let end = arch::x86_64::apic::ticks() + arch::x86_64::apic::HZ;
    while arch::x86_64::apic::ticks() < end {
        arch::x86_64::hlt();
    }
    STOP.store(true, Ordering::Relaxed);
    while sched::alive() > 1 {
        arch::x86_64::hlt();
    }
    let (a, b) = (COUNT[0].load(Ordering::Relaxed), COUNT[1].load(Ordering::Relaxed));
    let switches = sched::switches() - switches_before;
    assert!(a > 0 && b > 0, "un fir nu a rulat deloc: {} / {}", a, b);
    assert!(a <= 3 * b && b <= 3 * a, "impartire inegala a procesorului: {} / {}", a, b);
    info!("test: preemptiune OK ({} comutari intr-o secunda, raport {}:{})", switches, a * 10 / (a + b), b * 10 / (a + b));
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
            // SAFETY: VIRT is mapped, writable, in `space`, which shares the kernel half with the current tables.
            unsafe {
                paging::load(space.root());
                (VIRT as *mut u64).write_volatile(value);
                paging::load(paging::kernel_root());
            }
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

    while sched::alive() > 1 {
        sched::sleep_ms(10); // let the exited threads be reaped
    }
    let free_after = mem::frame::free_frames();
    assert!(free_after == free_before, "cadre pierdute dupa procese: {} inainte, {} dupa", free_before, free_after);
    info!("test: mod utilizator OK (cod 7; programul cu eroare oprit, nucleul continua)");
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

/// The test disk (GPT + FAT32, built by tools/build.sh with `test-disk`) is mounted at /disk: long names, a sub-directory,
/// a 100 KiB file over many clusters whose checksum matches the one computed at build time, read-only.
/// The same partition is then found through an MBR (the disk seen with a different sector 0).
#[cfg(feature = "test-disk")]
fn test_fat32() {
    use fs::{FsError, Kind};
    let list = fs::list("/disk").expect("test: /disk lipseste");
    let names: alloc::vec::Vec<_> = list.iter().map(|(n, k)| if *k == Kind::Dir { alloc::format!("{}/", n) } else { n.clone() }).collect();
    info!("test: /disk = {}", names.join(", "));
    assert!(fs::read_all("/disk/salut.txt").unwrap() == b"salut de pe disc\n", "salut.txt gresit");
    assert!(fs::read_all("/disk/Un nume lung de fisier.txt").is_ok(), "nume lung negasit");
    assert!(fs::read_all("/disk/SALUT.TXT").is_ok(), "numele nu sunt comparate fara majuscule");
    let data = fs::read_all("/disk/docs/mare.bin").expect("test: mare.bin");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in &data {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let expected = fs::read_all("/disk/docs/mare.fnv").unwrap();
    let hex = alloc::format!("{:016x}\n", h);
    assert!(data.len() == 100 * 1024 && hex.as_bytes() == &expected[..], "mare.bin: {} octeti, suma {}", data.len(), hex.trim());
    let mut part = [0u8; 1000];
    assert!(fs::read("/disk/docs/mare.bin", 50_000, &mut part) == Ok(1000), "citire de la mijloc");
    assert!(part.iter().enumerate().all(|(i, &b)| b == (((50_000 + i) * 7) ^ ((50_000 + i) >> 8)) as u8), "continut gresit la mijloc");
    assert!(fs::write("/disk/salut.txt", 0, b"x") == Err(FsError::ReadOnly), "scriere acceptata pe FAT32");

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
    info!("test: FAT32 OK (GPT si MBR, nume lungi, 100 KiB cu suma {}, doar citire)", hex.trim());
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
