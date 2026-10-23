//! System calls: the services user programs ask the kernel for with the `syscall` instruction.
//! The entry code is in `arch/x86_64/syscall.rs`; this file checks the arguments and does the work.

use crate::mem::{address_space::USER_END, map::PAGE_SIZE, paging};
use crate::sched;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_YIELD: u64 = 2;
pub const SYS_SLEEP_MS: u64 = 3;
pub const SYS_OPEN: u64 = 4;
pub const SYS_READ: u64 = 5;
pub const SYS_CLOSE: u64 = 6;
pub const SYS_STAT: u64 = 7;
pub const SYS_WINDOW_CREATE: u64 = 8;
pub const SYS_WINDOW_PRESENT: u64 = 9;
pub const SYS_EVENT_WAIT: u64 = 10;
pub const SYS_WINDOW_CLOSE: u64 = 11;
pub const SYS_SPAWN: u64 = 12;
pub const SYS_WAIT: u64 = 13;
pub const SYS_LIST: u64 = 14;
pub const SYS_TIME: u64 = 15;
pub const SYS_SCREEN_SIZE: u64 = 16;
pub const SYS_SYSINFO: u64 = 17;
pub const SYS_KILL: u64 = 18;
pub const SYS_CREATE: u64 = 19;
pub const SYS_REMOVE: u64 = 20;
pub const SYS_MODULE: u64 = 21;
pub const SYS_TERM_OPEN: u64 = 22;
pub const SYS_TERM_READ: u64 = 23;
pub const SYS_TERM_WRITE: u64 = 24;
pub const SYS_RESOLVE: u64 = 25;
pub const SYS_TCP_CONNECT: u64 = 26;
pub const SYS_UDP_OPEN: u64 = 27;
pub const SYS_UDP_SEND: u64 = 28;
pub const SYS_UDP_RECV: u64 = 29;
pub const SYS_NET_INFO: u64 = 30;
pub const SYS_PING: u64 = 31;
pub const SYS_RANDOM: u64 = 32;
pub const SYS_RENAME: u64 = 33;
pub const SYS_SYNC: u64 = 34;
pub const SYS_MEM_MAP: u64 = 35;
pub const SYS_MEM_UNMAP: u64 = 36;

mod net;
mod ui;

/// Error results (negative, like the convention most systems use).
pub const E_INVALID: i64 = -1; // unknown call or bad argument
pub const E_FAULT: i64 = -2; // pointer outside the program's memory
pub const E_NOT_FOUND: i64 = -3; // no such file or directory
pub const E_IS_DIR: i64 = -4; // a directory where a file is needed
pub const E_BAD_FD: i64 = -5; // descriptor not open
pub const E_TOO_MANY: i64 = -6; // too many open files
pub const E_EXISTS: i64 = -7; // a file or folder with that name exists
pub const E_READ_ONLY: i64 = -8; // the disk is not marked for writing
pub const E_NOT_EMPTY: i64 = -9; // folder not empty
pub const E_NO_SPACE: i64 = -10; // disk full
pub const E_NO_MEMORY: i64 = -22; // not enough free memory (mem_map, K6-03)
// Module manager (call 21).
pub const E_MOD_MISSING_DEPENDENCY: i64 = -11;
pub const E_MOD_NEEDED: i64 = -12;
pub const E_MOD_RUNNING: i64 = -13;
pub const E_MOD_NOT_RUNNING: i64 = -14;
pub const E_MOD_BAD_PACKAGE: i64 = -15;
pub const E_MOD_NO_PROGRAM: i64 = -16;
const PATH_MAX: u64 = 1024;

/// Called from `syscall_entry` with interrupts disabled, on the thread's kernel stack.
pub extern "C" fn dispatch(nr: u64, a1: u64, a2: u64, _a3: u64, a4: u64, a5: u64) -> i64 {
    // The kernel may now be interrupted (and preempted) while it serves the call.
    // SAFETY: the IDT and the scheduler are ready long before any program runs.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    let result = call(nr, a1, a2, _a3, a4, a5);
    if sched::killed() {
        exit_current(EXIT_KILLED);
    }
    result
}

