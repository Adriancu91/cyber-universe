//! 16550 UART driver for COM1 (port 0x3F8).
//!
//! Kept inside the kernel because it is the primary debug channel.
//! Written from the public 16550 datasheet / OSDev Wiki "Serial Ports" description.

use crate::arch::x86_64::{inb, outb};
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

const COM1: u16 = 0x3F8;
const DATA: u16 = COM1; // data register (DLAB=0) / divisor low (DLAB=1)
const INT_ENABLE: u16 = COM1 + 1; // interrupt enable (DLAB=0) / divisor high (DLAB=1)
const FIFO_CTRL: u16 = COM1 + 2;
const LINE_CTRL: u16 = COM1 + 3;
const MODEM_CTRL: u16 = COM1 + 4;
const LINE_STATUS: u16 = COM1 + 5;

const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

/// Upper bound on busy-wait iterations, so a machine without COM1 never hangs the kernel.
const MAX_SPINS: u32 = 100_000;

static PRESENT: AtomicBool = AtomicBool::new(false);
static LOCK: AtomicBool = AtomicBool::new(false);

/// Initialises COM1 at 38400 baud, 8 data bits, no parity, 1 stop bit, FIFO enabled.
/// Detects whether the port exists with the UART loopback self-test.
pub fn init() {
    // SAFETY: standard COM1 registers; on hardware without COM1 these writes are ignored.
    unsafe {
        outb(INT_ENABLE, 0x00); // no interrupts
        outb(LINE_CTRL, 0x80); // DLAB on
        outb(DATA, 0x03); // divisor 3 -> 38400 baud
        outb(INT_ENABLE, 0x00);
        outb(LINE_CTRL, 0x03); // DLAB off, 8N1
        outb(FIFO_CTRL, 0xC7); // enable + clear FIFOs, 14-byte threshold
        outb(MODEM_CTRL, 0x1E); // loopback mode for the self-test
        outb(DATA, 0xAE);
        if inb(DATA) != 0xAE {
            return; // no working UART
        }
        outb(MODEM_CTRL, 0x0F); // normal operation: DTR, RTS, OUT1, OUT2
    }
    PRESENT.store(true, Ordering::Release);
}

/// Whether COM1 was detected by `init`.
pub fn is_present() -> bool {
    PRESENT.load(Ordering::Acquire)
}

fn write_byte(byte: u8) {
    let mut spins = 0;
    // SAFETY: reading the line status register has no side effects.
    while unsafe { inb(LINE_STATUS) } & LSR_THR_EMPTY == 0 {
        spins += 1;
        if spins > MAX_SPINS {
            return;
        }
        core::hint::spin_loop();
    }
    // SAFETY: transmit holding register is empty, writing one byte sends it.
    unsafe { outb(DATA, byte) }
}

/// Returns a received byte, if one is waiting.
pub fn try_read_byte() -> Option<u8> {
    if !is_present() {
        return None;
    }
    // SAFETY: LSR read has no side effects; DATA read consumes the byte that LSR reported.
    unsafe {
        if inb(LINE_STATUS) & LSR_DATA_READY != 0 { Some(inb(DATA)) } else { None }
    }
}

/// Writes a string, translating `\n` to `\r\n`.
pub fn write_str(s: &str) {
    if !is_present() {
        return;
    }
    // Simple spinlock so lines from different contexts do not interleave.
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    for b in s.bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
    LOCK.store(false, Ordering::Release);
}

/// `core::fmt` adapter for the serial port.
pub struct SerialWriter;

impl fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let _ = SerialWriter.write_fmt(args);
}

/// Prints to COM1.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => { $crate::drivers::serial::_print(format_args!($($arg)*)) };
}

/// Prints to COM1, with a newline.
#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print!("\n") };
    ($($arg:tt)*) => { $crate::serial_print!("{}\n", format_args!($($arg)*)) };
}
