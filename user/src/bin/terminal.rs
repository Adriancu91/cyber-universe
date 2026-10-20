//! Terminal: the shell (`help`, `ls`, `cat`, `run`, `mem`...) in a window. Programs started from it
//! write here too. `exit` (or closing the window) ends it. Every finished line is also copied to the
//! serial port with the prefix `terminal: ` (for tests and debugging).
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::{error_text, println, term};

cyber_user::entry!(main);

const W: usize = 640;
const H: usize = 400;
const LINE_H: usize = 12;
const CHAR_W: usize = 6;
const COLS: usize = (W - 8) / CHAR_W;
const ROWS: usize = (H - 8) / LINE_H;

const BG: u32 = 0x00_10_18_28;
const TEXT: u32 = 0x00_D8_E6_F0;
const CURSOR: u32 = 0x00_40_D0_FF;

static mut PIXELS: [u32; W * H] = [0; W * H];

/// The last `ROWS` lines (bytes of UTF-8 text).
struct Screen {
    lines: [[u8; 4 * COLS]; ROWS],
    lens: [usize; ROWS],
    /// Characters on each line (for wrapping).
    chars: [usize; ROWS],
    /// Index of the current (last) line.
    cur: usize,
}

impl Screen {
    fn new_line(&mut self) {
        let line = core::str::from_utf8(&self.lines[self.cur][..self.lens[self.cur]]).unwrap_or("?");
        println!("terminal: {}", line);
        self.cur = (self.cur + 1) % ROWS;
        self.lens[self.cur] = 0;
        self.chars[self.cur] = 0;
    }

    fn put(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => self.new_line(),
                '\r' => {}
                '\u{8}' => {
                    // Erase the last character of the line.
                    let line = &self.lines[self.cur][..self.lens[self.cur]];
                    if let Some((i, _)) = core::str::from_utf8(line).unwrap_or("").char_indices().last() {
                        self.lens[self.cur] = i;
                        self.chars[self.cur] -= 1;
                    }
                }
                c => {
                    if self.chars[self.cur] == COLS {
                        self.new_line();
                    }
                    let mut b = [0u8; 4];
                    let s = c.encode_utf8(&mut b);
                    let at = self.lens[self.cur];
                    self.lines[self.cur][at..at + s.len()].copy_from_slice(s.as_bytes());
                    self.lens[self.cur] += s.len();
                    self.chars[self.cur] += 1;
                }
            }
        }
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        for row in 0..ROWS {
            let i = (self.cur + 1 + row) % ROWS;
            let text = core::str::from_utf8(&self.lines[i][..self.lens[i]]).unwrap_or("?");
            c.text(4, 4 + row * LINE_H, text, TEXT, 1);
        }
        let y = 4 + (ROWS - 1) * LINE_H;
        c.fill(4 + self.chars[self.cur] * CHAR_W, y, CHAR_W, LINE_H - 2, CURSOR);
        win.present_all(c);
    }
}

fn main() -> i64 {
    let win = match Window::open("Terminal", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("terminal: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    let id = term::open();
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let mut s = Screen { lines: [[0; 4 * COLS]; ROWS], lens: [0; ROWS], chars: [0; ROWS], cur: 0 };
    let mut buf = [0u8; 4096];
    let mut dirty = true;
    loop {
        // Show what the shell printed.
        loop {
            let n = term::read(id, &mut buf);
            if n < 0 {
                return 0; // the shell ended (`exit`)
            }
            if n == 0 {
                break;
            }
            s.put(core::str::from_utf8(&buf[..n as usize]).unwrap_or("?"));
            dirty = true;
        }
        if dirty {
            s.draw(&mut c, &win);
            dirty = false;
        }
        match win.wait_event_for(30) {
            Err(()) => return 0,
            Ok(Some(Event::Key(k))) if !('\u{F700}'..='\u{F8FF}').contains(&k) => {
                let mut b = [0u8; 4];
                term::write(id, k.encode_utf8(&mut b));
            }
            _ => {}
        }
    }
}
