//! K6-03: a program that asks the kernel for a large memory region (`mem_map`), writes a pattern in all of it, checks
//! it, frees it. `memorie [MiB]` (default 1024 = 1 GiB). Also checks: a small region (4 KiB pages), that a fresh region
//! reads as zeros, that a file can be read straight into a region (system calls accept region addresses), and that
//! an absurd request is refused. Exits with 0 if everything is right.
#![no_std]
#![no_main]

use cyber_user::println;

cyber_user::entry!(main);

/// One u64 per 512 bytes: enough to touch every 4 KiB page many times and to catch two pages mapped to one frame.
const STRIDE: usize = 512 / 8;

fn pattern(i: usize) -> u64 {
    (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xC1BE_0000_0000_0003
}

fn main() -> i64 {
    let mib: usize = cyber_user::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1024);
    let len = mib << 20;

    // Large region.
    let base = match cyber_user::mem_map(len) {
        Ok(p) => p,
        Err(e) => {
            println!("memorie: mem_map({} MiB) refuzat: {}", mib, cyber_user::error_text(e));
            return 1;
        }
    };
    if base as usize % (2 << 20) != 0 {
        println!("memorie: adresa {:#x} nu e aliniata la 2 MiB", base as usize);
        return 2;
    }
    let words = len / 8;
    // SAFETY: the kernel mapped `len` readable and writable bytes at `base`, owned by this program until mem_unmap.
    let mem = unsafe { core::slice::from_raw_parts_mut(base as *mut u64, words) };
    if (0..words).step_by(STRIDE * 7).any(|i| mem[i] != 0) {
        println!("memorie: regiunea noua nu e plina de zero");
        return 3;
    }
    for i in (0..words).step_by(STRIDE) {
        mem[i] = pattern(i);
    }
    let bad = (0..words).step_by(STRIDE).filter(|&i| mem[i] != pattern(i)).count();
    if bad != 0 {
        println!("memorie: {} valori gresite din {}", bad, words / STRIDE);
        return 4;
    }
    println!("memorie: {} MiB la {:#x} scrisi si verificati ({} valori)", mib, base as usize, words / STRIDE);

    // A file read straight into the region (the kernel checks region addresses like any other program memory).
    let fd = cyber_user::open("/etc/motd");
    if fd >= 0 {
        // SAFETY: inside the region mapped above.
        let buf = unsafe { core::slice::from_raw_parts_mut(base.add(len - 4096), 4096) };
        let n = cyber_user::read(fd, buf);
        cyber_user::close(fd);
        if n <= 0 || !buf[..n as usize].starts_with(b"Bun venit") {
            println!("memorie: citirea in regiune a esuat ({})", n);
            return 5;
        }
    }
    if cyber_user::mem_unmap(base, len) != 0 {
        println!("memorie: mem_unmap a esuat");
        return 6;
    }
    if cyber_user::mem_unmap(base, len) == 0 {
        println!("memorie: a doua eliberare a fost acceptata");
        return 7;
    }

    // Small region (not a multiple of 2 MiB: 4 KiB pages), then a refused request.
    let small = match cyber_user::mem_map(3 * 4096 + 100) {
        Ok(p) => p,
        Err(e) => {
            println!("memorie: regiunea mica refuzata: {}", cyber_user::error_text(e));
            return 8;
        }
    };
    // SAFETY: 4 pages (3 * 4096 + 100 rounded up) mapped at `small`.
    unsafe {
        small.add(4 * 4096 - 1).write_volatile(0x5A);
        if small.add(4 * 4096 - 1).read_volatile() != 0x5A {
            return 9;
        }
    }
    if cyber_user::mem_map(1 << 40) != Err(-1) || cyber_user::mem_map(60 << 30) != Err(-22) {
        println!("memorie: o cerere imposibila nu a fost refuzata");
        return 10;
    }
    println!("memorie: regiune mica OK, cererea imposibila refuzata (memorie insuficienta)");
    0 // the small region is freed by the kernel when the program ends
}
