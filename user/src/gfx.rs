//! Windows for programs: open one, draw in a picture kept in the program's memory, show it, wait for
//! keys and clicks. See kernel/src/syscall/README.md (calls 8-11).

use crate::{SYS_EVENT_WAIT, SYS_WINDOW_CLOSE, SYS_WINDOW_CREATE, SYS_WINDOW_PRESENT, syscall3};

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

/// Something that happened to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key press (Enter = '\n', Backspace = '\u{8}', Escape = '\u{1b}').
    Key(char),
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

    /// Shows the area (x, y, w, h) of `canvas` (cut to it), which must be the window's size.
    pub fn present(&self, canvas: &Canvas, x: usize, y: usize, w: usize, h: usize) -> i64 {
        let (x, y) = (x.min(canvas.width), y.min(canvas.height));
        let (w, h) = (w.min(canvas.width - x), h.min(canvas.height - y));
        let area = x as u64 | (y as u64) << 16 | (w as u64) << 32 | (h as u64) << 48;
        syscall3(SYS_WINDOW_PRESENT, self.id, canvas.pixels.as_ptr() as u64, area)
    }

    /// Shows the whole canvas.
    pub fn present_all(&self, canvas: &Canvas) -> i64 {
        self.present(canvas, 0, 0, canvas.width, canvas.height)
    }

    /// Waits for the next event; `None` once the window has been closed (by the user).
    pub fn wait_event(&self) -> Option<Event> {
        let r = syscall3(SYS_EVENT_WAIT, self.id, 0, 0);
        if r < 0 {
            return None;
        }
        let r = r as u64;
        match r >> 48 {
            1 => Some(Event::Key(char::from_u32((r & 0xFFFF_FFFF) as u32).unwrap_or('?'))),
            _ => Some(Event::Click { x: ((r >> 16) & 0xFFFF) as usize, y: (r & 0xFFFF) as usize, buttons: (r >> 32) as u8 }),
        }
    }

    pub fn close(self) {
        syscall3(SYS_WINDOW_CLOSE, self.id, 0, 0);
    }
}
