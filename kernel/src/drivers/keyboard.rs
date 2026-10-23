//! PS/2 keyboard through the 8042 controller: IRQ 1 -> I/O APIC -> vector 49.
//! Scancode set 1 (the controller translates), US layout; Romanian layout is a later task.
//! Written from the OSDev Wiki "8042 PS/2 Controller" and "PS/2 Keyboard" descriptions.

use crate::arch::x86_64::{apic, idt, inb, outb};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) const DATA: u16 = 0x60;
const STATUS_CMD: u16 = 0x64;
const IRQ: u8 = 1;

/// A decoded key press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    /// A letter pressed with Ctrl held (always lower case).
    Ctrl(char),
}

const QUEUE: usize = 64;
static mut KEYS: [Option<Key>; QUEUE] = [None; QUEUE];
static HEAD: AtomicUsize = AtomicUsize::new(0); // written by the interrupt handler
static TAIL: AtomicUsize = AtomicUsize::new(0); // written by the reader
/// Serialises the producers (interrupt handler, USB driver thread), which may run on different processors.
static PUSH_LOCK: AtomicBool = AtomicBool::new(false);
static SHIFT: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);
static EXTENDED: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
/// Right Alt (AltGr), used by the Romanian layout.
static ALTGR: AtomicBool = AtomicBool::new(false);
/// The layout in use: false = US, true = Romanian (Standard).
static ROMANIAN: AtomicBool = AtomicBool::new(false);

/// Keyboard layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Us,
    /// "Romanian (Standard)", SR 13392:2004 primary layout, as Windows defines it (KBDROST).
    Ro,
}

/// Chooses the layout for every keyboard (PS/2 and USB).
pub fn set_layout(l: Layout) {
    ROMANIAN.store(l == Layout::Ro, Ordering::Relaxed);
}

pub fn layout() -> Layout {
    if ROMANIAN.load(Ordering::Relaxed) { Layout::Ro } else { Layout::Us }
}

/// The Romanian (Standard) layout where it differs from US, by set-1 scan code: (normal, Shift,
/// AltGr, Shift+AltGr); '\0' = nothing (dead keys, accents, are not supported). Written from the
/// layout data Microsoft publishes for KBDROST ("Romanian (Standard)").
#[rustfmt::skip]
const RO: [(u8, [char; 4]); 18] = [
    (0x0C, ['-', '_', '\0', '–']),
    (0x0D, ['=', '+', '\0', '±']),
    (0x12, ['e', 'E', '€', '\0']),
    (0x19, ['p', 'P', '§', '\0']),
    (0x1A, ['ă', 'Ă', '[', '{']),
    (0x1B, ['î', 'Î', ']', '}']),
    (0x1F, ['s', 'S', 'ß', '\0']),
    (0x20, ['d', 'D', 'đ', 'Đ']),
    (0x26, ['l', 'L', 'ł', 'Ł']),
    (0x27, ['ș', 'Ș', ';', ':']),
    (0x28, ['ț', 'Ț', '\'', '"']),
    (0x29, ['„', '”', '`', '~']),
    (0x2B, ['â', 'Â', '\\', '|']),
    (0x2E, ['c', 'C', '©', '\0']),
    (0x33, [',', ';', '<', '«']),
    (0x34, ['.', ':', '>', '»']),
    (0x35, ['/', '?', '\0', '\0']),
    (0x56, ['\\', '|', '\0', '\0']),
];

/// The key for scan code `code` in the Romanian layout, if the layout defines it differently from US
/// (`Some(None)` = the key gives nothing, e.g. AltGr with a key that has no AltGr character).
pub fn romanian(code: u8, shift: bool, altgr: bool, caps: bool) -> Option<Option<Key>> {
    let (_, chars) = RO.iter().find(|(c, _)| *c == code)?;
    let c = chars[(shift as usize) | (altgr as usize) << 1];
    if c == '\0' {
        return Some(None);
    }
    // Caps Lock changes letters only (including ă î â ș ț).
    let c = if caps && !altgr && c.is_alphabetic() {
        let other = if c.is_lowercase() { c.to_uppercase().next() } else { c.to_lowercase().next() };
        other.unwrap_or(c)
    } else {
        c
    };
    Some(Some(Key::Char(c)))
}
/// Threads waiting for a key press (see `wait_key`).
static KEY_WAIT: crate::sched::WaitQueue = crate::sched::WaitQueue::new();

#[rustfmt::skip]
const NORMAL: [u8; 58] = *b"\0\x1b1234567890-=\x08\tqwertyuiop[]\n\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
#[rustfmt::skip]
const SHIFTED: [u8; 58] = *b"\0\x1b!@#$%^&*()_+\x08\tQWERTYUIOP{}\n\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";

