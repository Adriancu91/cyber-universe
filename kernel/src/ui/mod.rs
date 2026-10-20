//! User interface. `server` is the window server; this file holds the one shared instance used by
//! programs (window system calls, `syscall/ui.rs`) and the thread that feeds it mouse and keyboard input.
//! The server starts with the first program window and gives the screen back to the text console when
//! the last one closes.

pub mod server;

use crate::gfx::canvas::Rect;
use crate::sched::{self, Mutex, WaitQueue};
use core::sync::atomic::{AtomicBool, Ordering};
use server::{Event, Server, WindowId};

static SERVER: Mutex<Option<Server>> = Mutex::new(None);
/// Programs waiting for an event (woken after input was handled or a window closed).
static EVENTS: WaitQueue = WaitQueue::new();
/// The input thread waits here while there is no server.
static START: WaitQueue = WaitQueue::new();
static THREAD: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Why a window call failed.
#[derive(Debug, PartialEq, Eq)]
pub enum UiError {
    NoScreen,
    BadSize,
    /// No such window, or it belongs to another program.
    NoWindow,
}

const MAX_SIDE: usize = 4096;

/// Opens a window of `w` x `h` (contents) for the program on thread `owner`, centred, each new one a
/// little lower and to the right of the previous.
pub fn create_window(owner: u64, title: &str, w: usize, h: usize) -> Result<WindowId, UiError> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(UiError::BadSize);
    }
    let mut guard = SERVER.lock();
    if guard.is_none() {
        *guard = Some(Server::new().ok_or(UiError::NoScreen)?);
        RUNNING.store(true, Ordering::Release);
        if !THREAD.swap(true, Ordering::AcqRel) {
            sched::spawn_system(input_thread);
        }
        START.wake_all();
    }
    let s = guard.as_mut().unwrap();
    let (sw, sh) = s.size();
    let shift = 30 * (s.windows().count() % 8);
    let x = (sw.saturating_sub(w + 2 * server::BORDER) / 2 + shift).min(sw.saturating_sub(40));
    let y = (sh.saturating_sub(h + server::TITLE_H + server::BORDER) / 2 + shift).min(sh.saturating_sub(server::TITLE_H));
    let id = s.create_for(owner, title, x, y, w, h);
    s.present();
    Ok(id)
}

/// Copies the area `r` of `pixels` (the program's whole `w` x `h` image of the window) into the
/// window and shows it.
pub fn present(owner: u64, id: WindowId, pixels: &[u32], r: Rect) -> Result<(), UiError> {
    let mut guard = SERVER.lock();
    let s = guard.as_mut().ok_or(UiError::NoWindow)?;
    let win = s.window_mut(id).filter(|w| w.owner == owner).ok_or(UiError::NoWindow)?;
    let (w, h) = (win.client.width, win.client.height);
    if pixels.len() != w * h || r.x + r.w > w || r.y + r.h > h {
        return Err(UiError::BadSize);
    }
    for row in r.y..r.y + r.h {
        win.client.copy(r.x, row, r.w, 1, &pixels[row * w + r.x..row * w + r.x + r.w]);
    }
    s.present();
    Ok(())
}

/// Size of the window's contents.
pub fn window_size(owner: u64, id: WindowId) -> Result<(usize, usize), UiError> {
    let guard = SERVER.lock();
    let win = guard.as_ref().and_then(|s| s.window(id)).filter(|w| w.owner == owner).ok_or(UiError::NoWindow)?;
    Ok((win.client.width, win.client.height))
}

/// Waits for the next event of a window. `Err(NoWindow)` once the window is closed.
pub fn wait_event(owner: u64, id: WindowId) -> Result<Event, UiError> {
    let mut result = None;
    EVENTS.wait_until(|| {
        // Checked with interrupts off: take the lock only if it is free, never sleep here. If it is
        // taken, whoever holds it wakes us when it adds an event or closes a window.
        if let Some(mut guard) = SERVER.try_lock() {
            result = match guard.as_mut() {
                Some(s) if s.window(id).is_some_and(|w| w.owner == owner) => s.next_event(id).map(Ok),
                _ => Some(Err(UiError::NoWindow)),
            };
        }
        result.is_some()
    });
    result.unwrap()
}

/// Closes a window of the program on thread `owner`.
pub fn close_window(owner: u64, id: WindowId) -> Result<(), UiError> {
    let mut guard = SERVER.lock();
    let s = guard.as_mut().ok_or(UiError::NoWindow)?;
    if !s.window(id).is_some_and(|w| w.owner == owner) {
        return Err(UiError::NoWindow);
    }
    s.close(id);
    s.present();
    drop(guard);
    EVENTS.wake_all();
    Ok(())
}

/// Closes every window of a program that ended (called by `proc::on_exit`).
pub fn close_owned(owner: u64) {
    if !RUNNING.load(Ordering::Acquire) {
        return;
    }
    let mut guard = SERVER.lock();
    if let Some(s) = guard.as_mut() {
        let ids: alloc::vec::Vec<WindowId> = s.windows().filter(|w| w.owner == owner).map(|w| w.id).collect();
        for id in ids {
            s.close(id);
        }
        s.present();
    }
    drop(guard);
    EVENTS.wake_all();
}

/// Where the contents of the first window of a program start on the screen (for the self-tests).
#[cfg(feature = "qemu-test")]
pub fn window_of(owner: u64) -> Option<(usize, usize)> {
    SERVER.lock().as_ref().and_then(|s| s.windows().find(|w| w.owner == owner).map(|w| w.client_origin()))
}

/// Whether the window server has the screen now (for the self-tests).
#[cfg(feature = "qemu-test")]
pub fn running() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// Feeds mouse and keyboard input to the server every 10 ms while it runs; stops it when the last
/// window has closed and gives the screen back to the text console.
fn input_thread() {
    loop {
        START.wait_until(|| RUNNING.load(Ordering::Acquire));
        loop {
            let mut guard = SERVER.lock();
            let Some(s) = guard.as_mut() else { break };
            let any = s.pump();
            if s.is_empty() {
                *guard = None; // dropping the server turns text output back on
                crate::gfx::console::clear(); // still under the lock: a new server cannot draw first
                RUNNING.store(false, Ordering::Release);
                break;
            }
            drop(guard);
            if any {
                EVENTS.wake_all();
            }
            sched::sleep_ms(10);
        }
    }
}
