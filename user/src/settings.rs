//! System settings, kept in `PATH` as `key = "value"` lines (the same small TOML subset as module.toml).
//! Keys: `fus_orar` (hours added to the computer's clock, e.g. "+3") and `tastatura` ("us" or "ro").

pub const PATH: &str = "/disk/config/settings.toml";

/// Reads the settings file into `buf`; empty text if there is none.
pub fn read(buf: &mut [u8]) -> &str {
    let fd = crate::open(PATH);
    if fd < 0 {
        return "";
    }
    let n = crate::read(fd, buf).max(0) as usize;
    crate::close(fd);
    core::str::from_utf8(&buf[..n]).unwrap_or("")
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
