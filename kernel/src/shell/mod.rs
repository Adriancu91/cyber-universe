//! The first shell: reads a line from the keyboard and runs a command. Everything it prints goes to
//! the screen and to the serial port (so tests and a serial terminal see it too).

use crate::drivers::keyboard::{Key, wait_key};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

const PROMPT: &str = "cyber> ";
const MAX_LINE: usize = 256;

/// Prints to the screen and to the serial port, or to the terminal window this shell runs for.
macro_rules! out {
    ($($arg:tt)*) => {{
        let text = format!($($arg)*);
        emit(&text);
    }};
}

fn emit(text: &str) {
    match crate::term::of_shell(crate::sched::current_id()) {
        Some(t) => crate::term::write(t, text.as_bytes()),
        None => {
            crate::print!("{}", text);
            crate::serial_print!("{}", text);
        }
    }
}

/// The next key: from the keyboard, or from the terminal window (`None` when it has closed).
fn next_key() -> Option<Key> {
    match crate::term::of_shell(crate::sched::current_id()) {
        Some(t) => crate::term::wait_key(t),
        None => Some(wait_key()),
    }
}

/// Runs the shell forever (in test builds, until the `test-gata` command).
pub fn run() {
    out!("\nScrie `help` pentru lista de comenzi.\n");
    loop {
        out!("{}", PROMPT);
        let Some(line) = read_line() else { return }; // the terminal window closed
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&cmd, args)) = words.split_first() else { continue };
        match cmd {
            "help" => help(),
            "ls" => ls(args.first().copied().unwrap_or("/")),
            "cat" => cat(args),
            "run" => run_program(args),
            "desktop" => run_program(&["desktop"]),
            "mod" => module(args),
            "tastatura" => layout(args),
            "mem" => mem(),
            "lspci" => lspci(),
            "lsusb" => lsusb(),
            "net" => net(),
            "uptime" => uptime(),
            "reboot" => reboot(),
            "exit" => match crate::term::of_shell(crate::sched::current_id()) {
                Some(t) => {
                    crate::term::close(t);
                    return;
                }
                None => out!("exit: shell-ul consolei nu se inchide (merge cat calculatorul)\n"),
            },
            #[cfg(feature = "qemu-test")]
            "test-gata" => return,
            _ => out!("comanda necunoscuta: {} (scrie help)\n", cmd),
        }
    }
}

/// Reads one line, echoing it; Backspace erases the last character. `None` if the terminal closed.
fn read_line() -> Option<String> {
    let mut line = String::new();
    loop {
        match next_key()? {
            Key::Enter => {
                out!("\n");
                return Some(line);
            }
            Key::Backspace => {
                if line.pop().is_some() {
                    out!("\x08");
                }
            }
            Key::Char(c) if line.len() < MAX_LINE && !c.is_control() => {
                line.push(c);
                out!("{}", c);
            }
            _ => {}
        }
    }
}

fn help() {
    out!("comenzi:\n");
    out!("  help             aceasta lista\n");
    out!("  ls [director]    continutul unui director (implicit /)\n");
    out!("  cat <fisier>...  afiseaza fisiere\n");
    out!("  run <program> [argumente]  porneste un program (din /bin daca nu dai calea)\n");
    out!("  desktop          porneste desktopul (Esc il inchide si revine aici)\n");
    out!("  mod list|install <pachet.cum>|start|stop|remove <nume>  module\n");
    out!("  tastatura [ro|us]  aranjamentul tastaturii (romana standard sau engleza SUA)\n");
    out!("  mem              memoria libera si cea folosita\n");
    out!("  lspci            dispozitivele PCI (discuri, USB, retea...)\n");
    out!("  lsusb            dispozitivele USB conectate\n");
    out!("  net              placa de retea (adresa MAC, pachete)\n");
    out!("  uptime           de cat timp merge sistemul\n");
    out!("  reboot           reporneste calculatorul\n");
    out!("  exit             inchide terminalul (doar in fereastra Terminal)\n");
}

