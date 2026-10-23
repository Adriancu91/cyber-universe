//! K6-04: several threads in one program. `/bin/fire` runs 4 threads over one shared 64 MiB array (exact sum), checks
//! `join`, the refused `mem_unmap` while another thread runs and the shared heap. Here, besides its exit code:
//! - a thread that calls `exit(42)` ends the whole program with 42; a thread that faults stops it with -1;
//! - a program whose threads spin forever is stopped from outside (`kill`) with all its threads;
//! - after each, every frame and every thread comes back.

use crate::mem::frame;
use crate::sched;

/// Waits until only the test thread is left (the program's threads are freed after their last switch).
fn settle() {
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
}

fn run_program(args: &[&str]) -> i64 {
    let id = crate::proc::spawn_path("/bin/fire", args).expect("fire nu porneste");
    crate::proc::wait(id)
}

pub fn run() {
    // Warm-up: lets kernel tables reach their working size, so the comparisons below see only the program.
    assert!(run_program(&["fire", "2"]) == 0, "fire 2 a esuat");
    settle();
    let before = frame::free_frames();
    let t0 = crate::arch::x86_64::apic::ticks();
    let code = run_program(&["fire", "4"]);
    let ms = (crate::arch::x86_64::apic::ticks() - t0) * 1000 / crate::arch::x86_64::apic::HZ;
    assert!(code == 0, "fire 4 a iesit cu {}", code);
    settle();
    assert!(frame::free_frames() == before, "fire: {} cadre libere inainte, {} dupa", before, frame::free_frames());
    crate::info!("test: fire: 4 fire intr-un program, suma exacta, {} ms; cadre libere identice", ms);

    let code = run_program(&["fire", "iesire"]);
    assert!(code == 42, "fire iesire: codul {} (asteptat 42)", code);
    settle();
    assert!(frame::free_frames() == before, "fire iesire: cadre pierdute");
    crate::info!("test: fire: exit dintr-un fir opreste tot programul (cod 42)");

    let code = run_program(&["fire", "eroare"]);
    assert!(code == -1, "fire eroare: codul {} (asteptat -1)", code);
    settle();
    assert!(frame::free_frames() == before, "fire eroare: cadre pierdute");
    crate::info!("test: fire: eroarea unui fir opreste tot programul (cod -1)");

    let id = crate::proc::spawn_path("/bin/fire", &["fire", "bucla"]).expect("fire nu porneste");
    // main + 3 spinning threads + this test thread
    let start = crate::arch::x86_64::apic::ticks();
    while sched::alive() < 5 {
        assert!(crate::arch::x86_64::apic::ticks() - start < 500, "fire bucla: firele nu au pornit");
        sched::sleep_ms(10);
    }
    sched::sleep_ms(200);
    assert!(crate::proc::kill(id), "fire bucla: kill refuzat");
    let code = crate::proc::wait(id);
    assert!(code == crate::syscall::EXIT_KILLED, "fire bucla: codul {}", code);
    settle();
    assert!(frame::free_frames() == before, "fire bucla: cadre pierdute");
    crate::info!(
        "test: fire: program cu 4 fire in bucla oprit cu kill, toate firele terminate ({} nuclee)",
        sched::running_cpus()
    );
}
