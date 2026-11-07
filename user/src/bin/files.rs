//! File manager: browse folders, open (folders, programs; other files in the editor), copy, paste and
//! delete files and whole folders, make a folder, rename. Made for the stick (P6-61, C-84 item 7): the
//! "Stick" button goes to /usb, every change on the stick is written out at once (so it can be pulled
//! out), sizes are shown next to the files.
//! Mouse: click selects; the buttons at the top act on the selection. Keys: Enter = open,
//! Backspace = up one folder, j / k = next / previous, c = copy, v = paste, s = delete (a folder that
//! is not empty: press s a second time), n = new folder, r = rename (type the name, Enter; Esc gives up),
//! u = the stick, q = quit.
//! The first argument is the folder to start in (default: /usb if there is a stick, otherwise `/`).
//! Writing goes through the kernel's rules: only disks marked `cyber.rw` (never the computer's own disks).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::{close, create, error_text, open, println, read, remove, rename, spawn, stat, sync, write};

cyber_user::entry!(main);
cyber_user::heap!(1024 * 1024);

const W: usize = 680;
const H: usize = 360;
const BAR_H: usize = 28;
const LIST_Y: usize = 48;
const ROW_H: usize = 20;
const ROWS: usize = (H - LIST_Y - 18) / ROW_H;
/// Folders deeper than this are not copied or deleted (a loop guard; the stick's folders are shallow).
const MAX_DEPTH: usize = 8;
const STICK: &str = "/usb";

const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const BUTTON: u32 = 0x00_20_70_C0;
const WHITE: u32 = 0x00_FF_FF_FF;
const SELECTED: u32 = 0x00_C8_DC_F0;
const FOLDER: u32 = 0x00_C0_80_10;
const INPUT: u32 = 0x00_FF_F4_C0;

/// Toolbar: (label, key it stands for).
const BUTTONS: [(&str, char); 8] = [
    ("Sus", '\u{8}'),
    ("Deschide", '\n'),
    ("Copiaza", 'c'),
    ("Lipeste", 'v'),
    ("Sterge", 's'),
    ("Nou", 'n'),
    ("Nume", 'r'),
    ("Stick", 'u'),
];

static mut PIXELS: [u32; W * H] = [0; W * H];

struct Entry {
    name: String,
    dir: bool,
    /// Bytes (files only).
    size: u64,
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{}{}", dir, name) } else { format!("{}/{}", dir, name) }
}

fn parent(p: &str) -> String {
    match p.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => p[..i].to_string(),
    }
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or("")
}

/// The names in `dir`: (name, is a folder), or a negative error.
fn list_dir(dir: &str) -> Result<Vec<(String, bool)>, i64> {
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let n = cyber_user::list(dir, &mut buf);
    if n < 0 {
        return Err(n);
    }
    Ok(buf[..n as usize]
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let s = core::str::from_utf8(l).unwrap_or("?");
            match s.strip_suffix('/') {
                Some(d) => (d.to_string(), true),
                None => (s.to_string(), false),
            }
        })
        .collect())
}

fn is_dir(p: &str) -> bool {
    stat(p) == cyber_user::E_IS_DIR
}

/// "12 B", "3 KB", "4.2 MB".
fn human(n: u64) -> String {
    if n < 1024 {
        format!("{} B", n)
    } else if n < 1024 * 1024 {
        format!("{} KB", n.div_ceil(1024))
    } else {
        let t = n * 10 / (1024 * 1024);
        format!("{}.{} MB", t / 10, t % 10)
    }
}

/// What a copy or delete of a folder did.
#[derive(Default)]
struct Count {
    files: u64,
    dirs: u64,
    bytes: u64,
}

struct Files {
    cwd: String,
    entries: Vec<Entry>,
    selected: usize,
    top: usize,
    clipboard: Option<(String, bool)>,
    status: String,
    /// Rename in progress: the text typed so far.
    input: Option<String>,
    /// The folder waiting for a second `s`.
    confirm_delete: Option<String>,
}