fn ls(path: &str) {
    match fs::list(path) {
        Ok(entries) if entries.is_empty() => out!("(gol)\n"),
        Ok(entries) => {
            for (name, kind) in entries {
                match kind {
                    Kind::Dir => out!("  {}/\n", name),
                    Kind::File => out!("  {}  ({} octeti)\n", name, fs::size(&join(path, &name)).unwrap_or(0)),
                }
            }
        }
        Err(e) => out!("ls: {}: {}\n", path, e.message()),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{}{}", dir, name) } else { format!("{}/{}", dir, name) }
}

fn cat(args: &[&str]) {
    if args.is_empty() {
        out!("folosire: cat <fisier>...\n");
    }
    for path in args {
        match fs::read_all(path) {
            Ok(data) => out!("{}", String::from_utf8_lossy(&data)),
            Err(e) => out!("cat: {}: {}\n", path, e.message()),
        }
    }
}

fn run_program(args: &[&str]) {
    let Some(&name) = args.first() else {
        out!("folosire: run <program> [argumente]\n");
        return;
    };
    let path = if name.contains('/') { String::from(name) } else { format!("/bin/{}", name) };
    match crate::proc::spawn_path(&path, args) {
        Ok(id) => {
            let code = crate::proc::wait(id);
            if code != 0 {
                out!("({} a iesit cu codul {})\n", name, code);
            }
        }
        Err(e) => out!("run: {}: {}\n", path, e.message()),
    }
}

fn layout(args: &[&str]) {
    use crate::drivers::keyboard::{Layout, layout, set_layout};
    match args {
        ["ro"] => set_layout(Layout::Ro),
        ["us"] => set_layout(Layout::Us),
        [] => {}
        _ => {
            out!("folosire: tastatura ro | tastatura us\n");
            return;
        }
    }
    out!("tastatura: {}\n", if layout() == Layout::Ro { "romana (standard)" } else { "engleza (SUA)" });
}

fn module(args: &[&str]) {
    use crate::modmgr;
    let report = |r: Result<(), modmgr::ModError>| {
        if let Err(e) = r {
            out!("mod: {}\n", e.message());
        }
    };
    match args {
        ["list"] | [] => match modmgr::installed() {
            Ok(list) if list.is_empty() => out!("(niciun modul instalat in {})\n", modmgr::ROOT),
            Ok(list) => {
                for m in list {
                    let state = if modmgr::is_running(&m.name) { "merge" } else { "oprit" };
                    out!("  {} {}  [{}]  {}\n", m.name, m.version, state, m.description);
                }
            }
            Err(e) => out!("mod: {}\n", e.message()),
        },
        ["install", package] => report(modmgr::install(package).map(|m| out!("instalat: {} {}\n", m.name, m.version))),
        ["start", name] => report(modmgr::start(name).map(|_| ())),
        ["stop", name] => report(modmgr::stop(name)),
        ["remove", name] => report(modmgr::remove(name)),
        _ => out!("folosire: mod list | mod install <pachet.cum> | mod start|stop|remove <nume>\n"),
    }
}

fn mem() {
    use crate::mem::{frame, heap, map::PAGE_SIZE};
    let (heap_used, heap_mapped) = heap::stats();
    out!(
        "memorie: libera {} MiB, folosita {} KiB (heap {} / {} KiB)\n",
        frame::free_frames() * PAGE_SIZE / (1024 * 1024),
        frame::used_frames() * PAGE_SIZE / 1024,
        heap_used / 1024,
        heap_mapped / 1024
    );
}

fn lspci() {
    for d in crate::pci::devices() {
        out!("  {:02x}:{:02x}.{}  {:04x}:{:04x}  {}\n", d.bus, d.dev, d.func, d.vendor, d.device, d.kind());
    }
}

fn lsusb() {
    let devices = crate::drivers::usb::devices();
    if devices.is_empty() {
        out!("(niciun dispozitiv USB)\n");
    }
    for d in devices {
        out!("  port {}  {:04x}:{:04x}  {}\n", d.port, d.vendor, d.product, d.kind());
    }
}

fn net() {
    match crate::drivers::e1000::NIC.lock().as_ref() {
        Some(n) => {
            let m = n.mac;
            out!("  {}  MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\n", n.name, m[0], m[1], m[2], m[3], m[4], m[5]);
            out!("  pachete trimise {}, primite {}\n", n.sent, n.received);
        }
        None => out!("(nicio placa de retea cunoscuta)\n"),
    }
}

fn uptime() {
    match crate::time::uptime_ms() {
        Some(ms) => out!("pornit de {}.{:02} s\n", ms / 1000, (ms % 1000) / 10),
        None => out!("uptime: ceasul nu merge\n"),
    }
}

/// Restarts the machine through the keyboard controller's reset line (works on PCs and in QEMU).
fn reboot() -> ! {
    out!("repornesc...\n");
    // SAFETY: command 0xFE to the 8042 pulses the CPU reset line; nothing runs after it.
    unsafe { crate::arch::x86_64::outb(0x64, 0xFE) };
    crate::arch::x86_64::halt_forever()
}
