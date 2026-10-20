//! Processes: a user program = its own address space + one thread that runs it in ring 3, plus its
//! table of open files. Programs are static ELF64 files (`spawn_path`/`spawn_elf`, see elf.rs) or,
//! for tests, flat machine code (`spawn_flat`).
//!
//! Program start: rdi = argc, rsi = pointer to `argc` pairs (u64 address, u64 length) of UTF-8
//! arguments, all on the program's stack (argument 0 = the program's path).

pub mod elf;

use crate::arch::x86_64::syscall::enter_user;
use crate::mem::{address_space::AddressSpace, map::{PAGE_SIZE, phys_to_virt}, paging};
use crate::sched;
use alloc::{string::String, vec::Vec};

/// Where a flat program is loaded and starts.
pub const CODE_BASE: u64 = 0x0000_0000_0040_0000;
/// Top of the user stack (it grows down from here).
pub const STACK_TOP: u64 = 0x0000_7FFF_FFF0_0000;
/// 64 KiB: programs have no heap, so their tables and buffers live on the stack.
const STACK_PAGES: u64 = 16;
/// Space at the top of the stack for the arguments.
const ARGS_MAX: usize = PAGE_SIZE as usize;
/// File descriptors 0..3 are the console; files opened by the program start at 3.
pub const FIRST_FD: usize = 3;
const MAX_FILES: usize = 32;

/// A file opened by a program.
pub struct OpenFile {
    pub path: String,
    pub offset: u64,
}

struct Process {
    thread: u64,
    /// Program name (argument 0 without its folder), for lists of running programs.
    name: String,
    entry: u64,
    rsp: u64,
    argc: u64,
    argv: u64,
    files: Vec<Option<OpenFile>>,
    _space: AddressSpace,
}

/// Running processes (their address spaces live here until the program ends).
static PROCS: sched::Mutex<Vec<Process>> = sched::Mutex::new(Vec::new());
/// Exit codes not yet collected by `wait`.
static EXITED: sched::Mutex<Vec<(u64, i64)>> = sched::Mutex::new(Vec::new());
static EXIT_WAIT: sched::WaitQueue = sched::WaitQueue::new();

#[derive(Debug)]
#[allow(dead_code)] // the inner errors are read by the self-tests now, shown by the shell (`run`) later
pub enum SpawnError {
    NoMemory,
    NotFound(crate::fs::FsError),
    BadElf(elf::ElfError),
    ArgsTooLong,
}

impl SpawnError {
    #[allow(dead_code)] // used by the shell (`run`, P2-09)
    pub fn message(&self) -> &'static str {
        match self {
            SpawnError::NoMemory => "memorie insuficienta",
            SpawnError::NotFound(e) => e.message(),
            SpawnError::BadElf(e) => e.message(),
            SpawnError::ArgsTooLong => "argumente prea lungi",
        }
    }
}

#[allow(dead_code)] // used by the self-tests now, by the shell (`run`, P2-09)
/// Starts the program at `path` (an ELF file) with `args` (argument 0 should be the program's name).
pub fn spawn_path(path: &str, args: &[&str]) -> Result<u64, SpawnError> {
    let file = crate::fs::read_all(path).map_err(SpawnError::NotFound)?;
    spawn_elf(&file, args)
}

/// Starts the ELF executable `file` as a new process with `args`. Returns its thread id.
pub fn spawn_elf(file: &[u8], args: &[&str]) -> Result<u64, SpawnError> {
    let mut space = AddressSpace::new().ok_or(SpawnError::NoMemory)?;
    let entry = elf::load(file, &mut space).map_err(SpawnError::BadElf)?;
    start(space, entry, args)
}

/// Starts `code` (position-independent machine code) as a new process, without arguments.
#[allow(dead_code)] // used by the self-tests
pub fn spawn_flat(code: &[u8]) -> Result<u64, SpawnError> {
    let mut space = AddressSpace::new().ok_or(SpawnError::NoMemory)?;
    let pages = (code.len() as u64).div_ceil(PAGE_SIZE).max(1);
    for p in 0..pages {
        // Code: user-readable and executable, not writable.
        let f = space.map_new(CODE_BASE + p * PAGE_SIZE, 0).map_err(|_| SpawnError::NoMemory)?;
        let start = (p * PAGE_SIZE) as usize;
        let chunk = &code[start..code.len().min(start + PAGE_SIZE as usize)];
        // SAFETY: `f` is a fresh frame of this space, reachable through the HHDM; the chunk fits in a page.
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), phys_to_virt(f), chunk.len()) };
    }
    start(space, CODE_BASE, &[])
}

