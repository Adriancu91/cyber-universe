//! USB keyboards and mice through the HID "boot protocol". Keyboards: 8-byte reports (modifiers, reserved,
//! up to six keys), key presses go into the same queue as the PS/2 keyboard. Mice: 3-byte reports
//! (buttons, dx, dy), into the same queue as the PS/2 mouse. Written from the USB HID 1.11
//! specification (appendix B, boot protocol; SET_CONFIGURATION, SET_PROTOCOL) and the HID Usage Tables
//! (keyboard page).

use super::xhci::{Ring, UsbDevice, Xhci};
use crate::drivers::keyboard::{self, Key, push_key};
use crate::mem::{frame, map::phys_to_virt};

static FIRST_KEY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub struct Keyboard {
    /// A mouse instead of a keyboard (same protocol family, different reports).
    mouse: bool,
    slot: u8,
    dci: u8,
    ring: Ring,
    buffer: u64,
    previous: [u8; 8],
}

/// Keyboard page usages 0x04.. -> characters (US layout), without and with Shift.
const PLAIN: &[u8; 53] = b"abcdefghijklmnopqrstuvwxyz1234567890\n\x1b\x08\t -=[]\\\0;'`,./";
const SHIFTED: &[u8; 53] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ!@#$%^&*()\n\x1b\x08\t _+{}|\0:\"~<>?";

/// HID usages of the keys the Romanian layout changes -> their set-1 scan codes.
#[rustfmt::skip]
const USAGE_TO_SCANCODE: [(u8, u8); 18] = [
    (0x2D, 0x0C), (0x2E, 0x0D), (0x08, 0x12), (0x13, 0x19), (0x2F, 0x1A), (0x30, 0x1B), (0x16, 0x1F), (0x07, 0x20),
    (0x0F, 0x26), (0x33, 0x27), (0x34, 0x28), (0x35, 0x29), (0x31, 0x2B), (0x06, 0x2E), (0x36, 0x33), (0x37, 0x34),
    (0x38, 0x35), (0x64, 0x56),
];

fn key_for(usage: u8, shift: bool, ctrl: bool, altgr: bool) -> Option<Key> {
    if keyboard::layout() == keyboard::Layout::Ro && !ctrl {
        if let Some(&(_, code)) = USAGE_TO_SCANCODE.iter().find(|(u, _)| *u == usage) {
            return keyboard::romanian(code, shift, altgr, false)?;
        }
    }
    if usage == 0x64 {
        return Some(Key::Char(if shift { '|' } else { '\\' }));
    }
    match usage {
        0x50 => return Some(Key::Left),
        0x4F => return Some(Key::Right),
        0x52 => return Some(Key::Up),
        0x51 => return Some(Key::Down),
        0x4A => return Some(Key::Home),
        0x4D => return Some(Key::End),
        0x4C => return Some(Key::Delete),
        _ => {}
    }
    let i = usage.checked_sub(4)? as usize;
    if ctrl && i < 26 {
        return Some(Key::Ctrl(PLAIN[i] as char));
    }
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
    /// Configures `dev` (a boot-protocol keyboard or mouse) and starts listening for reports.
    pub fn start(x: &mut Xhci, dev: &UsbDevice) -> Option<Keyboard> {
        x.control_out(dev.slot, [0x00, 9, dev.config, 0, 0, 0, 0, 0]).ok()?; // SET_CONFIGURATION
        x.control_out(dev.slot, [0x21, 0x0B, 0, 0, dev.interface, 0, 0, 0]).ok()?; // SET_PROTOCOL(boot)
        let ep = dev.endpoint(super::xhci::INTERRUPT, true)?;
        let (dci, ring) = x.configure_endpoints(dev, &[ep])?.pop()?;
        let buffer = frame::alloc_frame()?;
        let mouse = dev.class == (0x03, 0x01, 0x02);
        let mut k = Keyboard { mouse, slot: dev.slot, dci, ring, buffer, previous: [0; 8] };
        x.queue(k.slot, k.dci, &mut k.ring, k.buffer, 8);
        Some(k)
    }

    /// Handles every finished report: new keys go to the keyboard queue; the next report is queued.
    pub fn poll(&mut self, x: &mut Xhci) {
        while x.take_transfer(self.slot, self.dci).is_some() {
            self.on_report(x);
        }
    }

    fn on_report(&mut self, x: &mut Xhci) {
        // SAFETY: the report buffer frame belongs to this driver; the controller finished writing it.
        let report: [u8; 8] = unsafe { *(phys_to_virt(self.buffer) as *const [u8; 8]) };
        if self.mouse {
            let e = crate::input::MouseEvent { dx: report[1] as i8 as i16, dy: report[2] as i8 as i16, buttons: report[0] & 7 };
            crate::input::push_mouse(e);
            x.queue(self.slot, self.dci, &mut self.ring, self.buffer, 8);
            return;
        }
        let shift = report[0] & 0x22 != 0; // left or right Shift
        let ctrl = report[0] & 0x11 != 0; // left or right Ctrl
        let altgr = report[0] & 0x40 != 0; // right Alt
        for &usage in &report[2..8] {
            if usage >= 4 && !self.previous[2..8].contains(&usage) {
                if let Some(key) = key_for(usage, shift, ctrl, altgr) {
                    if !FIRST_KEY.swap(true, core::sync::atomic::Ordering::Relaxed) {
                        crate::info!("usb: prima tasta de la tastatura USB");
                    }
                    push_key(key);
                }
            }
        }
        self.previous = report;
        x.queue(self.slot, self.dci, &mut self.ring, self.buffer, 8);
    }
}
