//! Limine boot protocol requests. Limine fills in the responses before jumping to `kmain`.

use limine::BaseRevision;
use limine::request::{
    FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest, RequestsEndMarker, RequestsStartMarker, RsdpRequest,
};

/// Highest base revision supported by the `limine` crate (0.5.0).
#[used]
#[unsafe(link_section = ".requests")]
pub static BASE_REVISION: BaseRevision = BaseRevision::new();

/// Linear framebuffer set up by the firmware/bootloader.
#[used]
#[unsafe(link_section = ".requests")]
pub static FRAMEBUFFER: FramebufferRequest = FramebufferRequest::new();

/// Physical memory map.
#[used]
#[unsafe(link_section = ".requests")]
pub static MEMORY_MAP: MemoryMapRequest = MemoryMapRequest::new();

/// Offset of the higher-half direct map (all physical memory mapped at `phys + offset`).
#[used]
#[unsafe(link_section = ".requests")]
pub static HHDM: HhdmRequest = HhdmRequest::new();

/// ACPI root pointer.
#[used]
#[unsafe(link_section = ".requests")]
pub static RSDP: RsdpRequest = RsdpRequest::new();

/// Files Limine loads next to the kernel (`module_path` in limine.conf): the user programs.
#[used]
#[unsafe(link_section = ".requests")]
pub static MODULES: ModuleRequest = ModuleRequest::new();

/// One file loaded by Limine. The data stays valid forever (Limine's "executable and modules" memory is
/// never reclaimed); the name is copied, because the response itself is in reclaimable memory.
pub struct BootFile {
    pub name: alloc::string::String,
    pub data: &'static [u8],
}

static FILES: crate::sched::Mutex<alloc::vec::Vec<BootFile>> = crate::sched::Mutex::new(alloc::vec::Vec::new());

/// Copies the module list from Limine's response. Must run after the heap is ready and before
/// `mem::reclaim_bootloader`.
pub fn collect_files() {
    let Some(response) = MODULES.get_response() else {
        crate::info!("boot: niciun fisier incarcat de Limine");
        return;
    };
    let mut files = FILES.lock();
    for f in response.modules() {
        let path = f.path().to_str().unwrap_or("?");
        let name = path.rsplit('/').next().unwrap_or(path);
        // SAFETY: Limine loaded `size` bytes at `addr` (an HHDM address) in memory that is never reused.
        let data = unsafe { core::slice::from_raw_parts(f.addr(), f.size() as usize) };
        files.push(BootFile { name: alloc::string::String::from(name), data });
    }
    crate::info!("boot: {} fisiere de la Limine", files.len());
}

/// Data of the boot file called `name`.
#[allow(dead_code)] // used by the self-tests now, by ramfs (P2-07)
pub fn file(name: &str) -> Option<&'static [u8]> {
    FILES.lock().iter().find(|f| f.name == name).map(|f| f.data)
}

#[used]
#[unsafe(link_section = ".requests_start_marker")]
static _REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static _REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();