/// Gives the process a stack with the arguments on top, then starts its thread at `entry`.
fn start(mut space: AddressSpace, entry: u64, args: &[&str]) -> Result<u64, SpawnError> {
    let mut top_frame = 0;
    for p in 1..=STACK_PAGES {
        let f = space
            .map_new(STACK_TOP - p * PAGE_SIZE, paging::WRITABLE | paging::NO_EXECUTE)
            .map_err(|_| SpawnError::NoMemory)?;
        if p == 1 {
            top_frame = f;
        }
    }
    // Arguments, in the top stack page: the strings first, then the (address, length) pairs below them.
    let strings: usize = args.iter().map(|a| a.len()).sum();
    if strings + args.len() * 16 + 32 > ARGS_MAX {
        return Err(SpawnError::ArgsTooLong);
    }
    let page = STACK_TOP - PAGE_SIZE;
    // SAFETY: `top_frame` is the fresh frame mapped at `page`, reachable through the HHDM.
    let mem = unsafe { core::slice::from_raw_parts_mut(phys_to_virt(top_frame), PAGE_SIZE as usize) };
    let mut cursor = PAGE_SIZE as usize;
    let mut pairs = Vec::with_capacity(args.len());
    for a in args {
        cursor -= a.len();
        mem[cursor..cursor + a.len()].copy_from_slice(a.as_bytes());
        pairs.push((page + cursor as u64, a.len() as u64));
    }
    cursor = (cursor - pairs.len() * 16) & !15;
    let argv = page + cursor as u64;
    for (i, (addr, len)) in pairs.iter().enumerate() {
        mem[cursor + i * 16..cursor + i * 16 + 8].copy_from_slice(&addr.to_le_bytes());
        mem[cursor + i * 16 + 8..cursor + i * 16 + 16].copy_from_slice(&len.to_le_bytes());
    }
    let rsp = argv - 16;

    // The process is registered before its thread can run (and exit).
    let mut procs = PROCS.lock();
    let thread = sched::spawn_in(space.root(), start_user);
    let name = String::from(args.first().map_or("(cod)", |a| a.rsplit('/').next().unwrap_or(a)));
    procs.push(Process { thread, name, entry, rsp, argc: args.len() as u64, argv, files: Vec::new(), _space: space });
    Ok(thread)
}

/// First code of a process thread (already running on the process's tables): jump to ring 3.
fn start_user() {
    let id = sched::current_id();
    let (entry, rsp, argc, argv) = PROCS
        .lock()
        .iter()
        .find(|p| p.thread == id)
        .map(|p| (p.entry, p.rsp, p.argc, p.argv))
        .expect("proc: fir fara proces");
    // SAFETY: `entry` (checked by the loader to be in an executable segment) and the stack at `rsp` are
    // mapped USER in this thread's address space; the scheduler set this thread's kernel stack.
    unsafe { enter_user(entry, rsp, argc, argv) }
}

/// Runs `f` with the open-file table of the process on the current thread.
pub fn with_files<R>(f: impl FnOnce(&mut Vec<Option<OpenFile>>) -> R) -> Option<R> {
    let id = sched::current_id();
    let mut procs = PROCS.lock();
    procs.iter_mut().find(|p| p.thread == id).map(|p| f(&mut p.files))
}

/// Adds `file` to the current process's table; returns its descriptor, or `None` if the table is full.
pub fn add_file(file: OpenFile) -> Option<usize> {
    with_files(|files| {
        let slot = match files.iter().position(|f| f.is_none()) {
            Some(i) => i,
            None if files.len() < MAX_FILES => {
                files.push(None);
                files.len() - 1
            }
            None => return None,
        };
        files[slot] = Some(file);
        Some(slot + FIRST_FD)
    })
    .flatten()
}

/// Called when the program on thread `id` ends: frees its memory and files, records the exit code.
pub fn on_exit(id: u64, code: i64) {
    let process = {
        let mut procs = PROCS.lock();
        // Recorded while PROCS is held, so `wait_child` always finds the program in one of the lists.
        EXITED.lock().push((id, code));
        procs.iter().position(|p| p.thread == id).map(|i| procs.swap_remove(i))
    };
    drop(process); // switches to the kernel's tables first if they were in use (see AddressSpace::drop)
    crate::ui::close_owned(id);
    EXIT_WAIT.wake_all();
}

/// Stops the program on thread `id` (it exits with `syscall::EXIT_KILLED`). Returns false if there is
/// no such program. A program waiting for another one (`wait`) stops only when that wait ends.
pub fn kill(id: u64) -> bool {
    PROCS.lock().iter().any(|p| p.thread == id) && sched::kill(id)
}

/// The running programs: (thread id, name).
pub fn list() -> Vec<(u64, String)> {
    PROCS.lock().iter().map(|p| (p.thread, p.name.clone())).collect()
}

/// Whether the program on thread `id` is still running.
pub fn running(id: u64) -> bool {
    PROCS.lock().iter().any(|p| p.thread == id)
}

/// `wait` for a program asking through a system call: only for programs that are running or have
/// ended and not been collected yet (anything else would wait forever).
pub fn wait_child(id: u64) -> i64 {
    let known = {
        let procs = PROCS.lock();
        procs.iter().any(|p| p.thread == id) || EXITED.lock().iter().any(|&(t, _)| t == id)
    };
    if !known {
        return crate::syscall::E_NOT_FOUND;
    }
    wait(id)
}

/// Waits until the program on thread `id` ends and returns its exit code.
#[allow(dead_code)] // used by the self-tests now, by the shell (`run`, P2-09)
pub fn wait(id: u64) -> i64 {
    let mut code = None;
    EXIT_WAIT.wait_until(|| {
        // Checked with interrupts off: take the lock only if it is free, never sleep here.
        if let Some(mut list) = EXITED.try_lock() {
            if let Some(i) = list.iter().position(|&(t, _)| t == id) {
                code = Some(list.swap_remove(i).1);
            }
        }
        code.is_some()
    });
    code.unwrap()
}
