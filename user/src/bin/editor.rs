//! Text editor: opens the file given as the first argument (or starts an empty one), lets you write
//! and move with the arrows, Home, End, Backspace and Delete; Ctrl+S saves, Ctrl+Q quits (twice if
//! there are unsaved changes). Files are UTF-8; the keyboard types ASCII for now (P4-12: Romanian).
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, KEY_DELETE, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_RIGHT, KEY_UP, Window};
use cyber_user::text::Line;
use cyber_user::{close, create, error_text, open, println, read, remove, write};

cyber_user::entry!(main);

const W: usize = 560;
const H: usize = 380;
const TOP: usize = 24;
const LINE_H: usize = 22;
const CHAR_W: usize = 12;
const ROWS: usize = (H - TOP - 20) / LINE_H;
const COLS: usize = (W - 8) / CHAR_W;
const MAX: usize = 64 * 1024;

const BG: u32 = 0x00_FF_FF_FF;
const BAR: u32 = 0x00_E8_EC_F0;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const CURSOR: u32 = 0x00_20_70_C0;

static mut PIXELS: [u32; W * H] = [0; W * H];
static mut TEXT_BUF: [u8; MAX] = [0; MAX];

struct Editor {
    text: &'static mut [u8; MAX],
    len: usize,
    /// Byte position of the cursor (always at a character boundary).
    cursor: usize,
    top: usize,
    left: usize,
    modified: bool,
    quit_warned: bool,
    path: [u8; 256],
    path_len: usize,
    status: Line,
}

impl Editor {
    fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len]).unwrap_or("")
    }

    fn line_start(&self, pos: usize) -> usize {
        self.text[..pos].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, pos: usize) -> usize {
        self.text[pos..self.len].iter().position(|&b| b == b'\n').map_or(self.len, |i| pos + i)
    }

    /// Characters between the start of the line and `pos`.
    fn column(&self, pos: usize) -> usize {
        let start = self.line_start(pos);
        self.text[start..pos].iter().filter(|&&b| b & 0xC0 != 0x80).count()
    }

    /// The position `col` characters into the line that starts at `start` (or its end).
    fn at_column(&self, start: usize, col: usize) -> usize {
        let end = self.line_end(start);
        let mut pos = start;
        let mut c = 0;
        while pos < end && c < col {
            pos = self.next(pos);
            c += 1;
        }
        pos
    }

    fn next(&self, pos: usize) -> usize {
        let mut p = (pos + 1).min(self.len);
        while p < self.len && self.text[p] & 0xC0 == 0x80 {
            p += 1;
        }
        p
    }

    fn prev(&self, pos: usize) -> usize {
        let mut p = pos.saturating_sub(1);
        while p > 0 && self.text[p] & 0xC0 == 0x80 {
            p -= 1;
        }
        p
    }

    fn insert(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > MAX {
            self.status = Line::new();
            self.status.push("fisierul e prea mare (maximum 64 KiB)");
            return;
        }
        self.text.copy_within(self.cursor..self.len, self.cursor + bytes.len());
        self.text[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        self.cursor += bytes.len();
        self.modified = true;
    }

    fn delete_range(&mut self, from: usize, to: usize) {
        if from < to {
            self.text.copy_within(to..self.len, from);
            self.len -= to - from;
            self.cursor = from;
            self.modified = true;
        }
    }

    fn line_index(&self, pos: usize) -> usize {
        self.text[..pos].iter().filter(|&&b| b == b'\n').count()
    }

    fn key(&mut self, k: char) {
        match k {
            KEY_LEFT => self.cursor = self.prev(self.cursor),
            KEY_RIGHT => self.cursor = self.next(self.cursor),
            KEY_HOME => self.cursor = self.line_start(self.cursor),
            KEY_END => self.cursor = self.line_end(self.cursor),
            KEY_UP | KEY_DOWN => {
                let col = self.column(self.cursor);
                let start = self.line_start(self.cursor);
                if k == KEY_UP && start > 0 {
                    self.cursor = self.at_column(self.line_start(start - 1), col);
                } else if k == KEY_DOWN {
                    let end = self.line_end(self.cursor);
                    if end < self.len {
                        self.cursor = self.at_column(end + 1, col);
                    }
                }
            }
            '\u{8}' => {
                let p = self.prev(self.cursor);
                self.delete_range(p, self.cursor);
            }
            KEY_DELETE => {
                let n = self.next(self.cursor);
                self.delete_range(self.cursor, n);
            }
            '\n' => self.insert(b"\n"),
            '\t' => self.insert(b"    "),
            c if !c.is_control() && !('\u{F700}'..='\u{F8FF}').contains(&c) => {
                let mut b = [0u8; 4];
                self.insert(c.encode_utf8(&mut b).as_bytes());
            }
            _ => {}
        }
        self.quit_warned = false;
        // Keep the cursor visible.
        let (line, col) = (self.line_index(self.cursor), self.column(self.cursor));
        if line < self.top {
            self.top = line;
        } else if line >= self.top + ROWS {
            self.top = line + 1 - ROWS;
        }
        if col < self.left {
            self.left = col;
        } else if col >= self.left + COLS {
            self.left = col + 1 - COLS;
        }
    }

    fn load(&mut self) {
        let fd = open(self.path());
        if fd < 0 {
            self.status.push("fisier nou");
            return;
        }
        loop {
            let n = read(fd, &mut self.text[self.len..]);
            if n <= 0 {
                break;
            }
            self.len += n as usize;
        }
        close(fd);
        if self.len == MAX {
            self.status.push("atentie: s-au citit doar primii 64 KiB");
        }
    }

    fn save(&mut self) {
        let path = self.path;
        let p = core::str::from_utf8(&path[..self.path_len]).unwrap_or("");
        let r = remove(p);
        let r = if r < 0 && r != -3 { r } else { create(p, false) };
        let fd = if r < 0 { r } else { open(p) };
        self.status = Line::new();
        if fd < 0 {
            self.status.push("nu pot salva: ").push(error_text(fd));
            return;
        }
        let mut done = 0;
        while done < self.len {
            let n = write(fd as u64, &self.text[done..self.len.min(done + 16 * 1024)]);
            if n <= 0 {
                break;
            }
            done += n as usize;
        }
        close(fd);
        if done == self.len {
            self.modified = false;
            self.status.push("salvat");
            println!("editor: salvat {} ({} octeti)", p, self.len);
        } else {
            self.status.push("salvare incompleta");
        }
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        c.fill(0, 0, W, TOP - 4, BAR);
        let mut title = Line::new();
        title.push(if self.path_len == 0 { "(fara nume)" } else { self.path() });
        if self.modified {
            title.push(" *");
        }
        c.text(6, 6, title.as_str(), TEXT, 1);
        let (cur_line, cur_col) = (self.line_index(self.cursor), self.column(self.cursor));
        let mut start = 0;
        let mut line = 0;
        while start <= self.len && line < self.top + ROWS {
            let end = self.line_end(start);
            if line >= self.top {
                let y = TOP + (line - self.top) * LINE_H;
                let text = core::str::from_utf8(&self.text[start..end]).unwrap_or("?");
                let visible: &str = match text.char_indices().nth(self.left) {
                    Some((i, _)) => &text[i..],
                    None => "",
                };
                let shown = match visible.char_indices().nth(COLS) {
                    Some((i, _)) => &visible[..i],
                    None => visible,
                };
                c.text(4, y, shown, TEXT, 2);
                if line == cur_line {
                    c.fill(4 + (cur_col - self.left) * CHAR_W, y, 2, LINE_H - 2, CURSOR);
                }
            }
            if end >= self.len {
                break;
            }
            start = end + 1;
            line += 1;
        }
        let mut s = Line::new();
        s.push("Ctrl+S salveaza  Ctrl+Q iese   rand ").num(cur_line as u64 + 1).push(", col ").num(cur_col as u64 + 1);
        s.push("   ").push(self.status.as_str());
        c.fill(0, H - 16, W, 16, BAR);
        c.text(6, H - 13, s.as_str(), DIM, 1);
        win.present_all(c);
    }
}

