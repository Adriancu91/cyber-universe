//! Reader for `tar` archives in the POSIX ustar format (the initrd). Written from the ustar header
//! description in POSIX (IEEE Std 1003.1, `pax` utility, "ustar Interchange Format").

/// One entry of the archive.
pub struct Entry<'a> {
    /// Full name (prefix + name), without a leading `./` or `/`.
    pub name: alloc::string::String,
    pub is_dir: bool,
    pub data: &'a [u8],
}

fn octal(field: &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for &b in field {
        match b {
            b'0'..=b'7' => v = v.checked_mul(8)?.checked_add((b - b'0') as u64)?,
            b' ' | 0 => {}
            _ => return None,
        }
    }
    Some(v)
}

fn text(field: &[u8]) -> &str {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).unwrap_or("")
}

/// Calls `f` for every regular file and directory of `archive`. Stops at the end marker; returns an
/// error message for a damaged archive. Other entry types (links, devices) are skipped.
pub fn for_each<'a>(archive: &'a [u8], mut f: impl FnMut(Entry<'a>)) -> Result<usize, &'static str> {
    let mut pos = 0;
    let mut count = 0;
    while pos + 512 <= archive.len() {
        let h = &archive[pos..pos + 512];
        if h.iter().all(|&b| b == 0) {
            return Ok(count); // end of archive
        }
        if &h[257..262] != b"ustar" {
            return Err("arhiva nu e in format ustar");
        }
        let stored = octal(&h[148..156]).ok_or("suma de control ilizibila")?;
        let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { b' ' as u64 } else { b as u64 }).sum();
        if sum != stored {
            return Err("suma de control gresita");
        }
        let size = octal(&h[124..136]).ok_or("marime ilizibila")? as usize;
        let data_start = pos + 512;
        let data_end = data_start.checked_add(size).filter(|&e| e <= archive.len()).ok_or("fisier trunchiat")?;
        let (prefix, name) = (text(&h[345..500]), text(&h[0..100]));
        let mut full = alloc::string::String::new();
        if !prefix.is_empty() {
            full.push_str(prefix);
            full.push('/');
        }
        full.push_str(name);
        let trimmed = full.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/');
        let kind = h[156];
        if !trimmed.is_empty() && (kind == b'0' || kind == 0 || kind == b'5') {
            f(Entry { name: alloc::string::String::from(trimmed), is_dir: kind == b'5', data: &archive[data_start..data_end] });
            count += 1;
        }
        pos = data_start + size.div_ceil(512) * 512;
    }
    Ok(count)
}
