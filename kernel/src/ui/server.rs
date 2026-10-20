//! Window server: a list of windows (bottom to top), each with its own pixel buffer, composed into one
//! screen-sized back buffer only where something changed, plus the mouse cursor. The left button raises
//! and focuses a window, drags it by its title bar and closes it with the button in the title bar;
//! key presses go to the focused (topmost) window. Clicks inside a window's contents and key presses are
//! queued as events for the window's program.

use crate::drivers::keyboard::{self, Key};
use crate::gfx::canvas::{Canvas, Rect};
use crate::input::{self, MouseEvent};
use alloc::{collections::VecDeque, string::String, vec::Vec};

pub const TITLE_H: usize = 22;
pub const BORDER: usize = 1;
const CLOSE: usize = 16;

pub const DESKTOP: u32 = 0x0018_2838;
const FRAME: u32 = 0x0060_7080;
pub const TITLE_FOCUSED: u32 = 0x0020_70C0;
pub const TITLE_IDLE: u32 = 0x0048_5868;
const TITLE_TEXT: u32 = 0x00FF_FFFF;
const CLOSE_BG: u32 = 0x00C0_3838;
pub const CLIENT_BG: u32 = 0x00F0_F0F0;

/// Arrow cursor: '#' = black outline, '.' = white inside, ' ' = transparent.
#[rustfmt::skip]
const CURSOR: [&str; 16] = [
    "#          ", "##         ", "#.#        ", "#..#       ", "#...#      ", "#....#     ", "#.....#    ", "#......#   ",
    "#.......#  ", "#........# ", "#.....#####", "#..#..#    ", "#.# #..#   ", "##  #..#   ", "#    #..#  ", "     ###   ",
];
const CURSOR_W: usize = 11;
const CURSOR_H: usize = 16;

pub type WindowId = u32;

/// Something that happened to a window, for the program that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key(Key),
    /// A button pressed inside the window's contents, at (x, y) from their top-left corner.
    Click { x: usize, y: usize, buttons: u8 },
}

pub struct Window {
    pub id: WindowId,
    /// Top-left corner of the frame on the screen.
    x: usize,
    y: usize,
    pub title: String,
    /// The window's contents (without frame and title bar); programs draw here.
    pub client: Canvas,
    events: VecDeque<Event>,
    /// Thread of the program that opened the window (0 = the kernel).
    pub owner: u64,
    /// The desktop: covers the screen, no frame, always below the other windows.
    pub background: bool,
}

impl Window {
    /// Frame, title bar and contents together, on the screen.
    pub fn outer(&self) -> Rect {
        if self.background {
            return Rect::new(self.x, self.y, self.client.width, self.client.height);
        }
        Rect::new(self.x, self.y, self.client.width + 2 * BORDER, self.client.height + TITLE_H + BORDER)
    }

    pub fn client_origin(&self) -> (usize, usize) {
        if self.background { (self.x, self.y) } else { (self.x + BORDER, self.y + TITLE_H) }
    }

    fn title_bar(&self) -> Rect {
        Rect::new(self.x, self.y, if self.background { 0 } else { self.outer().w }, TITLE_H)
    }

    fn close_button(&self) -> Rect {
        if self.background {
            return Rect::new(0, 0, 0, 0);
        }
        let o = self.outer();
        Rect::new(o.x + o.w - CLOSE - 3, o.y + (TITLE_H - CLOSE) / 2, CLOSE, CLOSE)
    }
}

pub struct Server {
    screen: Canvas,
    /// Bottom to top; the last one has the focus.
    windows: Vec<Window>,
    next_id: WindowId,
    cursor: (usize, usize),
    buttons: u8,
    /// Window being dragged, and where in its frame it was grabbed.
    drag: Option<(WindowId, usize, usize)>,
    /// Screen area to compose again.
    damage: Option<Rect>,
}

impl Server {
    /// Takes over the screen (text output goes only to the serial port from now on).
    pub fn new() -> Option<Server> {
        let (w, h) = crate::gfx::console::with_console(|slot| slot.as_mut().map(|c| (c.framebuffer().width, c.framebuffer().height)))?;
        crate::gfx::console::set_text_on_screen(false);
        Some(Server {
            screen: Canvas::new(w, h, DESKTOP),
            windows: Vec::new(),
            next_id: 1,
            cursor: (w / 2, h / 2),
            buttons: 0,
            drag: None,
            damage: Some(Rect::new(0, 0, w, h)),
        })
    }

