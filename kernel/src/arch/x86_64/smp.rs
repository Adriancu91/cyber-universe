//! Starting the other processors (K6-02, first part): every processor listed in the MADT gets INIT-SIPI-SIPI,
//! runs a small trampoline below 1 MiB (real mode -> protected mode -> long mode), then `ap_entry` in the kernel:
//! the kernel's page tables, CR0/CR4/EFER as on the boot CPU, its own GDT and TSS, the shared IDT, its local APIC,
//! the vector unit, `syscall`, its own block in percpu.rs; then it joins the scheduler (K6-02b) and runs threads
//! from the shared queue. Started only on request (`cpu porneste`), so the default boot does not change.
//! Written from the Intel SDM vol. 3 §8.4 (MP initialization, INIT-SIPI-SIPI), §9.8–9.9 (switching to protected
//! and long mode) and §10.6 (ICR), and the ACPI MADT.

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

core::arch::global_asm!(
    r#"
    .pushsection .text.cu_trampoline, "ax"
    .global cu_tramp_start
    .global cu_tramp_end
    .code16
cu_tramp_start:
    cli
    cld
    xorl %ebx, %ebx
    movw %cs, %bx
    shll $4, %ebx
    movw %cs, %ax
    movw %ax, %ds
    lgdtl (cu_t_gdtr - cu_tramp_start)
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl *(cu_t_pmjmp - cu_tramp_start)
    .code32
cu_t_pm32:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4
    movl (cu_t_cr3 - cu_tramp_start)(%ebx), %eax
    movl %eax, %cr3
    movl $0xC0000080, %ecx
    movl (cu_t_efer - cu_tramp_start)(%ebx), %eax
    xorl %edx, %edx
    wrmsr
    movl %cr0, %eax
    orl $0x80000001, %eax
    movl %eax, %cr0
    ljmpl *(cu_t_lmjmp - cu_tramp_start)(%ebx)
    .code64
cu_t_lm64:
    movl %ebx, %ebx
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movq (cu_t_stack - cu_tramp_start)(%rbx), %rsp
    movq (cu_t_arg - cu_tramp_start)(%rbx), %rdi
    movq (cu_t_entry - cu_tramp_start)(%rbx), %rax
    jmpq *%rax
    .balign 8
cu_t_gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
    .quad 0x00AF9A000000FFFF
cu_t_gdtr:
    .word 31
    .long 0
cu_t_pmjmp:
    .long 0
    .word 0x08
cu_t_lmjmp:
    .long 0
    .word 0x18
    .balign 8
cu_t_cr3:
    .quad 0
cu_t_efer:
    .quad 0
cu_t_stack:
    .quad 0
cu_t_arg:
    .quad 0
cu_t_entry:
    .quad 0
cu_tramp_end:
    .popsection
    "#,
    options(att_syntax)
);

unsafe extern "C" {
    static cu_tramp_start: u8;
    static cu_tramp_end: u8;
    static cu_t_gdt: u8;
    static cu_t_gdtr: u8;
    static cu_t_pmjmp: u8;
    static cu_t_lmjmp: u8;
    static cu_t_pm32: u8;
    static cu_t_lm64: u8;
    static cu_t_cr3: u8;
    static cu_t_efer: u8;
    static cu_t_stack: u8;
    static cu_t_arg: u8;
    static cu_t_entry: u8;
}

/// Start stack of each additional processor (afterwards the stack of its idle thread; threads have their own).
const AP_STACK: usize = 32 * 1024;

/// Physical page below 1 MiB for the trampoline (reserved at boot; 0 = none found).
static TRAMPOLINE: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Processors running (the boot CPU counts as 1).
static ONLINE: AtomicU32 = AtomicU32::new(1);
/// Set by the processor being started, once it runs in the kernel.
static ARRIVED: AtomicU64 = AtomicU64::new(u64::MAX);
static BSP_CR0: AtomicU64 = AtomicU64::new(0);
static BSP_CR4: AtomicU64 = AtomicU64::new(0);

/// Processors running now.
pub fn online() -> u32 {
    ONLINE.load(Ordering::Acquire)
}

/// Takes the page below 1 MiB that the frame allocator kept at start-up for the trampoline.
pub fn reserve_trampoline() {
    match frame::take_low_page() {
        Some(p) => {
            TRAMPOLINE.store(p, Ordering::Relaxed);
            crate::info!("smp: pagina pentru trambulina la {:#x}", p);
        }
        None => crate::warn!("smp: nicio pagina libera sub 1 MiB: celelalte nuclee nu pot porni"),
    }
}

