//! Interrupt controllers: legacy PIC (disabled), local APIC (timer, end-of-interrupt) and I/O APIC
//! (routes device IRQs such as the keyboard to CPU vectors).
//! Written from Intel SDM Vol. 3A ch. 11, the 82093AA I/O APIC datasheet and the OSDev Wiki descriptions.
//! P6-79 (C-109): the local APIC also in x2APIC mode (through MSRs, when the firmware left it so: SDM 3A §11.12), and
//! the calibration with a fallback reference clock (PIT, then the ACPI PM timer, then the TSC rate from CPUID) and a
//! time limit on every wait, so a firmware that gates the PIT clock no longer stops the boot.

use super::{inb, outb};
use crate::mem::paging;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

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
/// P6-79: the local APIC is in x2APIC mode (IA32_APIC_BASE bit 10): registers through MSRs 0x800 + offset/16.
static X2APIC: AtomicBool = AtomicBool::new(false);
/// P6-79: the reference clock used for the calibration and for `pit_sleep_ms` (see [`RefClock`]).
static REF_CLOCK: AtomicU8 = AtomicU8::new(RefClock::Pit as u8);
/// P6-79: TSC ticks per millisecond from the calibration (for the TSC reference clock).
static TSC_PER_MS: AtomicU64 = AtomicU64::new(0);

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_BASE_EXTD: u64 = 1 << 10;
const APIC_BASE_EN: u64 = 1 << 11;
const X2APIC_MSR_BASE: u32 = 0x800;
const X2APIC_ICR: u32 = 0x830;

/// The clock the APIC timer is measured against.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RefClock {
    /// PIT channel 2 (port 0x61 bit 5), as before P6-79.
    Pit = 1,
    /// The ACPI PM timer (3.579545 MHz, FADT `PM_TMR_BLK`).
    PmTimer = 2,
    /// The TSC at the rate CPUID leaf 0x15/0x16 states.
    Tsc = 3,
}

impl RefClock {
    fn name(self) -> &'static str {
        match self {
            RefClock::Pit => "PIT (8254)",
            RefClock::PmTimer => "temporizatorul ACPI PM",
            RefClock::Tsc => "TSC (frecventa din CPUID)",
        }
    }
}

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: only architectural MSRs that exist on every x86_64 processor with an APIC are read here
    // (IA32_APIC_BASE) or, in x2APIC mode only, the x2APIC register MSRs.
    unsafe { core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) }
    (u64::from(hi) << 32) | u64::from(lo)
}

fn wrmsr(msr: u32, value: u64) {
    // SAFETY: as in `rdmsr`; the callers write only the x2APIC registers (in x2APIC mode) or the mode bits of
    // IA32_APIC_BASE.
    unsafe {
        core::arch::asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags))
    }
}

fn cpuid(leaf: u32) -> core::arch::x86_64::CpuidResult {
    // `cpuid` only reads processor identification (a safe intrinsic on x86_64).
    core::arch::x86_64::__cpuid_count(leaf, 0)
}

/// TSC rate in Hz stated by the processor (CPUID 0x15: crystal x ratio; else 0x16: base frequency), if any.
fn cpuid_tsc_hz() -> Option<u64> {
    let max = cpuid(0).eax;
    if max >= 0x15 {
        let l = cpuid(0x15);
        if l.eax != 0 && l.ebx != 0 && l.ecx != 0 {
            return Some(u64::from(l.ecx) * u64::from(l.ebx) / u64::from(l.eax));
        }
    }
    if max >= 0x16 {
        let mhz = cpuid(0x16).eax & 0xFFFF;
        if mhz != 0 {
            return Some(u64::from(mhz) * 1_000_000);
        }
    }
    None
}

/// TSC cycles that surely last at least `ms` milliseconds before the rate is known (a 10 GHz processor).
const fn tsc_bound(ms: u64) -> u64 {
    ms * 10_000_000
}

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
    if X2APIC.load(Ordering::Relaxed) {
        return rdmsr(X2APIC_MSR_BASE + (reg as u32 >> 4)) as u32;
    }
    // SAFETY: LAPIC_BASE points at the mapped local APIC register page; registers are 32-bit aligned.
    unsafe { ((LAPIC_BASE.load(Ordering::Relaxed) as usize + reg) as *const u32).read_volatile() }
}

fn lapic_write(reg: usize, value: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        wrmsr(X2APIC_MSR_BASE + (reg as u32 >> 4), u64::from(value));
        return;
    }
    // SAFETY: as in `lapic_read`.
    unsafe { ((LAPIC_BASE.load(Ordering::Relaxed) as usize + reg) as *mut u32).write_volatile(value) }
}

