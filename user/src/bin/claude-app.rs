//! Claude (P6-05): a chat window with Claude. The conversation is above, the box to write in is below; Enter sends
//! the whole conversation to the API (src/claude_api.rs, the same client and configuration as /bin/claude), and the
//! answer appears when it comes ("Claude se gandeste..." meanwhile). Up/Down scroll, Esc or Ctrl+Q closes.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use cyber_user::gfx::{Canvas, Event, KEY_DOWN, KEY_UP, Window};
use cyber_user::{error_text, println};

#[path = "../claude_api.rs"]
mod claude_api;

cyber_user::entry!(main);
cyber_user::heap!(2 * 1024 * 1024);

const W: usize = 640;
const H: usize = 460;
const TOP: usize = 20;
const LINE_H: usize = 18;
const CHAR_W: usize = 12; // the font at scale 2
const COLS: usize = (W - 16) / CHAR_W;
const INPUT_H: usize = 2 * LINE_H + 10;
const ROWS: usize = (H - TOP - INPUT_H - 18) / LINE_H;
/// The longest message we let Adrian write.
const MAX_INPUT: usize = 4000;

const BG: u32 = 0x00_FF_FF_FF;
const BAR: u32 = 0x00_E8_EC_F0;
const TEXT: u32 = 0x00_10_18_20;
const YOU: u32 = 0x00_20_50_A0;
const CLAUDE: u32 = 0x00_A0_50_20;
const DIM: u32 = 0x00_60_70_80;
const ERROR: u32 = 0x00_C0_20_20;

static mut PIXELS: [u32; W * H] = [0; W * H];

#[derive(Clone, Copy, PartialEq)]
enum Who {
    You,
    Claude,
    Note,
}

struct Chat {
    /// The conversation; notes (errors) are shown but not sent.
    messages: Vec<(Who, String)>,
    input: String,
    /// How many screen lines up from the bottom the view is scrolled.
    scroll: usize,
    status: &'static str,
}

/// Splits `text` into screen lines of at most `cols` characters, at spaces when possible.
fn wrap(text: &str, cols: usize, out: &mut Vec<String>) {
    for para in text.split('\n') {
        let mut line = String::new();
        let mut len = 0;
        for word in para.split(' ') {
            let wl = word.chars().count();
            if len > 0 && len + 1 + wl > cols {
                out.push(core::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            // A word longer than a line is cut.
            for ch in word.chars() {
                if len == cols {
                    out.push(core::mem::take(&mut line));
                    len = 0;
                }
                line.push(ch);
                len += 1;
            }
        }
        out.push(line);
    }
}

impl Chat {
    fn lines(&self) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        for (who, text) in &self.messages {
            let (label, color) = match who {
                Who::You => ("Tu:", YOU),
                Who::Claude => ("Claude:", CLAUDE),
                Who::Note => ("!", ERROR),
            };
            out.push((color, String::from(label)));
            let mut l = Vec::new();
            wrap(text, COLS - 1, &mut l);
            out.extend(l.into_iter().map(|s| (if *who == Who::Note { ERROR } else { TEXT }, alloc::format!(" {}", s))));
            out.push((TEXT, String::new()));
        }
        out
    }

    fn draw(&self, c: &mut Canvas, win: &Window) {
        c.fill(0, 0, W, H, BG);
        c.fill(0, 0, W, TOP - 4, BAR);
        c.text(6, 4, "Claude - Enter trimite, sus/jos deruleaza, Esc inchide", DIM, 1);
        let lines = self.lines();
        let end = lines.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(ROWS);
        for (i, (color, text)) in lines[start..end].iter().enumerate() {
            c.text(8, TOP + i * LINE_H, text, *color, 2);
        }
        // The box to write in: the last two lines of what is being written, and the cursor.
        let y0 = H - INPUT_H - 16;
        c.fill(0, y0 - 2, W, 2, BAR);
        let mut l = Vec::new();
        wrap(&self.input, COLS, &mut l);
        let shown = &l[l.len().saturating_sub(2)..];
        for (i, s) in shown.iter().enumerate() {
            c.text(8, y0 + 4 + i * LINE_H, s, TEXT, 2);
        }
        let last = shown.last().map_or(0, |s| s.chars().count());
        c.fill(8 + last * CHAR_W, y0 + 4 + (shown.len().max(1) - 1) * LINE_H, 2, LINE_H - 2, YOU);
        c.fill(0, H - 16, W, 16, BAR);
        c.text(6, H - 13, self.status, DIM, 1);
        win.present_all(c);
    }

    /// The conversation as the API wants it: alternating user/assistant, without the notes.
    fn conversation(&self) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = Vec::new();
        for (who, text) in &self.messages {
            let role = match who {
                Who::You => "user",
                Who::Claude => "assistant",
                Who::Note => continue,
            };
            // Two of the same role in a row (after an error) are not allowed: keep the last one.
            if out.last().is_some_and(|(r, _)| *r == role) {
                out.pop();
            }
            out.push((role, text));
        }
        out
    }
}

fn main() -> i64 {
    // SAFETY: one thread; the only reference to the pixels.
    let pixels = unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) };
    let win = match Window::open("Claude", W, H) {
        Ok(w) => w,
        Err(err) => {
            println!("claude-app: nu pot deschide fereastra ({})", error_text(err));
            return 1;
        }
    };
    let mut c = Canvas::new(W, H, pixels);
    let conf = claude_api::config();
    let mut chat = Chat { messages: Vec::new(), input: String::new(), scroll: 0, status: "scrie un mesaj si apasa Enter" };
    let key = claude_api::key(&conf);
    if let Err(e) = &key {
        chat.messages.push((Who::Note, e.clone()));
    }
    println!("claude-app: deschis ({})", if key.is_ok() { "cheie gasita" } else { "fara cheie" });
    chat.draw(&mut c, &win);
    while let Some(ev) = win.wait_event() {
        match ev {
            Event::Key('\u{1b}') | Event::Ctrl('q') => break,
            Event::Key(KEY_UP) => chat.scroll += 1,
            Event::Key(KEY_DOWN) => chat.scroll = chat.scroll.saturating_sub(1),
            Event::Key('\u{8}') => {
                chat.input.pop();
            }
            Event::Key('\n') => {
                let text = String::from(chat.input.trim());
                if text.is_empty() {
                    continue;
                }
                let Ok(k) = &key else {
                    chat.status = "fara cheie API: pune-o in /usb/claude.key si porneste din nou";
                    chat.draw(&mut c, &win);
                    continue;
                };
                chat.input.clear();
                chat.messages.push((Who::You, text));
                chat.scroll = 0;
                chat.status = "Claude se gandeste...";
                chat.draw(&mut c, &win);
                let n = chat.conversation().len();
                println!("claude-app: trimit conversatia ({} mesaje)", n);
                let answer = claude_api::ask(&conf, k, &chat.conversation());
                match answer {
                    Ok(a) => {
                        println!("claude-app: raspuns primit ({} caractere)", a.chars().count());
                        chat.messages.push((Who::Claude, a));
                        chat.status = "gata";
                    }
                    Err(e) => {
                        println!("claude-app: eroare: {}", e);
                        chat.messages.push((Who::Note, e));
                        chat.status = "eroare (vezi mai sus)";
                    }
                }
            }
            Event::Key(ch) if !ch.is_control() && (ch as u32) < 0xF700 => {
                if chat.input.len() < MAX_INPUT {
                    chat.input.push(ch);
                }
            }
            _ => continue,
        }
        chat.draw(&mut c, &win);
    }
    win.close();
    println!("claude-app: inchis");
    0
}
