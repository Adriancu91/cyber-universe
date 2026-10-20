//! Unified kernel log: every message goes to the serial port and, once it exists, to the screen console.
//!
//! Format: `[  12.345] INFO  module: message` (time in seconds since boot; `-.---` until the timer exists).

use core::fmt::{self, Write};

#[allow(dead_code)] // every level is part of the public log API, even if not all are used yet
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERR",
        }
    }

    fn color(self) -> u32 {
        match self {
            Level::Debug => 0x0080_8890,
            Level::Info => crate::gfx::console::FOREGROUND,
            Level::Warn => 0x00FF_C040,
            Level::Error => 0x00FF_5050,
        }
    }
}

/// Lowest level that is printed. Debug messages only appear in debug builds.
const MIN_LEVEL: Level = if cfg!(debug_assertions) { Level::Debug } else { Level::Info };

/// Small fixed buffer so one log line is formatted once and sent to both outputs.
struct LineBuf {
    buf: [u8; 256],
    len: usize,
}

impl Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

impl LineBuf {
    fn as_str(&self) -> &str {
        // Truncation may cut a multi-byte character; keep only the valid prefix.
        match core::str::from_utf8(&self.buf[..self.len]) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&self.buf[..e.valid_up_to()]).unwrap_or(""),
        }
    }
}

#[doc(hidden)]
pub fn _log(level: Level, args: fmt::Arguments) {
    if level < MIN_LEVEL {
        return;
    }
    let mut line = LineBuf { buf: [0; 256], len: 0 };
    match crate::time::uptime_ms() {
        Some(ms) => {
            let _ = write!(line, "[{:>4}.{:03}] ", ms / 1000, ms % 1000);
        }
        None => {
            let _ = line.write_str("[   -.---] ");
        }
    }
    let _ = write!(line, "{:<5} {}\n", level.label(), args);
    let text = line.as_str();

    crate::drivers::serial::write_str(text);
    crate::gfx::console::with_console(|c| {
        if let Some(c) = c {
            let old = c.color;
            c.color = level.color();
            let _ = c.write_str(text);
            c.color = old;
        }
    });
}

/// Logs at DEBUG level (debug builds only).
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::log::_log($crate::log::Level::Debug, format_args!($($arg)*)) };
}

/// Logs at INFO level.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::_log($crate::log::Level::Info, format_args!($($arg)*)) };
}

/// Logs at WARN level.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log::_log($crate::log::Level::Warn, format_args!($($arg)*)) };
}

/// Logs at ERR level.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::_log($crate::log::Level::Error, format_args!($($arg)*)) };
}
