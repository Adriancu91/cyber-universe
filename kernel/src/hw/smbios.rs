//! P6-85 (C-116): which computer this is, from the firmware's SMBIOS tables (read only).
//!
//! Written from the DMTF "System Management BIOS (SMBIOS) Reference Specification" (DSP0134, 3.x): the 32-bit
//! (`_SM_`) and 64-bit (`_SM3_`) entry points, the structure header (type, length, handle), the string set after the
//! formatted area, type 1 (System Information: manufacturer, product name, version, SKU, family) and type 2 (Baseboard:
//! manufacturer, product, version). The serial numbers and the UUID are deliberately not read. Limine gives the entry
//! point addresses (SMBIOS request); the pages are mapped read-only when the HHDM does not cover them (same helper as
//! ACPI). Parsed once at boot, kept as text.

use crate::sched::Mutex;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// What `hw` shows. Empty fields are left out.
#[derive(Clone, Default)]
pub struct Info {
    pub version: (u8, u8),
    pub manufacturer: String,
    pub product: String,
    pub product_version: String,
    pub family: String,
    pub sku: String,
    pub board_manufacturer: String,
    pub board_product: String,
    pub board_version: String,
}

static INFO: Mutex<Option<Info>> = Mutex::new(None);
/// P6-86 (`analiza`): the raw table and the version, kept for the inventory (serials stay on the machine).
static RAW: Mutex<Option<((u8, u8), Vec<u8>)>> = Mutex::new(None);

/// The table is at most this big for us (real tables are a few KiB).
const MAX_TABLE: usize = 64 * 1024;

/// Reads the SMBIOS tables once (boot). Returns false when there is none.
pub fn init() -> bool {
    let Some(resp) = crate::boot::SMBIOS.get_response() else {
        crate::info!("smbios: Limine nu a gasit SMBIOS");
        return false;
    };
    let hhdm = crate::mem::map::phys_to_virt(0) as u64;
    let phys = |a: usize| {
        let a = a as u64;
        if a >= hhdm { a - hhdm } else { a }
    };
    let found = resp
        .entry_64()
        .and_then(|a| entry_64(phys(a.get())))
        .or_else(|| resp.entry_32().and_then(|a| entry_32(phys(a.get()))));
    let Some((version, table, len)) = found else {
        crate::warn!("smbios: punctul de intrare nu e valid");
        return false;
    };
    let len = len.min(MAX_TABLE);
    let p = crate::acpi::reach(table, len as u64);
    // SAFETY: `reach` made [table, table + len) readable (HHDM or a read-only mapping); firmware tables do not move.
    let bytes = unsafe { core::slice::from_raw_parts(p, len) };
    *RAW.lock() = Some((version, bytes.to_vec()));
    let mut info = parse(bytes);
    info.version = version;
    crate::info!("smbios: {} {}", version_text(version), summary(&info));
    *INFO.lock() = Some(info);
    true
}

/// The 64-bit entry point (`_SM3_`, 24 bytes): (version, table address, maximum table length).
fn entry_64(phys: u64) -> Option<((u8, u8), u64, usize)> {
    let p = crate::acpi::reach(phys, 24);
    // SAFETY: 24 bytes reachable.
    let b = unsafe { core::slice::from_raw_parts(p, 24) };
    let len = (b[6] as usize).clamp(24, 32);
    if &b[0..5] != b"_SM3_" || !sum_ok(phys, len) {
        return None;
    }
    let max = u32::from_le_bytes(b[12..16].try_into().ok()?) as usize;
    let addr = u64::from_le_bytes(b[16..24].try_into().ok()?);
    (addr != 0).then_some(((b[7], b[8]), addr, max))
}

/// The 32-bit entry point (`_SM_`, 31 bytes, with the `_DMI_` part at 16): (version, table address, table length).
fn entry_32(phys: u64) -> Option<((u8, u8), u64, usize)> {
    let p = crate::acpi::reach(phys, 31);
    // SAFETY: 31 bytes reachable.
    let b = unsafe { core::slice::from_raw_parts(p, 31) };
    let len = (b[5] as usize).clamp(16, 31);
    if &b[0..4] != b"_SM_" || !sum_ok(phys, len) || &b[16..21] != b"_DMI_" {
        return None;
    }
    let tlen = u16::from_le_bytes([b[22], b[23]]) as usize;
    let addr = u32::from_le_bytes(b[24..28].try_into().ok()?) as u64;
    (addr != 0).then_some(((b[6], b[7]), addr, tlen))
}

fn sum_ok(phys: u64, len: usize) -> bool {
    let p = crate::acpi::reach(phys, len as u64);
    // SAFETY: `len` bytes reachable.
    let b = unsafe { core::slice::from_raw_parts(p, len) };
    b.iter().fold(0u8, |a, x| a.wrapping_add(*x)) == 0
}

