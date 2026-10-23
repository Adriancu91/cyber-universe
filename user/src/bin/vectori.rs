//! K6-01: a program that computes with vector registers: the sum 1 + 2 + … + 1000 in 8 lanes with AVX2
//! (or 4 lanes with SSE2 when the CPU or the kernel does not offer AVX2). Exits with 0 if the sum is right.
#![no_std]
#![no_main]

use core::arch::x86_64::{__cpuid, __cpuid_count};
use cyber_user::println;

cyber_user::entry!(main);

/// AVX2 needs: the CPU (CPUID 7 EBX bit 5, CPUID 1 AVX + OSXSAVE) and the kernel (XCR0 saves SSE and AVX state).
fn avx2() -> bool {
    // SAFETY: CPUID is available on every x86-64 CPU.
    let l1 = unsafe { __cpuid(1) };
    if l1.ecx & (1 << 27) == 0 || l1.ecx & (1 << 28) == 0 {
        return false; // no OSXSAVE (kernel) or no AVX
    }
    let xcr0: u32;
    // SAFETY: XGETBV is allowed in ring 3 once CR4.OSXSAVE is set (checked above).
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") xcr0, out("edx") _, options(nomem, nostack)) };
    // SAFETY: as above.
    xcr0 & 0b110 == 0b110 && unsafe { __cpuid(0) }.eax >= 7 && unsafe { __cpuid_count(7, 0) }.ebx & (1 << 5) != 0
}

fn main() -> i64 {
    let mut lanes = [0u32; 8];
    let start: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    let kind = if avx2() {
        // SAFETY: AVX2 checked above; reads `start`, writes `lanes` (8 × u32 each).
        unsafe {
            core::arch::asm!(
                "vpxor ymm0, ymm0, ymm0",
                "vmovdqu ymm1, [{s}]",
                "mov eax, 8",
                "vmovd xmm2, eax",
                "vpbroadcastd ymm2, xmm2",
                "2:",
                "vpaddd ymm0, ymm0, ymm1",
                "vpaddd ymm1, ymm1, ymm2",
                "dec ecx",
                "jnz 2b",
                "vmovdqu [{o}], ymm0",
                "vzeroupper",
                s = in(reg) start.as_ptr(), o = in(reg) lanes.as_mut_ptr(),
                inout("ecx") 125 => _, out("eax") _,
                options(nostack)
            )
        }
        "AVX2 (8 benzi)"
    } else {
        // SAFETY: SSE2 is part of x86-64; reads `start`, writes the first 4 lanes.
        unsafe {
            core::arch::asm!(
                "pxor xmm0, xmm0",
                "movdqu xmm1, [{s}]",
                "mov eax, 4",
                "movd xmm2, eax",
                "pshufd xmm2, xmm2, 0",
                "2:",
                "paddd xmm0, xmm1",
                "paddd xmm1, xmm2",
                "dec ecx",
                "jnz 2b",
                "movdqu [{o}], xmm0",
                s = in(reg) start.as_ptr(), o = in(reg) lanes.as_mut_ptr(),
                inout("ecx") 250 => _, out("eax") _,
                options(nostack)
            )
        }
        "SSE2 (4 benzi)"
    };
    let sum: u32 = lanes.iter().sum();
    println!("vectori: {} suma 1..1000 = {}", kind, sum);
    if sum == 500_500 { 0 } else { 1 }
}
