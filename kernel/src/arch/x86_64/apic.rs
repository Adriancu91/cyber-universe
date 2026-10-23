//! Interrupt controllers: legacy PIC (disabled), local APIC (timer, end-of-interrupt) and I/O APIC
//! (routes device IRQs such as the keyboard to CPU vectors).
//! Written from Intel SDM Vol. 3A ch. 11, the 82093AA I/O APIC datasheet and the OSDev Wiki descriptions.

use super::{inb, outb};
use crate::mem::paging;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// CPU vector of the system timer.
pub const TIMER_VECTOR: u8 = 32;
/// First CPU vector used for device IRQs routed through the I/O APIC (IRQ n -> vector 48 + n).
pub const IRQ_BASE: u8 = 48;
pub const SPURIOUS_VECTOR: u8 = 0xFF;
/// Inter-processor interrupt: "remove the page SHOOT_ADDR from your TLB" (K6-02b).
pub const SHOOTDOWN_VECTOR: u8 = 0xF0;
/// Timer frequency.
pub const HZ: u64 = 100;

const LAPIC_EOI: usize = 0xB0;
const LAPIC_SVR: usize = 0xF0;
const LAPIC_LVT_TIMER: usize = 0x320;
const LAPIC_TIMER_INIT: usize = 0x380;
const LAPIC_TIMER_CURRENT: usize = 0x390;
const LAPIC_TIMER_DIVIDE: usize = 0x3E0;

static LAPIC_BASE: AtomicU64 = AtomicU64::new(0);
static IOAPIC_BASE: AtomicU64 = AtomicU64::new(0);
static IOAPIC_GSI_BASE: AtomicU64 = AtomicU64::new(0);
static TICKS: AtomicU64 = AtomicU64::new(0);
/// APIC id of the boot processor (device interrupts go there).
static BSP_APIC: AtomicU64 = AtomicU64::new(0);
/// APIC timer count for one tick (measured on the boot processor; the others use the same bus clock).
static PER_TICK: AtomicU64 = AtomicU64::new(0);

// TLB shootdown (K6-02b). The processors that run threads are marked in ACTIVE; the one that removes a kernel page
// sets PENDING for each of the others and sends them an IPI; each one invalidates SHOOT_ADDR and clears its flag
// (from the IPI handler, or from any spinlock wait loop if its interrupts are off).
use super::percpu::MAX_CPUS;
#[allow(clippy::declare_interior_mutable_const)]
const NOT_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE: [AtomicBool; MAX_CPUS] = [NOT_PENDING; MAX_CPUS];
static PENDING: [AtomicBool; MAX_CPUS] = [NOT_PENDING; MAX_CPUS];
/// Processors marked in ACTIVE besides the boot processor (0 = nothing to shoot down: fast path).
static OTHERS: AtomicU64 = AtomicU64::new(0);
static SHOOT_LOCK: AtomicBool = AtomicBool::new(false);
static SHOOT_ADDR: AtomicU64 = AtomicU64::new(0);
/// Shootdowns done since boot (for the tests).
static SHOOTDOWNS: AtomicU64 = AtomicU64::new(0);

/// Maps a memory-mapped device page (uncached) and returns its virtual address.
fn map_mmio(phys: u64) -> u64 {
    paging::map_mmio(phys, 4096).expect("apic: maparea registrelor a esuat")
}

fn lapic_read(reg: usize) -> u32 {
    // SAFETY: LAPIC_BASE points at the mapped local APIC register page; registers are 32-bit aligned.
    unsafe { ((LAPIC_BASE.load(Ordering::Relaxed) as usize + reg) as *const u32).read_volatile() }
}

fn lapic_write(reg: usize, value: u32) {
    // SAFETY: as in `lapic_read`.
    unsafe { ((LAPIC_BASE.load(Ordering::Relaxed) as usize + reg) as *mut u32).write_volatile(value) }
}

/// APIC id of the processor running this code.
pub fn lapic_id() -> u8 {
    (lapic_read(0x20) >> 24) as u8
}

/// Sends an inter-processor interrupt: `low` = ICR bits 0–31 (vector, delivery mode, level), to `apic_id`.
/// Waits until the local APIC has sent it (delivery status, bit 12).
pub fn send_ipi(apic_id: u8, low: u32) {
    lapic_write(0x310, (apic_id as u32) << 24);
    lapic_write(0x300, low);
    for _ in 0..1_000_000 {
        if lapic_read(0x300) & (1 << 12) == 0 {
            break;
        }
        core::hint::spin_loop();
    }
}

/// Switches on the local APIC of an additional processor (same spurious vector as the boot CPU; no timer yet).
pub fn enable_ap() {
    lapic_write(LAPIC_SVR, 0x100 | SPURIOUS_VECTOR as u32);
}

/// Starts this (additional) processor's timer at `HZ`, with the boot processor's calibration (K6-02b).
pub fn start_timer_ap() {
    lapic_write(LAPIC_TIMER_DIVIDE, 0x3);
    lapic_write(LAPIC_LVT_TIMER, TIMER_VECTOR as u32 | (1 << 17));
    lapic_write(LAPIC_TIMER_INIT, PER_TICK.load(Ordering::Relaxed) as u32);
}

