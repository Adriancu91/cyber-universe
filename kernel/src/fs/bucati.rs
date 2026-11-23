//! P6-146 (C-158 point 7): big data on the own disk and safe rewrites.
//!
//! FAT32 keeps a file's size in 32 bits, so one file holds at most 4 GiB − 1 bytes. A bigger piece of data (a training
//! set, a checkpoint) is kept as consecutive chunks `name.000`, `name.001`, … each at most `CHUNK_MAX` bytes; the
//! functions here read and write them as one file. `write_safe` rewrites a file so that a power cut at any moment leaves
//! either the old or the new content in place (never half of the new one); `recover` puts things back after such a cut.

use super::{FsError, Kind};
use alloc::{format, string::String, vec, vec::Vec};

/// The largest chunk: 4 GiB minus 64 KiB (under the FAT32 limit and a whole number of clusters of up to 64 KiB).
pub const CHUNK_MAX: u64 = 0x1_0000_0000 - 0x1_0000;

/// The most chunks one piece of data may have (`.000` … `.999`).
const MAX_CHUNKS: usize = 1000;

fn chunk_name(base: &str, i: usize) -> String {
    format!("{}.{:03}", base, i)
}

/// The chunks of `base` that exist, in order, from `.000` up to the first missing one: `(path, size)`.
pub fn chunks(base: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for i in 0..MAX_CHUNKS {
        let p = chunk_name(base, i);
        match super::size(&p) {
            Ok(n) if super::kind(&p) == Ok(Kind::File) => out.push((p, n)),
            _ => break,
        }
    }
    out
}

/// The total size of the data `base` (all its chunks).
pub fn size_chunked(base: &str) -> Result<u64, FsError> {
    let c = chunks(base);
    if c.is_empty() {
        return Err(FsError::NotFound);
    }
    Ok(c.iter().map(|(_, n)| n).sum())
}

/// Writes `data` at `offset` of the data `base`, split into chunks of `chunk` bytes (`CHUNK_MAX` on the real disk; the
/// tests use small ones). Every chunk before the last one written is full: writing past the end of a chunk continues in
/// the next one. Returns the bytes written.
pub fn write_chunked(base: &str, offset: u64, data: &[u8], chunk: u64) -> Result<usize, FsError> {
    if chunk == 0 || chunk > CHUNK_MAX {
        return Err(FsError::InvalidPath);
    }
    let mut done = 0usize;
    while done < data.len() {
        let at = offset + done as u64;
        let i = (at / chunk) as usize;
        if i >= MAX_CHUNKS {
            return Err(FsError::NoSpace);
        }
        let inside = at % chunk;
        let take = core::cmp::min((chunk - inside) as usize, data.len() - done);
        let p = chunk_name(base, i);
        if super::kind(&p).is_err() {
            super::create(&p, Kind::File)?;
        }
        let n = super::write(&p, inside, &data[done..done + take])?;
        if n != take {
            return Err(FsError::NoSpace);
        }
        done += take;
    }
    Ok(done)
}

/// Reads from `offset` of the data `base` into `buf`, across chunks; returns the bytes read (0 at the end). The chunk
/// size is taken from the first chunk (every chunk but the last one is full).
pub fn read_chunked(base: &str, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
    let c = chunks(base);
    if c.is_empty() {
        return Err(FsError::NotFound);
    }
    let mut start = 0u64;
    let mut done = 0usize;
    for (p, n) in &c {
        let end = start + n;
        let at = offset + done as u64;
        if done < buf.len() && at < end && at >= start {
            let inside = at - start;
            let take = core::cmp::min((end - at) as usize, buf.len() - done);
            let got = super::read(p, inside, &mut buf[done..done + take])?;
            done += got;
            if got < take {
                break;
            }
        }
        start = end;
    }
    Ok(done)
}

