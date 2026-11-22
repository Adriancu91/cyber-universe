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
const CELL_H: usize = (GLYPH_H + 1) * SCALE;
const MARGIN: usize = 16;

pub struct Console {
    fb: Framebuffer,
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
    pub color: u32,
    /// P6-111/P6-127 (`ecran` of the live channel): the text now on the screen, one row per screen row (the last one
    /// is the row of the cursor). Kept from `enable_shadow` on (it needs the heap).
    text: Option<ScreenText>,
}

/// The console's text, row by row, in memory allocated once (`enable_shadow`): a ring of `rows` buffers of `cols`
/// characters each, reused when the screen scrolls. Writing text never allocates afterwards, so the heap stays the
/// same however much is written (the `stress-smp` test checks the heap is identical at the end).
struct ScreenText {
    ring: alloc::collections::VecDeque<alloc::vec::Vec<char>>,
    /// How many of the last rows of `ring` are on the screen (from 1 up to `ring.len()`).
    used: usize,
    cols: usize,
}

impl ScreenText {
    fn new(rows: usize, cols: usize, used: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let mut ring = alloc::collections::VecDeque::with_capacity(rows);
        for _ in 0..rows {
            ring.push_back(alloc::vec::Vec::with_capacity(cols));
        }
        Self { ring, used: used.clamp(1, rows), cols }
    }

    fn newline(&mut self) {
        if let Some(mut first) = self.ring.pop_front() {
            first.clear();
            self.ring.push_back(first);
        }
        self.used = (self.used + 1).min(self.ring.len());
    }

    fn push(&mut self, c: char) {
        if let Some(l) = self.ring.back_mut() {
            if l.len() < self.cols {
                l.push(c);
            }
        }
    }

    fn pop(&mut self) {
        if let Some(l) = self.ring.back_mut() {
            l.pop();
        }
    }

    fn clear(&mut self) {
        for l in self.ring.iter_mut() {
            l.clear();
        }
        self.used = 1;
    }

    fn rows(&self) -> impl Iterator<Item = &alloc::vec::Vec<char>> {
        self.ring.iter().skip(self.ring.len() - self.used)
    }
}

impl Console {
    fn new(mut fb: Framebuffer) -> Self {
        fb.clear(BACKGROUND);
        let cols = (fb.width.saturating_sub(2 * MARGIN)) / CELL_W;
        let rows = (fb.height.saturating_sub(2 * MARGIN)) / CELL_H;
        Self { fb, col: 0, row: 0, cols, rows, color: FOREGROUND, text: None }
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
        if let Some(t) = &mut self.text {
            t.newline();
        }
        self.col = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.fb.scroll_up(CELL_H, BACKGROUND);
        }
    }

    /// The screen under the console (for drawing that is not text).
    #[allow(dead_code)] // used by the self-tests now, by the window server (P4-04)
    pub fn framebuffer(&mut self) -> &mut Framebuffer {
        &mut self.fb
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
                    if let Some(t) = self.text.as_mut() {
                        t.pop();
                    }
                }
            }
            _ => {
                if self.col >= self.cols {
                    self.newline();
                }
                self.draw_char(c);
                self.col += 1;
                if let Some(t) = self.text.as_mut() {
                    t.push(c);
                }
            }
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if !TEXT_ON_SCREEN.load(Ordering::Acquire) {
            return Ok(()); // the window server owns the screen
        }
        // P6-39 (C-48): moving the screen up costs about as much for many rows as for one (every changed pixel of the
        // screen is rewritten; on the workstation ~59 ms). A text with several line ends that would go past the
        // bottom moves the screen once, by all the rows it needs (at most up to the top), before it is drawn: the
        // same picture at the end as row by row. Lines wrapped for length still move it one row at a time.
        let lines = s.chars().filter(|&c| c == '\n').count();
        let over = (self.row + lines).saturating_sub(self.rows.saturating_sub(1)).min(self.row);
        if over > 1 {
            self.fb.scroll_up(over * CELL_H, BACKGROUND);
            self.row -= over;
        }
        for c in s.chars() {
            self.put_char(c);
        }
        Ok(())
    }
}

static mut CONSOLE: Option<Console> = None;
static LOCK: AtomicBool = AtomicBool::new(false);
/// `false` while the window server owns the screen: text still goes to the serial port, not to the screen.
static TEXT_ON_SCREEN: AtomicBool = AtomicBool::new(true);

/// Initialises the console on the boot framebuffer. Returns `false` if there is no usable screen.
pub fn init() -> bool {
    let Some(fb) = Framebuffer::from_limine() else { return false };
    let console = Console::new(fb);
    with_console(|slot| *slot = Some(console));
    true
}

/// Starts the RAM copy of the screen (P6-29), right after the heap exists. The console wrote rows 0..=row so far.
pub fn enable_shadow() -> bool {
    with_console(|slot| match slot.as_mut() {
        Some(c) => {
            let used = MARGIN + (c.row + 1) * CELL_H;
            // The rows written before this moment are not known as text: they stay empty in `screen_text`.
            c.text = Some(ScreenText::new(c.rows, c.cols, c.row + 1));
            c.fb.enable_shadow(used, BACKGROUND)
        }
        None => false,
    })
}

/// Runs `f` with exclusive access to the console slot.
pub fn with_console<R>(f: impl FnOnce(&mut Option<Console>) -> R) -> R {
    // Interrupts stay on: drawing can take long (the window server copies whole frames, tests wait for ticks).
    // The holder may be switched out; others wait until it runs again.
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
        crate::arch::x86_64::apic::serve_shootdown();
    }
    // SAFETY: LOCK guarantees exclusive access to CONSOLE until it is released below.
    let result = f(unsafe { &mut *core::ptr::addr_of_mut!(CONSOLE) });
    LOCK.store(false, Ordering::Release);
    result
}