impl Files {
    fn load(&mut self) {
        self.entries.clear();
        if self.cwd != "/" {
            self.entries.push(Entry { name: "..".to_string(), dir: true, size: 0 });
        }
        let names = match list_dir(&self.cwd) {
            Ok(n) => n,
            Err(e) => {
                self.status = format!("nu pot citi directorul: {}", error_text(e));
                return;
            }
        };
        let first = self.entries.len();
        for (name, dir) in names {
            let size = if dir { 0 } else { stat(&join(&self.cwd, &name)).max(0) as u64 };
            self.entries.push(Entry { name, dir, size });
        }
        // Folders first, then files, each by name without regard to case (".." stays first).
        self.entries[first..].sort_by(|a, b| (!a.dir, a.name.to_ascii_lowercase()).cmp(&(!b.dir, b.name.to_ascii_lowercase())));
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.top = 0;
        println!("files: {} ({} intrari)", self.cwd, self.entries.len() - first);
    }

    fn go(&mut self, dir: String) {
        self.cwd = dir;
        self.selected = 0;
        self.load();
    }

    /// One folder up, with the folder we were in selected.
    fn up(&mut self) {
        let from = file_name(&self.cwd).to_string();
        self.go(parent(&self.cwd));
        if let Some(i) = self.entries.iter().position(|e| e.dir && e.name == from) {
            self.selected = i;
            self.top = i.saturating_sub(ROWS - 1);
        }
    }

    fn selected_path(&self) -> Option<(String, bool)> {
        let e = self.entries.get(self.selected)?;
        if e.name == ".." {
            return None;
        }
        Some((join(&self.cwd, &e.name), e.dir))
    }

    /// After a change under `path`: on the stick, write out the cache now (it can be pulled out).
    fn written(&mut self, path: &str) {
        if path == STICK || path.starts_with("/usb/") {
            let r = sync(path);
            if r < 0 {
                self.status.push_str(&format!(" (scrierea pe stick: {})", error_text(r)));
            } else {
                self.status.push_str(" - scris pe stick");
            }
        }
    }

