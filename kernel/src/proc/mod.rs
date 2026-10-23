//! Processes: a user program = its own address space + a main thread that runs it in ring 3 (its id is the program's
//! id), optional additional threads in the same address space (K6-04, `thread_spawn`), plus its table of open files. Programs are static ELF64 files (`spawn_path`/`spawn_elf`, see elf.rs) or,
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
/// Additional threads a program may have at the same time (K6-04).
pub const MAX_THREADS: usize = 64;
/// Exit codes of ended additional threads kept for `thread_join`; the oldest are dropped past this.
const MAX_THREAD_CODES: usize = 256;

/// A file (or network socket) opened by a program.
pub struct OpenFile {
    pub path: String,
    pub offset: u64,
    /// Set for a network socket (`path` is then empty).
    pub socket: Option<Socket>,
}

/// A network socket held in the descriptor table: a TCP connection or an open UDP port.
#[derive(Clone, Copy, Debug)]
pub enum Socket {
    Tcp(u32),
    Udp(u16),
}

impl Drop for OpenFile {
    /// A program's sockets close with its descriptor (by `close` or when the program ends).
    fn drop(&mut self) {
        match self.socket {
            Some(Socket::Tcp(id)) => crate::net::tcp::close(id, 0),
            Some(Socket::Udp(port)) => crate::net::udp::unbind(port),
            None => {}
        }
    }
}

/// An additional thread of a program (K6-04): where it starts in ring 3.
struct UserThread {
    id: u64,
    entry: u64,
    rsp: u64,
    arg0: u64,
    arg1: u64,
}

