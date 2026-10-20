//! Physical memory map and the higher-half direct map (HHDM), both provided by Limine.
//! Written from the Limine protocol specification (memory map and HHDM features).

use limine::memory_map::EntryType;

/// Size of one physical frame / page.
pub const PAGE_SIZE: u64 = 4096;
const MAX_REGIONS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    Bad,
    BootloaderReclaimable,
    Kernel,
    Framebuffer,
}

impl Kind {
    fn from_limine(t: EntryType) -> Kind {
        match t {
            EntryType::USABLE => Kind::Usable,
            EntryType::ACPI_RECLAIMABLE => Kind::AcpiReclaimable,
            EntryType::ACPI_NVS => Kind::AcpiNvs,
            EntryType::BAD_MEMORY => Kind::Bad,
            EntryType::BOOTLOADER_RECLAIMABLE => Kind::BootloaderReclaimable,
            EntryType::EXECUTABLE_AND_MODULES => Kind::Kernel,
            EntryType::FRAMEBUFFER => Kind::Framebuffer,
            _ => Kind::Reserved,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Usable => "utilizabila",
            Kind::Reserved => "rezervata",
            Kind::AcpiReclaimable => "ACPI (recuperabila)",
            Kind::AcpiNvs => "ACPI NVS",
            Kind::Bad => "defecta",
            Kind::BootloaderReclaimable => "bootloader (recuperabila)",
            Kind::Kernel => "nucleu si module",
            Kind::Framebuffer => "ecran",
        }
    }
}

/// One physical memory region.
#[derive(Clone, Copy)]
pub struct Region {
    pub base: u64,
    pub length: u64,
    pub kind: Kind,
}

impl Region {
    pub fn end(&self) -> u64 {
        self.base + self.length
    }
}

static mut REGIONS: [Region; MAX_REGIONS] = [Region { base: 0, length: 0, kind: Kind::Reserved }; MAX_REGIONS];
static mut REGION_COUNT: usize = 0;
static mut HHDM_OFFSET: u64 = 0;

/// Copies the memory map and the HHDM offset from Limine and logs a summary.
/// Returns `false` if Limine did not provide them (the kernel cannot manage memory then).
pub fn init() -> bool {
    let (Some(map), Some(hhdm)) = (crate::boot::MEMORY_MAP.get_response(), crate::boot::HHDM.get_response())
    else {
        crate::error!("mem: Limine nu a trimis harta memoriei sau HHDM");
        return false;
    };

    let mut count = 0;
    let mut usable = 0u64;
    let mut top = 0u64;
    for e in map.entries().iter().take(MAX_REGIONS) {
        let region = Region { base: e.base, length: e.length, kind: Kind::from_limine(e.entry_type) };
        crate::info!(
            "mem: {:#014x}-{:#014x} {:>8} KiB {}",
            region.base,
            region.end(),
            region.length / 1024,
            region.kind.name()
        );
        if region.kind == Kind::Usable {
            usable += region.length;
        }
        top = top.max(region.end());
        // SAFETY: single-threaded boot; nothing reads REGIONS until `init` returns.
        unsafe { (*core::ptr::addr_of_mut!(REGIONS))[count] = region };
        count += 1;
    }
    if map.entries().len() > MAX_REGIONS {
        crate::warn!("mem: harta are {} regiuni, pastrez primele {}", map.entries().len(), MAX_REGIONS);
    }
    // SAFETY: single-threaded boot, as above.
    unsafe {
        REGION_COUNT = count;
        HHDM_OFFSET = hhdm.offset();
    }

    crate::info!(
        "mem: usable: {} MiB in {} regiuni, adresa maxima {:#x}, HHDM la {:#x}",
        usable / (1024 * 1024),
        count,
        top,
        hhdm.offset()
    );
    true
}

/// All regions of the memory map.
pub fn regions() -> &'static [Region] {
    // SAFETY: written once by `init` during boot, read-only afterwards.
    unsafe {
        let all: &'static [Region; MAX_REGIONS] = &*core::ptr::addr_of!(REGIONS);
        &all[..REGION_COUNT]
    }
}

/// Virtual address at which the physical address `phys` can be accessed (through the HHDM).
pub fn phys_to_virt(phys: u64) -> *mut u8 {
    // SAFETY: HHDM_OFFSET is written once by `init` during boot, read-only afterwards.
    (phys + unsafe { HHDM_OFFSET }) as *mut u8
}
