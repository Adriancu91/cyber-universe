//! Terminals (P4-13): a shell that runs for a window program instead of the text console. The
//! program (`/bin/terminal`) sends the keys typed in its window and reads what the shell, and the
//! programs started from it, print. Each terminal has its own shell thread.

use crate::drivers::keyboard::Key;
use crate::sched::{self, Mutex, WaitQueue};
use alloc::{collections::VecDeque, vec::Vec};

struct Term {
    id: u64,
    /// Thread of the program that opened it (the window).
    owner: u64,
    /// The shell's thread.
    shell: u64,
    input: VecDeque<Key>,
    output: VecDeque<u8>,
    /// The window program has ended (or the shell said `exit`).
    closed: bool,
}

static TERMS: Mutex<Vec<Term>> = Mutex::new(Vec::new());
static INPUT: WaitQueue = WaitQueue::new();
static NEXT_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
/// Output kept for a terminal before the window reads it.
const OUTPUT_MAX: usize = 64 * 1024;

/// Opens a terminal for the program on thread `owner` and starts its shell. Returns its id.
pub fn open(owner: u64) -> u64 {
    let id = NEXT_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut terms = TERMS.lock();
    // The shell thread looks itself up in TERMS, which stays locked until its entry exists.
    let shell = sched::spawn(shell_thread);
    terms.push(Term { id, owner, shell, input: VecDeque::new(), output: VecDeque::new(), closed: false });
    id
}

fn shell_thread() {
    crate::shell::run();
    let me = sched::current_id();
    let mut terms = TERMS.lock();
    if let Some(t) = terms.iter_mut().find(|t| t.shell == me) {
        t.closed = true;
        t.shell = 0;
    }
    terms.retain(|t| !(t.closed && t.shell == 0 && t.owner == 0));
}

/// The terminal whose shell runs on thread `thread`.
pub fn of_shell(thread: u64) -> Option<u64> {
    TERMS.lock().iter().find(|t| t.shell == thread && thread != 0).map(|t| t.id)
}

/// Adds text to a terminal's output (from its shell or from a program started there).
pub fn write(id: u64, bytes: &[u8]) {
    let mut terms = TERMS.lock();
    if let Some(t) = terms.iter_mut().find(|t| t.id == id && !t.closed) {
        for &b in bytes {
            if t.output.len() == OUTPUT_MAX {
                t.output.pop_front();
            }
            t.output.push_back(b);
        }
    }
}

/// Waits for the next key typed in the terminal's window; `None` once the window has closed.
pub fn wait_key(id: u64) -> Option<Key> {
    let mut result = None;
    INPUT.wait_until(|| {
        // Checked with interrupts off: never sleep on the lock here.
        if let Some(mut terms) = TERMS.try_lock() {
            result = match terms.iter_mut().find(|t| t.id == id) {
                Some(t) if t.closed => Some(None),
                Some(t) => t.input.pop_front().map(Some),
                None => Some(None),
            };
        }
        result.is_some()
    });
    result.flatten()
}

/// The shell of terminal `id` ended its session (`exit`): the window sees the terminal as closed.
pub fn close(id: u64) {
    if let Some(t) = TERMS.lock().iter_mut().find(|t| t.id == id) {
        t.closed = true;
    }
}

/// For the window program: copies waiting output into `buf`. `None` once the terminal is closed and
/// everything was read.
pub fn read_output(owner: u64, id: u64, buf: &mut [u8]) -> Option<usize> {
    let mut terms = TERMS.lock();
    let t = terms.iter_mut().find(|t| t.id == id && t.owner == owner)?;
    let n = buf.len().min(t.output.len());
    for (i, b) in t.output.drain(..n).enumerate() {
        buf[i] = b;
    }
    (n > 0 || !t.closed).then_some(n)
}

/// For the window program: keys typed in the window ('\n' = Enter, 8 = Backspace, 27 = Escape).
pub fn send_keys(owner: u64, id: u64, text: &str) -> bool {
    let mut terms = TERMS.lock();
    let Some(t) = terms.iter_mut().find(|t| t.id == id && t.owner == owner && !t.closed) else { return false };
    for c in text.chars() {
        t.input.push_back(match c {
            '\n' => Key::Enter,
            '\u{8}' => Key::Backspace,
            '\t' => Key::Tab,
            '\u{1b}' => Key::Escape,
            c => Key::Char(c),
        });
    }
    drop(terms);
    INPUT.wake_all();
    true
}

/// The window program ended: its terminals close, so their shells end too.
pub fn close_owned(owner: u64) {
    let mut terms = TERMS.lock();
    for t in terms.iter_mut().filter(|t| t.owner == owner) {
        t.closed = true;
        t.owner = 0;
    }
    terms.retain(|t| !(t.closed && t.shell == 0 && t.owner == 0));
    drop(terms);
    INPUT.wake_all();
}