/// Exit code of a program stopped by `proc::kill`.
pub const EXIT_KILLED: i64 = -9;

fn call(nr: u64, a1: u64, a2: u64, _a3: u64, a4: u64, a5: u64) -> i64 {
    match nr {
        SYS_EXIT => exit_current(a1 as i64),
        SYS_WRITE => write(a1, a2, _a3),
        SYS_YIELD => {
            sched::yield_now();
            0
        }
        SYS_SLEEP_MS => {
            sched::sleep_ms(a1);
            0
        }
        SYS_OPEN => open(a1, a2),
        SYS_READ => read(a1, a2, _a3),
        SYS_CLOSE => close(a1),
        SYS_STAT => stat(a1, a2),
        SYS_WINDOW_CREATE => ui::window_create(a1, a2, _a3),
        SYS_WINDOW_PRESENT => ui::window_present(a1, a2, _a3, a4),
        SYS_EVENT_WAIT => ui::event_wait(a1, a2),
        SYS_WINDOW_CLOSE => ui::window_close(a1),
        SYS_SPAWN => spawn(a1, a2, _a3, a4),
        SYS_WAIT => crate::proc::wait_child(a1),
        SYS_LIST => list(a1, a2, _a3, a4),
        SYS_TIME => time(),
        SYS_SCREEN_SIZE => ui::screen_size(),
        SYS_SYSINFO => sysinfo(a1, a2, _a3),
        SYS_KILL => {
            if crate::proc::kill(a1) { 0 } else { E_NOT_FOUND }
        }
        SYS_CREATE => create(a1, a2, _a3),
        SYS_REMOVE => remove(a1, a2),
        SYS_RENAME => rename(a1, a2, _a3, a4),
        SYS_SYNC => sync(a1, a2),
        SYS_MODULE => module(a1, a2, _a3, a4, a5),
        SYS_TERM_OPEN => crate::term::open(sched::current_id()) as i64,
        SYS_TERM_READ => term_read(a1, a2, _a3),
        SYS_TERM_WRITE => match user_path(a2, _a3) {
            Ok(text) => {
                if crate::term::send_keys(sched::current_id(), a1, &text) { 0 } else { E_NOT_FOUND }
            }
            Err(e) => e,
        },
        SYS_RESOLVE => net::resolve(a1, a2),
        SYS_TCP_CONNECT => net::tcp_connect(a1, a2, _a3),
        SYS_UDP_OPEN => net::udp_open(a1),
        SYS_UDP_SEND => net::udp_send(a1, a2, _a3, a4, a5),
        SYS_UDP_RECV => net::udp_recv(a1, a2, _a3, a4, a5),
        SYS_NET_INFO => net::net_info(a1, a2),
        SYS_PING => net::ping(a1, a2, _a3),
        SYS_RANDOM => random(a1, a2),
        SYS_MEM_MAP => mem_map(a1),
        SYS_MEM_UNMAP => match crate::proc::unmap_region(a1, a2) {
            Some(_) => 0,
            None => E_INVALID,
        },
        _ => E_INVALID,
    }
}

/// Ends the program running on this thread with `code` and the thread with it.
pub fn exit_current(code: i64) -> ! {
    // SAFETY: re-enables interrupts if an exception handler called us with them off; we are on the
    // thread's own kernel stack, which stays valid until the scheduler frees it after the switch.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    let id = sched::current_id();
    crate::info!("proces: firul {} a iesit cu codul {}", id, code);
    crate::proc::on_exit(id, code);
    sched::exit()
}

