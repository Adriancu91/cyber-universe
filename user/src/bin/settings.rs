//! Settings: the time zone of the clock and the keyboard layout, saved in /disk/config/settings.toml
//! (the desktop reads the time zone; the layout is applied at start-up once the Romanian layout
//! exists, P4-12). `q` or Escape closes the window.
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::settings;
use cyber_user::text::Line;
use cyber_user::{close, create, error_text, open, println, remove, write};

cyber_user::entry!(main);

const W: usize = 400;
const H: usize = 220;
const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const BUTTON: u32 = 0x00_20_70_C0;
const WHITE: u32 = 0x00_FF_FF_FF;

/// Buttons: (x, y, w, h, label).
const MINUS: (usize, usize, usize, usize, &str) = (260, 36, 40, 26, "-");
const PLUS: (usize, usize, usize, usize, &str) = (310, 36, 40, 26, "+");
const LAYOUT: (usize, usize, usize, usize, &str) = (260, 76, 100, 26, "schimba");
const SAVE: (usize, usize, usize, usize, &str) = (20, 150, 120, 28, "Salveaza");

static mut PIXELS: [u32; W * H] = [0; W * H];

fn inside(b: (usize, usize, usize, usize, &str), x: usize, y: usize) -> bool {
    (b.0..b.0 + b.2).contains(&x) && (b.1..b.1 + b.3).contains(&y)
}

struct Settings {
    zone: i32,
    ro: bool,
    status: Line,
}

impl Settings {
    fn text(&self) -> Line {
        let mut l = Line::new();
        l.push("# Setarile Cyber Universe (aplicatia Setari)\nfus_orar = \"");
        l.push(if self.zone < 0 { "-" } else { "+" }).num(self.zone.unsigned_abs() as u64);
        l.push("\"\ntastatura = \"").push(if self.ro { "ro" } else { "us" }).push("\"\n");
        l
    }

    fn save(&mut self) {
        let text = self.text();
        let _ = create("/disk/config", true); // may exist already
        let r = remove(settings::PATH);
        let r = if r < 0 && r != -3 { r } else { create(settings::PATH, false) };
        let fd = if r < 0 { r } else { open(settings::PATH) };
        self.status = Line::new();
        if fd < 0 {
            self.status.push("nu pot salva: ").push(error_text(fd));
            return;
        }
        let n = write(fd as u64, text.as_str().as_bytes());
        close(fd);
        if n == text.as_str().len() as i64 {
            self.status.push("salvat in ").push(settings::PATH);
            println!("settings: salvat fus_orar {}{}, tastatura {}", if self.zone < 0 { "-" } else { "+" }, self.zone.abs(), if self.ro { "ro" } else { "us" });
        } else {
            self.status.push("salvare incompleta");
        }
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        let mut zone = Line::new();
        zone.push("Fus orar: UTC").push(if self.zone < 0 { "-" } else { "+" }).num(self.zone.unsigned_abs() as u64);
        c.text(20, 42, zone.as_str(), TEXT, 2);
        let mut layout = Line::new();
        layout.push("Tastatura: ").push(if self.ro { "romana" } else { "engleza (SUA)" });
        c.text(20, 82, layout.as_str(), TEXT, 2);
        for b in [MINUS, PLUS, LAYOUT, SAVE] {
            c.fill(b.0, b.1, b.2, b.3, BUTTON);
            c.text(b.0 + (b.2 - b.4.len() * 12) / 2, b.1 + 4, b.4, WHITE, 2);
        }
        c.text(20, 120, "Aranjamentul tastaturii se aplica la urmatoarea pornire.", DIM, 1);
        c.text(20, 196, self.status.as_str(), DIM, 1);
        win.present_all(c);
    }
}

fn main() -> i64 {
    let mut buf = [0u8; 512];
    let text = settings::read(&mut buf);
    let mut s = Settings { zone: settings::time_zone(text), ro: settings::value(text, "tastatura") == Some("ro"), status: Line::new() };
    let win = match Window::open("Setari", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("settings: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    s.draw(&mut c, &win);
    while let Some(e) = win.wait_event() {
        match e {
            Event::Key('q' | '\u{1b}') => break,
            Event::Click { x, y, buttons } if buttons & 1 != 0 => {
                if inside(MINUS, x, y) {
                    s.zone = (s.zone - 1).max(-12);
                } else if inside(PLUS, x, y) {
                    s.zone = (s.zone + 1).min(14);
                } else if inside(LAYOUT, x, y) {
                    s.ro = !s.ro;
                } else if inside(SAVE, x, y) {
                    s.save();
                }
            }
            _ => {}
        }
        s.draw(&mut c, &win);
    }
    0
}
