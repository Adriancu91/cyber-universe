//! Limine boot protocol requests. Limine fills in the responses before jumping to `kmain`.

use limine::BaseRevision;
use limine::request::{FramebufferRequest, HhdmRequest, MemoryMapRequest, RsdpRequest, RequestsEndMarker, RequestsStartMarker};

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

#[used]
#[unsafe(link_section = ".requests_start_marker")]
static _REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static _REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();