/// Walks the structures (pure function: also used by the boot self-test on a hand-made table).
pub fn parse(t: &[u8]) -> Info {
    let mut info = Info::default();
    let mut off = 0usize;
    while off + 4 <= t.len() {
        let kind = t[off];
        let len = t[off + 1] as usize;
        if len < 4 || off + len > t.len() {
            break;
        }
        let formatted = &t[off..off + len];
        // The string set: NUL-terminated strings, ended by an extra NUL (two NULs when there are no strings).
        let mut end = off + len;
        let mut strings: Vec<&[u8]> = Vec::new();
        loop {
            let start = end;
            while end < t.len() && t[end] != 0 {
                end += 1;
            }
            if end >= t.len() {
                return info;
            }
            if end == start {
                end += 1;
                if strings.is_empty() && end < t.len() && t[end] == 0 {
                    end += 1;
                }
                break;
            }
            strings.push(&t[start..end]);
            end += 1;
        }
        let s = |i: usize| -> String {
            let idx = formatted.get(i).copied().unwrap_or(0) as usize;
            if idx == 0 || idx > strings.len() {
                return String::new();
            }
            clean(strings[idx - 1])
        };
        match kind {
            1 => {
                info.manufacturer = s(4);
                info.product = s(5);
                info.product_version = s(6);
                info.sku = s(0x19);
                info.family = s(0x1A);
            }
            2 if info.board_product.is_empty() => {
                info.board_manufacturer = s(4);
                info.board_product = s(5);
                info.board_version = s(6);
            }
            127 => break,
            _ => {}
        }
        off = end;
    }
    info
}

/// Printable ASCII only, trimmed; the usual placeholder texts become empty.
fn clean(b: &[u8]) -> String {
    let s: String = b.iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' }).collect();
    let s = String::from(s.trim());
    let lower = s.to_ascii_lowercase();
    if lower.is_empty()
        || lower.contains("to be filled")
        || lower == "default string"
        || lower == "not applicable"
        || lower == "system product name"
        || lower == "system manufacturer"
    {
        return String::new();
    }
    s
}

fn version_text(v: (u8, u8)) -> String {
    format!("SMBIOS {}.{}", v.0, v.1)
}

fn join(parts: &[&str]) -> String {
    let v: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    v.join(" ")
}

fn summary(i: &Info) -> String {
    let name = join(&[&i.manufacturer, &i.product]);
    if name.is_empty() { String::from("(model necunoscut)") } else { name }
}

/// P6-86: the raw structure table and the SMBIOS version (None when the firmware has no SMBIOS).
pub fn raw() -> Option<((u8, u8), Vec<u8>)> {
    RAW.lock().clone()
}

/// The lines of `hw` / `hw lista`.
pub fn lines() -> Vec<String> {
    let mut out = Vec::new();
    let Some(i) = INFO.lock().clone() else {
        out.push(String::from("calculator: necunoscut (SMBIOS absent)"));
        return out;
    };
    let mut first = format!("calculator: {}", summary(&i));
    if !i.product_version.is_empty() {
        first.push_str(&format!(" (versiune {})", i.product_version));
    }
    out.push(first);
    let mut more = Vec::new();
    if !i.family.is_empty() {
        more.push(format!("familia {}", i.family));
    }
    if !i.sku.is_empty() {
        more.push(format!("SKU {}", i.sku));
    }
    let board = join(&[&i.board_manufacturer, &i.board_product, &i.board_version]);
    if !board.is_empty() {
        more.push(format!("placa de baza {}", board));
    }
    more.push(version_text(i.version));
    out.push(format!("  {}", more.join("; ")));
    out
}

/// Boot self-test of the parser on a hand-made table (type 1 with strings, type 2, a type with no strings, end).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let mut t: Vec<u8> = Vec::new();
    // Type 1, length 0x1B: manufacturer 1, product 2, version 3, serial 0, SKU 4, family 5.
    let mut f = alloc::vec![0u8; 0x1B];
    f[0] = 1;
    f[1] = 0x1B;
    f[4] = 1;
    f[5] = 2;
    f[6] = 3;
    f[0x19] = 4;
    f[0x1A] = 5;
    t.extend_from_slice(&f);
    for s in ["ASUSTeK COMPUTER INC.", "ASUS TUF Gaming F15 FX506HM", "1.0       ", "To be filled by O.E.M.", "ASUS TUF Gaming F15"] {
        t.extend_from_slice(s.as_bytes());
        t.push(0);
    }
    t.push(0);
    // A type with no strings (type 32, length 11): two NULs.
    t.extend_from_slice(&[32, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // Type 2: manufacturer 1, product 2, version 0.
    t.extend_from_slice(&[2, 8, 0, 0, 1, 2, 0, 0]);
    t.extend_from_slice(b"ASUSTeK COMPUTER INC.\0FX506HM\0\0");
    t.extend_from_slice(&[127, 4, 0, 0, 0, 0]);
    let i = parse(&t);
    assert!(i.manufacturer == "ASUSTeK COMPUTER INC.", "smbios: producator {}", i.manufacturer);
    assert!(i.product == "ASUS TUF Gaming F15 FX506HM", "smbios: produs {}", i.product);
    assert!(i.product_version == "1.0" && i.sku.is_empty() && i.family == "ASUS TUF Gaming F15", "smbios: campuri");
    assert!(i.board_product == "FX506HM" && i.board_version.is_empty(), "smbios: placa de baza");
    // A truncated table must not panic.
    let _ = parse(&t[..20]);
    crate::info!("test: smbios OK (tipurile 1 si 2, sir lipsa, tabel trunchiat)");
}
