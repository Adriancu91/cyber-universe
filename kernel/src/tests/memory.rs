//! K6-03: large memory regions for programs. `/bin/memorie` asks for 1 GiB with `mem_map`, writes and checks it, frees
//! it, then a small region and a refused request. Here: the program must exit with 0, use 2 MiB pages, and every frame
//! must come back when it ends (freed regions and the region left for the kernel to free at exit).

use crate::mem::frame;
use crate::sched;

pub fn run() {
    // Warm-up run: lets kernel tables reach their working size, so the comparison below sees only the program.
    let small = crate::proc::spawn_path("/bin/memorie", &["memorie", "4"]).expect("memorie nu porneste");
    assert!(crate::proc::wait(small) == 0, "memorie 4 MiB a esuat");
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    let before = frame::free_frames();
    let t0 = crate::arch::x86_64::apic::ticks();
    let id = crate::proc::spawn_path("/bin/memorie", &["memorie", "1024"]).expect("memorie nu porneste");
    let code = crate::proc::wait(id);
    assert!(code == 0, "memorie a iesit cu {}", code);
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    let after = frame::free_frames();
    assert!(after == before, "memorie: {} cadre libere inainte, {} dupa", before, after);
    crate::info!(
        "test: memorie: 1 GiB alocat, scris, verificat si eliberat in {} ms; cadre libere identice ({})",
        (crate::arch::x86_64::apic::ticks() - t0) * 1000 / crate::arch::x86_64::apic::HZ,
        after
    );
}