/// True if every byte of `[addr, addr + len)` is in the program's half and mapped with USER rights
/// (and WRITABLE, if `write`) in the current tables.
pub(super) fn user_range_ok(addr: u64, len: u64, write: bool) -> bool {
    let Some(end) = addr.checked_add(len) else { return false };
    if end > USER_END {
        return false;
    }
    let mut page = addr & !(PAGE_SIZE - 1);
    while page < end {
        if !paging::user_accessible(paging::cr3(), page, write) {
            return false;
        }
        page += PAGE_SIZE;
    }
    true
}

/// write(fd, buf, len): fd 1 and 2 = the console (screen + serial); an open file is written at its
/// position, which then moves on. Returns the number of bytes written.
fn write(fd: u64, buf: u64, len: u64) -> i64 {
    if fd == 0 || len > 64 * 1024 {
        return E_INVALID;
    }
    if !user_range_ok(buf, len, false) {
        return E_FAULT;
    }
    // SAFETY: the whole range was just checked to be mapped user memory of the current address space.
    let bytes = unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) };
    if fd >= crate::proc::FIRST_FD as u64 {
        match net::socket_of(fd) {
            Some(crate::proc::Socket::Tcp(id)) => return net::tcp_write(id, &alloc::vec::Vec::from(bytes)),
            Some(crate::proc::Socket::Udp(_)) => return E_INVALID, // datagrams: udp_send
            None => {}
        }
        let index = fd as usize - crate::proc::FIRST_FD;
        let Some(Some((path, offset))) =
            crate::proc::with_files(|f| f.get(index).and_then(|o| o.as_ref()).map(|o| (o.path.clone(), o.offset)))
        else {
            return E_BAD_FD;
        };
        // Copied first, so no file-system lock is held while touching user memory.
        let data = alloc::vec::Vec::from(bytes);
        let n = match crate::fs::write(&path, offset, &data) {
            Ok(n) => n,
            Err(e) => return fs_error(e),
        };
        crate::proc::with_files(|f| {
            if let Some(Some(o)) = f.get_mut(index) {
                o.offset += n as u64;
            }
        });
        return n as i64;
    }
    if let Some(t) = crate::proc::terminal() {
        crate::term::write(t, bytes);
        return len as i64;
    }
    let text = core::str::from_utf8(bytes).unwrap_or("<text invalid>");
    crate::serial_print!("{}", text);
    crate::print!("{}", text);
    len as i64
}

fn fs_error(e: crate::fs::FsError) -> i64 {
    use crate::fs::FsError;
    match e {
        FsError::NotFound | FsError::NotADirectory => E_NOT_FOUND,
        FsError::IsADirectory => E_IS_DIR,
        FsError::AlreadyExists => E_EXISTS,
        FsError::ReadOnly => E_READ_ONLY,
        FsError::NotEmpty => E_NOT_EMPTY,
        FsError::NoSpace => E_NO_SPACE,
        _ => E_INVALID,
    }
}

/// Copies a path the program passed as (address, length) into the kernel.
fn user_path(ptr: u64, len: u64) -> Result<alloc::string::String, i64> {
    if len == 0 || len > PATH_MAX {
        return Err(E_INVALID);
    }
    if !user_range_ok(ptr, len, false) {
        return Err(E_FAULT);
    }
    // SAFETY: the range was just checked to be mapped user memory of the current address space.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    core::str::from_utf8(bytes).map(alloc::string::String::from).map_err(|_| E_INVALID)
}

/// open(path, path_len): opens a file for reading; returns its descriptor.
fn open(ptr: u64, len: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match crate::fs::kind(&path) {
        Ok(crate::fs::Kind::File) => {}
        Ok(crate::fs::Kind::Dir) => return E_IS_DIR,
        Err(e) => return fs_error(e),
    }
    match crate::proc::add_file(crate::proc::OpenFile { path, offset: 0, socket: None }) {
        Some(fd) => fd as i64,
        None => E_TOO_MANY,
    }
}

