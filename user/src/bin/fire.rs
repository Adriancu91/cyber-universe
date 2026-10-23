//! K6-04: a program with several threads in one address space.
//! `fire [n]` (default 4): n threads add up equal parts of one shared 64 MiB array (8 Mi numbers), several passes;
//! the total must be exact. Also checks: `mem_unmap` is refused while another thread runs (-23) and accepted after
//! `join`; a thread cannot be joined twice; the shared heap stays consistent when every thread allocates and frees
//! at the same time. Exits with 0 if everything is right.
//! Modes for the kernel test: `fire iesire` (a thread calls `exit(42)`: the whole program ends with 42),
//! `fire eroare` (a thread reads address 0: the whole program stops with -1), `fire bucla` (threads that spin forever,
//! stopped from outside with `kill`).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};
use cyber_user::{println, thread};

cyber_user::entry!(main);
cyber_user::heap!(1024 * 1024);

const WORDS: usize = 8 << 20; // 8 Mi u64 = 64 MiB
const PASSES: u64 = 4;
const MAX: usize = 16;

static DATA: AtomicU64 = AtomicU64::new(0);
static PARTS: AtomicU64 = AtomicU64::new(0);
/// Each thread writes its own sum here too (memory shared with the main thread).
static SUMS: [AtomicU64; MAX] = [const { AtomicU64::new(0) }; MAX];
static STARTED: AtomicU64 = AtomicU64::new(0);

/// Adds up part `i` of the array, PASSES times; returns the sum of one pass (it fits in i64).
extern "C" fn worker(i: u64) -> i64 {
    STARTED.fetch_add(1, Ordering::SeqCst);
    let parts = PARTS.load(Ordering::SeqCst) as usize;
    let per = WORDS / parts;
    // SAFETY: DATA is the mem_map region of WORDS u64, filled before the threads start and only read now.
    let data = unsafe { core::slice::from_raw_parts(DATA.load(Ordering::SeqCst) as *const u64, WORDS) };
    let part = &data[i as usize * per..(i as usize + 1) * per];
    let mut total = 0u64;
    for _ in 0..PASSES {
        total = total.wrapping_add(part.iter().fold(0u64, |a, &x| a.wrapping_add(x)));
    }
    let one = total / PASSES;
    SUMS[i as usize].store(one, Ordering::SeqCst);
    one as i64
}

/// Allocates and frees on the shared heap; returns how many blocks were checked.
extern "C" fn heap_worker(seed: u64) -> i64 {
    let mut n = 0;
    for round in 0..200u64 {
        let mut v: Vec<Box<[u64; 4]>> = Vec::new();
        for k in 0..20u64 {
            let x = seed * 1_000_000 + round * 100 + k;
            v.push(Box::new([x, x + 1, x + 2, x + 3]));
        }
        for (k, b) in v.iter().enumerate() {
            let x = seed * 1_000_000 + round * 100 + k as u64;
            if **b != [x, x + 1, x + 2, x + 3] {
                return -1;
            }
            n += 1;
        }
    }
    n
}

extern "C" fn sleeper(ms: u64) -> i64 {
    cyber_user::sleep_ms(ms);
    7
}

extern "C" fn exits(_: u64) -> i64 {
    cyber_user::sleep_ms(50);
    cyber_user::exit(42)
}

extern "C" fn faults(_: u64) -> i64 {
    cyber_user::sleep_ms(50);
    // SAFETY: intentionally wrong: address 0 is never mapped; the kernel stops the whole program.
    unsafe { core::ptr::read_volatile(core::ptr::null::<u64>()) as i64 }
}

extern "C" fn spins(_: u64) -> i64 {
    STARTED.fetch_add(1, Ordering::SeqCst);
    let mut x = 0u64;
    loop {
        x = core::hint::black_box(x.wrapping_add(1));
    }
}

fn spawn(f: extern "C" fn(u64) -> i64, arg: u64) -> i64 {
    let (id, _) = thread::spawn_with_stack(f, arg);
    if id < 0 {
        println!("fire: thread_spawn a esuat: {}", cyber_user::error_text(id));
        cyber_user::exit(20);
    }
    id
}