/// APIC id of the processor running this code (x2APIC: the full 32-bit id, cut to 8 bits; ids above 255 do not
/// occur on the machines we boot).
pub fn lapic_id() -> u8 {
    if X2APIC.load(Ordering::Relaxed) {
        return lapic_read(0x20) as u8;
    }
    (lapic_read(0x20) >> 24) as u8
}

/// Whether the local APIC runs in x2APIC mode (for `hw` and the tests).
#[allow(dead_code)]
pub fn x2apic() -> bool {
    X2APIC.load(Ordering::Relaxed)
}

/// Sends an inter-processor interrupt: `low` = ICR bits 0–31 (vector, delivery mode, level), to `apic_id`.
/// Waits until the local APIC has sent it (delivery status, bit 12; x2APIC has no such bit: one MSR write).
pub fn send_ipi(apic_id: u8, low: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        wrmsr(X2APIC_ICR, (u64::from(apic_id) << 32) | u64::from(low));
        return;
    }
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
/// P6-79: in x2APIC mode every processor must be in it too (the register accesses are global).
pub fn enable_ap() {
    if X2APIC.load(Ordering::Relaxed) {
        let base = rdmsr(IA32_APIC_BASE);
        if base & APIC_BASE_EXTD == 0 {
            wrmsr(IA32_APIC_BASE, base | APIC_BASE_EN);
            wrmsr(IA32_APIC_BASE, base | APIC_BASE_EN | APIC_BASE_EXTD);
        }
    }
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

/// Wakes processor `index` from `hlt` (P6-36, C-40): an IPI on the shootdown vector, whose handler does nothing when
/// no shootdown is pending; the idle loop then looks at the ready queue at once instead of at its next tick (10 ms).
pub fn kick(index: usize) {
    send_ipi(super::percpu::get(index).apic_id.load(Ordering::Relaxed) as u8, SHOOTDOWN_VECTOR as u32);
    KICKS.fetch_add(1, Ordering::Relaxed);
}

/// Wake-up kicks sent since boot (for the tests and `kronos --profil`).
static KICKS: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)] // used by the SMP test
pub fn kicks() -> u64 {
    KICKS.load(Ordering::Relaxed)
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
/// P6-79 (C-109): returns `false` if the PIT did not reach the count within a TSC limit (a firmware that gates the
/// 8254 clock: before, this loop never ended and the boot stopped after `acpi: MADT`).
fn pit_wait_ms(ms: u64) -> bool {
    let count = (1_193_182 * ms / 1000) as u16;
    let start = super::rdtsc();
    let limit = tsc_bound(ms) + tsc_bound(1);
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
            if super::rdtsc().wrapping_sub(start) > limit {
                return false;
            }
            core::hint::spin_loop();
        }
    }
    true
}

/// P6-79: the ACPI PM timer's counter (24 or 32 bits wide).
fn pm_read(port: u16) -> u32 {
    let v: u32;
    // SAFETY: `port` is the FADT's PM_TMR_BLK; reading it has no side effect.
    unsafe { core::arch::asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags)) }
    v
}

