//! Restarting the machine (`reboot`). Several methods are tried in order, because PCs differ:
//! legacy-free boards (e.g. the Dell workstation) ignore the 8042 keyboard controller.
//! Sources: ACPI 6.5 §4.8.3.6 (reset register), Intel PCH datasheets (Reset Control Register at I/O 0xCF9),
//! IBM PC/AT 8042 (command 0xFE), Intel SDM vol. 3 (triple fault -> shutdown -> platform reset).

use crate::acpi::fadt::{self, Space};
use crate::arch::x86_64::{inb, outb};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Acpi,
    Cf9,
    Kbd8042,
    Triple,
}

impl Method {
    pub fn parse(s: &str) -> Option<Method> {
        match s {
            "acpi" => Some(Method::Acpi),
            "cf9" => Some(Method::Cf9),
            "8042" => Some(Method::Kbd8042),
            "triplu" => Some(Method::Triple),
            _ => None,
        }
    }
}

/// Busy-waits about `us` microseconds with interrupts off (TSC when calibrated, otherwise a rough spin).
fn delay_us(us: u64) {
    match crate::time::since_boot_us() {
        Some(start) => {
            while crate::time::since_boot_us().unwrap_or(u64::MAX) < start + us {
                core::hint::spin_loop();
            }
        }
        None => {
            for _ in 0..us * 1000 {
                core::hint::spin_loop();
            }
        }
    }
}

fn try_acpi() {
    let Some(r) = fadt::reset_reg() else {
        crate::info!("reboot: ACPI: FADT fara RESET_REG, trec mai departe");
        return;
    };
    crate::info!(
        "reboot: incerc ACPI RESET_REG ({} {:#x} <- {:#04x}{})",
        fadt::space_name(r.space),
        r.address,
        r.value,
        if r.supported { "" } else { ", nedeclarat suportat" }
    );
    match r.space {
        // SAFETY: the firmware names this port as its reset register; writing RESET_VALUE is the documented use.
        Space::Io if r.address <= 0xFFFF => unsafe { outb(r.address as u16, r.value) },
        Space::Memory => {
            let virt = crate::mem::map::phys_to_virt(r.address) as u64;
            if crate::mem::paging::translate(virt & !0xFFF).is_some() {
                // SAFETY: mapped at boot by `fadt::init`; the firmware names this byte as its reset register.
                unsafe { (virt as *mut u8).write_volatile(r.value) }
            } else {
                crate::info!("reboot: ACPI: registrul din memorie nu e mapat, trec mai departe");
                return;
            }
        }
        Space::PciConfig => {
            // Encoding (ACPI 6.5 §5.2.3.2): device in bits 32..47, function in 16..31, offset in 0..15; bus 0.
            let dev = (r.address >> 32) as u8;
            let func = (r.address >> 16) as u8;
            let off = r.address as u16;
            match crate::pci::devices().into_iter().find(|d| d.bus == 0 && d.dev == dev && d.func == func) {
                Some(d) => {
                    let shift = (off & 3) * 8;
                    let old = d.read32(off & !3);
                    d.write32(off & !3, (old & !(0xFF << shift)) | ((r.value as u32) << shift));
                }
                None => {
                    crate::info!("reboot: ACPI: functia PCI 00:{:02x}.{} nu exista, trec mai departe", dev, func);
                    return;
                }
            }
        }
        _ => {
            crate::info!("reboot: ACPI: spatiu de adrese necunoscut, trec mai departe");
            return;
        }
    }
    delay_us(500_000);
}

fn try_cf9() {
    crate::info!("reboot: incerc portul 0xCF9 (0x02, apoi 0x06)");
    // SAFETY: 0xCF9 is the chipset Reset Control Register on Intel/AMD PCs: 0x02 selects a hard reset,
    // the 0 -> 1 edge of bit 2 (0x06) starts it. On machines without it the writes are ignored.
    unsafe {
        outb(0xCF9, 0x02);
        delay_us(50);
        outb(0xCF9, 0x06);
    }
    delay_us(500_000);
}

fn try_8042() {
    // SAFETY: reading the 8042 status port has no side effects; 0xFF means no controller answers.
    let status = unsafe { inb(0x64) };
    crate::info!("reboot: incerc controlerul 8042 (0xFE la portul 0x64; stare {:#04x}{})", status, if status == 0xFF { " = lipseste?" } else { "" });
    for _ in 0..100_000 {
        // SAFETY: as above.
        if unsafe { inb(0x64) } & 0x02 == 0 {
            break; // input buffer empty
        }
        core::hint::spin_loop();
    }
    // SAFETY: command 0xFE pulses the CPU reset line; nothing runs after it on machines that honour it.
    unsafe { outb(0x64, 0xFE) };
    delay_us(500_000);
}

fn try_triple() -> ! {
    crate::info!("reboot: incerc triple fault (IDT goala + int3)");
    #[repr(C, packed)]
    struct Idtr {
        limit: u16,
        base: u64,
    }
    let empty = Idtr { limit: 0, base: 0 };
    // SAFETY: deliberately loads an empty IDT and raises an exception: the CPU cannot deliver it (#GP, then #DF),
    // shuts down, and the chipset resets the platform. Interrupts are already off; nothing runs afterwards.
    unsafe { core::arch::asm!("lidt [{}]", "int3", in(reg) &empty, options(noreturn)) }
}

/// Restarts the machine with one method, or with all of them in order (`None`).
pub fn reboot(only: Option<Method>) -> ! {
    // SAFETY: `cli` only masks interrupts on this CPU; we never return.
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };
    match only {
        Some(Method::Acpi) => try_acpi(),
        Some(Method::Cf9) => try_cf9(),
        Some(Method::Kbd8042) => try_8042(),
        Some(Method::Triple) => try_triple(),
        None => {
            try_acpi();
            try_cf9();
            try_8042();
            try_triple();
        }
    }
    crate::warn!("reboot: metoda nu a repornit calculatorul");
    crate::arch::x86_64::halt_forever()
}