/// read(fd, buf, len): reads from the current position; returns the bytes read (0 = end of file).
fn read(fd: u64, buf: u64, len: u64) -> i64 {
    let len = len.min(64 * 1024);
    if !user_range_ok(buf, len, true) {
        return E_FAULT;
    }
    let Some(index) = (fd as usize).checked_sub(crate::proc::FIRST_FD) else { return E_BAD_FD };
    match net::socket_of(fd) {
        Some(crate::proc::Socket::Tcp(id)) => {
            let mut data = alloc::vec![0u8; len as usize];
            let n = net::tcp_read(id, &mut data);
            if n > 0 {
                // SAFETY: [buf, buf + len) was checked to be writable user memory; n <= len.
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, n as usize) };
            }
            return n;
        }
        Some(crate::proc::Socket::Udp(_)) => return E_INVALID, // datagrams: udp_recv
        None => {}
    }
    let Some(Some((path, offset))) =
        crate::proc::with_files(|f| f.get(index).and_then(|o| o.as_ref()).map(|o| (o.path.clone(), o.offset)))
    else {
        return E_BAD_FD;
    };
    // Read into a kernel buffer first, so no file-system lock is held while touching user memory.
    let mut data = alloc::vec![0u8; len as usize];
    let n = match crate::fs::read(&path, offset, &mut data) {
        Ok(n) => n,
        Err(e) => return fs_error(e),
    };
    // SAFETY: [buf, buf + len) was checked to be writable user memory of the current address space; n <= len.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, n) };
    crate::proc::with_files(|f| {
        if let Some(Some(o)) = f.get_mut(index) {
            o.offset += n as u64;
        }
    });
    n as i64
}

/// close(fd)
fn close(fd: u64) -> i64 {
    let Some(index) = (fd as usize).checked_sub(crate::proc::FIRST_FD) else { return E_BAD_FD };
    match crate::proc::with_files(|f| f.get_mut(index).and_then(|o| o.take())) {
        Some(Some(_)) => 0,
        _ => E_BAD_FD,
    }
}

/// stat(path, path_len): the size of a file in bytes (a directory gives E_IS_DIR).
fn stat(ptr: u64, len: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match crate::fs::kind(&path) {
        Ok(crate::fs::Kind::Dir) => E_IS_DIR,
        Ok(crate::fs::Kind::File) => crate::fs::size(&path).map(|s| s as i64).unwrap_or_else(fs_error),
        Err(e) => fs_error(e),
    }
}

/// spawn(path, length, arg, arg_length) -> id of the new program. Its arguments are its path and, if
/// `arg_length` > 0, the text `arg`.
fn spawn(ptr: u64, len: u64, arg: u64, arg_len: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let arg = if arg_len == 0 {
        None
    } else {
        match user_path(arg, arg_len) {
            Ok(a) => Some(a),
            Err(e) => return e,
        }
    };
    let args: alloc::vec::Vec<&str> = core::iter::once(path.as_str()).chain(arg.as_deref()).collect();
    match crate::proc::spawn_path(&path, &args) {
        Ok(id) => id as i64,
        Err(crate::proc::SpawnError::NotFound(e)) => fs_error(e),
        Err(_) => E_INVALID,
    }
}

/// list(path, length, buf, buf_len): the names in a directory, one per line ('/' after directories),
/// as many whole lines as fit in `buf`. Returns the number of bytes written.
fn list(ptr: u64, len: u64, buf: u64, buf_len: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if !user_range_ok(buf, buf_len, true) {
        return E_FAULT;
    }
    let entries = match crate::fs::list(&path) {
        Ok(e) => e,
        Err(e) => return fs_error(e),
    };
    // SAFETY: the range was checked to be writable user memory of this program.
    let out = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, buf_len as usize) };
    let mut n = 0;
    for (name, kind) in entries {
        let dir = kind == crate::fs::Kind::Dir;
        let need = name.len() + dir as usize + 1;
        if n + need > out.len() {
            break;
        }
        out[n..n + name.len()].copy_from_slice(name.as_bytes());
        n += name.len();
        if dir {
            out[n] = b'/';
            n += 1;
        }
        out[n] = b'\n';
        n += 1;
    }
    n as i64
}

