//! Processes: a user program = its own address space + one thread that runs it in ring 3.
//! Programs are static ELF64 files (`spawn_elf`, see elf.rs) or, for tests, flat machine code (`spawn_flat`).

pub mod elf;

use crate::arch::x86_64::syscall::enter_user;
use crate::mem::{address_space::AddressSpace, map::{PAGE_SIZE, phys_to_virt}, paging};
use crate::sched;
use alloc::vec::Vec;

/// Where a flat program is loaded and starts.
pub const CODE_BASE: u64 = 0x0000_0000_0040_0000;
/// Top of the user stack (it grows down from here).
pub const STACK_TOP: u64 = 0x0000_7FFF_FFF0_0000;
const STACK_PAGES: u64 = 4;

struct Process {
    thread: u64,
    entry: u64,
    _space: AddressSpace,
}

/// Running processes (their address spaces live here until the program ends).
static PROCS: sched::Mutex<Vec<Process>> = sched::Mutex::new(Vec::new());
/// Exit codes not yet collected by `wait`.
static EXITED: sched::Mutex<Vec<(u64, i64)>> = sched::Mutex::new(Vec::new());
static EXIT_WAIT: sched::WaitQueue = sched::WaitQueue::new();

#[derive(Debug)]
#[allow(dead_code)] // the ELF error is read by the self-tests now, shown by the shell (`run`) later
pub enum SpawnError {
    NoMemory,
    BadElf(elf::ElfError),
}

/// Starts the ELF executable `file` as a new process. Returns its thread id.
#[allow(dead_code)] // used by the self-tests now, by the shell (`run`) later
pub fn spawn_elf(file: &[u8]) -> Result<u64, SpawnError> {
    let mut space = AddressSpace::new().ok_or(SpawnError::NoMemory)?;
    let entry = elf::load(file, &mut space).map_err(SpawnError::BadElf)?;
    start(space, entry)
}

/// Gives the process a stack, then starts its thread at `entry`.
fn start(mut space: AddressSpace, entry: u64) -> Result<u64, SpawnError> {
    for p in 1..=STACK_PAGES {
        space
            .map_new(STACK_TOP - p * PAGE_SIZE, paging::WRITABLE | paging::NO_EXECUTE)
            .map_err(|_| SpawnError::NoMemory)?;
    }
    // The process is registered before its thread can run (and exit).
    let mut procs = PROCS.lock();
    let thread = sched::spawn_in(space.root(), start_user);
    procs.push(Process { thread, entry, _space: space });
    Ok(thread)
}

/// Starts `code` (position-independent machine code) as a new process. Returns its thread id.
#[allow(dead_code)] // used by the self-tests now, by the ELF loader (P2-06)
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
    start(space, CODE_BASE)
}

/// First code of a process thread (already running on the process's tables): jump to ring 3.
fn start_user() {
    let id = sched::current_id();
    let entry = PROCS.lock().iter().find(|p| p.thread == id).map(|p| p.entry).expect("proc: fir fara proces");
    // SAFETY: `entry` (checked by the loader to be in an executable segment) and the stack below
    // STACK_TOP are mapped USER in this thread's address space; the scheduler set this thread's kernel
    // stack for ring-3 entries.
    unsafe { enter_user(entry, STACK_TOP) }
}

/// Called when the program on thread `id` ends: frees its memory and records the exit code.
pub fn on_exit(id: u64, code: i64) {
    let space = {
        let mut procs = PROCS.lock();
        procs.iter().position(|p| p.thread == id).map(|i| procs.swap_remove(i))
    };
    drop(space); // switches to the kernel's tables first if they were in use (see AddressSpace::drop)
    EXITED.lock().push((id, code));
    EXIT_WAIT.wake_all();
}

/// Waits until the program on thread `id` ends and returns its exit code.
#[allow(dead_code)] // used by the self-tests now, by the shell (`run`) later
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
