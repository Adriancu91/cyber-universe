//! MADT ("APIC" table): processors, local APIC address, I/O APICs and legacy IRQ overrides.
//! Written from ACPI Specification 6.5, section 5.2.12.

use super::SdtHeader;

pub const MAX_CPUS: usize = 64;
pub const MAX_IOAPICS: usize = 8;
pub const MAX_OVERRIDES: usize = 16;

#[allow(dead_code)] // read by the I/O APIC driver (P1-09/P1-10)
#[derive(Clone, Copy, Default)]
pub struct IoApic {
    pub id: u8,
    pub address: u64,
    pub gsi_base: u32,
}

/// A legacy ISA IRQ that is wired to a different global system interrupt (GSI).
#[allow(dead_code)] // read by the I/O APIC driver (P1-09/P1-10)
#[derive(Clone, Copy, Default)]
pub struct IrqOverride {
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

pub struct Madt {
    pub local_apic: u64,
    pub cpu_apic_ids: [u8; MAX_CPUS],
    pub cpu_count: usize,
    pub ioapics: [IoApic; MAX_IOAPICS],
    pub ioapic_count: usize,
    pub overrides: [IrqOverride; MAX_OVERRIDES],
    pub override_count: usize,
}

static mut MADT: Madt = Madt {
    local_apic: 0,
    cpu_apic_ids: [0; MAX_CPUS],
    cpu_count: 0,
    ioapics: [IoApic { id: 0, address: 0, gsi_base: 0 }; MAX_IOAPICS],
    ioapic_count: 0,
    overrides: [IrqOverride { source: 0, gsi: 0, flags: 0 }; MAX_OVERRIDES],
    override_count: 0,
};

/// Parses the MADT. Must run after `acpi::init`.
pub fn init() -> bool {
    let Some((p, len)) = super::find(b"APIC") else {
        crate::warn!("acpi: tabela MADT (APIC) lipseste");
        return false;
    };
    // SAFETY: single-threaded boot; nothing else uses MADT yet.
    let m = unsafe { &mut *core::ptr::addr_of_mut!(MADT) };
    let hdr = core::mem::size_of::<SdtHeader>();
    // SAFETY: the table is reachable for its whole `len` (checked by `acpi::find`); reads are unaligned-safe.
    unsafe {
        m.local_apic = (p.add(hdr) as *const u32).read_unaligned() as u64;
        let mut off = hdr + 8;
        while off + 2 <= len {
            let kind = *p.add(off);
            let size = *p.add(off + 1) as usize;
            if size < 2 || off + size > len {
                break;
            }
            let e = p.add(off);
            match kind {
                0 => {
                    let flags = (e.add(4) as *const u32).read_unaligned();
                    if flags & 0b11 != 0 && m.cpu_count < MAX_CPUS {
                        m.cpu_apic_ids[m.cpu_count] = *e.add(3);
                        m.cpu_count += 1;
                    }
                }
                1 if m.ioapic_count < MAX_IOAPICS => {
                    m.ioapics[m.ioapic_count] = IoApic {
                        id: *e.add(2),
                        address: (e.add(4) as *const u32).read_unaligned() as u64,
                        gsi_base: (e.add(8) as *const u32).read_unaligned(),
                    };
                    m.ioapic_count += 1;
                }
                2 if m.override_count < MAX_OVERRIDES => {
                    m.overrides[m.override_count] = IrqOverride {
                        source: *e.add(3),
                        gsi: (e.add(4) as *const u32).read_unaligned(),
                        flags: (e.add(8) as *const u16).read_unaligned(),
                    };
                    m.override_count += 1;
                }
                5 => m.local_apic = (e.add(4) as *const u64).read_unaligned(),
                _ => {}
            }
            off += size;
        }
    }
    crate::info!(
        "acpi: MADT: {} procesoare, APIC local la {:#x}, {} I/O APIC (primul la {:#x}, GSI {}), {} redirectari IRQ",
        m.cpu_count,
        m.local_apic,
        m.ioapic_count,
        m.ioapics[0].address,
        m.ioapics[0].gsi_base,
        m.override_count
    );
    true
}

/// The parsed MADT (valid after `init`).
#[allow(dead_code)] // used by the APIC driver (P1-09) and SMP later
pub fn get() -> &'static Madt {
    // SAFETY: written once during boot, read-only afterwards.
    unsafe { &*core::ptr::addr_of!(MADT) }
}
