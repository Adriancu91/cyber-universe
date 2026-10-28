//! P6-27 (C-25): `/bin/kronos` — our KGR1 engine on the tiny random model of `tools/kgr-mini.py` (on the test disk,
//! /disk/kronos): F32 and Q8_0 against their float64 references (logits within the declared tolerance, the greedy
//! continuation identical), once with vectors (when the processor has AVX2 + FMA) and once scalar; a wrong SHA-256
//! must be refused (exit code 3). Every frame must come back.

use crate::mem::frame;
use crate::sched;

fn run_one(args: &[&str]) -> i64 {
    let id = crate::proc::spawn_path("/bin/kronos", args).expect("kronos nu porneste");
    crate::proc::wait(id)
}

pub fn run() {
    let before = frame::free_frames();
    let t0 = crate::arch::x86_64::apic::ticks();
    let f32 = ["kronos", "--verifica", "/disk/kronos/mini-f32.kgr", "/disk/kronos/mini-f32.ref"];
    let q8 = ["kronos", "--verifica", "/disk/kronos/mini-q8.kgr", "/disk/kronos/mini-q8.ref"];
    assert!(run_one(&f32) == 0, "kronos: F32 diferit de referinta");
    assert!(run_one(&q8) == 0, "kronos: Q8_0 diferit de referinta");
    let mut scalar = [""; 5];
    scalar[..4].copy_from_slice(&q8);
    scalar[4] = "--fara-vector";
    assert!(run_one(&scalar) == 0, "kronos: Q8_0 scalar diferit de referinta");
    let zero = "0000000000000000000000000000000000000000000000000000000000000000";
    let wrong = ["kronos", "--model", "/disk/kronos/mini-q8.kgr", "--suma", zero, "--simboluri", "1,2"];
    let code = run_one(&wrong);
    assert!(code == 3, "kronos: suma gresita acceptata (cod {})", code);
    let generate = ["kronos", "--model", "/disk/kronos/mini-q8.kgr", "--simboluri", "1,2,3,4,5", "--n", "8"];
    assert!(run_one(&generate) == 0, "kronos: generarea a esuat");
    let ms = (crate::arch::x86_64::apic::ticks() - t0) * 1000 / crate::arch::x86_64::apic::HZ;
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    assert!(frame::free_frames() == before, "kronos: {} cadre libere inainte, {} dupa", before, frame::free_frames());
    crate::info!(
        "test: kronos: motorul KGR1 = referinta (F32, Q8_0, vectori si scalar), suma gresita refuzata, {} ms, pe {} nuclee; cadre libere identice",
        ms,
        sched::running_cpus()
    );
}
