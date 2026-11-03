//! P6-27 (C-25): `/bin/kronos` — our KGR1 engine on the tiny random model of `tools/kgr-mini.py` (on the test disk,
//! /disk/kronos): F32 and Q8_0 against their float64 references (logits within the declared tolerance, the greedy
//! continuation identical), once with vectors (when the processor has AVX2 + FMA) and once scalar; a wrong SHA-256
//! must be refused (exit code 3). P6-27b: the official Kronos vectors (hub `integrare/kronos/vectori/`, copied to
//! tests/kronos): tokenizer, logits, argmax and greedy for F32 and Q8_0 (vectors and scalar), the tokenizer examples of
//! the specification, and `kronos <text>`. Every frame must come back.

use crate::mem::frame;
use crate::sched;

fn run_one(args: &[&str]) -> i64 {
    let id = crate::proc::spawn_path("/bin/kronos", args).expect("kronos nu porneste");
    crate::proc::wait(id)
}

pub fn run() {
    crate::gfx::console::self_test_block_scroll();
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
    // P6-27b: the official vectors, with the real w2 tokenizer inside the tiny random model.
    let vf32 = ["kronos", "--vectori", "/disk/kronos/vectori.json", "f32"];
    assert!(run_one(&vf32) == 0, "kronos: vectorii oficiali F32 nu se potrivesc");
    let vq8 = ["kronos", "--vectori", "/disk/kronos/vectori.json", "q8_0"];
    assert!(run_one(&vq8) == 0, "kronos: vectorii oficiali Q8_0 nu se potrivesc");
    let vq8s = ["kronos", "--vectori", "/disk/kronos/vectori.json", "q8_0", "--fara-vector"];
    assert!(run_one(&vq8s) == 0, "kronos: vectorii oficiali Q8_0 scalar nu se potrivesc");
    for text in [&["<|endoftext|>Ana"][..], &["991", "07"], &["x", "=", "1234;"]] {
        let mut a = alloc::vec!["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--codeaza"];
        a.extend_from_slice(text);
        assert!(run_one(&a) == 0, "kronos: codarea {:?} nu se decodeaza inapoi", text);
    }
    let words = ["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--n", "5", "--profil", "Ana", "are", "mere."];
    assert!(run_one(&words) == 0, "kronos: kronos <text> a esuat");
    // P6-29: the thread pool with more threads than processors (workers yield) must still match the reference.
    let pool = ["kronos", "--verifica", "/disk/kronos/mini-q8.kgr", "/disk/kronos/mini-q8.ref", "--fire", "4"];
    assert!(run_one(&pool) == 0, "kronos: Q8_0 pe 4 fire diferit de referinta");
    // P6-43 (C-59): the pool alone, the number of parts changing at every job (workers without a part are not waited
    // for, and must still take the next job that has one for them).
    let pool_test = ["kronos", "--test-fire", "6"];
    assert!(run_one(&pool_test) == 0, "kronos: testul grupului de fire a esuat");
    // P6-45 (C-59, „bucățele”): the pool in pieces mode (every element once, any thread), then the same tokens as the
    // reference with 8 pieces per thread on 4 threads, and the control texts.
    let pool_pieces = ["kronos", "--test-fire", "6", "--bucati", "8"];
    assert!(run_one(&pool_pieces) == 0, "kronos: testul grupului de fire cu bucati a esuat");
    let pieces = ["kronos", "--verifica", "/disk/kronos/mini-q8.kgr", "/disk/kronos/mini-q8.ref", "--fire", "4", "--bucati", "8"];
    assert!(run_one(&pieces) == 0, "kronos: Q8_0 cu bucati diferit de referinta");
    let control_pieces = ["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--control", "--fire", "3", "--bucati", "4"];
    assert!(run_one(&control_pieces) == 0, "kronos: --control cu bucati nu trece");
    // P6-39: the projection on its own thread count (1 of 4, and 3 = not a divisor of the heads): the same tokens.
    for n in ["1", "3"] {
        let proj = ["kronos", "--verifica", "/disk/kronos/mini-q8.kgr", "/disk/kronos/mini-q8.ref", "--fire", "4", "--fire-proiectie", n];
        assert!(run_one(&proj) == 0, "kronos: Q8_0 cu proiectia pe alt numar de fire diferit de referinta");
    }
    // P6-36: text only at the end, 3 threads (more than the tiny model's processors here): the same tokens.
    let quiet = ["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--n", "5", "--fara-ecran", "--fire", "3", "Ana", "are", "mere."];
    assert!(run_one(&quiet) == 0, "kronos: --fara-ecran a esuat");
    let penalty = ["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--n", "5", "--repetitie", "1.5", "Ana", "are", "mere."];
    assert!(run_one(&penalty) == 0, "kronos: --repetitie a esuat");
    // P6-38 (C-46): the control texts of a known model (here the tiny model of the official vectors) must pass; an
    // unknown model has none (exit 1). The default choice: #477 before #459 before any other .kgr, and the built-in sum
    // of the chosen name must refuse a file that is not that model (exit 3).
    let control = ["kronos", "--model", "/disk/kronos/mic_q8_0.kgr", "--control"];
    assert!(run_one(&control) == 0, "kronos: --control pe modelul mic nu trece");
    let unknown = ["kronos", "--model", "/disk/kronos/mini-q8.kgr", "--control"];
    let code = run_one(&unknown);
    assert!(code == 1, "kronos: --control pe un model necunoscut a dat codul {}", code);
    for dir in ["/disk/kronos-477", "/disk/kronos-459"] {
        let pick = ["kronos", "--dosar", dir, "--simboluri", "1,2"];
        let code = run_one(&pick);
        assert!(code == 3, "kronos: {}: copia modelului mic cu numele modelului mare acceptata (cod {})", dir, code);
    }
    let ms = (crate::arch::x86_64::apic::ticks() - t0) * 1000 / crate::arch::x86_64::apic::HZ;
    while sched::alive() > 1 {
        sched::sleep_ms(10);
    }
    assert!(frame::free_frames() == before, "kronos: {} cadre libere inainte, {} dupa", before, frame::free_frames());
    crate::info!(
        "test: kronos: motorul KGR1 = referinta (F32, Q8_0, vectori si scalar), suma gresita refuzata, vectorii oficiali Kronos si tokenizerul trec, control si alegerea modelului (477 > 459) OK, {} ms, pe {} nuclee; cadre libere identice",
        ms,
        sched::running_cpus()
    );
}
