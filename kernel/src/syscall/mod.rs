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

mod ui;

/// Error results (negative, like the convention most systems use).
pub const E_INVALID: i64 = -1; // unknown call or bad argument
pub const E_FAULT: i64 = -2; // pointer outside the program's memory
pub const E_NOT_FOUND: i64 = -3; // no such file or directory
pub const E_IS_DIR: i64 = -4; // a directory where a file is needed
pub const E_BAD_FD: i64 = -5; // descriptor not open
pub const E_TOO_MANY: i64 = -6; // too many open files
const PATH_MAX: u64 = 1024;

/// Called from `syscall_entry` with interrupts disabled, on the thread's kernel stack.
pub extern "C" fn dispatch(nr: u64, a1: u64, a2: u64, _a3: u64, a4: u64, _a5: u64) -> i64 {
    // The kernel may now be interrupted (and preempted) while it serves the call.
    // SAFETY: the IDT and the scheduler are ready long before any program runs.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
    let result = call(nr, a1, a2, _a3, a4);
    if sched::killed() {
        exit_current(EXIT_KILLED);
    }
    result
}

/// Exit code of a program stopped by `proc::kill`.
pub const EXIT_KILLED: i64 = -9;

fn call(nr: u64, a1: u64, a2: u64, _a3: u64, a4: u64) -> i64 {
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
        SYS_SPAWN => spawn(a1, a2),
        SYS_WAIT => crate::proc::wait_child(a1),
        SYS_LIST => list(a1, a2, _a3, a4),
        SYS_TIME => time(),
        SYS_SCREEN_SIZE => ui::screen_size(),
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

/// write(fd, buf, len): fd 1 and 2 = the console (screen + serial). Returns the number of bytes written.
fn write(fd: u64, buf: u64, len: u64) -> i64 {
    if !(1..=2).contains(&fd) || len > 64 * 1024 {
        return E_INVALID;
    }
    if !user_range_ok(buf, len, false) {
        return E_FAULT;
    }
    // SAFETY: the whole range was just checked to be mapped user memory of the current address space.
    let bytes = unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) };
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
    match crate::proc::add_file(crate::proc::OpenFile { path, offset: 0 }) {
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

/// spawn(path, length) -> id of the new program (it gets its path as its only argument).
fn spawn(ptr: u64, len: u64) -> i64 {
    let path = match user_path(ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match crate::proc::spawn_path(&path, &[&path]) {
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
