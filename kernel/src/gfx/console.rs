//! Text console on the framebuffer: cursor, newline and scrolling.

use super::font::{GLYPH_H, GLYPH_W, glyph};
use super::framebuffer::Framebuffer;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

pub const BACKGROUND: u32 = 0x0010_1828;
pub const FOREGROUND: u32 = 0x00D8_E6F0;
pub const ACCENT: u32 = 0x0040_D0FF;

/// Each font pixel is drawn as `SCALE` x `SCALE` screen pixels.
const SCALE: usize = 2;
/// Character cell: glyph plus spacing.
const CELL_W: usize = (GLYPH_W + 1) * SCALE;
const CELL_H: usize = (GLYPH_H + 3) * SCALE;
const MARGIN: usize = 16;

pub struct Console {
    fb: Framebuffer,
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
    pub color: u32,
}

impl Console {
    fn new(mut fb: Framebuffer) -> Self {
        fb.clear(BACKGROUND);
        let cols = (fb.width.saturating_sub(2 * MARGIN)) / CELL_W;
        let rows = (fb.height.saturating_sub(2 * MARGIN)) / CELL_H;
        Self { fb, col: 0, row: 0, cols, rows, color: FOREGROUND }
    }

    fn draw_char(&mut self, c: char) {
        let bitmap = glyph(c);
        let x0 = MARGIN + self.col * CELL_W;
        let y0 = MARGIN + self.row * CELL_H;
        self.fb.fill_rect(x0, y0, CELL_W, CELL_H, BACKGROUND);
        for (gy, bits) in bitmap.iter().enumerate() {
            for gx in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - gx)) != 0 {
                    self.fb.fill_rect(x0 + gx * SCALE, y0 + gy * SCALE, SCALE, SCALE, self.color);
                }
            }
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.fb.scroll_up(CELL_H, BACKGROUND);
        }
    }

    pub fn put_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.col = 0,
            '\x08' => {
                // Backspace: erase the previous character on this line.
                if self.col > 0 {
                    self.col -= 1;
                    self.draw_char(' ');
                }
            }
            _ => {
                if self.col >= self.cols {
                    self.newline();
                }
                self.draw_char(c);
                self.col += 1;
            }
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.put_char(c);
        }
        Ok(())
    }
}

static mut CONSOLE: Option<Console> = None;
static LOCK: AtomicBool = AtomicBool::new(false);

/// Initialises the console on the boot framebuffer. Returns `false` if there is no usable screen.
pub fn init() -> bool {
    let Some(fb) = Framebuffer::from_limine() else { return false };
    let console = Console::new(fb);
    with_console(|slot| *slot = Some(console));
    true
}

/// Runs `f` with exclusive access to the console slot.
pub fn with_console<R>(f: impl FnOnce(&mut Option<Console>) -> R) -> R {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: LOCK guarantees exclusive access to CONSOLE until it is released below.
    let result = f(unsafe { &mut *core::ptr::addr_of_mut!(CONSOLE) });
    LOCK.store(false, Ordering::Release);
    result
}

/// Releases the console lock unconditionally. Only for the panic handler (see `serial::force_unlock`).
pub fn force_unlock() {
    LOCK.store(false, Ordering::Release);
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    with_console(|c| {
        if let Some(c) = c {
            let _ = c.write_fmt(args);
        }
    });
}

/// Sets the text color for subsequent output.
pub fn set_color(color: u32) {
    with_console(|c| {
        if let Some(c) = c {
            c.color = color;
        }
    });
}

/// Prints to the screen console.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::gfx::console::_print(format_args!($($arg)*)) };
}

/// Prints to the screen console, with a newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}
