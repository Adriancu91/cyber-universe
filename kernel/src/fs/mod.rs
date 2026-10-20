//! Files: the VFS layer (vfs.rs), the in-memory root file system (ramfs.rs) and the initrd reader (tar.rs).
//! The functions here take absolute paths and work on whatever file system is mounted there.

pub mod ramfs;
pub mod tar;
pub mod vfs;

pub use vfs::{FsError, Kind};
use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

/// Mounts an empty ramfs at `/` and fills it from the `initrd.tar` boot file, if there is one.
pub fn init() {
    if vfs::mount("/", Box::new(ramfs::RamFs::new())).is_err() {
        crate::error!("fs: nu pot monta radacina");
        return;
    }
    let Some(archive) = crate::boot::file("initrd.tar") else {
        crate::warn!("fs: initrd.tar lipseste, radacina e goala");
        return;
    };
    let mut bytes = 0;
    let result = tar::for_each(archive, |e| {
        let path = format!("/{}", e.name);
        let ok = if e.is_dir {
            create_dirs(&path)
        } else {
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            create_dirs(if parent.is_empty() { "/" } else { parent })
                .and_then(|_| create(&path, Kind::File))
                .and_then(|_| write(&path, 0, e.data).map(|_| ()))
        };
        match ok {
            Ok(()) => bytes += e.data.len(),
            Err(err) => crate::warn!("fs: {} din initrd: {}", path, err.message()),
        }
    });
    match result {
        Ok(n) => crate::info!("fs: radacina din initrd: {} intrari, {} KiB", n, bytes / 1024),
        Err(msg) => crate::error!("fs: initrd stricat: {}", msg),
    }
}

/// Creates `path` and every missing directory above it.
fn create_dirs(path: &str) -> Result<(), FsError> {
    let mut current = String::new();
    for c in vfs::components(path)? {
        current.push('/');
        current.push_str(c);
        match create(&current, Kind::Dir) {
            Ok(()) | Err(FsError::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[allow(dead_code)] // used by the self-tests now, by the shell (P2-09)
pub fn kind(path: &str) -> Result<Kind, FsError> {
    vfs::with_fs(path, |fs, p| fs.kind(p))
}

#[allow(dead_code)] // used by the self-tests now, by the shell and file syscalls (P2-08, P2-09)
pub fn size(path: &str) -> Result<u64, FsError> {
    vfs::with_fs(path, |fs, p| fs.size(p))
}

#[allow(dead_code)] // used by file syscalls (P2-08)
pub fn read(path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
    vfs::with_fs(path, |fs, p| fs.read(p, offset, buf))
}

/// The whole content of the file at `path`.
#[allow(dead_code)] // used by the self-tests now, by the shell and the program loader (P2-08, P2-09)
pub fn read_all(path: &str) -> Result<Vec<u8>, FsError> {
    vfs::with_fs(path, |fs, p| {
        let mut data = vec![0u8; fs.size(p)? as usize];
        if fs.kind(p)? == Kind::Dir {
            return Err(FsError::IsADirectory);
        }
        let n = fs.read(p, 0, &mut data)?;
        data.truncate(n);
        Ok(data)
    })
}

pub fn write(path: &str, offset: u64, data: &[u8]) -> Result<usize, FsError> {
    vfs::with_fs(path, |fs, p| fs.write(p, offset, data))
}

pub fn create(path: &str, kind: Kind) -> Result<(), FsError> {
    vfs::with_fs(path, |fs, p| fs.create(p, kind))
}

#[allow(dead_code)] // used by the self-tests now, by the shell (P2-09)
pub fn list(path: &str) -> Result<Vec<(String, Kind)>, FsError> {
    vfs::with_fs(path, |fs, p| fs.list(p))
}

#[allow(dead_code)] // used by the self-tests now, by the shell (P2-09)
pub fn remove(path: &str) -> Result<(), FsError> {
    vfs::with_fs(path, |fs, p| fs.remove(p))
}
