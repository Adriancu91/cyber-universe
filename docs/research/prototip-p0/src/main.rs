#![no_std]
#![no_main]

use core::arch::asm;
use limine::BaseRevision;
use limine::request::{FramebufferRequest, RequestsEndMarker, RequestsStartMarker};

#[used] #[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();
#[used] #[unsafe(link_section = ".requests")]
static FRAMEBUFFER: FramebufferRequest = FramebufferRequest::new();
#[used] #[unsafe(link_section = ".requests_start_marker")]
static _START: RequestsStartMarker = RequestsStartMarker::new();
#[used] #[unsafe(link_section = ".requests_end_marker")]
static _END: RequestsEndMarker = RequestsEndMarker::new();

unsafe fn outb(port: u16, v: u8) { unsafe { asm!("out dx, al", in("dx") port, in("al") v) } }
unsafe fn inb(port: u16) -> u8 { let v: u8; unsafe { asm!("in al, dx", out("al") v, in("dx") port) }; v }

fn serial_write(s: &str) {
    for b in s.bytes() {
        unsafe { while inb(0x3F8 + 5) & 0x20 == 0 {} outb(0x3F8, b); }
    }
}

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    assert!(BASE_REVISION.is_supported());
    serial_write("CYBER UNIVERSE OS: BOOT OK\n");
    if let Some(r) = FRAMEBUFFER.get_response() {
        if let Some(fb) = r.framebuffers().next() {
            for y in 0..fb.height() { for x in 0..fb.width() {
                let off = (y * fb.pitch() + x * 4) as usize;
                unsafe { *(fb.addr().add(off) as *mut u32) = 0x0010_2040 + ((x as u32 * 255 / fb.width() as u32) << 8); }
            }}
            serial_write("FRAMEBUFFER OK\n");
        }
    }
    unsafe { outb(0xf4, 0x10) } // isa-debug-exit
    loop { unsafe { asm!("hlt") } }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { serial_write("PANIC\n"); loop {} }
