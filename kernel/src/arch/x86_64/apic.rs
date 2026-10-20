//! Interrupt controllers: legacy PIC (disabled), local APIC (timer, end-of-interrupt) and I/O APIC
//! (routes device IRQs such as the keyboard to CPU vectors).
//! Written from Intel SDM Vol. 3A ch. 11, the 82093AA I/O APIC datasheet and the OSDev Wiki descriptions.

use super::{inb, outb};
use crate::mem::paging;
use core::sync::atomic::{AtomicU64, Ordering};

/// CPU vector of the system timer.
pub const TIMER_VECTOR: u8 = 32;
/// First CPU vector used for device IRQs routed through the I/O APIC (IRQ n -> vector 48 + n).
pub const IRQ_BASE: u8 = 48;
pub const SPURIOUS_VECTOR: u8 = 0xFF;
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

extern "x86-interrupt" fn timer_handler(_frame: super::idt::InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    eoi();
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
    let dest = (madt.cpu_apic_ids[0] as u32) << 24;
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