fn main() -> i64 {
    let arg = cyber_user::args().nth(1).unwrap_or("");
    // SAFETY: one thread; these are the only references to the two buffers.
    let (pixels, text) = unsafe { (&mut *core::ptr::addr_of_mut!(PIXELS), &mut *core::ptr::addr_of_mut!(TEXT_BUF)) };
    let mut e = Editor { text, len: 0, cursor: 0, top: 0, left: 0, modified: false, quit_warned: false, path: [0; 256], path_len: 0, status: Line::new() };
    e.path_len = arg.len().min(256);
    e.path[..e.path_len].copy_from_slice(&arg.as_bytes()[..e.path_len]);
    if e.path_len > 0 {
        e.load();
    }
    let win = match Window::open("Editor", W, H) {
        Ok(w) => w,
        Err(err) => {
            println!("editor: nu pot deschide fereastra ({})", error_text(err));
            return 1;
        }
    };
    let mut c = Canvas::new(W, H, pixels);
    println!("editor: deschis {} ({} octeti)", e.path(), e.len);
    e.draw(&mut c, &win);
    while let Some(ev) = win.wait_event() {
        match ev {
            Event::Ctrl('s') => {
                if e.path_len == 0 {
                    e.status = Line::new();
                    e.status.push("fara nume: porneste editorul cu o cale");
                } else {
                    e.save();
                }
            }
            Event::Ctrl('q') => {
                if !e.modified || e.quit_warned {
                    break;
                }
                e.quit_warned = true;
                e.status = Line::new();
                e.status.push("modificari nesalvate! Ctrl+Q din nou ca sa iesi");
            }
            Event::Key(k) => e.key(k),
            _ => {}
        }
        e.draw(&mut c, &win);
    }
    0
}
