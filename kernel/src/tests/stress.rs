//! Stability test (P1-11): allocates/frees heap memory and maps/unmaps pages in a loop while the
//! timer interrupts the work, for a duration measured by the kernel's own clock.
//! Once a second it also starts a user process (`/bin/cat` without arguments) and waits for it, so
//! address spaces, ELF loading and process exit are stressed too (P2-11).
//! Every 10 seconds it logs a summary; at the end it checks that no frames and no heap bytes leaked.

use alloc::{boxed::Box, vec::Vec};

use crate::arch::x86_64::apic;
use crate::mem::{self, paging};

/// Test duration in seconds: 5 minutes, or 15 seconds for the quick variant run on every push.
#[cfg(not(feature = "test-stress-quick"))]
const DURATION_S: u64 = 300;
#[cfg(feature = "test-stress-quick")]
const DURATION_S: u64 = 15;

const REPORT_EVERY_S: u64 = 10;
/// Unused kernel address range for the mapping part (separate from the heap and the paging test).
const VIRT_BASE: u64 = 0xFFFF_C000_0010_0000;
const PAGES: u64 = 16;

/// Small deterministic pseudo-random generator (xorshift64), so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// One round of work: heap allocations of random sizes plus mapping, using and unmapping pages.
fn round(rng: &mut Rng) {
    // Heap: a Vec of random length and many small boxes, freed in mixed order.
    let len = (rng.next() % 50_000) as usize + 1;
    let seed = rng.next();
    let v: Vec<u64> = (0..len as u64).map(|i| i ^ seed).collect();
    assert!(v[len - 1] == (len as u64 - 1) ^ seed, "stres: Vec stricat");
    let mut boxes: Vec<Box<[u64; 8]>> = (0..256u64).map(|i| Box::new([i; 8])).collect();
    while !boxes.is_empty() {
        let i = (rng.next() as usize) % boxes.len();
        let b = boxes.swap_remove(i);
        assert!(b[0] == b[7], "stres: Box stricat");
    }
    drop(v);

    // Paging: map fresh frames, write through the new mapping, check through the HHDM, unmap, free.
    let mut frames = [0u64; PAGES as usize];
    for (i, slot) in frames.iter_mut().enumerate() {
        let f = mem::frame::alloc_frame().expect("stres: niciun cadru liber");
        let virt = VIRT_BASE + i as u64 * mem::map::PAGE_SIZE;
        paging::map(virt, f, paging::WRITABLE | paging::NO_EXECUTE).expect("stres: maparea a esuat");
        // SAFETY: `virt` was just mapped to frame `f`, which this test owns.
        unsafe { (virt as *mut u64).write_volatile(seed ^ f) };
        *slot = f;
    }
    for (i, &f) in frames.iter().enumerate() {
        // SAFETY: frame `f` is still owned by this test and reachable through the HHDM.
        let seen = unsafe { (mem::map::phys_to_virt(f) as *const u64).read_volatile() };
        assert!(seen == seed ^ f, "stres: pagina {} are continut gresit", i);
        let virt = VIRT_BASE + i as u64 * mem::map::PAGE_SIZE;
        assert!(paging::unmap(virt) == Some(f), "stres: demapare gresita");
        mem::frame::free_frame(f);
    }
}

/// Starts `/bin/cat` (no arguments: prints nothing, exits with 0) and waits for it.
fn run_process() {
    let id = crate::proc::spawn_path("/bin/cat", &["cat"]).expect("stres: /bin/cat nu porneste");
    let code = crate::proc::wait(id);
    assert!(code == 0, "stres: procesul a iesit cu {}", code);
    // Let the scheduler free the exited thread's stack before memory is compared.
    while crate::sched::alive() > 1 {
        crate::sched::yield_now();
    }
}

/// Runs the stability test; panics (test failure) on any inconsistency or leak.
pub fn run() {
    let mut rng = Rng(0xC1BE_2026_0000_0001);
    // Warm-up: lets the heap, the process tables and the page tables for VIRT_BASE reach their
    // working size once, so the baseline below does not count them as a leak.
    round(&mut rng);
    run_process();
    let frames_start = mem::frame::free_frames();
    let (heap_start, _) = mem::heap::stats();
    let t0 = apic::ticks();
    let end = t0 + DURATION_S * apic::HZ;
    let mut next_report = t0 + REPORT_EVERY_S * apic::HZ;
    let mut rounds: u64 = 0;
    let mut processes: u64 = 0;
    let mut next_process = t0 + apic::HZ;
    let mut last_ticks = t0;
    crate::info!("stres: pornit pentru {} s; cadre libere {}, heap folosit {} B", DURATION_S, frames_start, heap_start);

    loop {
        round(&mut rng);
        rounds += 1;
        if apic::ticks() >= next_process {
            run_process();
            processes += 1;
            next_process += apic::HZ;
        }
        let now = apic::ticks();
        if now >= next_report {
            let (heap_used, heap_mapped) = mem::heap::stats();
            crate::info!(
                "stres: {} s, {} runde, {} procese, {} intreruperi ceas (+{}), cadre libere {}, heap {} B / {} KiB",
                (now - t0) / apic::HZ,
                rounds,
                processes,
                now - t0,
                now - last_ticks,
                mem::frame::free_frames(),
                heap_used,
                heap_mapped / 1024
            );
            last_ticks = now;
            next_report += REPORT_EVERY_S * apic::HZ;
        }
        if now >= end {
            break;
        }
    }

    let frames_end = mem::frame::free_frames();
    let (heap_end, _) = mem::heap::stats();
    assert!(frames_end == frames_start, "stres: scurgere de cadre: {} la inceput, {} la final", frames_start, frames_end);
    assert!(heap_end == heap_start, "stres: scurgere in heap: {} B la inceput, {} B la final", heap_start, heap_end);
    crate::info!("stres: {} runde si {} procese in {} s, cadre si heap identice la final", rounds, processes, DURATION_S);
    crate::serial_println!("STRESS OK");
}
