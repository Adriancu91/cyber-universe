//! ACPI tables: finds the system description tables and reads the MADT (processors and interrupt controllers).
//!
//! Only table discovery and the MADT for now; the AML interpreter (power management, devices) is a separate,
//! much larger task for Phase 3. Written from the ACPI Specification 6.5, chapter 5.

pub mod madt;

use crate::mem::map::{PAGE_SIZE, phys_to_virt};
use crate::mem::paging;

/// Common 36-byte header of every system description table.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

const MAX_TABLES: usize = 32;
static mut TABLES: [u64; MAX_TABLES] = [0; MAX_TABLES]; // physical addresses
static mut TABLE_COUNT: usize = 0;

/// Makes sure `[phys, phys + len)` is reachable through the HHDM and returns its virtual address.
/// Limine's HHDM does not have to cover firmware-reserved memory, so missing pages are mapped read-only.
fn reach(phys: u64, len: u64) -> *const u8 {
    let virt = phys_to_virt(phys) as u64;
    let first = virt & !(PAGE_SIZE - 1);
    let last = (virt + len.max(1) - 1) & !(PAGE_SIZE - 1);
    let mut page = first;
    while page <= last {
        if paging::translate(page).is_none() {
            let phys_page = phys - (virt - page);
            if let Err(e) = paging::map(page, phys_page, paging::NO_EXECUTE) {
                crate::warn!("acpi: nu pot mapa {:#x}: {:?}", phys_page, e);
            }
        }
        page += PAGE_SIZE;
    }
    virt as *const u8
}

fn checksum_ok(ptr: *const u8, len: usize) -> bool {
    // SAFETY: the caller made [ptr, ptr + len) reachable with `reach`.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0
}

/// Reads a table header at physical address `phys` (after mapping the whole table).
fn header(phys: u64) -> Option<(SdtHeader, *const u8)> {
    let p = reach(phys, core::mem::size_of::<SdtHeader>() as u64);
    // SAFETY: the header bytes are reachable; the struct is packed so unaligned reads are fine.
    let h = unsafe { (p as *const SdtHeader).read_unaligned() };
    let p = reach(phys, h.length as u64);
    if !checksum_ok(p, h.length as usize) {
        crate::warn!("acpi: suma de control gresita la tabela {}", core::str::from_utf8(&h.signature).unwrap_or("????"));
        return None;
    }
    Some((h, p))
}

/// Finds all ACPI tables starting from the RSDP given by Limine.
pub fn init() -> bool {
    let Some(rsdp) = crate::boot::RSDP.get_response() else {
        crate::warn!("acpi: Limine nu a gasit RSDP (sistem fara ACPI?)");
        return false;
    };
    // Base revision 3 returns a physical address; older revisions a virtual (HHDM) one. Accept both.
    let hhdm = phys_to_virt(0) as u64;
    let addr = rsdp.address() as u64;
    let rsdp_phys = if addr >= hhdm { addr - hhdm } else { addr };

    let p = reach(rsdp_phys, 36);
    // SAFETY: 36 bytes reachable (enough for both RSDP versions).
    let bytes = unsafe { core::slice::from_raw_parts(p, 36) };
    if &bytes[0..8] != b"RSD PTR " || !checksum_ok(p, 20) {
        crate::error!("acpi: RSDP invalid");
        return false;
    }
    let revision = bytes[15];
    let (root, entry_size) = if revision >= 2 && checksum_ok(p, 36) {
        (u64::from_le_bytes(bytes[24..32].try_into().unwrap()), 8) // XSDT
    } else {
        (u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as u64, 4) // RSDT
    };

    let Some((h, rp)) = header(root) else { return false };
    let count = (h.length as usize - core::mem::size_of::<SdtHeader>()) / entry_size;
    let mut names = [[b' '; 4]; MAX_TABLES];
    let mut n = 0;
    for i in 0..count.min(MAX_TABLES) {
        // SAFETY: entries follow the header inside the (reachable) root table.
        let entry = unsafe { rp.add(core::mem::size_of::<SdtHeader>() + i * entry_size) };
        let phys = if entry_size == 8 {
            // SAFETY: as above; unaligned read.
            unsafe { (entry as *const u64).read_unaligned() }
        } else {
            // SAFETY: as above; unaligned read.
            unsafe { (entry as *const u32).read_unaligned() as u64 }
        };
        if let Some((th, _)) = header(phys) {
            names[n] = th.signature;
            // SAFETY: single-threaded boot.
            unsafe { (*core::ptr::addr_of_mut!(TABLES))[n] = phys };
            n += 1;
        }
    }
    // SAFETY: single-threaded boot.
    unsafe { TABLE_COUNT = n };

    let mut list = alloc::string::String::new();
    for name in &names[..n] {
        list.push_str(core::str::from_utf8(name).unwrap_or("????"));
        list.push(' ');
    }
    crate::info!("acpi: {} tabele ({}): {}", n, if entry_size == 8 { "XSDT" } else { "RSDT" }, list.trim_end());
    true
}

/// Finds a table by its 4-letter signature; returns a pointer to its header and its length.
pub fn find(signature: &[u8; 4]) -> Option<(*const u8, usize)> {
    // SAFETY: written once during boot, read-only afterwards.
    let tables = unsafe { &(&*core::ptr::addr_of!(TABLES))[..TABLE_COUNT] };
    tables.iter().find_map(|&phys| {
        let (h, p) = header(phys)?;
        (&h.signature == signature).then_some((p, h.length as usize))
    })
}
