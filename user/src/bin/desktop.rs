//! The "Cyber Universe" desktop: the background with the name, a bar at the bottom with the Start
//! button and the clock. Start opens a menu with the programs in /bin; a click on one starts it.
//! Escape closes the desktop (the screen goes back to the text console).
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, GLYPH_H, Window};
use cyber_user::{println, spawn, time};

cyber_user::entry!(main);

const BACKGROUND: u32 = 0x00_18_28_38; // same as the window server's desktop color
const BAR: u32 = 0x00_0D_15_20;
const START: u32 = 0x00_20_70_C0;
const MENU: u32 = 0x00_24_30_3C;
const TEXT: u32 = 0x00_FF_FF_FF;
const LOGO: u32 = 0x00_40_D0_FF;

const BAR_H: usize = 32;
const MAX_W: usize = 2560;
const START_X: usize = 4;
const START_W: usize = 84;
const ITEM_H: usize = 24;
const MENU_W: usize = 200;
const MAX_ITEMS: usize = 20;
const MENU_H_MAX: usize = MAX_ITEMS * ITEM_H + 8;

static mut BAR_PIXELS: [u32; MAX_W * BAR_H] = [0; MAX_W * BAR_H];
static mut MENU_PIXELS: [u32; MENU_W * MENU_H_MAX] = [0; MENU_W * MENU_H_MAX];
static mut LOGO_PIXELS: [u32; 320 * 40] = [0; 320 * 40];

/// The programs in /bin (without the desktop itself), sorted by the file system.
struct Apps {
    names: [[u8; 32]; MAX_ITEMS],
    lens: [usize; MAX_ITEMS],
    count: usize,
}

impl Apps {
    fn read() -> Apps {
        let mut apps = Apps { names: [[0; 32]; MAX_ITEMS], lens: [0; MAX_ITEMS], count: 0 };
        let mut buf = [0u8; 1024];
        let n = cyber_user::list("/bin", &mut buf);
        if n <= 0 {
            return apps;
        }
        for line in buf[..n as usize].split(|&b| b == b'\n') {
            if line.is_empty() || line.ends_with(b"/") || line == b"desktop" || line.len() > 32 || apps.count == MAX_ITEMS {
                continue;
            }
            apps.names[apps.count][..line.len()].copy_from_slice(line);
            apps.lens[apps.count] = line.len();
            apps.count += 1;
        }
        apps
    }

    fn name(&self, i: usize) -> &str {
        core::str::from_utf8(&self.names[i][..self.lens[i]]).unwrap_or("?")
    }
}

struct Desktop {
    win: Window,
    apps: Apps,
    menu_open: bool,
    clock: Option<(u8, u8)>,
}

impl Desktop {
    fn bar_y(&self) -> usize {
        self.win.height - BAR_H
    }

    fn menu_rect(&self) -> (usize, usize, usize, usize) {
        let h = self.apps.count * ITEM_H + 8;
        (START_X, self.bar_y() - h, MENU_W, h)
    }

    fn draw_logo(&self) {
        // SAFETY: one thread; the only reference to LOGO_PIXELS.
        let mut c = Canvas::new(320, 40, unsafe { &mut *core::ptr::addr_of_mut!(LOGO_PIXELS) });
        c.fill(0, 0, 320, 40, BACKGROUND);
        let w = c.text(0, 0, "Cyber Universe", LOGO, 3);
        let (x, y) = ((self.win.width.saturating_sub(w)) / 2, self.win.height / 3);
        self.win.draw(&c, x, y);
    }

    fn draw_bar(&mut self) {
        let w = self.win.width.min(MAX_W);
        // SAFETY: one thread; the only reference to BAR_PIXELS.
        let pixels = unsafe { &mut (&mut *core::ptr::addr_of_mut!(BAR_PIXELS))[..w * BAR_H] };
        let mut c = Canvas::new(w, BAR_H, pixels);
        c.fill(0, 0, w, BAR_H, BAR);
        c.fill(START_X, 4, START_W, BAR_H - 8, if self.menu_open { LOGO } else { START });
        c.text(START_X + 12, (BAR_H - 2 * GLYPH_H) / 2 + 1, "Start", TEXT, 2);
        self.clock = local_time();
        let mut s = [b'-', b'-', b':', b'-', b'-'];
        if let Some((h, m)) = self.clock {
            s = [b'0' + h / 10, b'0' + h % 10, b':', b'0' + m / 10, b'0' + m % 10];
        }
        let text = core::str::from_utf8(&s).unwrap_or("--:--");
        c.text(w - 5 * 12 - 12, (BAR_H - 2 * GLYPH_H) / 2 + 1, text, TEXT, 2);
        let y = self.bar_y();
        self.win.draw(&c, 0, y);
    }

