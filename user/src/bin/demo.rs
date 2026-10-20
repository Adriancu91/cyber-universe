//! Window demo: opens a 320 x 200 window with a red rectangle, marks every click with a small green
//! square and prints it, prints every key; `q` (or closing the window) ends the program.
#![no_std]
#![no_main]

use cyber_user::gfx::{Canvas, Event, Window};
use cyber_user::println;

cyber_user::entry!(main);

const W: usize = 320;
const H: usize = 200;
static mut PIXELS: [u32; W * H] = [0; W * H];

fn main() -> i64 {
    // SAFETY: the program has one thread and this is the only reference to PIXELS.
    let pixels = unsafe { &mut *core::ptr::addr_of_mut!(PIXELS) };
    let mut c = Canvas::new(W, H, pixels);
    c.fill(0, 0, W, H, 0x00_20_30_50);
    c.fill(20, 20, 100, 60, 0x00_E0_30_30);
    let win = match Window::open("Demo", W, H) {
        Ok(w) => w,
        Err(e) => {
            println!("demo: nu pot deschide fereastra ({})", cyber_user::error_text(e));
            return 1;
        }
    };
    win.present_all(&c);
    println!("demo: fereastra deschisa");
    while let Some(e) = win.wait_event() {
        match e {
            Event::Key('q') => {
                println!("demo: tasta q, ies");
                win.close();
                return 0;
            }
            Event::Key(k) => println!("demo: tasta {:?}", k),
            Event::Click { x, y, .. } => {
                println!("demo: clic la ({}, {})", x, y);
                c.fill(x.saturating_sub(3), y.saturating_sub(3), 7, 7, 0x00_30_D0_60);
                win.present(&c, x.saturating_sub(3), y.saturating_sub(3), 7, 7);
            }
        }
    }
    println!("demo: fereastra a fost inchisa");
    0
}