pub(super) fn wait_write() {
    // SAFETY: reading the 8042 status register has no side effects.
    for _ in 0..100_000 {
        if unsafe { inb(STATUS_CMD) } & 0x02 == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

pub(super) fn wait_read() -> bool {
    for _ in 0..100_000 {
        // SAFETY: as above.
        if unsafe { inb(STATUS_CMD) } & 0x01 != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

pub(super) fn command(cmd: u8) {
    wait_write();
    // SAFETY: documented 8042 controller command.
    unsafe { outb(STATUS_CMD, cmd) }
}

fn decode(scancode: u8) -> Option<Key> {
    if scancode == 0xE0 {
        EXTENDED.store(true, Ordering::Relaxed);
        return None;
    }
    let released = scancode & 0x80 != 0;
    let code = scancode & 0x7F;
    if EXTENDED.swap(false, Ordering::Relaxed) {
        // E0-prefixed keys: the cursor block, the right Ctrl and the right Alt (AltGr).
        if code == 0x1D {
            CTRL.store(!released, Ordering::Relaxed);
            return None;
        }
        if code == 0x38 {
            ALTGR.store(!released, Ordering::Relaxed);
            return None;
        }
        if released {
            return None;
        }
        return match code {
            0x4B => Some(Key::Left),
            0x4D => Some(Key::Right),
            0x48 => Some(Key::Up),
            0x50 => Some(Key::Down),
            0x47 => Some(Key::Home),
            0x4F => Some(Key::End),
            0x53 => Some(Key::Delete),
            0x1C => Some(Key::Enter), // keypad Enter
            _ => None,
        };
    }
    match code {
        0x1D => {
            CTRL.store(!released, Ordering::Relaxed);
            return None;
        }
        0x2A | 0x36 => {
            SHIFT.store(!released, Ordering::Relaxed);
            return None;
        }
        0x3A if !released => {
            CAPS.fetch_xor(true, Ordering::Relaxed);
            return None;
        }
        _ => {}
    }
    if released {
        return None;
    }
    let shift = SHIFT.load(Ordering::Relaxed);
    let altgr = ALTGR.load(Ordering::Relaxed);
    if ROMANIAN.load(Ordering::Relaxed) && !CTRL.load(Ordering::Relaxed) {
        if let Some(k) = romanian(code, shift, altgr, CAPS.load(Ordering::Relaxed)) {
            return k;
        }
    }
    if code == 0x56 {
        return Some(Key::Char(if shift { '|' } else { '\\' })); // the extra key of ISO keyboards
    }
    if code as usize >= NORMAL.len() {
        return None;
    }
    if CTRL.load(Ordering::Relaxed) && NORMAL[code as usize].is_ascii_lowercase() {
        return Some(Key::Ctrl(NORMAL[code as usize] as char));
    }
    let mut c = if shift { SHIFTED[code as usize] } else { NORMAL[code as usize] };
    if CAPS.load(Ordering::Relaxed) && c.is_ascii_alphabetic() {
        c = if shift { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() };
    }
    match c {
        0 => None,
        b'\n' => Some(Key::Enter),
        0x08 => Some(Key::Backspace),
        b'\t' => Some(Key::Tab),
        0x1B => Some(Key::Escape),
        c => Some(Key::Char(c as char)),
    }
}

extern "x86-interrupt" fn handler(_frame: idt::InterruptStackFrame) {
    // SAFETY: an IRQ 1 means the controller has a byte for us.
    let scancode = unsafe { inb(DATA) };
    if let Some(key) = decode(scancode) {
        push_key(key);
    }
    apic::eoi();
}

/// Adds a key press to the queue read by `read_key`/`wait_key`. Called by the PS/2 interrupt handler and
/// by the USB keyboard driver; interrupts are off while the queue changes, so the two cannot mix.
pub fn push_key(key: Key) {
    let flags = crate::sched::spin::lock(&PUSH_LOCK);
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % QUEUE;
    if next != TAIL.load(Ordering::Acquire) {
        // SAFETY: producers are serialised by PUSH_LOCK (from interrupts and from driver threads on any processor); the reader only reads slots before HEAD.
        unsafe { (*core::ptr::addr_of_mut!(KEYS))[head] = Some(key) };
        HEAD.store(next, Ordering::Release);
        KEY_WAIT.wake_all();
    } // queue full: the key is dropped
    crate::sched::spin::unlock(&PUSH_LOCK, flags);
}

/// Initialises the 8042 controller and routes IRQ 1. Must run after `apic::init`.
pub fn init() -> bool {
    command(0xAD); // disable port 1
    command(0xA7); // disable port 2
    // SAFETY: draining stale bytes from the output buffer.
    while unsafe { inb(STATUS_CMD) } & 0x01 != 0 {
        unsafe { inb(DATA) };
    }
    command(0x20);
    if !wait_read() {
        crate::warn!("tastatura: controlerul PS/2 nu raspunde");
        return false;
    }
    // SAFETY: response to the "read config" command.
    let mut config = unsafe { inb(DATA) };
    config &= !0b0000_0011; // interrupts off during setup
    config |= 0b0100_0000; // translation to scancode set 1 on
    command(0x60);
    wait_write();
    // SAFETY: writing the configuration byte requested by command 0x60.
    unsafe { outb(DATA, config) };

    command(0xAA); // controller self-test
    // SAFETY: response to the self-test command.
    if !wait_read() || unsafe { inb(DATA) } != 0x55 {
        crate::warn!("tastatura: autotestul controlerului PS/2 a esuat");
        return false;
    }
    command(0x60); // the self-test can reset the configuration: write it again, now with IRQ 1 on
    wait_write();
    // SAFETY: as above.
    unsafe { outb(DATA, config | 0b0000_0001) };
    command(0xAE); // enable port 1

    idt::set_handler(apic::IRQ_BASE + IRQ, handler as idt::Handler as u64);
    apic::route_irq(IRQ);
    crate::info!("tastatura: PS/2 gata (aranjament US)");
    true
}

/// Returns the next key press, if any.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-keyboard")), allow(dead_code))]
pub fn read_key() -> Option<Key> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: slot `tail` was published by the handler before it advanced HEAD.
    let key = unsafe { (*core::ptr::addr_of!(KEYS))[tail] };
    TAIL.store((tail + 1) % QUEUE, Ordering::Release);
    key
}

/// Waits (sleeping, without using the CPU) for the next key press.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-keyboard")), allow(dead_code))]
pub fn wait_key() -> Key {
    let mut key = None;
    KEY_WAIT.wait_until(|| {
        key = read_key();
        key.is_some()
    });
    key.unwrap()
}
