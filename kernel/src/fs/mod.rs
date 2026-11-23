//! Files: the VFS layer (vfs.rs), the in-memory root file system (ramfs.rs) and the initrd reader (tar.rs).
//! The functions here take absolute paths and work on whatever file system is mounted there.

pub mod bucati;
pub mod fat32;
pub mod part;
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
    if let Some(image) = crate::boot::file("disk.img") {
        mount_disk(alloc::boxed::Box::new(crate::block::MemDisk::new(image)), "/disk");
    }
}

/// Why mounting failed, per mount point (shown by `hw`): (mount point, reason).
static FAILURES: crate::sched::Mutex<alloc::vec::Vec<(String, String)>> = crate::sched::Mutex::new(alloc::vec::Vec::new());

/// Remembers why `at` could not be mounted.
pub fn note_failure(at: &str, why: &str) {
    let mut f = FAILURES.lock();
    f.retain(|(a, _)| a != at);
    f.push((String::from(at), String::from(why)));
}

/// The reason `at` is not mounted, if one was noted.
pub fn failure(at: &str) -> Option<String> {
    FAILURES.lock().iter().find(|(a, _)| a == at).map(|(_, w)| w.clone())
}

/// Finds the FAT partition of `dev` and mounts it at `at` (writable only if it has `fat32::WRITE_MARKER`).
pub fn mount_disk(dev: Box<dyn crate::block::BlockDevice>, at: &str) -> bool {
    match open_fat(dev, at) {
        Some((fat, p)) => mount_fat(fat, p, at),
        None => false,
    }
}

/// The FAT file system of `dev` (failures are logged and noted for `at`).
fn open_fat(dev: Box<dyn crate::block::BlockDevice>, at: &str) -> Option<(fat32::Fat32, part::Found)> {
    let Some(p) = part::find_fat(dev.as_ref()) else {
        crate::warn!("fs: disc fara partitie FAT ({})", at);
        note_failure(at, "nicio partitie FAT (nici GPT, nici MBR, nici fara tabela)");
        return None;
    };
    match fat32::Fat32::new(Box::new(crate::block::Partition::new(dev, p.start, p.sectors))) {
        Ok(f) => Some((f, p)),
        Err(msg) => {
            crate::warn!("fs: partitia {} de la sectorul {}: {}", p.table, p.start, msg);
            note_failure(at, &alloc::format!("partitia {} de la sectorul {}: {}", p.table, p.start, msg));
            None
        }
    }
}

/// Mounts at `at` the disk the OS started from, out of `disks` (P6-15, the workstation has a card reader and
/// several sticks): the first one with both `cyber.rw` and `VERSIUNE` in its root (what a Release stick has),
/// otherwise the first one with a FAT file system.
pub fn mount_best(disks: Vec<Box<dyn crate::block::BlockDevice>>, at: &str) -> bool {
    let total = disks.len();
    let mut opened: Vec<(fat32::Fat32, part::Found)> = disks.into_iter().filter_map(|d| open_fat(d, at)).collect();
    if opened.is_empty() {
        return false;
    }
    use vfs::FileSystem as _;
    let has = |f: &fat32::Fat32, name: &str| f.kind(&[name]).is_ok();
    let pick = opened.iter().position(|(f, _)| has(f, fat32::WRITE_MARKER) && has(f, "VERSIUNE"));
    if total > 1 {
        match pick {
            Some(i) => crate::info!("fs: {}: aleg discul {} din {} (are cyber.rw si VERSIUNE)", at, i + 1, total),
            None => crate::info!("fs: {}: niciun disc cu cyber.rw si VERSIUNE, aleg primul din {}", at, total),
        }
    }
    let (fat, p) = opened.swap_remove(pick.unwrap_or(0));
    let ok = mount_fat(fat, p, at);
    // C-74: the test stick (written as an image) has no room for the NVIDIA firmware, so Adrian keeps both sticks in:
    // the first other disk that has the firmware folder is mounted read-only at `<at>2` (e.g. /usb2).
    if let Some(i) = opened.iter().position(|(f, _)| f.kind(&FIRMWARE_DIR).is_ok_and(|k| k == Kind::Dir)) {
        let (mut fat, p) = opened.swap_remove(i);
        fat.set_read_only();
        let second = alloc::format!("{}2", at);
        if mount_fat(fat, p, &second) {
            crate::info!("fs: {}: al doilea stick, cu dosarul firmware/nvidia/ga102/gsp (doar citire)", second);
        }
    }
    ok
}

/// P6-100 (C-148 point 5): set when `/disk` is a partition of the computer's own disk (AHCI/NVMe) marked `cyber.rw`: the
/// gpu journal and the Kronos model then go there before the stick.
static OWN_RW: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Whether `/disk` is the computer's own disk with a partition marked `cyber.rw` (P6-100).
pub fn own_disk_writable() -> bool {
    OWN_RW.load(core::sync::atomic::Ordering::Relaxed)
}

