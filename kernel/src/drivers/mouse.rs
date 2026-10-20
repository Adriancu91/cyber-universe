//! PS/2 mouse on the second port of the 8042 controller (IRQ 12): 3-byte packets -> `input::MouseEvent`.
//! Written from the PS/2 mouse protocol description (OSDev Wiki "PS/2 Mouse", descriptions only).

use super::keyboard::{DATA, command, wait_read, wait_write};
use crate::arch::x86_64::{apic, idt, inb, outb};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

const IRQ: u8 = 12;
static PACKET: [AtomicU8; 3] = [AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0)];
static INDEX: AtomicUsize = AtomicUsize::new(0);

/// Sends one byte to the mouse and waits for its acknowledgement (0xFA).
fn send(byte: u8) -> bool {
    command(0xD4); // next data byte goes to port 2
    wait_write();
    // SAFETY: data byte for the device on port 2, requested by command 0xD4.
    unsafe { outb(DATA, byte) };
    // SAFETY: the mouse answers on the data port.
    wait_read() && unsafe { inb(DATA) } == 0xFA
}

extern "x86-interrupt" fn handler(_frame: idt::InterruptStackFrame) {
    // SAFETY: IRQ 12 means the controller has a byte from the mouse.
    let byte = unsafe { inb(DATA) };
    let i = INDEX.load(Ordering::Relaxed);
    if i == 0 && byte & 0x08 == 0 {
        apic::eoi(); // not the first byte of a packet: wait for one that is (resynchronise)
        return;
    }
    PACKET[i].store(byte, Ordering::Relaxed);
    if i < 2 {
        INDEX.store(i + 1, Ordering::Relaxed);
    } else {
        INDEX.store(0, Ordering::Relaxed);
        let b0 = PACKET[0].load(Ordering::Relaxed);
        if b0 & 0xC0 == 0 {
            // 9-bit two's complement values; PS/2 counts y upwards, the screen downwards.
            let dx = PACKET[1].load(Ordering::Relaxed) as i16 - (((b0 as i16) << 4) & 0x100);
            let dy = PACKET[2].load(Ordering::Relaxed) as i16 - (((b0 as i16) << 3) & 0x100);
            crate::input::push_mouse(crate::input::MouseEvent { dx, dy: -dy, buttons: b0 & 0x07 });
        }
    }
    apic::eoi();
}

/// Enables the PS/2 mouse. Must run after `keyboard::init` (which sets up the controller).
pub fn init() -> bool {
    // The controller's answers would otherwise raise IRQ 1 and be eaten by the keyboard handler.
    let flags = crate::sched::irq_save();
    let ok = setup();
    crate::sched::irq_restore(flags);
    if ok {
        idt::set_handler(apic::IRQ_BASE + IRQ, handler as idt::Handler as u64);
        apic::route_irq(IRQ);
        crate::info!("mouse: PS/2 gata");
    }
    ok
}

fn setup() -> bool {
    command(0xA8); // enable port 2
    if !(send(0xF6) && send(0xF4)) {
        // defaults, then start sending packets
        crate::info!("mouse: niciun mouse PS/2");
        return false;
    }
    command(0x20);
    if !wait_read() {
        crate::warn!("mouse: controlerul PS/2 nu raspunde");
        return false;
    }
    // SAFETY: response to the "read config" command.
    let config = unsafe { inb(DATA) };
    command(0x60);
    wait_write();
    // SAFETY: configuration byte: IRQ 12 on, port 2 clock on.
    unsafe { outb(DATA, (config | 0b10) & !0b10_0000) };
    true
}
