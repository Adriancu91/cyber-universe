//! File manager: browse folders, open (folders, programs; other files in the editor), copy, paste and
//! delete files.
//! Mouse: click selects; the buttons at the top act on the selection. Keys: Enter = open,
//! Backspace = up one folder, j / k = next / previous, c = copy, v = paste, s = delete, q = quit.
//! The first argument is the folder to start in (default `/`).
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::text::Line;
use cyber_user::{close, create, error_text, open, println, read, remove, spawn, write};

cyber_user::entry!(main);

const W: usize = 480;
const H: usize = 360;
const BAR_H: usize = 28;
const LIST_Y: usize = 48;
const ROW_H: usize = 20;
const ROWS: usize = (H - LIST_Y - 18) / ROW_H;
const MAX: usize = 128;
const NAME: usize = 64;
const PATH: usize = 256;

const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const BUTTON: u32 = 0x00_20_70_C0;
const WHITE: u32 = 0x00_FF_FF_FF;
const SELECTED: u32 = 0x00_C8_DC_F0;
const FOLDER: u32 = 0x00_C0_80_10;

/// Toolbar: (label, key it stands for).
const BUTTONS: [(&str, char); 5] = [("Sus", '\u{8}'), ("Deschide", '\n'), ("Copiaza", 'c'), ("Lipeste", 'v'), ("Sterge", 's')];

static mut PIXELS: [u32; W * H] = [0; W * H];

#[derive(Clone, Copy)]
struct Entry {
    name: [u8; NAME],
    len: usize,
    dir: bool,
}

impl Entry {
    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.len]).unwrap_or("?")
    }
}

/// A path without a heap.
#[derive(Clone, Copy)]
struct Path {
    buf: [u8; PATH],
    len: usize,
}

impl Path {
    fn new(s: &str) -> Path {
        let mut p = Path { buf: [0; PATH], len: 0 };
        p.set(s);
        p
    }
    fn set(&mut self, s: &str) {
        self.len = s.len().min(PATH);
        self.buf[..self.len].copy_from_slice(&s.as_bytes()[..self.len]);
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("/")
    }
    /// This folder + `/name`.
    fn join(&self, name: &str) -> Path {
        let mut p = *self;
        if !p.as_str().ends_with('/') && p.len < PATH {
            p.buf[p.len] = b'/';
            p.len += 1;
        }
        let n = name.len().min(PATH - p.len);
        p.buf[p.len..p.len + n].copy_from_slice(&name.as_bytes()[..n]);
        p.len += n;
        p
    }
    fn parent(&self) -> Path {
        let s = self.as_str();
        match s.rfind('/') {
            Some(0) | None => Path::new("/"),
            Some(i) => Path::new(&s[..i]),
        }
    }
    fn file_name(&self) -> &str {
        self.as_str().rsplit('/').next().unwrap_or("")
    }
}

struct Files {
    cwd: Path,
    entries: [Entry; MAX],
    count: usize,
    selected: usize,
    top: usize,
    clipboard: Option<Path>,
    status: Line,
}