/// Marks processor `index` as one that runs threads: from now on it takes part in TLB shootdowns.
pub fn mark_active(index: usize) {
    ACTIVE[index].store(true, Ordering::Release);
    if index != 0 {
        OTHERS.fetch_add(1, Ordering::AcqRel);
    }
}

/// Answers a pending shootdown request for this processor, if there is one. Cheap when only one processor runs.
#[inline]
pub fn serve_shootdown() {
    if OTHERS.load(Ordering::Relaxed) == 0 {
        return;
    }
    let me = super::percpu::index();
    if PENDING[me].load(Ordering::Acquire) {
        let virt = SHOOT_ADDR.load(Ordering::Acquire);
        // SAFETY: invalidating a TLB entry is always safe.
        unsafe { core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags)) }
        PENDING[me].store(false, Ordering::Release);
    }
}

/// Removes the kernel page `virt` (already unmapped here) from the TLBs of every other processor that runs threads,
/// and waits until each has done it.
pub fn shootdown(virt: u64) {
    if OTHERS.load(Ordering::Acquire) == 0 {
        return;
    }
    let irq = crate::sched::spin::lock(&SHOOT_LOCK);
    let me = super::percpu::index();
    SHOOT_ADDR.store(virt, Ordering::Release);
    for i in 0..MAX_CPUS {
        if i != me && ACTIVE[i].load(Ordering::Acquire) {
            PENDING[i].store(true, Ordering::Release);
            send_ipi(super::percpu::get(i).apic_id.load(Ordering::Relaxed) as u8, SHOOTDOWN_VECTOR as u32);
        }
    }
    let start = crate::time::since_boot_us().unwrap_or(0);
    let mut late = false;
    while (0..MAX_CPUS).any(|i| PENDING[i].load(Ordering::Acquire)) {
        core::hint::spin_loop();
        if crate::time::since_boot_us().is_some_and(|t| t > start + 1_000_000) {
            late = true;
            break;
        }
    }
    SHOOTDOWNS.fetch_add(1, Ordering::Relaxed);
    crate::sched::spin::unlock(&SHOOT_LOCK, irq);
    if late {
        crate::warn!("apic: TLB shootdown {:#x}: un nucleu nu a raspuns in 1 s", virt);
    }
}

/// TLB shootdowns since boot.
#[allow(dead_code)] // used by the SMP test
pub fn shootdowns() -> u64 {
    SHOOTDOWNS.load(Ordering::Relaxed)
}

extern "x86-interrupt" fn shootdown_handler(_frame: super::idt::InterruptStackFrame) {
    serve_shootdown();
    eoi();
}

/// Signals end-of-interrupt to the local APIC. Every handler of an APIC-delivered interrupt must call it.
pub fn eoi() {
    lapic_write(LAPIC_EOI, 0);
}

/// Remaps the legacy 8259 PICs away from the exception vectors and masks every line.
fn disable_pic() {
    // SAFETY: standard 8259 initialisation sequence on its fixed ports.
    unsafe {
        outb(0x20, 0x11);
        outb(0xA0, 0x11);
        outb(0x21, 0x20);
        outb(0xA1, 0x28);
        outb(0x21, 0x04);
        outb(0xA1, 0x02);
        outb(0x21, 0x01);
        outb(0xA1, 0x01);
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
    }
}

/// Waits `ms` milliseconds by polling PIT channel 2 (works without interrupts). Max ~54 ms per call.
fn pit_wait_ms(ms: u64) {
    let count = (1_193_182 * ms / 1000) as u16;
    // SAFETY: PIT channel 2 and the speaker gate (port 0x61) are standard PC hardware; the speaker stays off.
    unsafe {
        let gate = inb(0x61);
        outb(0x61, (gate & !0x02) | 0x01); // gate on, speaker off
        outb(0x43, 0b1011_0000); // channel 2, lobyte/hibyte, mode 0 (interrupt on terminal count)
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);
        let g = inb(0x61); // restart counting by toggling the gate
        outb(0x61, g & !0x01);
        outb(0x61, g | 0x01);
        while inb(0x61) & 0x20 == 0 {
            core::hint::spin_loop();
        }
    }
}

extern "x86-interrupt" fn timer_handler(frame: super::idt::InterruptStackFrame) {
    // Every processor has its own timer; only the boot processor's counts the time.
    if super::percpu::index() == 0 {
        TICKS.fetch_add(1, Ordering::Relaxed);
    }
    eoi();
    if frame.cs & 3 == 3 && crate::sched::killed() {
        // A user program asked to stop (`proc::kill`) that never makes system calls.
        crate::syscall::exit_current(crate::syscall::EXIT_KILLED);
    }
    // The interrupted thread's registers are saved on its stack by this handler; switching here and
    // returning later through `iretq` resumes it exactly where it was.
    crate::sched::timer_tick();
}

extern "x86-interrupt" fn spurious_handler(_frame: super::idt::InterruptStackFrame) {
    // Spurious interrupts must not be acknowledged with EOI.
}