/// Releases the console lock unconditionally. Only for the panic handler (see `serial::force_unlock`).
pub fn force_unlock() {
    LOCK.store(false, Ordering::Release);
    TEXT_ON_SCREEN.store(true, Ordering::Release);
}

/// Clears the screen and starts the text again from the top (after the window server gave it back).
pub fn clear() {
    with_console(|c| {
        if let Some(c) = c {
            c.fb.clear(BACKGROUND);
            c.col = 0;
            c.row = 0;
            if let Some(t) = &mut c.text {
                t.clear();
            }
        }
    });
}

/// The text now on the screen console, row by row (`None` before the heap or without a screen). While the window server
/// owns the screen the console writes nothing (`text_on_screen`), so this is the text from before it started.
pub fn screen_text() -> Option<alloc::string::String> {
    with_console(|slot| {
        let t = slot.as_ref()?.text.as_ref()?;
        let mut out = alloc::string::String::new();
        for (i, l) in t.rows().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let end = l.iter().rposition(|c| !c.is_whitespace()).map_or(0, |k| k + 1);
            out.extend(l[..end].iter());
        }
        Some(out)
    })
}

/// P6-127 (`captura` of the live channel): the screen reduced `scale` times in each direction, in shades of gray — for
/// each block the brightest of four samples (the console's strokes are 2 pixels wide, so text stays visible). Reads the
/// RAM copy of the screen when there is one (only four reads per block: video memory is slow to read). `None` = no screen.
pub fn capture_gray(scale: usize) -> Option<(usize, usize, alloc::vec::Vec<u8>)> {
    let scale = scale.max(2);
    with_console(|slot| {
        let fb = &slot.as_ref()?.fb;
        let (w, h) = (fb.width / scale, fb.height / scale);
        let mut out = alloc::vec::Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut best = 0u8;
                for (dx, dy) in [(0, 0), (scale / 2, 0), (0, scale / 2), (scale / 2, scale / 2)] {
                    let p = fb.pixel(x * scale + dx, y * scale + dy).unwrap_or(0);
                    let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                    best = best.max(((r * 77 + g * 150 + b * 29) >> 8) as u8);
                }
                out.push(best);
            }
        }
        Some((w, h, out))
    })
}

/// Whether the console's text is on the screen now (`false` while the window server owns it).
pub fn text_on_screen() -> bool {
    TEXT_ON_SCREEN.load(Ordering::Acquire)
}

/// Turns text output on the screen on or off (the window server turns it off while it draws).
pub fn set_text_on_screen(on: bool) {
    TEXT_ON_SCREEN.store(on, Ordering::Release);
}

/// True when interrupts are off on this processor (an exception or interrupt handler, or a section under `irq_save`).
fn interrupts_off() -> bool {
    let flags: u64;
    // SAFETY: only reads RFLAGS through the stack.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) };
    flags & (1 << 9) == 0
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    output(|c| {
        if let Some(c) = c {
            let _ = c.write_fmt(args);
        }
    });
}

/// Runs `f` on the console for text output. With interrupts off, a holder of the console that was switched out on
/// this processor can never run again until we return, so waiting would hang the machine (seen in CI: the #BP
/// handler logging while the DHCP thread was scrolling the screen): then, if the console is busy, `f` is skipped —
/// the text still reaches the serial port and the log, only the screen misses it.
pub fn output(f: impl FnOnce(&mut Option<Console>)) {
    if interrupts_off() {
        if LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return;
        }
        // SAFETY: LOCK was taken just above and is released below.
        f(unsafe { &mut *core::ptr::addr_of_mut!(CONSOLE) });
        LOCK.store(false, Ordering::Release);
        return;
    }
    with_console(f);
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

/// P6-39 self-test: text written in one block (the screen moved once) leaves exactly the picture of the same text
/// written line by line (the screen moved once per line), with a long line that wraps; both on memory framebuffers.
#[cfg(feature = "test-kronos-kgr")]
pub fn self_test_block_scroll() {
    use core::fmt::Write;
    let (w, h) = (2 * MARGIN + 30 * CELL_W, 2 * MARGIN + 6 * CELL_H);
    let mut a = alloc::vec![0u8; w * h * 4];
    let mut b = alloc::vec![0u8; w * h * 4];
    // SAFETY: each buffer has w * h * 4 bytes, lives until the end of this function and is used only by its console.
    let (fa, fb) = unsafe { (Framebuffer::in_memory(a.as_mut_ptr(), w, h), Framebuffer::in_memory(b.as_mut_ptr(), w, h)) };
    let (mut ca, mut cb) = (Console::new(fa), Console::new(fb));
    let lines = ["intai 1\n", "intai 2\n", "intai 3\n", "Am fost la un restaurant de vis,\n", "rand 5\n", "un rand foarte lung care trece de marginea ecranului mic\n", "rand 7\n", "fara sfarsit"];
    for c in [&mut ca, &mut cb] {
        let _ = c.write_str(lines[0]);
    }
    for l in &lines[1..] {
        let _ = ca.write_str(l);
    }
    let _ = cb.write_str(&lines[1..].concat());
    drop((ca, cb));
    assert!(a == b, "consola: textul scris intr-un bloc difera de acelasi text scris rand cu rand");
    assert!(a.chunks_exact(4).any(|p| p != BACKGROUND.to_le_bytes()), "consola: testul n-a desenat nimic");
    crate::info!("test: consola: un bloc de randuri = aceeasi imagine ca rand cu rand (ecranul mutat o data)");
}