    fn key(&mut self, k: char) -> bool {
        if self.input.is_some() {
            self.typing(k);
            return true;
        }
        if k != 's' {
            self.confirm_delete = None;
        }
        match k {
            'q' | '\u{1b}' => return false,
            '\u{8}' => self.up(),
            '\n' => self.open_selected(),
            'j' => self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1)),
            'k' => self.selected = self.selected.saturating_sub(1),
            'c' => match self.selected_path() {
                Some((p, dir)) => {
                    self.status = format!("copiat{}: {}", if dir { " (dosar)" } else { "" }, p);
                    self.clipboard = Some((p, dir));
                }
                None => self.status = "alege un fisier sau un dosar".to_string(),
            },
            'v' => self.paste(),
            's' => self.delete(),
            'n' => self.new_folder(),
            'r' => match self.selected_path() {
                Some((p, _)) => {
                    self.input = Some(file_name(&p).to_string());
                    self.status = "numele nou, apoi Enter (Esc = renunt)".to_string();
                }
                None => self.status = "alege ce redenumesti".to_string(),
            },
            'u' => {
                if is_dir(STICK) {
                    self.go(STICK.to_string());
                } else {
                    self.status = "niciun stick la /usb".to_string();
                }
            }
            _ => {}
        }
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
        true
    }

    /// A key while the new name is typed.
    fn typing(&mut self, k: char) {
        let Some(text) = self.input.as_mut() else { return };
        match k {
            '\u{1b}' => {
                self.input = None;
                self.status = "redenumire anulata".to_string();
            }
            '\u{8}' => {
                text.pop();
            }
            '\n' => {
                let name = self.input.take().unwrap_or_default();
                self.rename_to(name.trim());
            }
            c if (c as u32) >= 0x20 && c != '/' && text.len() < 64 => text.push(c),
            _ => {}
        }
    }

    fn rename_to(&mut self, name: &str) {
        let Some((from, _)) = self.selected_path() else { return };
        if name.is_empty() || name == file_name(&from) {
            self.status = "numele nu s-a schimbat".to_string();
            return;
        }
        let to = join(&self.cwd, name);
        let r = rename(&from, &to);
        if r < 0 {
            self.status = format!("nu pot redenumi: {}", error_text(r));
            return;
        }
        println!("files: redenumit {} -> {}", from, to);
        self.load();
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.selected = i;
        }
        self.status = format!("redenumit: {}", name);
        self.written(&to);
    }

    fn new_folder(&mut self) {
        let mut name = "dosar-nou".to_string();
        let mut i = 2;
        while stat(&join(&self.cwd, &name)) != cyber_user::E_NOT_FOUND {
            name = format!("dosar-nou-{}", i);
            i += 1;
            if i > 99 {
                self.status = "prea multe dosare noi".to_string();
                return;
            }
        }
        let p = join(&self.cwd, &name);
        let r = create(&p, true);
        if r < 0 {
            self.status = format!("nu pot face dosarul: {}", error_text(r));
            return;
        }
        println!("files: dosar nou {}", p);
        self.load();
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.selected = i;
        }
        self.status = format!("dosar nou: {} (r = alt nume)", name);
        self.written(&p);
    }

    fn open_selected(&mut self) {
        let Some(e) = self.entries.get(self.selected) else { return };
        let (name, dir) = (e.name.clone(), e.dir);
        if name == ".." {
            self.up();
        } else if dir {
            let d = join(&self.cwd, &name);
            self.go(d);
        } else {
            let p = join(&self.cwd, &name);
            if is_program(&p) {
                let r = spawn(&p);
                self.status = if r < 0 { format!("nu porneste: {}", error_text(r)) } else { format!("pornit: {}", p) };
            } else {
                // Everything else opens in the text editor.
                let r = cyber_user::spawn_with("/bin/editor", &p);
                self.status =
                    if r < 0 { format!("editorul nu porneste: {}", error_text(r)) } else { format!("deschis in editor: {}", p) };
            }
        }
    }

    fn paste(&mut self) {
        let Some((src, dir)) = self.clipboard.clone() else {
            self.status = "nimic copiat".to_string();
            return;
        };
        let dst = join(&self.cwd, file_name(&src));
        if dst == src {
            self.status = "e deja aici (alege alt dosar)".to_string();
            return;
        }
        if dir {
            if self.cwd == src || self.cwd.starts_with(&format!("{}/", src)) {
                self.status = "nu pot copia un dosar in el insusi".to_string();
                return;
            }
            let mut n = Count::default();
            let r = copy_tree(&src, &dst, 0, &mut n);
            println!("files: copiat dosarul {} -> {} ({} fisiere, {} dosare, {} octeti)", src, dst, n.files, n.dirs, n.bytes);
            self.load();
            self.status = match r {
                Ok(()) => format!("lipit dosarul: {} ({} fisiere, {})", dst, n.files, human(n.bytes)),
                Err(e) => format!("copiere oprita dupa {} fisiere: {}", n.files, error_text(e)),
            };
        } else {
            match copy(&src, &dst) {
                Ok(n) => {
                    println!("files: copiat {} -> {} ({} octeti)", src, dst, n);
                    self.load();
                    self.status = format!("lipit: {}", dst);
                }
                Err(e) => {
                    self.status = format!("nu pot lipi: {}", error_text(e));
                    return;
                }
            }
        }
        self.written(&dst);
    }

    fn delete(&mut self) {
        let Some((p, dir)) = self.selected_path() else { return };
        if dir {
            let inside = list_dir(&p).map(|v| v.len()).unwrap_or(0);
            if inside > 0 && self.confirm_delete.as_deref() != Some(p.as_str()) {
                self.status = format!("dosarul are {} intrari: apasa s inca o data ca sa-l stergi tot", inside);
                self.confirm_delete = Some(p);
                return;
            }
        }
        self.confirm_delete = None;
        let mut n = Count::default();
        let r = if dir { remove_tree(&p, 0, &mut n) } else { remove(&p) };
        if r < 0 {
            self.status = format!("nu pot sterge: {}", error_text(r));
            return;
        }
        if dir && n.files + n.dirs > 1 {
            println!("files: sters dosarul {} ({} fisiere, {} dosare)", p, n.files, n.dirs);
        } else {
            println!("files: sters {}", p);
        }
        self.load();
        self.status = format!("sters: {}", p);
        let cwd = self.cwd.clone();
        self.written(&cwd);
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
        } else if y >= LIST_Y && self.input.is_none() {
            let i = self.top + (y - LIST_Y) / ROW_H;
            if i < self.entries.len() && (y - LIST_Y) / ROW_H < ROWS {
                if i != self.selected {
                    self.confirm_delete = None;
                }
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
        c.text(6, 34, &self.cwd, DIM, 1);
        for (row, i) in (self.top..self.entries.len()).take(ROWS).enumerate() {
            let e = &self.entries[i];
            let y = LIST_Y + row * ROW_H;
            if i == self.selected {
                c.fill(0, y, W, ROW_H, SELECTED);
            }
            if e.dir {
                c.fill(8, y + 5, 12, 10, FOLDER);
            }
            match (&self.input, i == self.selected) {
                (Some(text), true) => {
                    c.fill(26, y + 2, W - 120, ROW_H - 4, INPUT);
                    let end = 28 + c.text(28, y + 4, text, TEXT, 1);
                    c.fill(end + 1, y + 4, 2, 12, TEXT);
                }
                _ => {
                    let end = 28 + c.text(28, y + 4, &e.name, TEXT, 1);
                    if e.dir && e.name != ".." {
                        c.text(end, y + 4, "/", TEXT, 1);
                    }
                }
            }
            if !e.dir {
                let s = human(e.size);
                c.text(W - 8 - s.len() * 6, y + 4, &s, DIM, 1);
            }
        }
        c.text(6, H - 14, &self.status, DIM, 1);
        win.present_all(c);
    }
}