fn ioapic_write(reg: u32, value: u32) {
    let base = IOAPIC_BASE.load(Ordering::Relaxed) as usize;
    // SAFETY: IOAPIC_BASE points at the mapped I/O APIC (IOREGSEL at +0, IOWIN at +0x10).
    unsafe {
        (base as *mut u32).write_volatile(reg);
        ((base + 0x10) as *mut u32).write_volatile(value);
    }
}

fn ioapic_read(reg: u32) -> u32 {
    let base = IOAPIC_BASE.load(Ordering::Relaxed) as usize;
    // SAFETY: as in `ioapic_write`.
    unsafe {
        (base as *mut u32).write_volatile(reg);
        ((base + 0x10) as *const u32).read_volatile()
    }
}

/// Routes legacy ISA IRQ `irq` to CPU vector `IRQ_BASE + irq` on the boot CPU, honouring MADT overrides.
pub fn route_irq(irq: u8) {
    let madt = crate::acpi::madt::get();
    let mut gsi = irq as u32;
    let mut flags = 0u16;
    for o in &madt.overrides[..madt.override_count] {
        if o.source == irq {
            gsi = o.gsi;
            flags = o.flags;
        }
    }
    let pin = gsi - IOAPIC_GSI_BASE.load(Ordering::Relaxed) as u32;
    let mut low = (IRQ_BASE + irq) as u32;
    if flags & 0b11 == 0b11 {
        low |= 1 << 13; // active low
    }
    if (flags >> 2) & 0b11 == 0b11 {
        low |= 1 << 15; // level triggered
    }
    let dest = (BSP_APIC.load(Ordering::Relaxed) as u32) << 24;
    ioapic_write(0x10 + pin * 2 + 1, dest);
    ioapic_write(0x10 + pin * 2, low);
}

/// Disables the PIC, enables the local APIC timer at `HZ` and prepares the I/O APIC (all lines masked).
/// Interrupts are enabled at the end.
pub fn init() -> bool {
    let madt = crate::acpi::madt::get();
    if madt.local_apic == 0 || madt.ioapic_count == 0 {
        crate::error!("apic: MADT nu descrie APIC-urile, nu pot porni ceasul");
        return false;
    }
    disable_pic();

    LAPIC_BASE.store(map_mmio(madt.local_apic), Ordering::Relaxed);
    IOAPIC_BASE.store(map_mmio(madt.ioapics[0].address), Ordering::Relaxed);
    IOAPIC_GSI_BASE.store(madt.ioapics[0].gsi_base as u64, Ordering::Relaxed);

    super::idt::set_handler(TIMER_VECTOR, timer_handler as super::idt::Handler as u64);
    super::idt::set_handler(SPURIOUS_VECTOR, spurious_handler as super::idt::Handler as u64);
    super::idt::set_handler(SHOOTDOWN_VECTOR, shootdown_handler as super::idt::Handler as u64);
    BSP_APIC.store(lapic_id() as u64, Ordering::Relaxed);
    super::percpu::get(0).apic_id.store(lapic_id() as u64, Ordering::Relaxed);
    mark_active(0);

    // Mask every I/O APIC line until a driver routes it.
    let lines = ((ioapic_read(1) >> 16) & 0xFF) + 1;
    for pin in 0..lines {
        ioapic_write(0x10 + pin * 2, 1 << 16);
    }

    lapic_write(LAPIC_SVR, 0x100 | SPURIOUS_VECTOR as u32); // enable the local APIC

    // Calibrate: count APIC timer ticks during 10 ms measured by the PIT.
    lapic_write(LAPIC_TIMER_DIVIDE, 0x3); // divide by 16
    lapic_write(LAPIC_LVT_TIMER, 1 << 16); // masked while calibrating
    lapic_write(LAPIC_TIMER_INIT, u32::MAX);
    let tsc_start = super::rdtsc();
    pit_wait_ms(10);
    let per_10ms = u32::MAX - lapic_read(LAPIC_TIMER_CURRENT);
    crate::time::set_tsc_per_ms((super::rdtsc() - tsc_start) / 10);
    let per_tick = per_10ms as u64 * 100 / HZ;
    PER_TICK.store(per_tick, Ordering::Relaxed);

    lapic_write(LAPIC_LVT_TIMER, TIMER_VECTOR as u32 | (1 << 17)); // periodic
    lapic_write(LAPIC_TIMER_INIT, per_tick as u32);

    // SAFETY: the IDT has handlers for every vector that can now arrive.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    crate::info!(
        "apic: temporizator {} Hz pornit ({} impulsuri APIC pe perioada), {} linii I/O APIC",
        HZ,
        per_tick,
        lines
    );
    true
}

/// Timer interrupts since `init`.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Busy-waits `ms` milliseconds using the PIT (independent of the APIC timer; used to check it).
#[allow(dead_code)] // used by tests
pub fn pit_sleep_ms(ms: u64) {
    let mut left = ms;
    while left > 0 {
        let step = left.min(50);
        pit_wait_ms(step);
        left -= step;
    }
}