    #[allow(dead_code)] // used by the self-tests now, by the desktop (P4-06)
    pub fn cursor(&self) -> (usize, usize) {
        self.cursor
    }

    fn add_damage(&mut self, r: Rect) {
        self.damage = Some(self.damage.map_or(r, |d| d.union(r)));
    }

    pub fn size(&self) -> (usize, usize) {
        (self.screen.width, self.screen.height)
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter()
    }

    /// Opens a window on top of the others (it gets the focus). `w` x `h` is the size of its contents.
    #[allow(dead_code)] // used by the self-tests now, by the desktop (P4-06)
    pub fn create(&mut self, title: &str, x: usize, y: usize, w: usize, h: usize) -> WindowId {
        self.create_for(0, title, x, y, w, h)
    }

    /// Like `create`, for the program on thread `owner`.
    pub fn create_for(&mut self, owner: u64, title: &str, x: usize, y: usize, w: usize, h: usize) -> WindowId {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(top) = self.windows.last() {
            self.add_damage(top.title_bar()); // loses the focus color
        }
        let win = Window { id, x, y, title: String::from(title), client: Canvas::new(w, h, CLIENT_BG), events: VecDeque::new(), owner, background: false };
        self.add_damage(win.outer());
        self.windows.push(win);
        id
    }

    /// Opens the desktop window for the program on thread `owner`: the whole screen, below every other
    /// window. `None` if there is one already.
    pub fn create_background(&mut self, owner: u64) -> Option<WindowId> {
        if self.windows.iter().any(|w| w.background) {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (w, h) = (self.screen.width, self.screen.height);
        let win = Window { id, x: 0, y: 0, title: String::new(), client: Canvas::new(w, h, DESKTOP), events: VecDeque::new(), owner, background: true };
        self.add_damage(win.outer());
        self.windows.insert(0, win);
        Some(id)
    }

    pub fn close(&mut self, id: WindowId) {
        if let Some(i) = self.index(id) {
            let win = self.windows.remove(i);
            self.add_damage(win.outer());
            if let Some(top) = self.windows.last() {
                self.add_damage(top.title_bar()); // gains the focus color
            }
            if self.drag.is_some_and(|(d, _, _)| d == id) {
                self.drag = None;
            }
        }
    }

    fn index(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    #[allow(dead_code)] // used by the self-tests now, by the desktop (P4-06)
    pub fn focused(&self) -> Option<WindowId> {
        self.windows.last().map(|w| w.id)
    }

    /// Next event for this window (key press or click).
    pub fn next_event(&mut self, id: WindowId) -> Option<Event> {
        self.window_mut(id)?.events.pop_front()
    }

    fn cursor_rect(&self) -> Rect {
        Rect::new(self.cursor.0, self.cursor.1, CURSOR_W, CURSOR_H)
    }

    /// Moves the cursor and reacts to the left button: raise and focus, close, start or end a drag.
    pub fn handle_mouse(&mut self, e: MouseEvent) {
        let old = self.cursor_rect();
        let (x, y) = (self.cursor.0 as isize + e.dx as isize, self.cursor.1 as isize + e.dy as isize);
        self.cursor = (x.clamp(0, self.screen.width as isize - 1) as usize, y.clamp(0, self.screen.height as isize - 1) as usize);
        let moved = old != self.cursor_rect();
        if moved {
            self.add_damage(old);
            self.add_damage(self.cursor_rect());
        }

        let (pressed, released) = (e.buttons & 1 != 0 && self.buttons & 1 == 0, e.buttons & 1 == 0 && self.buttons & 1 != 0);
        self.buttons = e.buttons;
        let (cx, cy) = self.cursor;
        if pressed {
            if let Some(mut i) = self.windows.iter().rposition(|w| w.outer().contains(cx, cy)) {
                if i + 1 != self.windows.len() && !self.windows[i].background {
                    let previous = self.windows.last().map(|w| w.title_bar());
                    let win = self.windows.remove(i);
                    self.add_damage(win.outer());
                    self.windows.push(win);
                    if let Some(r) = previous {
                        self.add_damage(r);
                    }
                    i = self.windows.len() - 1;
                }
                let top = &mut self.windows[i];
                let id = top.id;
                let (ox, oy) = top.client_origin();
                if top.close_button().contains(cx, cy) {
                    self.close(id);
                } else if top.title_bar().contains(cx, cy) {
                    self.drag = Some((id, cx - top.x, cy - top.y));
                } else if Rect::new(ox, oy, top.client.width, top.client.height).contains(cx, cy) {
                    top.events.push_back(Event::Click { x: cx - ox, y: cy - oy, buttons: e.buttons });
                }
            }
        }
        if released {
            self.drag = None;
        }
        if let (Some((id, ox, oy)), true) = (self.drag, moved) {
            let (max_x, max_y) = (self.screen.width.saturating_sub(40), self.screen.height.saturating_sub(TITLE_H));
            let (nx, ny) = (cx.saturating_sub(ox).min(max_x), cy.saturating_sub(oy).min(max_y));
            if let Some(i) = self.index(id) {
                let old = self.windows[i].outer();
                if (nx, ny) != (old.x, old.y) {
                    self.windows[i].x = nx;
                    self.windows[i].y = ny;
                    let new = self.windows[i].outer();
                    self.add_damage(old);
                    self.add_damage(new);
                }
            }
        }
    }

    /// Key presses go to the focused window.
    pub fn handle_key(&mut self, k: Key) {
        if let Some(top) = self.windows.last_mut() {
            top.events.push_back(Event::Key(k));
        }
    }

    /// Handles every waiting mouse and keyboard event, then updates the screen. Returns whether there was any.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Some(e) = input::read_mouse() {
            self.handle_mouse(e);
            any = true;
        }
        while let Some(k) = keyboard::read_key() {
            self.handle_key(k);
            any = true;
        }
        self.present();
        any
    }

