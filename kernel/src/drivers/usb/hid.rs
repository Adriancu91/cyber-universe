//! USB keyboards through the HID "boot protocol": 8-byte reports (modifiers, reserved, up to six keys).
//! Key presses go into the same queue as the PS/2 keyboard. Written from the USB HID 1.11
//! specification (appendix B, boot protocol; SET_CONFIGURATION, SET_PROTOCOL) and the HID Usage Tables
//! (keyboard page).

use super::xhci::{Ring, Transfer, UsbDevice, Xhci};
use crate::drivers::keyboard::{Key, push_key};
use crate::mem::{frame, map::phys_to_virt};

static FIRST_KEY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub struct Keyboard {
    slot: u8,
    dci: u8,
    ring: Ring,
    buffer: u64,
    previous: [u8; 8],
}

/// Keyboard page usages 0x04.. -> characters (US layout), without and with Shift.
const PLAIN: &[u8; 53] = b"abcdefghijklmnopqrstuvwxyz1234567890\n\x1b\x08\t -=[]\\\0;'`,./";
const SHIFTED: &[u8; 53] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ!@#$%^&*()\n\x1b\x08\t _+{}|\0:\"~<>?";

fn key_for(usage: u8, shift: bool) -> Option<Key> {
    let i = usage.checked_sub(4)? as usize;
    let c = *if shift { SHIFTED } else { PLAIN }.get(i)?;
    match c {
        0 => None,
        b'\n' => Some(Key::Enter),
        0x08 => Some(Key::Backspace),
        b'\t' => Some(Key::Tab),
        0x1B => Some(Key::Escape),
        c => Some(Key::Char(c as char)),
    }
}

impl Keyboard {
    /// Configures `dev` (a boot-protocol keyboard) and starts listening for reports.
    pub fn start(x: &mut Xhci, dev: &UsbDevice) -> Option<Keyboard> {
        x.control_out(dev.slot, [0x00, 9, dev.config, 0, 0, 0, 0, 0]).ok()?; // SET_CONFIGURATION
        x.control_out(dev.slot, [0x21, 0x0B, 0, 0, dev.interface, 0, 0, 0]).ok()?; // SET_PROTOCOL(boot)
        let (dci, ring) = x.configure_interrupt_in(dev)?;
        let buffer = frame::alloc_frame()?;
        let mut k = Keyboard { slot: dev.slot, dci, ring, buffer, previous: [0; 8] };
        x.queue_in(k.slot, k.dci, &mut k.ring, k.buffer, 8);
        Some(k)
    }

    /// Handles a finished transfer, if it is ours: new keys go to the keyboard queue; the next report is queued.
    pub fn on_transfer(&mut self, x: &mut Xhci, t: Transfer) {
        let (slot, dci, _code, _left) = t;
        if slot != self.slot || dci != self.dci {
            return;
        }
        // SAFETY: the report buffer frame belongs to this driver; the controller finished writing it.
        let report: [u8; 8] = unsafe { *(phys_to_virt(self.buffer) as *const [u8; 8]) };
        let shift = report[0] & 0x22 != 0; // left or right Shift
        for &usage in &report[2..8] {
            if usage >= 4 && !self.previous[2..8].contains(&usage) {
                if let Some(key) = key_for(usage, shift) {
                    if !FIRST_KEY.swap(true, core::sync::atomic::Ordering::Relaxed) {
                        crate::info!("usb: prima tasta de la tastatura USB");
                    }
                    push_key(key);
                }
            }
        }
        self.previous = report;
        x.queue_in(self.slot, self.dci, &mut self.ring, self.buffer, 8);
    }
}
