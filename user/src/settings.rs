//! System settings, kept in `PATHS` (stick first) as `key = "value"` lines (the same small TOML subset as module.toml).
//! Keys: `fus_orar` (hours added to the computer's clock, e.g. "+3") and `tastatura` ("us" or "ro").

/// P6-61 (C-84 item 7): the settings live on the stick when it is mounted with writing (they travel with it);
/// the test disk / older place `/disk/config/` is the fallback (read when the stick has none, written when the
/// stick cannot be written).
pub const PATHS: [(&str, &str); 2] = [("/usb/config", "/usb/config/settings.toml"), ("/disk/config", "/disk/config/settings.toml")];

/// Reads the first settings file that exists into `buf`; empty text if there is none.
pub fn read(buf: &mut [u8]) -> &str {
    for (_, path) in PATHS {
        let fd = crate::open(path);
        if fd < 0 {
            continue;
        }
        let n = crate::read(fd, buf).max(0) as usize;
        crate::close(fd);
        return core::str::from_utf8(&buf[..n]).unwrap_or("");
    }
    ""
}

/// Writes `text` to the first place that accepts it; returns that path, or the last error.
pub fn save(text: &[u8]) -> Result<&'static str, i64> {
    let mut last = -3;
    for (dir, path) in PATHS {
        let _ = crate::create(dir, true); // may exist already; fails (and so does the file) when nothing is mounted there
        let r = crate::remove(path);
        let r = if r < 0 && r != -3 { r } else { crate::create(path, false) };
        let fd = if r < 0 { r } else { crate::open(path) };
        if fd < 0 {
            last = fd;
            continue;
        }
        let n = crate::write(fd as u64, text);
        crate::close(fd);
        if n == text.len() as i64 {
            return Ok(path);
        }
        last = if n < 0 { n } else { -5 };
    }
    Err(last)
}

/// The value of `key` in settings text.
pub fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        let v = v.trim();
        (k.trim() == key && v.len() >= 2 && v.starts_with('"') && v.ends_with('"')).then(|| &v[1..v.len() - 1])
    })
}

/// The time zone in hours (from "+3", "-1", "0"); 0 if not set.
pub fn time_zone(text: &str) -> i32 {
    let Some(v) = value(text, "fus_orar") else { return 0 };
    let (sign, digits) = match v.strip_prefix('-') {
        Some(d) => (-1, d),
        None => (1, v.trim_start_matches('+')),
    };
    digits.parse::<i32>().map(|h| (sign * h).clamp(-12, 14)).unwrap_or(0)
}
