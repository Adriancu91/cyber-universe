//! The context switch itself: saves the callee-saved registers of the current thread on its stack,
//! stores its stack pointer and continues on the stack of the next thread.
//! Written from the System V AMD64 ABI (callee-saved registers: rbx, rbp, r12-r15).

/// Saves the current thread's context, stores its `rsp` in `*old_rsp` and resumes the thread whose
/// saved stack pointer is `new_rsp`. Returns when some thread switches back to the saved context.
///
/// # Safety
/// `old_rsp` must be valid for writes; `new_rsp` must be a stack pointer saved by this function or
/// prepared by `initial_stack`. Interrupts must be disabled.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_context(old_rsp: *mut u64, new_rsp: u64) {
    core::arch::naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
    )
}

/// First code a new thread runs (reached through `ret` in `switch_context`): r12 holds the entry
/// function, which is passed to `thread_entry`. The stack is 16-byte aligned here, so `call` leaves it
/// the way the ABI expects.
#[unsafe(naked)]
unsafe extern "C" fn thread_trampoline() {
    core::arch::naked_asm!("mov rdi, r12", "call {entry}", "ud2", entry = sym super::thread_entry)
}

/// Prepares the stack of a thread that has never run, so that `switch_context` "returns" into
/// `thread_trampoline` with r12 = `entry`. `top` must be 16-byte aligned. Returns the initial `rsp`.
///
/// # Safety
/// `[top - 56, top)` must be writable memory owned by the new thread's stack.
pub unsafe fn initial_stack(top: u64, entry: usize) -> u64 {
    let frame = top - 7 * 8;
    let words = frame as *mut u64;
    // Layout (low to high): r15, r14, r13, r12, rbp, rbx, return address.
    let values = [0, 0, 0, entry as u64, 0, 0, thread_trampoline as *const () as u64];
    for (i, v) in values.iter().enumerate() {
        // SAFETY: inside the region the caller guarantees.
        unsafe { words.add(i).write(*v) };
    }
    frame
}
