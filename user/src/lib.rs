//! Runtime for Cyber Universe OS user programs: system call wrappers, `print!`, the entry point
//! and the panic handler. Every program starts with `cyber_user::entry!(main);`.

#![no_std]

use core::arch::asm;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_YIELD: u64 = 2;
pub const SYS_SLEEP_MS: u64 = 3;

/// Raw system call with up to three arguments (see kernel/src/syscall/README.md).
#[inline(always)]
pub fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    // SAFETY: `syscall` enters the kernel, which checks every argument; only rax, rcx and r11 change.
    unsafe {
        asm!("syscall", inlateout("rax") nr as i64 => ret, in("rdi") a1, in("rsi") a2, in("rdx") a3,
             lateout("rcx") _, lateout("r11") _, options(nostack))
    }
    ret
}

/// Ends the program with `code`.
pub fn exit(code: i64) -> ! {
    syscall3(SYS_EXIT, code as u64, 0, 0);
    unreachable!()
}

/// Writes `bytes` to the file descriptor `fd` (1 = console). Returns bytes written or a negative error.
pub fn write(fd: u64, bytes: &[u8]) -> i64 {
    syscall3(SYS_WRITE, fd, bytes.as_ptr() as u64, bytes.len() as u64)
}

/// Gives the processor to another thread.
pub fn yield_now() {
    syscall3(SYS_YIELD, 0, 0, 0);
}

/// Sleeps at least `ms` milliseconds.
pub fn sleep_ms(ms: u64) {
    syscall3(SYS_SLEEP_MS, ms, 0, 0);
}

/// Console output used by `print!`.
pub struct Console;

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if write(1, s.as_bytes()) < 0 { Err(core::fmt::Error) } else { Ok(()) }
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{ let _ = core::fmt::Write::write_fmt(&mut $crate::Console, format_args!($($arg)*)); }};
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}

/// Defines the program's entry point: calls `main` and exits with the code it returns.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn _start() -> ! {
            let f: fn() -> i64 = $main;
            $crate::exit(f())
        }
    };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("program: panica: {}", info.message());
    exit(101)
}
