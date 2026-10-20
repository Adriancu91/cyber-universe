//! System calls: the services user programs ask the kernel for with the `syscall` instruction.
//! The entry code is in `arch/x86_64/syscall.rs`; this file checks the arguments and does the work.

use crate::mem::{address_space::USER_END, map::PAGE_SIZE, paging};
use crate::sched;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_YIELD: u64 = 2;
pub const SYS_SLEEP_MS: u64 = 3;

/// Error results (negative, like the convention most systems use).
pub const E_INVALID: i64 = -1; // unknown call or bad argument
pub const E_FAULT: i64 = -2; // pointer outside the program's memory

/// Called from `syscall_entry` with interrupts disabled, on the thread's kernel stack.
pub extern "C" fn dispatch(nr: u64, a1: u64, a2: u64, _a3: u64, _a4: u64, _a5: u64) -> i64 {
    // The kernel may now be interrupted (and preempted) while it serves the call.
    // SAFETY: the IDT and the scheduler are ready long before any program runs.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
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
fn user_range_ok(addr: u64, len: u64, write: bool) -> bool {
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

/// write(fd, buf, len): fd 1 = the console (screen + serial). Returns the number of bytes written.
fn write(fd: u64, buf: u64, len: u64) -> i64 {
    if fd != 1 || len > 64 * 1024 {
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