fn offset(sym: *const u8) -> usize {
    // SAFETY: only the addresses of the trampoline symbols are taken, never dereferenced here.
    sym as usize - unsafe { core::ptr::addr_of!(cu_tramp_start) } as usize
}

fn delay_us(us: u64) {
    match crate::time::since_boot_us() {
        Some(start) => {
            while crate::time::since_boot_us().is_some_and(|t| t < start + us) {
                core::hint::spin_loop();
            }
        }
        None => (0..us * 100).for_each(|_| core::hint::spin_loop()),
    }
}

fn read_cr(n: u8) -> u64 {
    let v: u64;
    // SAFETY: reading a control register has no side effects.
    unsafe {
        match n {
            0 => core::arch::asm!("mov {}, cr0", out(reg) v, options(nomem, nostack)),
            _ => core::arch::asm!("mov {}, cr4", out(reg) v, options(nomem, nostack)),
        }
    }
    v
}

/// First kernel code on an additional processor (from the trampoline, on its own stack, with the temporary tables).
/// `index` = the processor's index in percpu.rs (1, 2, ...).
extern "C" fn ap_entry(index: u64) -> ! {
    // SAFETY: the kernel tables map everything this code touches (upper half); CR0/CR4 as on the boot CPU, which
    // runs the same kernel. PAE and paging are already on (set by the trampoline).
    unsafe {
        paging::load(paging::kernel_root());
        core::arch::asm!("mov cr4, {}", in(reg) BSP_CR4.load(Ordering::Relaxed), options(nostack));
        core::arch::asm!("mov cr0, {}", in(reg) BSP_CR0.load(Ordering::Relaxed), options(nostack));
    }
    let tss = super::gdt::init_ap();
    super::idt::load_ap();
    super::syscall::init(); // STAR/LSTAR/FMASK are per-processor MSRs
    super::fpu::init_ap();
    super::apic::enable_ap();
    super::percpu::init(index as usize, tss, super::apic::lapic_id());
    ONLINE.fetch_add(1, Ordering::AcqRel);
    ARRIVED.store(index, Ordering::Release);
    crate::sched::ap_join(index as usize)
}

