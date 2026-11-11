//! FADT (signature `FACP`): the reset register, used by `reboot`, and the ACPI PM timer port (P6-79, a reference
//! clock for the APIC calibration when the PIT does not count).
//! Written from the ACPI Specification 6.5, §5.2.9 (FADT layout) and §5.2.3.2 (Generic Address Structure).

/// Where the reset register lives (`RESET_REG.AddressSpaceId`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Space {
    Memory,
    Io,
    /// PCI configuration space of a function on segment 0, bus 0.
    PciConfig,
    Other(u8),
}

/// The reset register and the value that resets the machine when written to it.
#[derive(Clone, Copy, Debug)]
pub struct ResetReg {
    pub space: Space,
    pub bit_width: u8,
    pub address: u64,
    pub value: u8,
    /// `RESET_REG_SUP` (FADT Flags bit 10): the firmware says the register works.
    pub supported: bool,
}

// FADT byte offsets (ACPI 6.5, table 5.9).
const FLAGS: usize = 112;
const RESET_REG: usize = 116;
const RESET_VALUE: usize = 128;
const RESET_REG_SUP: u32 = 1 << 10;
const PM_TMR_BLK: usize = 76;
const X_PM_TMR_BLK: usize = 208;
/// FADT Flags bit 8: the PM timer counter is 32 bits wide (else 24).
const TMR_VAL_EXT: u32 = 1 << 8;

static mut RESET: Option<ResetReg> = None;
/// P6-79: the PM timer's I/O port and whether it is 32 bits wide (0 = none).
static PM_PORT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// P6-79: the ACPI PM timer from the raw FADT: (I/O port, 32-bit counter). `X_PM_TMR_BLK` first (ACPI 2+, only
/// when it is in I/O space), else `PM_TMR_BLK`.
pub fn decode_pm_timer(fadt: &[u8]) -> Option<(u16, bool)> {
    if fadt.len() < FLAGS + 4 {
        return None;
    }
    let flags = u32::from_le_bytes(fadt[FLAGS..FLAGS + 4].try_into().ok()?);
    let wide = flags & TMR_VAL_EXT != 0;
    if fadt.len() >= X_PM_TMR_BLK + 12 && fadt[X_PM_TMR_BLK] == 1 {
        let a = u64::from_le_bytes(fadt[X_PM_TMR_BLK + 4..X_PM_TMR_BLK + 12].try_into().ok()?);
        if a != 0 && a <= 0xFFFF {
            return Some((a as u16, wide));
        }
    }
    let a = u32::from_le_bytes(fadt[PM_TMR_BLK..PM_TMR_BLK + 4].try_into().ok()?);
    (a != 0 && a <= 0xFFFF).then_some((a as u16, wide))
}

/// P6-79: the ACPI PM timer read at boot, if the FADT has one in I/O space.
pub fn pm_timer() -> Option<(u16, bool)> {
    let v = PM_PORT.load(core::sync::atomic::Ordering::Relaxed);
    (v & 0xFFFF != 0).then_some((v as u16, v >> 16 != 0))
}

/// Decodes the reset register from the raw FADT bytes (`None` if the table is too short to have one).
pub fn decode(fadt: &[u8]) -> Option<ResetReg> {
    if fadt.len() <= RESET_VALUE {
        return None;
    }
    let flags = u32::from_le_bytes(fadt[FLAGS..FLAGS + 4].try_into().ok()?);
    let gas = &fadt[RESET_REG..RESET_REG + 12];
    let space = match gas[0] {
        0 => Space::Memory,
        1 => Space::Io,
        2 => Space::PciConfig,
        n => Space::Other(n),
    };
    let address = u64::from_le_bytes(gas[4..12].try_into().ok()?);
    if address == 0 {
        return None;
    }
    Some(ResetReg { space, bit_width: gas[1], address, value: fadt[RESET_VALUE], supported: flags & RESET_REG_SUP != 0 })
}

/// Reads the FADT once at boot (the reset path must not map memory or take locks).
pub fn init() {
    let Some((p, len)) = super::find(b"FACP") else {
        crate::warn!("acpi: FADT lipseste (reboot fara RESET_REG)");
        return;
    };
    // SAFETY: `find` returned a checked, reachable table of `len` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(p, len) };
    if let Some((port, wide)) = decode_pm_timer(bytes) {
        PM_PORT.store(u32::from(port) | (u32::from(wide) << 16), core::sync::atomic::Ordering::Relaxed);
        crate::info!("acpi: temporizator PM la portul {:#x} ({} biti)", port, if wide { 32 } else { 24 });
    }
    let r = decode(bytes);
    match r {
        Some(r) => {
            // A memory reset register is mapped now, while mapping is still safe.
            if r.space == Space::Memory {
                if let Err(e) = crate::mem::paging::map_mmio(r.address, 1) {
                    crate::warn!("acpi: nu pot mapa RESET_REG {:#x}: {:?}", r.address, e);
                }
            }
            crate::info!(
                "acpi: FADT rev {}, RESET_REG {} {:#x} <- {:#04x} ({}, {} biti)",
                bytes[8],
                space_name(r.space),
                r.address,
                r.value,
                if r.supported { "declarat suportat" } else { "NEdeclarat suportat" },
                r.bit_width
            );
        }
        None => crate::info!("acpi: FADT rev {} ({} octeti) fara RESET_REG", bytes[8], len),
    }
    // SAFETY: single-threaded boot; read-only afterwards.
    unsafe { RESET = r };
}

/// The reset register read at boot, if any.
pub fn reset_reg() -> Option<ResetReg> {
    // SAFETY: written once during boot, read-only afterwards.
    unsafe { *core::ptr::addr_of!(RESET) }
}

pub fn space_name(s: Space) -> &'static str {
    match s {
        Space::Memory => "mem",
        Space::Io => "io",
        Space::PciConfig => "pci",
        Space::Other(_) => "alt-spatiu",
    }
}

#[cfg(feature = "qemu-test")]
pub fn self_test() -> bool {
    let mut t = [0u8; 132];
    t[FLAGS + 1] = 0x04; // bit 10
    t[RESET_REG] = 1;
    t[RESET_REG + 1] = 8;
    t[RESET_REG + 4] = 0xF9;
    t[RESET_REG + 5] = 0x0C;
    t[RESET_VALUE] = 6;
    let ok1 = matches!(decode(&t), Some(ResetReg { space: Space::Io, address: 0xCF9, value: 6, supported: true, .. }));
    let ok2 = decode(&t[..116]).is_none(); // FADT 1.0: too short
    t[FLAGS + 1] = 0;
    let ok3 = matches!(decode(&t), Some(ResetReg { supported: false, .. }));
    // P6-79: PM_TMR_BLK 0x608, 24 bits; then X_PM_TMR_BLK (I/O) 0xB008 with TMR_VAL_EXT -> 32 bits.
    let mut f = [0u8; 220];
    f[PM_TMR_BLK] = 0x08;
    f[PM_TMR_BLK + 1] = 0x06;
    let ok4 = decode_pm_timer(&f) == Some((0x608, false));
    f[FLAGS + 1] = 0x01; // bit 8
    f[X_PM_TMR_BLK] = 1;
    f[X_PM_TMR_BLK + 4] = 0x08;
    f[X_PM_TMR_BLK + 5] = 0xB0;
    let ok5 = decode_pm_timer(&f) == Some((0xB008, true));
    ok1 && ok2 && ok3 && ok4 && ok5
}
