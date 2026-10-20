//! Runtime for Cyber Universe OS user programs: system call wrappers, `print!`, the entry point
//! and the panic handler. Every program starts with `cyber_user::entry!(main);`.

#![no_std]

use core::arch::asm;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_YIELD: u64 = 2;
pub const SYS_SLEEP_MS: u64 = 3;
pub const SYS_OPEN: u64 = 4;
pub const SYS_READ: u64 = 5;
pub const SYS_CLOSE: u64 = 6;
pub const SYS_STAT: u64 = 7;

/// Negative results of system calls.
pub const E_NOT_FOUND: i64 = -3;
pub const E_IS_DIR: i64 = -4;

/// Short text for an error result.
pub fn error_text(e: i64) -> &'static str {
    match e {
        -1 => "argument invalid",
        -2 => "adresa invalida",
        E_NOT_FOUND => "nu exista",
        E_IS_DIR => "e director",
        -5 => "descriptor inchis",
        -6 => "prea multe fisiere deschise",
        _ => "eroare",
    }
}

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

/// Opens the file at `path` for reading. Returns a descriptor (>= 3) or a negative error.
pub fn open(path: &str) -> i64 {
    syscall3(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, 0)
}

/// Reads into `buf` from the current position. Returns bytes read (0 = end of file) or a negative error.
pub fn read(fd: i64, buf: &mut [u8]) -> i64 {
    syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Closes a descriptor.
pub fn close(fd: i64) -> i64 {
    syscall3(SYS_CLOSE, fd as u64, 0, 0)
}

/// Size of the file at `path`, or a negative error.
pub fn stat(path: &str) -> i64 {
    syscall3(SYS_STAT, path.as_ptr() as u64, path.len() as u64, 0)
}

static mut ARGC: usize = 0;
static mut ARGV: *const (u64, u64) = core::ptr::null();

/// Called by `entry!` with what the kernel passed in rdi and rsi.
#[doc(hidden)]
pub fn set_args(argc: usize, argv: *const (u64, u64)) {
    // SAFETY: called once, before `main`, by the only thread of the program.
    unsafe {
        ARGC = argc;
        ARGV = argv;
    }
}

/// The program's arguments (argument 0 = its name).
pub fn args() -> impl Iterator<Item = &'static str> {
    // SAFETY: written once by `set_args` before `main`; the kernel put `argc` (address, length) pairs
    // of UTF-8 text on our stack, which stays in place for the program's whole life.
    let (argc, argv) = unsafe { (ARGC, ARGV) };
    (0..argc).map(move |i| {
        // SAFETY: as above; i < argc.
        let (ptr, len) = unsafe { *argv.add(i) };
        // SAFETY: as above; the kernel copied valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr as *const u8, len as usize)) }
    })
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
        pub extern "C" fn _start(argc: usize, argv: *const (u64, u64)) -> ! {
            $crate::set_args(argc, argv);
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
