//! Per-program permissions (task P6-23, command C-22): "promises" and "visible paths".
//!
//! - **Promises:** a program may declare the groups of system calls it uses (`promise(mask)`); from then on a call
//!   outside those groups fails with `E_DENIED`. The mask can only shrink.
//! - **Visible paths:** after the first `unveil(path, rights)` a program sees only the listed paths (and what is
//!   under them), each with read / write / create rights; `unveil` with an empty path closes the list for good.
//! - **Secret files** (`claude.key`, `github.token` in `/usb`, `/disk`, `/etc`) are invisible to every program that did not ask for
//!   that exact file with `secret(1, path)`; `secret(0)` gives up every secret for the rest of the program's life.
//!   The Claude programs and `gh-trimite` / `actualizeaza` ask for theirs; nothing else sees them (not even in `list`).
//!
//! A program started by another inherits its promises and visible paths (it cannot escape by spawning), but no
//! secret: it must ask itself.
//!
//! Ideas from the public documentation of OpenBSD pledge(2)/unveil(2), SerenityOS (man2 pledge, unveil) and Fuchsia
//! handle rights ("rights only go down"); the code is ours.

use alloc::{string::String, vec::Vec};

/// Groups of system calls (bits of the promise mask).
pub const P_STDIO: u64 = 1 << 0; // exit, write/read/close on open descriptors, yield, sleep, time, random, memory, threads, sysinfo
pub const P_RPATH: u64 = 1 << 1; // open, stat, list
pub const P_WPATH: u64 = 1 << 2; // create, remove, rename, sync, writing to files
pub const P_INET: u64 = 1 << 3; // resolve, tcp, udp, net_info, ping
pub const P_PROC: u64 = 1 << 4; // spawn, wait, kill
pub const P_UI: u64 = 1 << 5; // windows, events, terminals, screen size
pub const P_MODULE: u64 = 1 << 6; // the module manager
pub const P_ALL: u64 = (1 << 7) - 1;

/// Rights on a visible path.
pub const R_READ: u8 = 1;
pub const R_WRITE: u8 = 2;
pub const R_CREATE: u8 = 4;

/// Files only programs that ask for them may see: these names in these folders (compared without regard to case:
/// FAT). The real ones live on the stick; /disk and /etc are covered too (a copy there, and the QEMU tests).
pub const SECRET_NAMES: &[&str] = &["claude.key", "github.token"];
pub const SECRET_DIRS: &[&str] = &["/usb", "/disk", "/etc"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The call is outside the program's promises.
    Promise,
    /// The path is outside the visible paths, or a secret not asked for: the program sees "no such file".
    Hidden,
    /// The path is visible, but without the right needed.
    NoRight,
}

#[derive(Clone, Debug)]
pub struct Rights {
    promises: u64,
    /// `None` = every path visible (no `unveil` yet).
    unveiled: Option<Vec<(String, u8)>>,
    unveil_closed: bool,
    secrets: Vec<String>,
    secrets_closed: bool,
}

impl Default for Rights {
    fn default() -> Self {
        Rights { promises: P_ALL, unveiled: None, unveil_closed: false, secrets: Vec::new(), secrets_closed: false }
    }
}

/// `path` made absolute and canonical for the checks: lower case, no empty or `.` parts, `..` applied.
/// `None` for a relative path (the file system refuses those anyway).
pub fn canonical(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p.to_ascii_lowercase()),
        }
    }
    let mut s = String::new();
    for p in &parts {
        s.push('/');
        s.push_str(p);
    }
    if s.is_empty() {
        s.push('/');
    }
    Some(s)
}

/// Whether `path` (canonical) is `dir` (canonical) or below it.
fn under(path: &str, dir: &str) -> bool {
    dir == "/" || path == dir || (path.starts_with(dir) && path.as_bytes().get(dir.len()) == Some(&b'/'))
}

pub fn is_secret(canon: &str) -> bool {
    canon.rsplit_once('/').is_some_and(|(dir, name)| SECRET_DIRS.contains(&dir) && SECRET_NAMES.contains(&name))
}

impl Rights {
    /// What a program started by this one gets: the same limits, no secrets.
    pub fn inherit(&self) -> Rights {
        Rights { secrets: Vec::new(), secrets_closed: false, ..self.clone() }
    }

    pub fn promises(&self) -> u64 {
        self.promises
    }

    /// `promise(mask)`: keeps only the groups in `mask`; returns the new mask.
    pub fn promise(&mut self, mask: u64) -> u64 {
        self.promises &= mask & P_ALL;
        self.promises
    }

    pub fn check_call(&self, group: u64) -> Result<(), Denied> {
        if self.promises & group == group { Ok(()) } else { Err(Denied::Promise) }
    }