impl Files {
    fn load(&mut self) {
        let mut buf = [0u8; 4096];
        let n = cyber_user::list(self.cwd.as_str(), &mut buf);
        self.count = 0;
        if self.cwd.as_str() != "/" {
            self.add("..", true);
        }
        if n < 0 {
            self.say("nu pot citi directorul: ").push(error_text(n));
            return;
        }
        for line in buf[..n as usize].split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let s = core::str::from_utf8(line).unwrap_or("?");
            match s.strip_suffix('/') {
                Some(d) => self.add(d, true),
                None => self.add(s, false),
            }
        }
        // Folders first, then files, each by name without regard to case (".." stays first).
        let first = if self.cwd.as_str() != "/" { 1 } else { 0 };
        let key = |e: &Entry| (!e.dir, e.name);
        for i in first + 1..self.count {
            let mut j = i;
            while j > first && lower_key(key(&self.entries[j])) < lower_key(key(&self.entries[j - 1])) {
                self.entries.swap(j, j - 1);
                j -= 1;
            }
        }
        self.selected = 0;
        self.top = 0;
        println!("files: {} ({} intrari)", self.cwd.as_str(), self.count - first);
    }

    fn add(&mut self, name: &str, dir: bool) {
        if self.count == MAX {
            return;
        }
        let len = name.len().min(NAME);
        let mut e = Entry { name: [0; NAME], len, dir };
        e.name[..len].copy_from_slice(&name.as_bytes()[..len]);
        self.entries[self.count] = e;
        self.count += 1;
    }

    fn say(&mut self, s: &str) -> &mut Line {
        self.status = Line::new();
        self.status.push(s);
        &mut self.status
    }

    fn selected_path(&self) -> Option<(Path, bool)> {
        let e = self.entries.get(self.selected).filter(|_| self.selected < self.count)?;
        if e.name() == ".." {
            return None;
        }
        Some((self.cwd.join(e.name()), e.dir))
    }

    fn key(&mut self, k: char) -> bool {
        match k {
            'q' | '\u{1b}' => return false,
            '\u{8}' => {
                self.cwd = self.cwd.parent();
                self.load();
            }
            '\n' => self.open_selected(),
            'j' => self.selected = (self.selected + 1).min(self.count.saturating_sub(1)),
            'k' => self.selected = self.selected.saturating_sub(1),
            'c' => match self.selected_path() {
                Some((p, false)) => {
                    self.clipboard = Some(p);
                    self.say("copiat: ").push(p.as_str());
                }
                _ => {
                    self.say("alege un fisier (directoarele nu se copiaza)");
                }
            },
            'v' => self.paste(),
            's' => self.delete(),
            _ => {}
        }
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
        true
    }

    fn open_selected(&mut self) {
        let Some(e) = self.entries.get(self.selected).copied().filter(|_| self.selected < self.count) else { return };
        if e.name() == ".." {
            self.cwd = self.cwd.parent();
            self.load();
        } else if e.dir {
            self.cwd = self.cwd.join(e.name());
            self.load();
        } else {
            let p = self.cwd.join(e.name());
            if is_program(p.as_str()) {
                let r = spawn(p.as_str());
                if r < 0 {
                    self.say("nu porneste: ").push(error_text(r));
                } else {
                    self.say("pornit: ").push(p.as_str());
                }
            } else {
                // Everything else opens in the text editor.
                let r = cyber_user::spawn_with("/bin/editor", p.as_str());
                if r < 0 {
                    self.say("editorul nu porneste: ").push(error_text(r));
                } else {
                    self.say("deschis in editor: ").push(p.as_str());
                }
            }
        }
    }

    fn paste(&mut self) {
        let Some(src) = self.clipboard else {
            self.say("nimic copiat");
            return;
        };
        let dst = self.cwd.join(src.file_name());
        match copy(src.as_str(), dst.as_str()) {
            Ok(n) => {
                println!("files: copiat {} -> {} ({} octeti)", src.as_str(), dst.as_str(), n);
                self.load();
                self.say("lipit: ").push(dst.as_str());
            }
            Err(e) => {
                self.say("nu pot lipi: ").push(error_text(e));
            }
        }
    }

    fn delete(&mut self) {
        let Some((p, _)) = self.selected_path() else { return };
        let r = remove(p.as_str());
        if r < 0 {
            self.say("nu pot sterge: ").push(error_text(r));
        } else {
            println!("files: sters {}", p.as_str());
            self.load();
            self.say("sters: ").push(p.as_str());
        }
    }

    fn click(&mut self, x: usize, y: usize) -> bool {
        if y < BAR_H {
            let mut bx = 4;
            for (label, k) in BUTTONS {
                let w = label.len() * 12 + 12;
                if (bx..bx + w).contains(&x) {
                    return self.key(k);
                }
                bx += w + 4;
            }
        } else if y >= LIST_Y {
            let i = self.top + (y - LIST_Y) / ROW_H;
            if i < self.count && (y - LIST_Y) / ROW_H < ROWS {
                self.selected = i;
            }
        }
        true
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        let mut bx = 4;
        for (label, _) in BUTTONS {
            let w = label.len() * 12 + 12;
            c.fill(bx, 4, w, BAR_H - 6, BUTTON);
            c.text(bx + 6, 6, label, WHITE, 2);
            bx += w + 4;
        }
        c.text(6, 34, self.cwd.as_str(), DIM, 1);
        for (row, i) in (self.top..self.count).take(ROWS).enumerate() {
            let e = &self.entries[i];
            let y = LIST_Y + row * ROW_H;
            if i == self.selected {
                c.fill(0, y, W, ROW_H, SELECTED);
            }
            if e.dir {
                c.fill(8, y + 5, 12, 10, FOLDER);
            }
            c.text(28, y + 4, e.name(), TEXT, 1);
            if e.dir && e.name() != ".." {
                c.text(28 + e.len * 6, y + 4, "/", TEXT, 1);
            }
        }
        c.text(6, H - 14, self.status.as_str(), DIM, 1);
        win.present_all(c);
    }
}

/// Sort key: folders first, then the name in lower case.
fn lower_key((file, name): (bool, [u8; NAME])) -> (bool, [u8; NAME]) {
    (file, name.map(|b| b.to_ascii_lowercase()))
}

/// Programs are files in /bin, or ELF files.
fn is_program(path: &str) -> bool {
    if path.starts_with("/bin/") {
        return true;
    }
    let fd = open(path);
    if fd < 0 {
        return false;
    }
    let mut magic = [0u8; 4];
    let n = read(fd, &mut magic);
    close(fd);
    n == 4 && magic == *b"\x7fELF"
}

/// Copies a file; returns the bytes copied or a negative error.
fn copy(src: &str, dst: &str) -> Result<u64, i64> {
    let r = create(dst, false);
    if r < 0 {
        return Err(r);
    }
    let (from, to) = (open(src), open(dst));
    if from < 0 || to < 0 {
        close(from);
        close(to);
        return Err(if from < 0 { from } else { to });
    }
    let mut buf = [0u8; 4096];
    let mut total = 0u64;
    let result = loop {
        let n = read(from, &mut buf);
        if n <= 0 {
            break if n < 0 { Err(n) } else { Ok(total) };
        }
        let w = write(to as u64, &buf[..n as usize]);
        if w != n {
            break Err(if w < 0 { w } else { -10 });
        }
        total += n as u64;
    };
    close(from);
    close(to);
    result
}

fn main() -> i64 {
    let start = cyber_user::args().nth(1).unwrap_or("/");
    let win = match Window::open("Fisiere", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("files: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let empty = Entry { name: [0; NAME], len: 0, dir: false };
    let mut f = Files { cwd: Path::new(start), entries: [empty; MAX], count: 0, selected: 0, top: 0, clipboard: None, status: Line::new() };
    f.load();
    f.draw(&mut c, &win);
    while let Some(e) = win.wait_event() {
        let go_on = match e {
            Event::Key(k) => f.key(k),
            Event::Click { x, y, buttons } if buttons & 1 != 0 => f.click(x, y),
            Event::Click { .. } | Event::Ctrl(_) => true,
        };
        if !go_on {
            break;
        }
        f.draw(&mut c, &win);
    }
    0
}