/// time() -> date and time from the computer's clock: year << 40 | month << 32 | day << 24 | hour << 16 |
/// minute << 8 | second.
fn time() -> i64 {
    match crate::drivers::rtc::now() {
        Some(t) => ((t.year as i64) << 40)
            | ((t.month as i64) << 32)
            | ((t.day as i64) << 24)
            | ((t.hour as i64) << 16)
            | ((t.minute as i64) << 8)
            | t.second as i64,
        None => E_NOT_FOUND,
    }
}

/// random(buf, len): fills the program's buffer with random bytes (at most 4096); returns len.
fn random(buf: u64, len: u64) -> i64 {
    if len > 4096 {
        return E_INVALID;
    }
    if !user_range_ok(buf, len, true) {
        return E_FAULT;
    }
    // SAFETY: the whole range was just checked to be writable user memory of the current address space.
    let out = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, len as usize) };
    crate::random::fill(out);
    len as i64
}

/// create(path, length, kind): a new empty file (kind 0) or folder (kind 1).
fn create(ptr: u64, len: u64, kind: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let kind = match kind {
        0 => crate::fs::Kind::File,
        1 => crate::fs::Kind::Dir,
        _ => return E_INVALID,
    };
    crate::fs::create(&path, kind).map_or_else(fs_error, |()| 0)
}

/// rename(from, length, to, length): a file gets a new name on the same disk (P6-08).
fn rename(p1: u64, l1: u64, p2: u64, l2: u64) -> i64 {
    let (from, to) = match (user_path(p1, l1), user_path(p2, l2)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    crate::fs::rename(&from, &to).map_or_else(fs_error, |()| 0)
}

/// sync(path, length): what the disk holding `path` still keeps in its cache is written out (P6-08).
fn sync(ptr: u64, len: u64) -> i64 {
    match user_path(ptr, len) {
        Ok(path) => crate::fs::flush(&path).map_or_else(fs_error, |()| 0),
        Err(e) => e,
    }
}

/// remove(path, length): deletes a file or an empty folder.
fn remove(ptr: u64, len: u64) -> i64 {
    match user_path(ptr, len) {
        Ok(path) => crate::fs::remove(&path).map_or_else(fs_error, |()| 0),
        Err(e) => e,
    }
}

/// sysinfo(kind, buf, len). Kind 0 = memory and time, 7 u64 values: total memory, used memory, kernel
/// heap used, heap reserved (bytes), uptime (ms), idle processor time (ticks), ticks per second.
/// Kind 1 = running programs, one per line: `id processor_ticks name`. Returns the bytes written.
fn sysinfo(kind: u64, buf: u64, len: u64) -> i64 {
    use crate::mem::{frame, heap, map::PAGE_SIZE};
    let data: alloc::vec::Vec<u8> = match kind {
        0 => {
            let (heap_used, heap_mapped) = heap::stats();
            let used = frame::used_frames() * PAGE_SIZE;
            let values = [
                used + frame::free_frames() * PAGE_SIZE,
                used,
                heap_used as u64,
                heap_mapped as u64,
                crate::time::uptime_ms().unwrap_or(0),
                sched::cpu_ticks(sched::IDLE).unwrap_or(0),
                crate::arch::x86_64::apic::HZ,
            ];
            values.iter().flat_map(|v| v.to_le_bytes()).collect()
        }
        1 => {
            let mut text = alloc::string::String::new();
            for (id, name) in crate::proc::list() {
                text.push_str(&alloc::format!("{} {} {}\n", id, sched::cpu_ticks(id).unwrap_or(0), name));
            }
            text.into_bytes()
        }
        _ => return E_INVALID,
    };
    if (data.len() as u64) > len {
        return E_INVALID;
    }
    if !user_range_ok(buf, data.len() as u64, true) {
        return E_FAULT;
    }
    // SAFETY: the range was checked to be writable user memory of this program.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, data.len()) };
    data.len() as i64
}

/// module(op, text, text_len, buf, buf_len): the module manager. Op 0 = list: one line per installed
/// module, `name version running|stopped description`, written to `buf` (returns the bytes written);
/// op 1 = install the package at path `text`; 2 = start, 3 = stop, 4 = remove the module named `text`.
fn module(op: u64, ptr: u64, len: u64, buf: u64, buf_len: u64) -> i64 {
    use crate::modmgr::{self, ModError};
    let err = |e: ModError| match e {
        ModError::Fs(e) => fs_error(e),
        ModError::BadPackage(_) | ModError::Spawn => E_MOD_BAD_PACKAGE,
        ModError::AlreadyInstalled => E_EXISTS,
        ModError::NotInstalled => E_NOT_FOUND,
        ModError::MissingDependency(_) => E_MOD_MISSING_DEPENDENCY,
        ModError::NeededBy(_) => E_MOD_NEEDED,
        ModError::Running => E_MOD_RUNNING,
        ModError::NotRunning => E_MOD_NOT_RUNNING,
        ModError::NoProgram => E_MOD_NO_PROGRAM,
    };
    if op == 0 {
        let list = match modmgr::installed() {
            Ok(l) => l,
            Err(e) => return err(e),
        };
        let mut text = alloc::string::String::new();
        for m in list {
            let state = if modmgr::is_running(&m.name) { "running" } else { "stopped" };
            text.push_str(&alloc::format!("{} {} {} {}\n", m.name, m.version, state, m.description));
        }
        if text.len() as u64 > buf_len {
            return E_INVALID;
        }
        if !user_range_ok(buf, text.len() as u64, true) {
            return E_FAULT;
        }
        // SAFETY: the range was checked to be writable user memory of this program.
        unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buf as *mut u8, text.len()) };
        return text.len() as i64;
    }
    let arg = match user_path(ptr, len) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let r = match op {
        1 => modmgr::install(&arg).map(|_| ()),
        2 => modmgr::start(&arg).map(|_| ()),
        3 => modmgr::stop(&arg),
        4 => modmgr::remove(&arg),
        _ => return E_INVALID,
    };
    r.map_or_else(err, |()| 0)
}