/// Copies the folder `src` with everything in it to `dst` (which must not exist).
fn copy_tree(src: &str, dst: &str, depth: usize, n: &mut Count) -> Result<(), i64> {
    if depth > MAX_DEPTH {
        return Err(-1);
    }
    let r = create(dst, true);
    if r < 0 {
        return Err(r);
    }
    n.dirs += 1;
    for (name, dir) in list_dir(src)? {
        let (s, d) = (join(src, &name), join(dst, &name));
        if dir {
            copy_tree(&s, &d, depth + 1, n)?;
        } else {
            n.bytes += copy(&s, &d)?;
            n.files += 1;
        }
    }
    Ok(())
}

/// Deletes the folder `path` with everything in it; 0 or a negative error.
fn remove_tree(path: &str, depth: usize, n: &mut Count) -> i64 {
    if depth > MAX_DEPTH {
        return -1;
    }
    let names = match list_dir(path) {
        Ok(v) => v,
        Err(e) => return e,
    };
    for (name, dir) in names {
        let p = join(path, &name);
        let r = if dir { remove_tree(&p, depth + 1, n) } else { remove(&p) };
        if r < 0 {
            return r;
        }
        if !dir {
            n.files += 1;
        }
    }
    let r = remove(path);
    if r >= 0 {
        n.dirs += 1;
    }
    r
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
    let start = match cyber_user::args().nth(1) {
        Some(a) => a.to_string(),
        None if is_dir(STICK) => STICK.to_string(),
        None => "/".to_string(),
    };
    let win = match Window::open("Fisiere", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("files: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let mut f = Files {
        cwd: start,
        entries: Vec::new(),
        selected: 0,
        top: 0,
        clipboard: None,
        status: String::new(),
        input: None,
        confirm_delete: None,
    };
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
