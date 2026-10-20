//! The common layer for files: paths, errors, the `FileSystem` interface and the mount table.
//! Every path is absolute (`/bin/hello`); a file system mounted at `/disk` sees `/disk/a.txt` as `/a.txt`.

use alloc::{boxed::Box, string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotADirectory,
    IsADirectory,
    AlreadyExists,
    NotEmpty,
    InvalidPath,
    ReadOnly,
    NoSpace,
    /// The disk refused a read or a write.
    Io,
}

impl FsError {
    pub fn message(self) -> &'static str {
        match self {
            FsError::NotFound => "nu exista",
            FsError::NotADirectory => "nu e director",
            FsError::IsADirectory => "e director",
            FsError::AlreadyExists => "exista deja",
            FsError::NotEmpty => "directorul nu e gol",
            FsError::InvalidPath => "cale invalida",
            FsError::ReadOnly => "doar citire",
            FsError::NoSpace => "nu mai e loc pe disc",
            FsError::Io => "eroare de citire sau scriere pe disc",
        }
    }
}

/// Operations every file system provides. Paths given to it are relative to its mount point and
/// already split into components (no empty parts, no `.` or `..`).
pub trait FileSystem: Send + Sync {
    fn kind(&self, path: &[&str]) -> Result<Kind, FsError>;
    fn size(&self, path: &[&str]) -> Result<u64, FsError>;
    /// Reads from `offset` into `buf`; returns the number of bytes read (0 at the end of the file).
    fn read(&self, path: &[&str], offset: u64, buf: &mut [u8]) -> Result<usize, FsError>;
    /// Writes `data` at `offset`, growing the file if needed.
    fn write(&self, path: &[&str], offset: u64, data: &[u8]) -> Result<usize, FsError>;
    fn create(&self, path: &[&str], kind: Kind) -> Result<(), FsError>;
    fn list(&self, path: &[&str]) -> Result<Vec<(String, Kind)>, FsError>;
    fn remove(&self, path: &[&str]) -> Result<(), FsError>;
}

/// Splits an absolute path into components, resolving `.` and `..`.
pub fn components(path: &str) -> Result<Vec<&str>, FsError> {
    if !path.starts_with('/') {
        return Err(FsError::InvalidPath);
    }
    let mut parts = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    Ok(parts)
}

struct Mount {
    at: Vec<String>,
    fs: Box<dyn FileSystem>,
}

static MOUNTS: crate::sched::Mutex<Vec<Mount>> = crate::sched::Mutex::new(Vec::new());

/// Makes `fs` visible at the directory `at` (`/` for the root file system).
pub fn mount(at: &str, fs: Box<dyn FileSystem>) -> Result<(), FsError> {
    let at: Vec<String> = components(at)?.into_iter().map(String::from).collect();
    MOUNTS.lock().push(Mount { at, fs });
    Ok(())
}

/// Runs `f` with the file system that holds `path` (longest matching mount point) and the path inside it.
pub fn with_fs<R>(path: &str, f: impl FnOnce(&dyn FileSystem, &[&str]) -> Result<R, FsError>) -> Result<R, FsError> {
    let parts = components(path)?;
    let mounts = MOUNTS.lock();
    let best = mounts
        .iter()
        .filter(|m| m.at.len() <= parts.len() && m.at.iter().zip(&parts).all(|(a, b)| a == b))
        .max_by_key(|m| m.at.len())
        .ok_or(FsError::NotFound)?;
    f(best.fs.as_ref(), &parts[best.at.len()..])
}