/// Starts every processor of the MADT that is not running yet. Returns how many processors run afterwards.
pub fn start_all() -> u32 {
    if STARTED.swap(true, Ordering::AcqRel) {
        return online();
    }
    let tramp = TRAMPOLINE.load(Ordering::Relaxed);
    if tramp == 0 {
        crate::warn!("smp: fara pagina de trambulina, raman pe un nucleu");
        return online();
    }
    let madt = crate::acpi::madt::get();
    let me = super::apic::lapic_id();
    BSP_CR0.store(read_cr(0), Ordering::Relaxed);
    BSP_CR4.store(read_cr(4), Ordering::Relaxed);

    // Copy the trampoline and fill in its addresses.
    // SAFETY: symbols of the trampoline section; `tramp` is a reserved page reachable through the HHDM.
    let (start, end) = unsafe { (core::ptr::addr_of!(cu_tramp_start), core::ptr::addr_of!(cu_tramp_end)) };
    let len = end as usize - start as usize;
    assert!(len <= PAGE_SIZE as usize, "smp: trambulina prea mare");
    let page = phys_to_virt(tramp);
    // SAFETY: as above; the page belongs to us.
    unsafe { core::ptr::copy_nonoverlapping(start, page, len) };
    let put32 = |sym: *const u8, extra: usize, v: u32| {
        // SAFETY: inside the copied trampoline (offsets of its own labels).
        unsafe { (page.add(offset(sym) + extra) as *mut u32).write_unaligned(v) }
    };
    let put64 = |sym: *const u8, v: u64| {
        // SAFETY: as above.
        unsafe { (page.add(offset(sym)) as *mut u64).write_unaligned(v) }
    };
    // SAFETY (all addr_of! below): addresses of trampoline labels, not dereferenced.
    unsafe {
        put32(core::ptr::addr_of!(cu_t_gdtr), 2, (tramp as usize + offset(core::ptr::addr_of!(cu_t_gdt))) as u32);
        put32(core::ptr::addr_of!(cu_t_pmjmp), 0, (tramp as usize + offset(core::ptr::addr_of!(cu_t_pm32))) as u32);
        put32(core::ptr::addr_of!(cu_t_lmjmp), 0, (tramp as usize + offset(core::ptr::addr_of!(cu_t_lm64))) as u32);
    }

    // Temporary tables: the kernel's upper half + the trampoline page mapped at its own address. The PML4 must be
    // below 4 GiB (loaded while still in 32-bit mode).
    let Some(temp) = frame::alloc_contiguous_between(1, 0x1000, 1 << 32) else {
        crate::warn!("smp: nicio pagina sub 4 GB pentru tabelele temporare");
        return online();
    };
    // SAFETY: fresh page; the kernel PML4 is reachable through the HHDM; only the upper half is copied.
    unsafe {
        let dst = phys_to_virt(temp) as *mut u64;
        let src = phys_to_virt(paging::kernel_root()) as *const u64;
        for i in 0..512 {
            dst.add(i).write(if i >= 256 { src.add(i).read() } else { 0 });
        }
    }
    if paging::map_in(temp, tramp, tramp, paging::WRITABLE).is_err() {
        crate::warn!("smp: nu pot mapa trambulina");
        return online();
    }
    let efer = {
        let (lo, hi): (u32, u32);
        // SAFETY: EFER exists on every x86-64 CPU.
        unsafe { core::arch::asm!("rdmsr", in("ecx") 0xC000_0080u32, out("eax") lo, out("edx") hi, options(nomem, nostack)) };
        (hi as u64) << 32 | lo as u64
    };
    // SAFETY: label addresses only.
    unsafe {
        put64(core::ptr::addr_of!(cu_t_cr3), temp);
        put64(core::ptr::addr_of!(cu_t_efer), efer & !(1 << 10)); // LMA is set by the CPU, not written
        put64(core::ptr::addr_of!(cu_t_entry), ap_entry as *const () as u64);
    }

    let mut stacks: Vec<alloc::boxed::Box<[u8]>> = Vec::new();
    let begin = crate::time::since_boot_us().unwrap_or(0);
    let mut next_index = 1usize;
    for (k, &id) in madt.cpu_apic_ids[..madt.cpu_count].iter().enumerate() {
        if id == me {
            continue;
        }
        if next_index >= super::percpu::MAX_CPUS {
            crate::warn!("smp: nucleul {} (APIC {}) sarit: cel mult {} nuclee", k, id, super::percpu::MAX_CPUS);
            continue;
        }
        let index = next_index;
        let stack = alloc::vec![0u8; AP_STACK].into_boxed_slice();
        let top = (stack.as_ptr() as u64 + AP_STACK as u64) & !15;
        stacks.push(stack);
        // SAFETY: label addresses only.
        unsafe {
            put64(core::ptr::addr_of!(cu_t_stack), top);
            put64(core::ptr::addr_of!(cu_t_arg), index as u64);
        }
        ARRIVED.store(u64::MAX, Ordering::Release);
        let t0 = crate::time::since_boot_us().unwrap_or(0);
        super::apic::send_ipi(id, 0x0000_4500); // INIT, level assert
        delay_us(10_000);
        let sipi = 0x0000_4600 | (tramp >> 12) as u32; // start-up IPI: begin at tramp (real mode, CS = tramp >> 4)
        super::apic::send_ipi(id, sipi);
        delay_us(200);
        let arrived = |limit_us: u64| {
            let t = crate::time::since_boot_us().unwrap_or(0);
            loop {
                if ARRIVED.load(Ordering::Acquire) == index as u64 {
                    return true;
                }
                if crate::time::since_boot_us().is_none_or(|n| n > t + limit_us) {
                    return false;
                }
                core::hint::spin_loop();
            }
        };
        let ok = arrived(1000) || {
            super::apic::send_ipi(id, sipi); // the second SIPI, as the SDM sequence says
            arrived(200_000)
        };
        let ms = crate::time::since_boot_us().unwrap_or(0).saturating_sub(t0) / 1000;
        if ok {
            next_index += 1;
            crate::info!("smp: nucleul {} (APIC {}) pornit in {} ms", k, id, ms);
        } else {
            crate::warn!("smp: nucleul {} (APIC {}) NU a raspuns in {} ms", k, id, ms);
        }
    }
    core::mem::forget(stacks); // the started processors keep running on them (their idle threads)
    let n = online();
    crate::info!(
        "smp: {} nuclee pornite din {} (in {} ms); planificatorul ruleaza fire pe {} nuclee",
        n,
        madt.cpu_count,
        crate::time::since_boot_us().unwrap_or(0).saturating_sub(begin) / 1000,
        crate::sched::running_cpus()
    );
    n
}
