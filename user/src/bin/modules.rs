//! Modules: the installed modules (start, stop, remove) and the packages in /disk/pachete (install).
//! Click a line, then a button at the top. `q` or Escape closes the window.
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::modules;
use cyber_user::text::Line;
use cyber_user::{error_text, println};

cyber_user::entry!(main);

const W: usize = 520;
const H: usize = 360;
const BAR_H: usize = 28;
const INSTALLED_Y: usize = 48;
const PACKAGES_Y: usize = 190;
const ROW_H: usize = 20;
const ROWS: usize = 6;
const PACKAGES: &str = "/disk/pachete";

const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const BUTTON: u32 = 0x00_20_70_C0;
const WHITE: u32 = 0x00_FF_FF_FF;
const SELECTED: u32 = 0x00_C8_DC_F0;
const RUNNING: u32 = 0x00_20_A0_50;

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Install,
    Start,
    Stop,
    Remove,
}

const BUTTONS: [(&str, Action); 4] = [("Instaleaza", Action::Install), ("Porneste", Action::Start), ("Opreste", Action::Stop), ("Sterge", Action::Remove)];

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    None,
    Installed(usize),
    Package(usize),
}

static mut PIXELS: [u32; W * H] = [0; W * H];

/// Up to `ROWS` short texts.
struct Rows {
    text: [[u8; 80]; ROWS],
    len: [usize; ROWS],
    count: usize,
}

impl Rows {
    fn new() -> Rows {
        Rows { text: [[0; 80]; ROWS], len: [0; ROWS], count: 0 }
    }
    fn push(&mut self, s: &[u8]) {
        if self.count < ROWS {
            let n = s.len().min(80);
            self.text[self.count][..n].copy_from_slice(&s[..n]);
            self.len[self.count] = n;
            self.count += 1;
        }
    }
    fn get(&self, i: usize) -> &str {
        core::str::from_utf8(&self.text[i][..self.len[i]]).unwrap_or("?")
    }
    /// First word (the module name).
    fn name(&self, i: usize) -> &str {
        self.get(i).split(' ').next().unwrap_or("")
    }
}

struct App {
    installed: Rows,
    packages: Rows,
    selected: Selection,
    status: Line,
}

impl App {
    fn load(&mut self) {
        self.installed = Rows::new();
        let mut buf = [0u8; 1024];
        let n = modules::list(&mut buf);
        if n > 0 {
            for line in buf[..n as usize].split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
                self.installed.push(line);
            }
        }
        self.packages = Rows::new();
        let n = cyber_user::list(PACKAGES, &mut buf);
        if n > 0 {
            for line in buf[..n as usize].split(|&b| b == b'\n').filter(|l| l.ends_with(b".cum")) {
                self.packages.push(line);
            }
        }
    }

    fn act(&mut self, action: Action) {
        let mut path = Line::new();
        let (r, what) = match (action, self.selected) {
            (Action::Install, Selection::Package(i)) => {
                path.push(PACKAGES).push("/").push(self.packages.get(i));
                (modules::install(path.as_str()), "instalat")
            }
            (Action::Start, Selection::Installed(i)) => (modules::start(self.installed.name(i)), "pornit"),
            (Action::Stop, Selection::Installed(i)) => (modules::stop(self.installed.name(i)), "oprit"),
            (Action::Remove, Selection::Installed(i)) => (modules::remove(self.installed.name(i)), "sters"),
            (Action::Install, _) => return self.say("alege un pachet din lista de jos"),
            _ => return self.say("alege un modul instalat"),
        };
        let target = match self.selected {
            Selection::Installed(i) => self.installed.name(i),
            _ => path.as_str(),
        };
        let mut msg = Line::new();
        if r < 0 {
            msg.push("eroare: ").push(error_text(r));
        } else {
            msg.push(what).push(" ").push(target);
        }
        println!("module: {}", msg.as_str());
        self.status = msg;
        let keep = self.selected;
        self.load();
        self.selected = match keep {
            Selection::Installed(i) if i < self.installed.count => keep,
            Selection::Package(i) if i < self.packages.count => keep,
            _ => Selection::None,
        };
    }

    fn say(&mut self, s: &str) {
        self.status = Line::new();
        self.status.push(s);
    }

    fn click(&mut self, x: usize, y: usize) {
        if y < BAR_H {
            let mut bx = 4;
            for (label, action) in BUTTONS {
                let w = label.len() * 12 + 12;
                if (bx..bx + w).contains(&x) {
                    self.act(action);
                    return;
                }
                bx += w + 4;
            }
        } else if (INSTALLED_Y..INSTALLED_Y + ROWS * ROW_H).contains(&y) && (y - INSTALLED_Y) / ROW_H < self.installed.count {
            self.selected = Selection::Installed((y - INSTALLED_Y) / ROW_H);
        } else if (PACKAGES_Y..PACKAGES_Y + ROWS * ROW_H).contains(&y) && (y - PACKAGES_Y) / ROW_H < self.packages.count {
            self.selected = Selection::Package((y - PACKAGES_Y) / ROW_H);
        }
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
        c.text(8, 34, "Module instalate:", DIM, 1);
        for i in 0..self.installed.count {
            let y = INSTALLED_Y + i * ROW_H;
            if self.selected == Selection::Installed(i) {
                c.fill(0, y, W, ROW_H, SELECTED);
            }
            let line = self.installed.get(i);
            let running = line.split(' ').nth(2) == Some("running");
            c.fill(10, y + 6, 8, 8, if running { RUNNING } else { DIM });
            let mut shown = Line::new();
            let mut parts = line.splitn(4, ' ');
            let (name, version, _, desc) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next(), parts.next().unwrap_or(""));
            shown.push(name).push(" ").push(version).push(if running { "  (merge)  " } else { "  " }).push(desc);
            c.text(26, y + 6, shown.as_str(), TEXT, 1);
        }
        if self.installed.count == 0 {
            c.text(26, INSTALLED_Y + 6, "(niciunul)", DIM, 1);
        }
        c.text(8, PACKAGES_Y - 14, "Pachete in /disk/pachete:", DIM, 1);
        for i in 0..self.packages.count {
            let y = PACKAGES_Y + i * ROW_H;
            if self.selected == Selection::Package(i) {
                c.fill(0, y, W, ROW_H, SELECTED);
            }
            c.text(26, y + 6, self.packages.get(i), TEXT, 1);
        }
        c.text(8, H - 14, self.status.as_str(), DIM, 1);
        win.present_all(c);
    }
}

fn main() -> i64 {
    let win = match Window::open("Module", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("module: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let mut app = App { installed: Rows::new(), packages: Rows::new(), selected: Selection::None, status: Line::new() };
    app.load();
    println!("module: {} pachete, {} instalate", app.packages.count, app.installed.count);
    app.draw(&mut c, &win);
    loop {
        match win.wait_event_for(1000) {
            Err(()) => break,
            Ok(Some(Event::Key('q' | '\u{1b}'))) => break,
            Ok(Some(Event::Click { x, y, buttons })) if buttons & 1 != 0 => app.click(x, y),
            Ok(None) => app.load(), // a module program may have ended
            _ => {}
        }
        app.draw(&mut c, &win);
    }
    0
}
