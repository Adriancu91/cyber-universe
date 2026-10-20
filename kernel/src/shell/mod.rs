//! The first shell: reads a line from the keyboard and runs a command. Everything it prints goes to
//! the screen and to the serial port (so tests and a serial terminal see it too).

use crate::drivers::keyboard::{Key, wait_key};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

const PROMPT: &str = "cyber> ";
const MAX_LINE: usize = 256;

/// Prints to the screen and to the serial port.
macro_rules! out {
    ($($arg:tt)*) => {{
        let text = format!($($arg)*);
        crate::print!("{}", text);
        crate::serial_print!("{}", text);
    }};
}

/// Runs the shell forever (in test builds, until the `test-gata` command).
pub fn run() {
    out!("\nScrie `help` pentru lista de comenzi.\n");
    loop {
        out!("{}", PROMPT);
        let line = read_line();
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&cmd, args)) = words.split_first() else { continue };
        match cmd {
            "help" => help(),
            "ls" => ls(args.first().copied().unwrap_or("/")),
            "cat" => cat(args),
            "run" => run_program(args),
            "mem" => mem(),
            "lspci" => lspci(),
            "lsusb" => lsusb(),
            "net" => net(),
            "uptime" => uptime(),
            "reboot" => reboot(),
            #[cfg(feature = "qemu-test")]
            "test-gata" => return,
            _ => out!("comanda necunoscuta: {} (scrie help)\n", cmd),
        }
    }
}

/// Reads one line, echoing it; Backspace erases the last character.
fn read_line() -> String {
    let mut line = String::new();
    loop {
        match wait_key() {
            Key::Enter => {
                out!("\n");
                return line;
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
    out!("  mem              memoria libera si cea folosita\n");
    out!("  lspci            dispozitivele PCI (discuri, USB, retea...)\n");
    out!("  lsusb            dispozitivele USB conectate\n");
    out!("  net              placa de retea (adresa MAC, pachete)\n");
    out!("  uptime           de cat timp merge sistemul\n");
    out!("  reboot           reporneste calculatorul\n");
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
