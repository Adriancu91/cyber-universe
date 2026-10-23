//! P6-19 (C-16): `/bin/kronos-calcul` — Kronos' operators from Cyber-GPU's `gpu-compute` (CPU backend) on every
//! processor that runs threads, compared with the library's reference (declared tolerances in its output).
//! Here: the exit code, and that every frame comes back.

use crate::mem::frame;
use crate::sched;

pub fn run() {
    let cpus = sched::running_cpus();
    let before = frame::free_frames();
    let t0 = crate::arch::x86_64::apic::ticks();
    let id = crate::proc::spawn_path("/bin/kronos-calcul", &["kronos-calcul"]).expect("kronos-calcul nu porneste");
    let code = crate::proc::wait(id);
    let ms = (crate::arch::x86_64::apic::ticks() - t0) * 1000 / crate::arch::x86_64::apic::HZ;
    assert!(code == 0, "kronos-calcul a iesit cu {}", code);
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    assert!(
        frame::free_frames() == before,
        "kronos-calcul: {} cadre libere inainte, {} dupa",
        before,
        frame::free_frames()
    );
    crate::info!(
        "test: kronos-calcul: gpu-compute pe {} {}, in toleranta, {} ms; cadre libere identice",
        cpus,
        if cpus == 1 { "nucleu" } else { "nuclee" },
        ms
    );
}