    fn draw_menu(&self) {
        let (x, y, w, h) = self.menu_rect();
        // SAFETY: one thread; the only reference to MENU_PIXELS.
        let pixels = unsafe { &mut (&mut *core::ptr::addr_of_mut!(MENU_PIXELS))[..w * h] };
        let mut c = Canvas::new(w, h, pixels);
        c.fill(0, 0, w, h, MENU);
        c.fill(0, 0, w, 1, START);
        for i in 0..self.apps.count {
            c.text(12, 4 + i * ITEM_H + (ITEM_H - 2 * GLYPH_H) / 2 + 1, self.apps.name(i), TEXT, 2);
        }
        self.win.draw(&c, x, y);
    }

    fn set_menu(&mut self, open: bool) {
        self.menu_open = open;
        if open {
            self.draw_menu();
            println!("desktop: meniu deschis ({} programe)", self.apps.count);
        } else {
            let (x, y, w, h) = self.menu_rect();
            self.win.fill(x, y, w, h, BACKGROUND);
        }
        self.draw_bar();
    }

    fn click(&mut self, x: usize, y: usize) {
        let (mx, my, mw, mh) = self.menu_rect();
        if y >= self.bar_y() + 4 && y < self.bar_y() + BAR_H - 4 && (START_X..START_X + START_W).contains(&x) {
            self.set_menu(!self.menu_open);
        } else if self.menu_open && (mx..mx + mw).contains(&x) && (my + 4..my + mh - 4).contains(&y) {
            let i = (y - my - 4) / ITEM_H;
            let mut path = [0u8; 40];
            path[..5].copy_from_slice(b"/bin/");
            path[5..5 + self.apps.lens[i]].copy_from_slice(&self.apps.names[i][..self.apps.lens[i]]);
            let path = core::str::from_utf8(&path[..5 + self.apps.lens[i]]).unwrap_or("");
            println!("desktop: pornesc {}", path);
            let r = spawn(path);
            if r < 0 {
                println!("desktop: {}: {}", path, cyber_user::error_text(r));
            }
            self.set_menu(false);
        } else if self.menu_open {
            self.set_menu(false);
        }
    }
}

/// Hour and minute of the computer's clock moved by the time zone from the settings (`settings` app).
fn local_time() -> Option<(u8, u8)> {
    let mut buf = [0u8; 512];
    let zone = cyber_user::settings::time_zone(cyber_user::settings::read(&mut buf));
    time().map(|t| ((t.hour as i32 + zone).rem_euclid(24) as u8, t.minute))
}

fn main() -> i64 {
    let win = match Window::open_desktop() {
        Ok(w) => w,
        Err(e) => {
            println!("desktop: nu pot deschide desktopul ({})", cyber_user::error_text(e));
            return 1;
        }
    };
    println!("desktop: pornit ({}x{})", win.width, win.height);
    let mut d = Desktop { win, apps: Apps::read(), menu_open: false, clock: None };
    d.draw_logo();
    d.draw_bar();
    match d.clock {
        Some((h, m)) => println!("desktop: ceas {:02}:{:02}", h, m),
        None => println!("desktop: fara ceas"),
    }
    loop {
        match d.win.wait_event_for(1000) {
            Err(()) => break,
            Ok(Some(Event::Key('\u{1b}'))) => break,
            Ok(Some(Event::Click { x, y, buttons })) if buttons & 1 != 0 => d.click(x, y),
            Ok(_) => {
                if local_time() != d.clock {
                    d.draw_bar();
                }
            }
        }
    }
    println!("desktop: inchis");
    0
}