/// P6-100: the computer's own disks (`(name, disk)`, NVMe first, then SATA). Every partition goes into the disk inventory
/// (`block::inventory`, reads only). Unless something is already mounted at `at` (the test image), mounts there the first
/// FAT partition marked `cyber.rw` on any of them; without one, as before P6-100: the first FAT partition of the first
/// disk that has one (read only). Nothing is written to a disk here.
pub fn mount_own(disks: Vec<(&'static str, Box<dyn crate::block::BlockDevice>)>, at: &str) {
    use alloc::sync::Arc;
    use crate::block::{BlockDevice, Partition, Shared};
    let busy = kind(at).is_ok();
    // (file system, partition, disk name, partition number)
    let mut marked: Option<(fat32::Fat32, part::Found, &'static str, usize)> = None;
    let mut first: Option<(fat32::Fat32, part::Found, &'static str, usize)> = None;
    for (name, dev) in disks {
        let dev: Arc<dyn BlockDevice> = Arc::from(dev);
        let entries = part::list(dev.as_ref());
        let mut rw_at: Vec<(u64, bool)> = Vec::new();
        for c in part::fat_candidates(dev.as_ref()) {
            let Ok(fat) = fat32::Fat32::new(Box::new(Partition::new(Box::new(Shared(dev.clone())), c.start, c.sectors))) else {
                continue;
            };
            rw_at.push((c.start, fat.writable()));
            let number = entries.iter().find(|e| e.start == c.start).map(|e| e.number).unwrap_or(0);
            if fat.writable() && marked.is_none() {
                marked = Some((fat, c, name, number));
            } else if first.is_none() {
                first = Some((fat, c, name, number));
            }
        }
        if entries.is_empty() {
            let what = match rw_at.first() {
                Some((_, true)) => "FAT fara tabela de partitii, cu cyber.rw",
                Some((_, false)) => "FAT fara tabela de partitii, fara cyber.rw",
                None => "nicio tabela de partitii inteleasa",
            };
            crate::block::note(format!("  {}: {}", name, what));
        }
        for e in &entries {
            let content = part::content(dev.as_ref(), e.start);
            let marker = match rw_at.iter().find(|(s, _)| *s == e.start) {
                Some((_, true)) => ", are cyber.rw",
                Some((_, false)) => ", fara cyber.rw",
                None => "",
            };
            crate::block::note(format!(
                "  {} partitia {} ({}, {}): de la sectorul {}, {} MiB, {}{}",
                name,
                e.number,
                e.table,
                e.kind,
                e.start,
                e.sectors / 2048,
                content,
                marker
            ));
        }
    }
    if busy {
        return;
    }
    let own = marked.is_some();
    match marked.or(first) {
        Some((fat, p, name, number)) => {
            if own {
                crate::info!("fs: {}: partitia {} ({}) de pe {} are cyber.rw: discul propriu, scriere permisa", at, number, p.table, name);
            }
            if mount_fat(fat, p, at) {
                OWN_RW.store(own, core::sync::atomic::Ordering::Relaxed);
                crate::block::note(format!(
                    "montat la {}: {} partitia {}{}",
                    at,
                    name,
                    number,
                    if own { " (cyber.rw: jurnalul gpu si modelul Kronos de aici)" } else { " (doar citire)" }
                ));
            }
        }
        None => note_failure(at, "niciun disc propriu cu o partitie FAT"),
    }
}

/// C-74: the folder of the NVIDIA GSP files on a stick (`nvidia_ga104::firmware::STICK_DIR` without `/usb/`).
const FIRMWARE_DIR: [&str; 4] = ["firmware", "nvidia", "ga102", "gsp"];

fn mount_fat(fat: fat32::Fat32, p: part::Found, at: &str) -> bool {
    let rw = if fat.writable() { " (scriere permisa)" } else { "" };
    let ok = create_dirs(at).is_ok() && vfs::mount(at, Box::new(fat)).is_ok();
    if ok {
        crate::info!("fs: FAT32 ({}, {} MiB) montat la {}{}", p.table, p.sectors * 512 / (1024 * 1024), at, rw);
    }
    ok
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

/// Renames a file (both paths on the same file system; used by /bin/actualizeaza, P6-08).
pub fn rename(from: &str, to: &str) -> Result<(), FsError> {
    vfs::with_fs2(from, to, |fs, a, b| fs.rename(a, b))
}

/// Writes out the cache of the disk that holds `path`.
pub fn flush(path: &str) -> Result<(), FsError> {
    vfs::with_fs(path, |fs, _| fs.flush())
}

/// P6-146 (C-158 point 7): flushes every mounted disk before a restart; returns one line for the log.
pub fn flush_all() -> String {
    match vfs::flush_all() {
        None => String::from("golirea discurilor: sarita (o scriere e in curs)"),
        Some(r) => {
            let bad: Vec<String> = r.iter().filter_map(|(at, x)| x.err().map(|e| format!("{} ({})", at, e.message()))).collect();
            if bad.is_empty() {
                format!("golirea discurilor: {} sisteme de fisiere golite", r.len())
            } else {
                format!("golirea discurilor: {} golite, refuzate: {}", r.len() - bad.len(), bad.join(", "))
            }
        }
    }
}