/// P6-79: waits `ms` milliseconds on the ACPI PM timer; `false` if it does not count (within a TSC limit).
fn pm_wait_ms(port: u16, bits32: bool, ms: u64) -> bool {
    const PM_HZ: u64 = 3_579_545;
    let mask: u32 = if bits32 { u32::MAX } else { 0x00FF_FFFF };
    let need = PM_HZ * ms / 1000;
    let start_tsc = super::rdtsc();
    let limit = tsc_bound(ms) + tsc_bound(1);
    let mut last = pm_read(port) & mask;
    let mut done = 0u64;
    while done < need {
        let now = pm_read(port) & mask;
        done += u64::from(now.wrapping_sub(last) & mask);
        last = now;
        if super::rdtsc().wrapping_sub(start_tsc) > limit {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

/// P6-79: waits `ms` milliseconds on the TSC at the calibrated rate.
fn tsc_wait_ms(ms: u64) {
    let per_ms = TSC_PER_MS.load(Ordering::Relaxed).max(1);
    let start = super::rdtsc();
    while super::rdtsc().wrapping_sub(start) < ms * per_ms {
        core::hint::spin_loop();
    }
}

/// Waits `ms` milliseconds (at most ~54) on the reference clock chosen at calibration.
fn ref_wait_ms(ms: u64) -> bool {
    match REF_CLOCK.load(Ordering::Relaxed) {
        x if x == RefClock::PmTimer as u8 => match crate::acpi::fadt::pm_timer() {
            Some((port, bits32)) => pm_wait_ms(port, bits32, ms),
            None => false,
        },
        x if x == RefClock::Tsc as u8 => {
            tsc_wait_ms(ms);
            true
        }
        _ => pit_wait_ms(ms),
    }
}

/// One calibration run against `clock`: APIC timer counts and TSC cycles in 10 ms (`None` if the clock does not count).
fn measure_10ms(clock: RefClock, tsc_hz: Option<u64>) -> Option<(u64, u64)> {
    lapic_write(LAPIC_TIMER_INIT, u32::MAX);
    let tsc_start = super::rdtsc();
    let ok = match clock {
        RefClock::Pit => pit_wait_ms(10),
        RefClock::PmTimer => match crate::acpi::fadt::pm_timer() {
            Some((port, bits32)) => pm_wait_ms(port, bits32, 10),
            None => false,
        },
        RefClock::Tsc => match tsc_hz {
            Some(hz) => {
                while super::rdtsc().wrapping_sub(tsc_start) < hz / 100 {
                    core::hint::spin_loop();
                }
                true
            }
            None => false,
        },
    };
    let apic = u64::from(u32::MAX - lapic_read(LAPIC_TIMER_CURRENT));
    let tsc = super::rdtsc().wrapping_sub(tsc_start);
    if !ok {
        return None;
    }
    // The TSC reference states its own rate; the others measure it.
    let tsc_per_ms = match (clock, tsc_hz) {
        (RefClock::Tsc, Some(hz)) => hz / 1000,
        _ => tsc / 10,
    };
    Some((apic, tsc_per_ms))
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

    // P6-79 (C-109): x2APIC left on by the firmware -> the registers through MSRs (the memory page does not answer).
    let base = rdmsr(IA32_APIC_BASE);
    X2APIC.store(base & APIC_BASE_EXTD != 0, Ordering::Relaxed);
    crate::info!(
        "apic: mod {} (IA32_APIC_BASE {:#x}); pasul urmator: rutele I/O APIC, apoi calibrarea ceasului",
        if base & APIC_BASE_EXTD != 0 { "x2APIC (registre prin MSR)" } else { "xAPIC (registre prin memorie)" },
        base
    );

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

    // Calibrate: count APIC timer ticks during 10 ms measured by a reference clock. P6-79 (C-109): the PIT first (as
    // before), then the ACPI PM timer, then the TSC rate from CPUID; each wait has a time limit.
    lapic_write(LAPIC_TIMER_DIVIDE, 0x3); // divide by 16
    lapic_write(LAPIC_LVT_TIMER, 1 << 16); // masked while calibrating
    crate::info!("apic: calibrez ceasul (PIT, apoi ACPI PM, apoi CPUID)");
    let tsc_hz = cpuid_tsc_hz();
    let mut chosen = None;
    for clock in [RefClock::Pit, RefClock::PmTimer, RefClock::Tsc] {
        match measure_10ms(clock, tsc_hz) {
            // A sane result: the APIC timer moved, and the TSC rate is one a 64-bit processor can have (200 MHz–10 GHz);
            // a clock that "ends" at once (an output stuck high) gives a TSC rate far below that.
            Some((apic, tsc_per_ms)) if apic > 0 && apic < u64::from(u32::MAX) && (200_000..=10_000_000).contains(&tsc_per_ms) => {
                chosen = Some((clock, apic, tsc_per_ms));
                break;
            }
            Some((apic, tsc_per_ms)) => crate::warn!(
                "apic: {}: in \"10 ms\" contorul APIC {} si TSC {} kHz (fara sens); incerc urmatorul ceas",
                clock.name(),
                apic,
                tsc_per_ms
            ),
            None => crate::warn!("apic: {} nu numara (oprit de firmware?); incerc urmatorul ceas", clock.name()),
        }
    }
    let Some((clock, per_10ms, tsc_per_ms)) = chosen else {
        crate::error!("apic: niciun ceas de referinta nu numara (PIT, ACPI PM, CPUID); continui fara ceas");
        return false;
    };
    REF_CLOCK.store(clock as u8, Ordering::Relaxed);
    TSC_PER_MS.store(tsc_per_ms, Ordering::Relaxed);
    crate::time::set_tsc_per_ms(tsc_per_ms);
    let per_tick = per_10ms * 100 / HZ;
    PER_TICK.store(per_tick, Ordering::Relaxed);
    crate::info!("apic: ceasul de referinta: {}; TSC {} kHz", clock.name(), tsc_per_ms);

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
        ref_wait_ms(step); // P6-79: the reference clock chosen at calibration (PIT on most machines)
        left -= step;
    }
}
