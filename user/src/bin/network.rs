//! Network app: the card's address, mask, gateway, DNS server, DHCP lease and packet counts (updated every
//! second), plus a ping box: type an address or a name, press Enter, see four answers. `Esc` closes it.
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::net::{self, Ipv4};
use cyber_user::text::Line;
use cyber_user::{error_text, println};

cyber_user::entry!(main);

const W: usize = 440;
const H: usize = 300;
const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const BOX: u32 = 0x00_FF_FF_FF;
const GOOD: u32 = 0x00_1A_7F_37;
const BAD: u32 = 0x00_B0_20_20;

static mut PIXELS: [u32; W * H] = [0; W * H];

/// Results of the last ping run: one line each.
struct Results {
    lines: [Line; 5],
    ok: [bool; 5],
    count: usize,
}

struct App {
    target: [u8; 64],
    target_len: usize,
    results: Results,
}

fn ip_line(line: &mut Line, ip: Ipv4) {
    line.num(ip[0] as u64).push(".").num(ip[1] as u64).push(".").num(ip[2] as u64).push(".").num(ip[3] as u64);
}

fn hex(line: &mut Line, b: u8) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let pair = [DIGITS[(b >> 4) as usize], DIGITS[(b & 15) as usize]];
    line.push(core::str::from_utf8(&pair).unwrap_or("??"));
}

impl App {
    fn target(&self) -> &str {
        core::str::from_utf8(&self.target[..self.target_len]).unwrap_or("")
    }

    fn add_result(&mut self, line: Line, ok: bool) {
        if self.results.count < self.results.lines.len() {
            println!("retea: {}", line.as_str());
            self.results.lines[self.results.count] = line;
            self.results.ok[self.results.count] = ok;
            self.results.count += 1;
        }
    }

    /// Resolves the typed name, then pings it four times (drawing after each answer).
    fn ping(&mut self, c: &mut Canvas, win: &Window) {
        self.results.count = 0;
        let target = self.target_copy();
        let name = core::str::from_utf8(&target.0[..target.1]).unwrap_or("");
        let ip = match net::resolve(name) {
            Ok(ip) => ip,
            Err(e) => {
                let mut l = Line::new();
                l.push(name).push(": ").push(error_text(e));
                self.add_result(l, false);
                return;
            }
        };
        for seq in 1..=4u16 {
            let mut l = Line::new();
            l.push("ping ");
            ip_line(&mut l, ip);
            match net::ping(ip, seq, 2000) {
                Ok(ms) => {
                    l.push(": raspuns in ").num(ms).push(" ms");
                    self.add_result(l, true);
                }
                Err(e) => {
                    l.push(": ").push(error_text(e));
                    self.add_result(l, false);
                }
            }
            self.draw(c, win);
        }
    }

    fn target_copy(&self) -> ([u8; 64], usize) {
        (self.target, self.target_len)
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        c.text(10, 10, "Retea", TEXT, 2);
        let Some(i) = net::info() else {
            c.text(10, 44, "Nicio placa de retea cunoscuta.", BAD, 1);
            win.present_all(c);
            return;
        };
        let mut l = Line::new();
        l.push("Placa (MAC) ");
        for (k, b) in i.mac.iter().enumerate() {
            if k > 0 {
                l.push(":");
            }
            hex(&mut l, *b);
        }
        c.text(10, 40, l.as_str(), TEXT, 1);
        let mut l = Line::new();
        if i.ip == [0; 4] {
            l.push("Adresa: astept DHCP...");
        } else {
            l.push("Adresa ");
            ip_line(&mut l, i.ip);
            l.push("  masca ");
            ip_line(&mut l, i.mask);
        }
        c.text(10, 56, l.as_str(), TEXT, 1);
        let mut l = Line::new();
        l.push("Ruta ");
        ip_line(&mut l, i.gateway);
        l.push("   DNS ");
        ip_line(&mut l, i.dns);
        c.text(10, 72, l.as_str(), TEXT, 1);
        let mut l = Line::new();
        l.push("Pachete trimise ").num(i.sent).push(", primite ").num(i.received);
        if i.lease_s > 0 {
            l.push("   DHCP inca ").num(i.lease_s as u64 / 60).push(" min");
        }
        c.text(10, 88, l.as_str(), DIM, 1);

        c.text(10, 118, "Ping la (adresa sau nume), apoi Enter:", TEXT, 1);
        c.fill(10, 132, W - 20, 20, 0x00_A0_A8_B0);
        c.fill(11, 133, W - 22, 18, BOX);
        let mut t = Line::new();
        t.push(self.target()).push("_");
        c.text(16, 138, t.as_str(), TEXT, 1);
        for k in 0..self.results.count {
            let color = if self.results.ok[k] { GOOD } else { BAD };
            c.text(10, 164 + k * 16, self.results.lines[k].as_str(), color, 1);
        }
        c.text(10, H - 14, "Enter = ping    Esc = iesire", DIM, 1);
        win.present_all(c);
    }
}

fn main() -> i64 {
    let win = match Window::open("Retea", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("network: nu pot deschide fereastra ({})", error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let mut app = App {
        target: [0; 64],
        target_len: 0,
        results: Results { lines: [Line::new(), Line::new(), Line::new(), Line::new(), Line::new()], ok: [false; 5], count: 0 },
    };
    // Start with the gateway filled in: the simplest check that the network works.
    if let Some(i) = net::info().filter(|i| i.gateway != [0; 4]) {
        let mut l = Line::new();
        ip_line(&mut l, i.gateway);
        let s = l.as_str().as_bytes();
        app.target[..s.len()].copy_from_slice(s);
        app.target_len = s.len();
    }
    app.draw(&mut c, &win);
    loop {
        match win.wait_event_for(1000) {
            Err(()) => break,
            Ok(Some(Event::Key('\u{1b}'))) => break,
            Ok(Some(Event::Key('\n'))) => {
                if app.target_len > 0 {
                    app.ping(&mut c, &win);
                }
                app.draw(&mut c, &win);
            }
            Ok(Some(Event::Key('\u{8}'))) => {
                app.target_len = app.target_len.saturating_sub(1);
                app.draw(&mut c, &win);
            }
            Ok(Some(Event::Key(ch))) if (ch.is_ascii_alphanumeric() || ch == '.' || ch == '-') && app.target_len < 63 => {
                app.target[app.target_len] = ch as u8;
                app.target_len += 1;
                app.draw(&mut c, &win);
            }
            Ok(_) => app.draw(&mut c, &win),
        }
    }
    0
}