    /// `unveil(path, rights)`; an empty path closes the list. Refused once closed.
    pub fn unveil(&mut self, path: &str, rights: u8) -> Result<(), Denied> {
        if self.unveil_closed {
            return Err(Denied::NoRight);
        }
        if path.is_empty() {
            self.unveil_closed = true;
            self.unveiled.get_or_insert_with(Vec::new);
            return Ok(());
        }
        let canon = canonical(path).ok_or(Denied::Hidden)?;
        let list = self.unveiled.get_or_insert_with(Vec::new);
        match list.iter_mut().find(|(p, _)| *p == canon) {
            Some(e) => e.1 = rights & (R_READ | R_WRITE | R_CREATE),
            None => list.push((canon, rights & (R_READ | R_WRITE | R_CREATE))),
        }
        Ok(())
    }

    /// `secret(1, path)`: asks for one secret file. Refused after `secret(0)` or for a path that is not a secret.
    pub fn ask_secret(&mut self, path: &str) -> Result<(), Denied> {
        let canon = canonical(path).ok_or(Denied::Hidden)?;
        if self.secrets_closed || !is_secret(&canon) {
            return Err(Denied::NoRight);
        }
        if !self.secrets.contains(&canon) {
            self.secrets.push(canon);
        }
        Ok(())
    }

    /// `secret(0)`: drops every secret, for good.
    pub fn drop_secrets(&mut self) {
        self.secrets.clear();
        self.secrets_closed = true;
    }

    /// May the program reach `path` with `need` (R_* bits)?
    pub fn check_path(&self, path: &str, need: u8) -> Result<(), Denied> {
        let Some(canon) = canonical(path) else { return Ok(()) }; // relative: the file system refuses it
        if is_secret(&canon) && !self.secrets.contains(&canon) {
            return Err(Denied::Hidden);
        }
        let Some(list) = &self.unveiled else { return Ok(()) };
        // The most specific listed path that contains this one decides.
        let best = list.iter().filter(|(p, _)| under(&canon, p)).max_by_key(|(p, _)| p.len());
        match best {
            None => Err(Denied::Hidden),
            Some((_, r)) if r & need == need => Ok(()),
            Some(_) => Err(Denied::NoRight),
        }
    }

    /// Whether the entry `name` of directory `dir` shows in `list` (hidden secrets and hidden paths do not).
    pub fn visible_entry(&self, dir: &str, name: &str) -> bool {
        let full = if dir.ends_with('/') { alloc::format!("{}{}", dir, name) } else { alloc::format!("{}/{}", dir, name) };
        let Some(canon) = canonical(&full) else { return true };
        if is_secret(&canon) && !self.secrets.contains(&canon) {
            return false;
        }
        match &self.unveiled {
            None => true,
            // Listed below (so a path to it must stay visible), or inside a listed path.
            Some(list) => list.iter().any(|(p, _)| under(&canon, p) || under(p, &canon)),
        }
    }
}

/// Boot-time self test (every test build): the rules above on paths, without any program.
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let mut r = Rights::default();
    assert!(r.check_path("/usb/claude.key", R_READ) == Err(Denied::Hidden), "drepturi: cheia vizibila implicit");
    assert!(r.check_path("/USB//./x/../Claude.KEY", R_READ) == Err(Denied::Hidden), "drepturi: ocolire prin cale");
    assert!(r.check_path("/usb/altceva.txt", R_READ).is_ok() && r.visible_entry("/usb", "readme.txt"));
    assert!(r.check_path("/etc/github.token", R_READ).is_err() && r.check_path("/usb/x/claude.key", R_READ).is_ok());
    assert!(!r.visible_entry("/usb", "CLAUDE.KEY"), "drepturi: cheia apare in lista");
    assert!(r.ask_secret("/usb/claude.key").is_ok() && r.check_path("/usb/claude.key", R_READ).is_ok());
    assert!(r.check_path("/usb/github.token", R_READ).is_err(), "drepturi: alt secret dat odata cu primul");
    assert!(r.inherit().check_path("/usb/claude.key", R_READ).is_err(), "drepturi: secret mostenit");
    r.drop_secrets();
    assert!(r.check_path("/usb/claude.key", R_READ).is_err() && r.ask_secret("/usb/claude.key").is_err());
    assert!(r.unveil("/disk/date", R_READ).is_ok() && r.unveil("/disk/date/scris", R_READ | R_WRITE | R_CREATE).is_ok());
    assert!(r.check_path("/disk/date/a.txt", R_READ).is_ok() && r.check_path("/disk/date/a.txt", R_WRITE) == Err(Denied::NoRight));
    assert!(r.check_path("/disk/date/scris/b", R_CREATE).is_ok() && r.check_path("/disk/datele", R_READ) == Err(Denied::Hidden));
    assert!(r.visible_entry("/", "disk") && !r.visible_entry("/disk", "altul") && r.visible_entry("/disk", "DATE"));
    assert!(r.unveil("", 0).is_ok() && r.unveil("/", R_READ).is_err(), "drepturi: lista redeschisa");
    assert!(r.promise(P_STDIO | P_RPATH) == P_STDIO | P_RPATH && r.promise(P_ALL) == P_STDIO | P_RPATH, "drepturi: promisiuni largite");
    assert!(r.check_call(P_INET) == Err(Denied::Promise) && r.check_call(P_RPATH).is_ok());
    crate::info!("test: drepturi pe program OK (promisiuni, cai vizibile, secrete)");
}