/// term_read(id, buf, len): output of terminal `id` waiting to be shown (0 = nothing new);
/// E_NOT_FOUND once its shell has ended and everything was read.
fn term_read(id: u64, buf: u64, len: u64) -> i64 {
    let len = len.min(16 * 1024);
    if !user_range_ok(buf, len, true) {
        return E_FAULT;
    }
    let mut data = alloc::vec![0u8; len as usize];
    match crate::term::read_output(sched::current_id(), id, &mut data) {
        Some(n) => {
            // SAFETY: the range was checked to be writable user memory of this program; n <= len.
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, n) };
            n as i64
        }
        None => E_NOT_FOUND,
    }
}

/// Largest region one `mem_map` call may ask for (64 GiB; the free memory is the real limit).
const MEM_MAP_MAX: u64 = 64 << 30;

/// `mem_map(len)` (K6-03): a new zero-filled memory region; returns its address, or `E_INVALID` / `E_NO_MEMORY`.
fn mem_map(len: u64) -> i64 {
    if len == 0 || len > MEM_MAP_MAX {
        return E_INVALID;
    }
    match crate::proc::map_region(len) {
        Ok(m) => {
            if len >= crate::mem::paging::HUGE_PAGE {
                crate::info!(
                    "mem_map: {} KiB la {:#x} ({} pagini de 2 MiB, {} de 4 KiB)",
                    len.div_ceil(1024),
                    m.addr,
                    m.huge_pages,
                    m.small_pages
                );
            }
            m.addr as i64
        }
        Err(_) => E_NO_MEMORY,
    }
}
