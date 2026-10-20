//! Windows for programs: open one, draw in a picture kept in the program's memory, show it, wait for
//! keys and clicks. See kernel/src/syscall/README.md (calls 8-11).

use crate::{SYS_EVENT_WAIT, SYS_WINDOW_CLOSE, SYS_WINDOW_CREATE, SYS_WINDOW_PRESENT, syscall3, syscall4};

// The same font as the kernel's console (drawn for this project, MIT).
#[path = "../../kernel/src/gfx/font.rs"]
mod font;
pub use font::{GLYPH_H, GLYPH_W};

/// A picture of `width` x `height` pixels (0x00RRGGBB) in memory the program owns.
pub struct Canvas<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a mut [u32],
}

impl<'a> Canvas<'a> {
    /// `pixels` must hold exactly `width * height` values.
    pub fn new(width: usize, height: usize, pixels: &'a mut [u32]) -> Canvas<'a> {
        assert!(pixels.len() == width * height, "Canvas: marime gresita");
        Canvas { width, height, pixels }
    }

    /// Fills a rectangle (cut to the picture).
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let (r, b) = ((x + w).min(self.width), (y + h).min(self.height));
        for row in y.min(b)..b {
            self.pixels[row * self.width + x.min(r)..row * self.width + r].fill(color);
        }
    }
}

impl Canvas<'_> {
    /// Text with the built-in font, each font pixel drawn as `scale` x `scale` pixels. Returns the
    /// width drawn. A glyph is `GLYPH_W` x `GLYPH_H` font pixels plus one column of space.
    pub fn text(&mut self, x: usize, y: usize, s: &str, color: u32, scale: usize) -> usize {
        let advance = (GLYPH_W + 1) * scale;
        for (i, c) in s.chars().enumerate() {
            let x0 = x + i * advance;
            for (gy, bits) in font::glyph(c).iter().enumerate() {
                for gx in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - gx)) != 0 {
                        self.fill(x0 + gx * scale, y + gy * scale, scale, scale, color);
                    }
                }
            }
        }
        s.chars().count() * advance
    }
}

pub const KEY_UP: char = '\u{F700}';
pub const KEY_DOWN: char = '\u{F701}';
pub const KEY_LEFT: char = '\u{F702}';
pub const KEY_RIGHT: char = '\u{F703}';
pub const KEY_DELETE: char = '\u{F728}';
pub const KEY_HOME: char = '\u{F729}';
pub const KEY_END: char = '\u{F72B}';

/// Something that happened to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key press (Enter = '\n', Backspace = '\u{8}', Escape = '\u{1b}', and the `KEY_*` characters).
    Key(char),
    /// A letter pressed with Ctrl (lower case).
    Ctrl(char),
    /// A click at (x, y) inside the window; bit 0 of `buttons` = left.
    Click { x: usize, y: usize, buttons: u8 },
}

pub struct Window {
    pub id: u64,
    pub width: usize,
    pub height: usize,
}

impl Window {
    /// Opens a window whose contents are `width` x `height`. Returns the negative error if it fails.
    pub fn open(title: &str, width: usize, height: usize) -> Result<Window, i64> {
        let size = width as u64 | (height as u64) << 32;
        let id = syscall3(SYS_WINDOW_CREATE, size, title.as_ptr() as u64, title.len() as u64);
        if id < 0 { Err(id) } else { Ok(Window { id: id as u64, width, height }) }
    }

    /// Shows the area (x, y, w, h) of `canvas` (a picture of the whole window) at the same place.
    pub fn present(&self, canvas: &Canvas, x: usize, y: usize, w: usize, h: usize) -> i64 {
        let (x, y) = (x.min(canvas.width), y.min(canvas.height));
        let (w, h) = (w.min(canvas.width - x), h.min(canvas.height - y));
        self.raw_present(&canvas.pixels[y * canvas.width + x..], canvas.width, x, y, w, h)
    }

    /// Shows the whole canvas.
    pub fn present_all(&self, canvas: &Canvas) -> i64 {
        self.present(canvas, 0, 0, canvas.width, canvas.height)
    }

    /// Shows `canvas` (any size, e.g. a toolbar) with its top-left corner at (x, y) in the window.
    pub fn draw(&self, canvas: &Canvas, x: usize, y: usize) -> i64 {
        let (w, h) = (canvas.width.min(self.width.saturating_sub(x)), canvas.height.min(self.height.saturating_sub(y)));
        self.raw_present(canvas.pixels, canvas.width, x, y, w, h)
    }

    /// Fills the area (x, y, w, h) of the window with one color.
    pub fn fill(&self, x: usize, y: usize, w: usize, h: usize, color: u32) -> i64 {
        let row = [color; 256];
        let mut done = 0;
        while done < w {
            let n = (w - done).min(row.len());
            let r = self.raw_present(&row, 0, x + done, y, n, h);
            if r < 0 {
                return r;
            }
            done += n;
        }
        0
    }

    fn raw_present(&self, pixels: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize) -> i64 {
        if w == 0 || h == 0 {
            return 0;
        }
        assert!(pixels.len() >= (h - 1) * stride + w, "present: imagine prea mica");
        let area = x as u64 | (y as u64) << 16 | (w as u64) << 32 | (h as u64) << 48;
        syscall4(SYS_WINDOW_PRESENT, self.id, pixels.as_ptr() as u64, area, stride as u64)
    }

    /// Opens the desktop: a window over the whole screen, always below the others, without a frame.
    pub fn open_desktop() -> Result<Window, i64> {
        let (width, height) = crate::screen_size().ok_or(-1i64)?;
        let id = syscall3(SYS_WINDOW_CREATE, 1 << 63, 0, 0);
        if id < 0 { Err(id) } else { Ok(Window { id: id as u64, width, height }) }
    }

    /// Waits for the next event; `None` once the window has been closed (by the user).
    pub fn wait_event(&self) -> Option<Event> {
        loop {
            match self.wait_event_for(0) {
                Ok(Some(e)) => return Some(e),
                Ok(None) => continue,
                Err(()) => return None,
            }
        }
    }

    /// Waits at most `ms` milliseconds: `Ok(None)` if nothing happened, `Err(())` once the window is closed.
    pub fn wait_event_for(&self, ms: u64) -> Result<Option<Event>, ()> {
        let r = syscall3(SYS_EVENT_WAIT, self.id, ms, 0);
        if r < 0 {
            return Err(());
        }
        if r == 0 {
            return Ok(None);
        }
        let r = r as u64;
        Ok(Some(match r >> 48 {
            1 => Event::Key(char::from_u32((r & 0xFFFF_FFFF) as u32).unwrap_or('?')),
            3 => Event::Ctrl(char::from_u32((r & 0xFFFF_FFFF) as u32).unwrap_or('?')),
            _ => Event::Click { x: ((r >> 16) & 0xFFFF) as usize, y: (r & 0xFFFF) as usize, buttons: (r >> 32) as u8 },
        }))
    }

    pub fn close(self) {
        syscall3(SYS_WINDOW_CLOSE, self.id, 0, 0);
    }
}