struct Process {
    /// The main thread: its id is the program's id (`wait`, `kill`, `list`).
    thread: u64,
    /// Additional threads that have not ended yet (K6-04).
    threads: Vec<UserThread>,
    /// Exit codes of ended additional threads, not yet collected by `thread_join`.
    thread_codes: Vec<(u64, i64)>,
    /// The program's exit code when a thread other than the main one ended it (`exit`, or a fault).
    exit_code: Option<i64>,
    /// The program is ending: no new threads.
    stopping: bool,
    /// Program name (argument 0 without its folder), for lists of running programs.
    name: String,
    /// Terminal its output goes to (started from a terminal's shell), or `None` = the console.
    terminal: Option<u64>,
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
/// Woken when an additional thread ends (for `thread_join` and for the program's exit).
static THREAD_WAIT: sched::WaitQueue = sched::WaitQueue::new();

impl Process {
    /// Whether thread `id` belongs to this program (the main thread or an additional one).
    fn owns(&self, id: u64) -> bool {
        self.thread == id || self.threads.iter().any(|t| t.id == id)
    }
}

/// The program the current thread belongs to.
fn current(procs: &mut [Process]) -> Option<&mut Process> {
    let id = sched::current_id();
    procs.iter_mut().find(|p| p.owns(id))
}

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
    let terminal = crate::term::of_shell(sched::current_id())
        .or_else(|| current(&mut procs).and_then(|p| p.terminal));
    procs.push(Process {
        thread,
        threads: Vec::new(),
        thread_codes: Vec::new(),
        exit_code: None,
        stopping: false,
        name,
        terminal,
        entry,
        rsp,
        argc: args.len() as u64,
        argv,
        files: Vec::new(),
        _space: space,
    });
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

/// Runs `f` with the open-file table of the process on the current thread (shared by all its threads).
pub fn with_files<R>(f: impl FnOnce(&mut Vec<Option<OpenFile>>) -> R) -> Option<R> {
    let mut procs = PROCS.lock();
    current(&mut procs).map(|p| f(&mut p.files))
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

/// `mem_map` for the program on the current thread (K6-03): a new zero-filled memory region of `len` bytes.
pub fn map_region(len: u64) -> Result<crate::mem::address_space::Mapped, paging::MapError> {
    let mut procs = PROCS.lock();
    let p = current(&mut procs).ok_or(paging::MapError::NoMemory)?;
    p._space.map_region(len)
}

/// Why `mem_unmap` refused.
pub enum UnmapError {
    /// No region starts at that address with that length.
    NoSuchRegion,
    /// Other threads of the program still run (K6-04): they could be using the region on another processor.
    ThreadsRunning,
}

/// `mem_unmap` for the program on the current thread. Returns the frames freed. Only while the program has a single
/// thread: then no other processor can still hold the region's pages in its TLB (the switch away from the program's
/// tables flushed them) and no system call of another thread can be reading the region.
pub fn unmap_region(addr: u64, len: u64) -> Result<u64, UnmapError> {
    let mut procs = PROCS.lock();
    let p = current(&mut procs).ok_or(UnmapError::NoSuchRegion)?;
    if !p.threads.is_empty() {
        return Err(UnmapError::ThreadsRunning);
    }
    p._space.unmap_region(addr, len).ok_or(UnmapError::NoSuchRegion)
}

/// `thread_spawn` (K6-04): a new thread of the current program that starts in ring 3 at `entry` with the stack
/// pointer `rsp`, `rdi = arg0`, `rsi = arg1`. Returns its id, or a negative error.
pub fn thread_spawn(entry: u64, rsp: u64, arg0: u64, arg1: u64) -> i64 {
    let mut procs = PROCS.lock();
    let Some(p) = current(&mut procs) else { return crate::syscall::E_INVALID };
    if p.stopping {
        return crate::syscall::E_INVALID;
    }
    if p.threads.len() >= MAX_THREADS {
        return crate::syscall::E_TOO_MANY;
    }
    // Registered while PROCS is held, before the new thread can look itself up (`start_thread`).
    let id = sched::spawn_in(p._space.root(), start_thread);
    p.threads.push(UserThread { id, entry, rsp, arg0, arg1 });
    id as i64
}

/// First code of an additional thread: jump to ring 3 at the place given to `thread_spawn`.
fn start_thread() {
    let id = sched::current_id();
    let (entry, rsp, a0, a1) = PROCS
        .lock()
        .iter()
        .flat_map(|p| p.threads.iter())
        .find(|t| t.id == id)
        .map(|t| (t.entry, t.rsp, t.arg0, t.arg1))
        .expect("proc: fir fara proces");
    // SAFETY: the stack top was checked by `syscall::thread_spawn` to be writable user memory; a bad `entry` only
    // makes this program fault in ring 3 (and end). The scheduler set this thread's kernel stack and tables.
    unsafe { enter_user(entry, rsp, a0, a1) }
}

/// The program whose additional thread `id` is (its main thread's id), `None` for a main thread or a kernel thread.
pub fn main_of(id: u64) -> Option<u64> {
    PROCS.lock().iter().find(|p| p.threads.iter().any(|t| t.id == id)).map(|p| p.thread)
}

/// A thread other than the main one ends the whole program with `code` (`exit`, a fault): the main thread is
/// stopped, and the program's exit code is `code` instead of the stop code.
pub fn end_program(main: u64, code: i64) {
    let mut procs = PROCS.lock();
    if let Some(p) = procs.iter_mut().find(|p| p.thread == main) {
        p.exit_code.get_or_insert(code);
        sched::kill(main);
    }
}

/// Called when the additional thread `id` ends with `code` (it returned, called `thread_exit`, or was stopped).
pub fn thread_exited(id: u64, code: i64) {
    sched::use_kernel_tables(); // the program may be freed as soon as it is off the list
    {
        let mut procs = PROCS.lock();
        if let Some(p) = procs.iter_mut().find(|p| p.threads.iter().any(|t| t.id == id)) {
            p.threads.retain(|t| t.id != id);
            if p.thread_codes.len() >= MAX_THREAD_CODES {
                p.thread_codes.remove(0);
            }
            p.thread_codes.push((id, code));
        }
    }
    crate::ui::close_owned(id);
    crate::term::close_owned(id);
    THREAD_WAIT.wake_all();
}

/// `thread_join(id)`: waits until the additional thread `id` of the current program ends; returns its exit code.
pub fn thread_join(id: u64) -> i64 {
    let me = sched::current_id();
    {
        let mut procs = PROCS.lock();
        let Some(p) = current(&mut procs) else { return crate::syscall::E_INVALID };
        if id == me || id == p.thread {
            return crate::syscall::E_INVALID;
        }
        if !p.threads.iter().any(|t| t.id == id) && !p.thread_codes.iter().any(|&(t, _)| t == id) {
            return crate::syscall::E_NOT_FOUND;
        }
    }
    loop {
        let mut result = None;
        let finished = THREAD_WAIT.wait_until_or(0, || {
            // Interrupts are off here: only try the lock (see `wait`).
            let Some(mut procs) = PROCS.try_lock() else { return true };
            let Some(p) = procs.iter_mut().find(|p| p.owns(me)) else {
                result = Some(crate::syscall::E_NOT_FOUND);
                return true;
            };
            if let Some(i) = p.thread_codes.iter().position(|&(t, _)| t == id) {
                result = Some(p.thread_codes.remove(i).1);
            } else if !p.threads.iter().any(|t| t.id == id) {
                result = Some(crate::syscall::E_NOT_FOUND); // collected by another thread meanwhile
            }
            result.is_some()
        });
        if let Some(r) = result {
            return r;
        }
        if !finished {
            return crate::syscall::EXIT_KILLED; // this thread was asked to stop
        }
        sched::yield_now();
    }
}

/// The exit code the program on main thread `id` ends with: `code`, unless another thread chose one (`end_program`).
pub fn exit_code(id: u64, code: i64) -> i64 {
    PROCS.lock().iter().find(|p| p.thread == id).and_then(|p| p.exit_code).unwrap_or(code)
}

/// Stops every additional thread of the program on main thread `id` and waits until all have ended: they run in the
/// address space that is freed next.
fn stop_threads(id: u64) {
    {
        let mut procs = PROCS.lock();
        let Some(p) = procs.iter_mut().find(|p| p.thread == id) else { return };
        p.stopping = true;
        if p.threads.is_empty() {
            return;
        }
        for t in &p.threads {
            sched::kill(t.id);
        }
    }
    loop {
        let mut done = false;
        THREAD_WAIT.wait_until(|| {
            let Some(procs) = PROCS.try_lock() else { return true };
            done = procs.iter().find(|p| p.thread == id).is_none_or(|p| p.threads.is_empty());
            done
        });
        if done {
            return;
        }
        sched::yield_now();
    }
}

/// Called when the program on thread `id` ends: frees its memory and files, records the exit code.
pub fn on_exit(id: u64, code: i64) {
    if sched::current_id() == id {
        sched::use_kernel_tables(); // the process's tables are freed below
    }
    stop_threads(id);
    let code = exit_code(id, code);
    let process = {
        let mut procs = PROCS.lock();
        // Recorded while PROCS is held, so `wait_child` always finds the program in one of the lists.
        EXITED.lock().push((id, code));
        procs.iter().position(|p| p.thread == id).map(|i| procs.swap_remove(i))
    };
    drop(process); // switches to the kernel's tables first if they were in use (see AddressSpace::drop)
    crate::ui::close_owned(id);
    crate::term::close_owned(id);
    EXIT_WAIT.wake_all();
}

/// Stops the program on thread `id` (it exits with `syscall::EXIT_KILLED`). Returns false if there is
/// no such program. A program waiting for another one (`wait`) stops only when that wait ends.
pub fn kill(id: u64) -> bool {
    PROCS.lock().iter().any(|p| p.thread == id) && sched::kill(id)
}

/// The terminal the program on the current thread writes to (`None` = the console).
pub fn terminal() -> Option<u64> {
    current(&mut PROCS.lock()).and_then(|p| p.terminal)
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
    loop {
        EXIT_WAIT.wait_until(|| {
            // Checked with interrupts off: take the lock only if it is free, never sleep here. If someone else holds it
            // (maybe on another processor), do not sleep either: nobody would wake us when they release it; try again.
            match EXITED.try_lock() {
                Some(mut list) => {
                    if let Some(i) = list.iter().position(|&(t, _)| t == id) {
                        code = Some(list.swap_remove(i).1);
                    }
                    code.is_some()
                }
                None => true,
            }
        });
        if let Some(c) = code {
            return c;
        }
        sched::yield_now();
    }
}
