//! System monitor: memory, processor use and the running programs, updated every second. Click a
//! program and press `o` to stop it; `q` or Escape closes the monitor.
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::text::Line;
use cyber_user::{mem_info, println};

cyber_user::entry!(main);

const W: usize = 420;
const H: usize = 300;
const ROW_Y: usize = 60;
const ROW_H: usize = 20;
const ROWS: usize = 11;
const MAX: usize = 64;

const BG: u32 = 0x00_F4_F6_F8;
const TEXT: u32 = 0x00_10_18_20;
const DIM: u32 = 0x00_60_70_80;
const SELECTED: u32 = 0x00_C8_DC_F0;

static mut PIXELS: [u32; W * H] = [0; W * H];

#[derive(Clone, Copy)]
struct Program {
    id: u64,
    ticks: u64,
    name: [u8; 24],
    name_len: usize,
    /// Processor use since the previous update, in percent.
    percent: u64,
}

impl Program {
    fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

struct Monitor {
    list: [Program; MAX],
    count: usize,
    selected: Option<u64>,
    last_uptime: u64,
    last_idle: u64,
    cpu: u64,
    reported: usize,
}

fn parse_u64(s: &[u8]) -> u64 {
    s.iter().take_while(|c| c.is_ascii_digit()).fold(0, |v, &c| v * 10 + (c - b'0') as u64)
}

impl Monitor {
    fn refresh(&mut self) {
        let Some(m) = mem_info() else { return };
        let elapsed = m.uptime_ms.saturating_sub(self.last_uptime).max(1) * m.ticks_per_second / 1000;
        if self.last_uptime != 0 {
            self.cpu = 100u64.saturating_sub(m.idle_ticks.saturating_sub(self.last_idle) * 100 / elapsed.max(1));
        }
        let mut buf = [0u8; 2048];
        let n = cyber_user::programs(&mut buf).max(0) as usize;
        let old = self.list;
        let old_count = self.count;
        self.count = 0;
        for line in buf[..n].split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            if self.count == MAX {
                break;
            }
            let mut parts = line.splitn(3, |&b| b == b' ');
            let (Some(id), Some(ticks), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let mut p = Program { id: parse_u64(id), ticks: parse_u64(ticks), name: [0; 24], name_len: name.len().min(24), percent: 0 };
            p.name[..p.name_len].copy_from_slice(&name[..p.name_len]);
            if let Some(before) = old[..old_count].iter().find(|o| o.id == p.id) {
                p.percent = (p.ticks.saturating_sub(before.ticks) * 100 / elapsed.max(1)).min(100);
            }
            self.list[self.count] = p;
            self.count += 1;
        }
        self.last_uptime = m.uptime_ms;
        self.last_idle = m.idle_ticks;
        if self.count != self.reported {
            self.reported = self.count;
            print_names(&self.list[..self.count]);
        }
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        if let Some(m) = mem_info() {
            let mut line = Line::new();
            line.push("Memorie ").num(m.used >> 20).push(" / ").num(m.total >> 20).push(" MiB");
            line.push("  Procesor ").num(self.cpu).push("%");
            c.text(10, 10, line.as_str(), TEXT, 2);
            let mut up = Line::new();
            up.push("pornit de ").num(m.uptime_ms / 60_000).push(" min ").num(m.uptime_ms / 1000 % 60).push(" s");
            c.text(10, 34, up.as_str(), DIM, 1);
        }
        c.text(10, 46, "ID     CPU   PROGRAM", DIM, 1);
        for (i, p) in self.list[..self.count].iter().take(ROWS).enumerate() {
            let y = ROW_Y + i * ROW_H;
            if self.selected == Some(p.id) {
                c.fill(0, y, W, ROW_H, SELECTED);
            }
            let mut line = Line::new();
            line.num(p.id);
            line.pad(7).num(p.percent).push("%");
            line.pad(13).push(p.name());
            c.text(10, y + 4, line.as_str(), TEXT, 1);
        }
        c.text(10, H - 14, "clic pe un program + o = opreste    q = iesire", DIM, 1);
        win.present_all(c);
    }
}

fn print_names(list: &[Program]) {
    let mut line = Line::new();
    line.push("monitor: ").num(list.len() as u64).push(" programe (");
    for (i, p) in list.iter().enumerate() {
        if i > 0 {
            line.push(", ");
        }
        line.push(p.name());
    }
    line.push(")");
    println!("{}", line.as_str());
}

fn main() -> i64 {
    let win = match Window::open("Monitor sistem", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("monitor: nu pot deschide fereastra ({})", cyber_user::error_text(e));
            return 1;
        }
    };
    // SAFETY: one thread; the only reference to PIXELS.
    let mut c = Canvas::new(W, H, unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) });
    let empty = Program { id: 0, ticks: 0, name: [0; 24], name_len: 0, percent: 0 };
    let mut m = Monitor { list: [empty; MAX], count: 0, selected: None, last_uptime: 0, last_idle: 0, cpu: 0, reported: 0 };
    m.refresh();
    m.draw(&mut c, &win);
    loop {
        match win.wait_event_for(1000) {
            Err(()) => break,
            Ok(Some(Event::Key('q' | '\u{1b}'))) => break,
            Ok(Some(Event::Key('o'))) => {
                if let Some(p) = m.list[..m.count].iter().find(|p| Some(p.id) == m.selected) {
                    println!("monitor: opresc {} ({})", p.name(), p.id);
                    if cyber_user::kill(p.id) < 0 {
                        println!("monitor: programul nu mai exista");
                    }
                    m.selected = None;
                }
                m.refresh();
                m.draw(&mut c, &win);
            }
            Ok(Some(Event::Click { y, .. })) => {
                if y >= ROW_Y && (y - ROW_Y) / ROW_H < m.count.min(ROWS) {
                    m.selected = Some(m.list[(y - ROW_Y) / ROW_H].id);
                }
                m.draw(&mut c, &win);
            }
            Ok(_) => {
                m.refresh();
                m.draw(&mut c, &win);
            }
        }
    }
    0
}