fn main() -> i64 {
    match cyber_user::args().nth(1) {
        Some("iesire") => {
            let id = spawn(exits, 0);
            thread::join(id); // the program ends while waiting here
            println!("fire: join s-a intors, desi firul a cerut iesirea programului");
            return 1;
        }
        Some("eroare") => {
            let id = spawn(faults, 0);
            thread::join(id);
            println!("fire: join s-a intors dupa eroarea firului");
            return 1;
        }
        Some("bucla") => {
            for _ in 0..3 {
                spawn(spins, 0);
            }
            while STARTED.load(Ordering::SeqCst) < 3 {
                cyber_user::yield_now();
            }
            println!("fire: 3 fire pornite, in bucla");
            spins(0);
            return 1;
        }
        _ => {}
    }
    let n: usize = cyber_user::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(4).clamp(1, MAX);

    // 1. Shared array, n threads, exact total.
    let base = match cyber_user::mem_map(WORDS * 8) {
        Ok(p) => p as *mut u64,
        Err(e) => {
            println!("fire: mem_map refuzat: {}", cyber_user::error_text(e));
            return 2;
        }
    };
    // SAFETY: a fresh region of WORDS u64, owned by this program.
    let data = unsafe { core::slice::from_raw_parts_mut(base, WORDS) };
    for (i, x) in data.iter_mut().enumerate() {
        *x = i as u64;
    }
    DATA.store(base as u64, Ordering::SeqCst);
    PARTS.store(n as u64, Ordering::SeqCst);
    let ids: Vec<i64> = (0..n as u64).map(|i| spawn(worker, i)).collect();
    let mut total = 0u64;
    for (i, &id) in ids.iter().enumerate() {
        let r = thread::join(id);
        if r < 0 || r as u64 != SUMS[i].load(Ordering::SeqCst) {
            println!("fire: firul {} a dat {} (in memorie {})", i, r, SUMS[i].load(Ordering::SeqCst));
            return 3;
        }
        total += r as u64;
    }
    let expected = (WORDS as u64) * (WORDS as u64 - 1) / 2;
    if total != expected {
        println!("fire: suma {} in loc de {}", total, expected);
        return 4;
    }
    println!("fire: {} fire x {} treceri peste 64 MiB comuni, suma exacta {}", n, PASSES, total);

    // 2. Joining twice, or a thread that does not exist.
    if thread::join(ids[0]) != cyber_user::E_NOT_FOUND || thread::join(999_999) != cyber_user::E_NOT_FOUND {
        println!("fire: join dublu sau pe un fir inexistent acceptat");
        return 5;
    }

    // 3. mem_unmap waits for the other threads.
    let s = spawn(sleeper, 300);
    let busy = cyber_user::mem_unmap(base as *mut u8, WORDS * 8);
    let slept = thread::join(s);
    let freed = cyber_user::mem_unmap(base as *mut u8, WORDS * 8);
    if busy != -23 || slept != 7 || freed != 0 {
        println!("fire: mem_unmap cu fire: {} (asteptat -23), join {}, dupa join {}", busy, slept, freed);
        return 6;
    }
    println!("fire: mem_unmap refuzat cat timp ruleaza alt fir, acceptat dupa join");

    // 4. The heap shared by threads that allocate at the same time.
    let before = HEAP.free();
    let hs: Vec<i64> = (1..=n as u64).map(|k| spawn(heap_worker, k)).collect();
    let mut blocks = 0;
    for id in hs {
        let r = thread::join(id);
        if r < 0 {
            println!("fire: heap comun stricat");
            return 7;
        }
        blocks += r;
    }
    let after = HEAP.free();
    if before != after {
        println!("fire: heap: {} octeti liberi inainte, {} dupa", before, after);
        return 8;
    }
    println!("fire: heap comun: {} fire, {} blocuri verificate, memoria libera revine exact", n, blocks);
    0
}
