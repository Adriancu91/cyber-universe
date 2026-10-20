//! Window system calls (P4-05): a program opens windows, copies its picture into them and waits for
//! key presses and clicks. The picture stays in the program's memory; `window_present` copies the
//! changed area into the window server (simpler and safer than sharing frames: nothing to free twice).

use super::{E_FAULT, E_INVALID, E_NOT_FOUND, user_range_ok};
use crate::drivers::keyboard::Key;
use crate::gfx::canvas::Rect;
use crate::sched;
use crate::ui::{self, UiError, server::Event};

fn ui_error(e: UiError) -> i64 {
    match e {
        UiError::NoWindow => E_NOT_FOUND,
        UiError::NoScreen | UiError::BadSize => E_INVALID,
    }
}

/// window_create(size = width | height << 32, title, title_len) -> window id (> 0). With bit 63 of
/// `size` set, the window is the desktop (whole screen, below the others; size and title ignored).
pub fn window_create(size: u64, title: u64, title_len: u64) -> i64 {
    if size & 1 << 63 != 0 {
        return match ui::create_desktop(sched::current_id()) {
            Ok(id) => id as i64,
            Err(e) => ui_error(e),
        };
    }
    if title_len > 128 {
        return E_INVALID;
    }
    if !user_range_ok(title, title_len, false) {
        return E_FAULT;
    }
    // SAFETY: the range was checked to be mapped user memory of this program.
    let bytes = unsafe { core::slice::from_raw_parts(title as *const u8, title_len as usize) };
    let Ok(title) = core::str::from_utf8(bytes) else { return E_INVALID };
    let (w, h) = ((size & 0xFFFF_FFFF) as usize, (size >> 32) as usize);
    match ui::create_window(sched::current_id(), title, w, h) {
        Ok(id) => id as i64,
        Err(e) => ui_error(e),
    }
}

/// window_present(id, pixels, area = x | y << 16 | w << 32 | h << 48, stride): copies a w x h picture
/// (0x00RRGGBB) from the program's memory to (x, y) in the window and shows it. Row i of the picture
/// starts `stride` pixels after row i - 1 (stride 0 = one row repeated, for plain colors).
pub fn window_present(id: u64, pixels: u64, area: u64, stride: u64) -> i64 {
    let f = |shift: u32| ((area >> shift) & 0xFFFF) as usize;
    let r = Rect::new(f(0), f(16), f(32), f(48));
    if stride > 1 << 16 {
        return E_INVALID;
    }
    let len = if r.h == 0 { 0 } else { (r.h as u64 - 1) * stride + r.w as u64 };
    if pixels % 4 != 0 || !user_range_ok(pixels, len * 4, false) {
        return E_FAULT;
    }
    // SAFETY: the range was checked to be mapped user memory of this program, and it is 4-byte aligned.
    let image = unsafe { core::slice::from_raw_parts(pixels as *const u32, len as usize) };
    match ui::present(sched::current_id(), id as u32, image, stride as usize, r) {
        Ok(()) => 0,
        Err(e) => ui_error(e),
    }
}

/// event_wait(id, timeout_ms) -> next event, packed: kind << 48 | data. Kind 1 = key (data = the
/// character; Enter = '\n', Backspace = 8, Tab = '\t', Escape = 27); kind 2 = click (buttons << 32 |
/// x << 16 | y); 0 = no event within `timeout_ms` (0 = wait without limit). E_NOT_FOUND once the
/// window is closed.
pub fn event_wait(id: u64, timeout_ms: u64) -> i64 {
    match ui::wait_event(sched::current_id(), id as u32, timeout_ms) {
        Ok(None) => 0,
        Ok(Some(Event::Key(k))) => {
            let c = match k {
                Key::Char(c) => c as u64,
                Key::Enter => '\n' as u64,
                Key::Backspace => 8,
                Key::Tab => '\t' as u64,
                Key::Escape => 27,
            };
            (1 << 48 | c) as i64
        }
        Ok(Some(Event::Click { x, y, buttons })) => (2u64 << 48 | (buttons as u64) << 32 | (x as u64 & 0xFFFF) << 16 | (y as u64 & 0xFFFF)) as i64,
        Err(e) => ui_error(e),
    }
}

/// window_close(id).
pub fn window_close(id: u64) -> i64 {
    match ui::close_window(sched::current_id(), id as u32) {
        Ok(()) => 0,
        Err(e) => ui_error(e),
    }
}

/// screen_size() -> width | height << 32.
pub fn screen_size() -> i64 {
    match ui::screen_size() {
        Some((w, h)) => (w as u64 | (h as u64) << 32) as i64,
        None => E_INVALID,
    }
}
