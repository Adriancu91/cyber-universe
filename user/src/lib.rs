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

/// A shell running for this program (see kernel/src/term.rs).
pub mod term {
    use super::{SYS_TERM_OPEN, SYS_TERM_READ, SYS_TERM_WRITE, syscall3};
    /// Starts a shell for this program; returns its terminal id.
    pub fn open() -> i64 {
        syscall3(SYS_TERM_OPEN, 0, 0, 0)
    }
    /// Output waiting to be shown (0 = nothing new; negative once the shell has ended).
    pub fn read(id: i64, buf: &mut [u8]) -> i64 {
        syscall3(SYS_TERM_READ, id as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
    }
    /// Keys typed in the window ('\n' = Enter, '\u{8}' = Backspace).
    pub fn write(id: i64, text: &str) -> i64 {
        syscall3(SYS_TERM_WRITE, id as u64, text.as_ptr() as u64, text.len() as u64)
    }
}

pub mod gfx;
pub mod settings;
pub mod text;

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
        -7 => "exista deja",
        -8 => "discul nu e marcat pentru scriere",
        -9 => "directorul nu e gol",
        -10 => "nu mai e loc pe disc",
        -11 => "lipseste un modul necesar",
        -12 => "alt modul are nevoie de el",
        -13 => "modulul merge; opreste-l intai",
        -14 => "modulul nu merge",
        -15 => "pachet stricat",
        -16 => "modulul nu are program",
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

/// Raw system call with four arguments (the fourth goes in r10).
#[inline(always)]
pub fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    // SAFETY: as in `syscall3`.
    unsafe {
        asm!("syscall", inlateout("rax") nr as i64 => ret, in("rdi") a1, in("rsi") a2, in("rdx") a3, in("r10") a4,
             lateout("rcx") _, lateout("r11") _, options(nostack))
    }
    ret
}

/// Raw system call with five arguments (the fourth in r10, the fifth in r8).
#[inline(always)]
pub fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> i64 {
    let ret: i64;
    // SAFETY: as in `syscall3`.
    unsafe {
        asm!("syscall", inlateout("rax") nr as i64 => ret, in("rdi") a1, in("rsi") a2, in("rdx") a3, in("r10") a4,
             in("r8") a5, lateout("rcx") _, lateout("r11") _, options(nostack))
    }
    ret
}

/// The module manager (see kernel/src/syscall/README.md, call 21).
pub mod modules {
    use super::{SYS_MODULE, syscall5};

    /// Installed modules, one per line: `name version running|stopped description`.
    pub fn list(buf: &mut [u8]) -> i64 {
        syscall5(SYS_MODULE, 0, 0, 0, buf.as_mut_ptr() as u64, buf.len() as u64)
    }
    fn op(code: u64, text: &str) -> i64 {
        syscall5(SYS_MODULE, code, text.as_ptr() as u64, text.len() as u64, 0, 0)
    }
    /// Installs the package (.cum) at `path`.
    pub fn install(path: &str) -> i64 {
        op(1, path)
    }
    pub fn start(name: &str) -> i64 {
        op(2, name)
    }
    pub fn stop(name: &str) -> i64 {
        op(3, name)
    }
    pub fn remove(name: &str) -> i64 {
        op(4, name)
    }
}

/// Starts the program at `path`. Returns its id or a negative error.
pub fn spawn(path: &str) -> i64 {
    syscall4(SYS_SPAWN, path.as_ptr() as u64, path.len() as u64, 0, 0)
}

/// Starts the program at `path` with one argument (e.g. the file to open).
pub fn spawn_with(path: &str, arg: &str) -> i64 {
    syscall4(SYS_SPAWN, path.as_ptr() as u64, path.len() as u64, arg.as_ptr() as u64, arg.len() as u64)
}

/// Waits for the program `id` to end and returns its exit code (negative error if there is no such program).
pub fn wait(id: i64) -> i64 {
    syscall3(SYS_WAIT, id as u64, 0, 0)
}

/// Writes the names in directory `path` into `buf`, one per line ('/' after directories).
/// Returns the bytes written or a negative error.
pub fn list(path: &str, buf: &mut [u8]) -> i64 {
    syscall4(SYS_LIST, path.as_ptr() as u64, path.len() as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Memory and time figures from the kernel.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemInfo {
    pub total: u64,
    pub used: u64,
    pub heap_used: u64,
    pub heap_reserved: u64,
    pub uptime_ms: u64,
    /// Processor time when nothing else had work, in ticks.
    pub idle_ticks: u64,
    pub ticks_per_second: u64,
}

pub fn mem_info() -> Option<MemInfo> {
    let mut v = [0u64; 7];
    let r = syscall3(SYS_SYSINFO, 0, v.as_mut_ptr() as u64, 56);
    (r == 56).then(|| MemInfo {
        total: v[0],
        used: v[1],
        heap_used: v[2],
        heap_reserved: v[3],
        uptime_ms: v[4],
        idle_ticks: v[5],
        ticks_per_second: v[6],
    })
}

/// The running programs, one per line: `id processor_ticks name`. Returns the bytes written to `buf`.
pub fn programs(buf: &mut [u8]) -> i64 {
    syscall3(SYS_SYSINFO, 1, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Stops the program `id` (it ends with code -9).
pub fn kill(id: u64) -> i64 {
    syscall3(SYS_KILL, id, 0, 0)
}

/// Creates an empty file, or a folder if `dir`.
pub fn create(path: &str, dir: bool) -> i64 {
    syscall3(SYS_CREATE, path.as_ptr() as u64, path.len() as u64, dir as u64)
}

/// Deletes a file or an empty folder.
pub fn remove(path: &str) -> i64 {
    syscall3(SYS_REMOVE, path.as_ptr() as u64, path.len() as u64, 0)
}

/// Date and time from the computer's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// The current date and time, if the computer has a clock.
pub fn time() -> Option<DateTime> {
    let r = syscall3(SYS_TIME, 0, 0, 0);
    if r < 0 {
        return None;
    }
    let b = |shift: u32| (r >> shift) as u8;
    Some(DateTime { year: (r >> 40) as u16, month: b(32), day: b(24), hour: b(16), minute: b(8), second: b(0) })
}

/// Width and height of the screen in pixels.
pub fn screen_size() -> Option<(usize, usize)> {
    let r = syscall3(SYS_SCREEN_SIZE, 0, 0, 0);
    (r > 0).then(|| ((r as u64 & 0xFFFF_FFFF) as usize, (r as u64 >> 32) as usize))
}

/// Ends the program with `code`.
pub fn exit(code: i64) -> ! {
    syscall3(SYS_EXIT, code as u64, 0, 0);
    unreachable!()
}

/// Writes `bytes` to the file descriptor `fd` (1 = console; an open file: at its position). Returns bytes written or a negative error.
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