    /// Composes the changed areas (moved windows, cursor, window contents) and copies them to the screen.
    pub fn present(&mut self) {
        for i in 0..self.windows.len() {
            if let Some(r) = self.windows[i].client.take_dirty() {
                let (ox, oy) = self.windows[i].client_origin();
                self.add_damage(Rect::new(ox + r.x, oy + r.y, r.w, r.h));
            }
        }
        if let Some(d) = self.damage.take() {
            self.compose(d);
        }
        let screen = &mut self.screen;
        crate::gfx::console::with_console(|slot| {
            if let Some(c) = slot {
                screen.present(c.framebuffer(), 0, 0);
            }
        });
    }

    /// Redraws `area` of the back buffer: desktop, windows from bottom to top, cursor.
    fn compose(&mut self, area: Rect) {
        let s = &mut self.screen;
        s.set_clip(Some(area));
        s.fill(area, DESKTOP);
        let top = self.windows.len().saturating_sub(1);
        for (i, w) in self.windows.iter().enumerate() {
            let o = w.outer();
            if o.intersect(area).is_none() {
                continue;
            }
            if w.background {
                s.copy(o.x, o.y, o.w, o.h, w.client.pixels());
                continue;
            }
            s.outline(o, BORDER, FRAME);
            let bar = w.title_bar();
            s.fill(Rect::new(bar.x + BORDER, bar.y + BORDER, bar.w - 2 * BORDER, bar.h - BORDER), if i == top { TITLE_FOCUSED } else { TITLE_IDLE });
            s.text(bar.x + 8, bar.y + (TITLE_H - 10) / 2 + 1, &w.title, TITLE_TEXT, 1);
            let c = w.close_button();
            s.fill(c, CLOSE_BG);
            for d in 4..CLOSE - 4 {
                s.put(c.x + d, c.y + d, TITLE_TEXT);
                s.put(c.x + CLOSE - 1 - d, c.y + d, TITLE_TEXT);
            }
            let (cx, cy) = w.client_origin();
            s.copy(cx, cy, w.client.width, w.client.height, w.client.pixels());
        }
        for (row, line) in CURSOR.iter().enumerate() {
            for (col, b) in line.bytes().enumerate() {
                match b {
                    b'#' => s.put(self.cursor.0 + col, self.cursor.1 + row, 0x000000),
                    b'.' => s.put(self.cursor.0 + col, self.cursor.1 + row, 0xFFFFFF),
                    _ => {}
                }
            }
        }
        s.set_clip(None);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        crate::gfx::console::set_text_on_screen(true);
    }
}