/// The whole data `base` (all chunks) in memory.
pub fn read_all_chunked(base: &str) -> Result<Vec<u8>, FsError> {
    let total = size_chunked(base)?;
    let mut buf = vec![0u8; total as usize];
    let n = read_chunked(base, 0, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Removes every chunk of `base`; returns how many there were.
pub fn remove_chunked(base: &str) -> Result<usize, FsError> {
    let c = chunks(base);
    for (p, _) in &c {
        super::remove(p)?;
    }
    Ok(c.len())
}

fn new_name(path: &str) -> String {
    format!("{}.nou", path)
}

fn old_name(path: &str) -> String {
    format!("{}.vechi", path)
}

/// Rewrites the file `path` with `data` so that a power cut at any step leaves a whole version in place: the new
/// content goes to `path.nou` and is flushed; the old file becomes `path.vechi`; `path.nou` becomes `path`; a flush;
/// `path.vechi` is removed; a last flush. `recover` finishes or undoes a rewrite cut in the middle.
pub fn write_safe(path: &str, data: &[u8]) -> Result<(), FsError> {
    recover(path)?;
    let (new, old) = (new_name(path), old_name(path));
    if super::kind(&new).is_ok() {
        super::remove(&new)?;
    }
    super::create(&new, Kind::File)?;
    if super::write(&new, 0, data)? != data.len() {
        return Err(FsError::NoSpace);
    }
    super::flush(&new)?;
    let had_old = super::kind(path).is_ok();
    if had_old {
        super::rename(path, &old)?;
    }
    super::rename(&new, path)?;
    super::flush(path)?;
    if had_old {
        super::remove(&old)?;
        super::flush(path)?;
    }
    Ok(())
}

/// What `recover` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovered {
    /// Nothing was left over.
    Clean,
    /// The cut came before the new file was in place: the old version is back at `path`.
    OldBack,
    /// The cut came after the new file was in place: the leftover old version was removed.
    OldRemoved,
    /// A new file whose writing did not finish was removed; `path` is unchanged.
    NewDropped,
}

/// Puts `path` back in order after a rewrite cut by a power loss (see `write_safe`).
pub fn recover(path: &str) -> Result<Recovered, FsError> {
    let (new, old) = (new_name(path), old_name(path));
    let has = |p: &str| super::kind(p).is_ok();
    let mut what = Recovered::Clean;
    if has(&old) {
        if has(path) {
            super::remove(&old)?;
            what = Recovered::OldRemoved;
        } else {
            super::rename(&old, path)?;
            what = Recovered::OldBack;
        }
    }
    if has(&new) {
        super::remove(&new)?;
        if what == Recovered::Clean {
            what = Recovered::NewDropped;
        }
    }
    if what != Recovered::Clean {
        super::flush(path)?;
    }
    Ok(what)
}

/// `disc proba` (P6-146): on the own disk marked `cyber.rw` only — chunked data across several chunks and a safe
/// rewrite in `/disk/proba-disc/`, read back, compared and removed. Returns the report lines and whether all passed.
pub fn self_check() -> (Vec<String>, bool) {
    let mut rows = Vec::new();
    if !super::own_disk_writable() {
        rows.push(String::from("disc proba: /disk nu e partitia proprie marcata cyber.rw; nu scriu nimic"));
        return (rows, false);
    }
    let dir = "/disk/proba-disc";
    let r = (|| -> Result<(), String> {
        let e = |what: &str, err: FsError| format!("{}: {}", what, err.message());
        if super::kind(dir).is_err() {
            super::create(dir, Kind::Dir).map_err(|x| e("dosarul", x))?;
        }
        let base = "/disk/proba-disc/date";
        let _ = remove_chunked(base);
        let chunk = 64 * 1024;
        let data: Vec<u8> = (0..300 * 1024u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        write_chunked(base, 0, &data, chunk).map_err(|x| e("scrierea in bucati", x))?;
        super::flush(base).ok();
        let c = chunks(base);
        rows.push(format!("disc proba: {} octeti in {} bucati de cel mult {} KiB", data.len(), c.len(), chunk / 1024));
        let back = read_all_chunked(base).map_err(|x| e("citirea bucatilor", x))?;
        if back != data {
            return Err(String::from("bucatile citite inapoi difera"));
        }
        let mut mid = vec![0u8; 1000];
        let n = read_chunked(base, chunk - 500, &mut mid).map_err(|x| e("citirea peste granita", x))?;
        if n != 1000 || mid[..] != data[(chunk - 500) as usize..(chunk + 500) as usize] {
            return Err(String::from("citirea peste granita dintre bucati difera"));
        }
        rows.push(String::from("disc proba: bucatile citite inapoi identic (si peste granita)"));
        let f = "/disk/proba-disc/sigur.txt";
        write_safe(f, b"versiunea 1\n").map_err(|x| e("golirea sigura (1)", x))?;
        write_safe(f, b"versiunea 2, mai lunga\n").map_err(|x| e("golirea sigura (2)", x))?;
        if super::read_all(f).map_err(|x| e("citirea", x))? != b"versiunea 2, mai lunga\n" {
            return Err(String::from("golirea sigura: continutul difera"));
        }
        if super::kind(&new_name(f)).is_ok() || super::kind(&old_name(f)).is_ok() {
            return Err(String::from("golirea sigura: au ramas fisiere .nou / .vechi"));
        }
        rows.push(String::from("disc proba: golirea sigura a rescris fisierul (fara resturi)"));
        remove_chunked(base).map_err(|x| e("stergerea bucatilor", x))?;
        super::remove(f).map_err(|x| e("stergerea", x))?;
        super::remove(dir).map_err(|x| e("stergerea dosarului", x))?;
        super::flush("/disk").map_err(|x| e("golirea discului", x))?;
        Ok(())
    })();
    match r {
        Ok(()) => {
            rows.push(String::from("disc proba: TRECUT (fisierele de proba sterse)"));
            (rows, true)
        }
        Err(why) => {
            rows.push(format!("disc proba: ESUAT: {}", why));
            (rows, false)
        }
    }
}
