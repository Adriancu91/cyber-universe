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
            "fetch" => run_program(&words), // same as `run fetch ...` (test 6 L: typed without `run`)
            "desktop" => run_program(&["desktop"]),
            "mod" => module(args),
            "tastatura" => layout(args),
            "ping" => ping(args),
            "host" => host(args),
            "http" => http(args),
            "mem" => mem(),
            "lspci" => lspci(),
            "lsusb" => lsusb(),
            "net" => net(),
            "rtl" => rtl(args),
            "hw" => hw(),
            "gpu" => gpu(args),
            "log" => show_log(args),
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
    out!("  ping <adresa>    trimite 4 ping-uri (ICMP); merge si cu un nume\n");
    out!("  host <nume>      afla adresa IPv4 a unui nume (DNS)\n");
    out!("  fetch <url>      descarca o pagina (HTTP/HTTPS), la fel ca `run fetch <url>`\n");
    out!("  http <nume> [port] [cale]  cere o pagina web (HTTP prin TCP), arata inceputul\n");
    out!("  tastatura [ro|us]  aranjamentul tastaturii (romana standard sau engleza SUA)\n");
    out!("  mem              memoria libera si cea folosita\n");
    out!("  lspci            dispozitivele PCI (discuri, USB, retea...)\n");
    out!("  lsusb            dispozitivele USB conectate\n");
    out!("  net              placa de retea (adresa MAC, pachete)\n");
    out!("  rtl porneste [variante]  reporneste placa Realtek cu variantele date (fara: ca la pornire; `rtl` le arata)\n");
    out!("  gpu [identifica|plan]  placile video (nivel 0: doar citire; NVIDIA: registrele de identitate)\n");
    out!("  hw               raport hardware pe o pagina (pentru o singura poza)\n");
    out!("  log [cuvant] [n] ultimele n randuri din jurnal (implicit 30), doar cele cu cuvantul\n");
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

fn ping(args: &[&str]) {
    let Some(target) = args.first() else {
        out!("folosire: ping <a.b.c.d sau nume>\n");
        return;
    };
    if crate::net::config().is_none() {
        out!("ping: nicio placa de retea\n");
        return;
    }
    if !crate::net::wait_ready(3000) {
        out!("ping: reteaua nu are inca adresa (DHCP)\n");
        return;
    }
    let ip = match crate::net::dns::resolve(target) {
        Ok((ip, _)) => ip,
        Err(e) => {
            out!("ping: {}: {}\n", target, e.message());
            return;
        }
    };
    for seq in 1..=4u16 {
        match crate::net::icmp::ping(ip, seq, 2000) {
            Some(ms) => out!("raspuns de la {}.{}.{}.{}: seq={} timp={} ms\n", ip[0], ip[1], ip[2], ip[3], seq, ms),
            None => out!("seq={}: niciun raspuns\n", seq),
        }
        crate::sched::sleep_ms(200);
    }
}

fn host(args: &[&str]) {
    let Some(name) = args.first() else {
        out!("folosire: host <nume>\n");
        return;
    };
    match crate::net::dns::resolve(name) {
        Ok((ip, cache)) => {
            out!("{} = {}.{}.{}.{}{}\n", name, ip[0], ip[1], ip[2], ip[3], if cache { " (din memorie)" } else { "" })
        }
        Err(e) => out!("host: {}: {}\n", name, e.message()),
    }
}

/// Fetches a page over plain HTTP (TCP) and shows the start of the answer: a manual check of the
/// network stack on a real machine.
fn http(args: &[&str]) {
    use crate::net::{dns, tcp};
    let Some(name) = args.first() else {
        out!("folosire: http <nume> [port] [cale]\n");
        return;
    };
    let port = match args.get(1).map(|p| p.parse::<u16>()) {
        None => 80,
        Some(Ok(p)) => p,
        Some(Err(_)) => {
            out!("http: port invalid\n");
            return;
        }
    };
    let path = args.get(2).copied().unwrap_or("/");
    let ip = match dns::resolve(name) {
        Ok((ip, _)) => ip,
        Err(e) => {
            out!("http: {}: {}\n", name, e.message());
            return;
        }
    };
    let conn = match tcp::connect(ip, port, 5000) {
        Ok(c) => c,
        Err(e) => {
            out!("http: {}:{}: {}\n", name, port, e.message());
            return;
        }
    };
    let request = alloc::format!("GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: CyberUniverse\r\nConnection: close\r\n\r\n", path, name);
    let mut body = Vec::new();
    let mut error = tcp::send(conn, request.as_bytes(), 5000).err();
    while error.is_none() {
        match tcp::recv(conn, 4096, 10_000) {
            Ok(d) if d.is_empty() => break,
            Ok(d) => body.extend_from_slice(&d),
            Err(e) => error = Some(e),
        }
    }
    tcp::close(conn, 1000);
    let shown = &body[..body.len().min(400)];
    out!("{}\n", String::from_utf8_lossy(shown));
    match error {
        Some(e) => out!("http: {} (primiti {} octeti)\n", e.message(), body.len()),
        None => out!("-- {} octeti primiti\n", body.len()),
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

fn gpu(args: &[&str]) {
    match args.first().copied() {
        None | Some("identifica") => {
            for l in crate::drivers::gpu::identify() {
                out!("{}\n", l);
            }
        }
        Some("plan") => {
            for l in crate::drivers::gpu::PLAN {
                out!("{}\n", l);
            }
        }
        Some(other) => out!("gpu: necunoscut: {} (folosire: gpu identifica | gpu plan)\n", other),
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
    match crate::drivers::nic::NIC.lock().as_mut() {
        Some(n) => {
            let m = n.mac();
            out!("  {}  MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\n", n.name(), m[0], m[1], m[2], m[3], m[4], m[5]);
            let (sent, received) = n.counters();
            out!("  pachete trimise {}, primite {}\n", sent, received);
            if let Some(info) = n.link_info() {
                for line in info.lines() {
                    out!("  {}\n", line);
                }
            }
            if let Some(c) = crate::net::config().filter(|c| c.ip != [0; 4]) {
                let ip = |a: [u8; 4]| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
                out!("  IPv4 {}  masca {}  ruta {}  DNS {}\n", ip(c.ip), ip(c.mask), ip(c.gateway), ip(c.dns));
            } else {
                out!("  fara adresa (DHCP fara raspuns; reincearca la 10 s)\n");
            }
        }
        None => out!("(nicio placa de retea cunoscuta)\n"),
    }
}

/// `rtl porneste [bloc] [rxdv] [cache|faracache] [pagini] [rece|mcu|phy|cplus|ordine]`: resets and restarts the Realtek card with the given start-up choices, so
/// several variants can be tried on the laptop from one boot, one change at a time (`log rtl` shows each start).
fn rtl(args: &[&str]) {
    let Some((&"porneste", words)) = args.split_first() else {
        out!("folosire: rtl porneste [variante]\n");
        out!("  bloc   bufferele de primire intr-un singur bloc sub 4 GB\n");
        out!("  rxdv   deschide poarta RXDV (MISC bit 19), daca e inchisa\n");
        out!("  (fara variante = ca la pornire: golirea cache-ului + un bloc DMA peste 16 MiB)\n");
        out!("  cache     goleste cache-ul pentru fiecare pachet primit/trimis (implicit)\n");
        out!("  faracache fara golirea cache-ului (exact ca os-125)\n");
        out!("  pagini    inele si buffere pagina cu pagina (ca os-149), nu intr-un bloc peste 16 MiB\n");
        out!("  rece   pornirea completa la rece = mcu phy cplus ordine rxdv (pasii in `log rtl`)\n");
        out!("  mcu    preia placa de la firmware (MCU, OCP E8DE) inainte de resetare\n");
        out!("  phy    trezeste PHY-ul (PMCH bit 7, PHY 1F=0, 0E=0)\n");
        out!("  cplus  C+ Command bitii 0, 3, 7\n");
        out!("  ordine TCR/RCR (cu EarlyOff v2) scrise inainte de pornirea RX/TX\n");
        return;
    };
    let options = match crate::drivers::rtl8168::parse_options(words) {
        Ok(o) => o,
        Err(w) => {
            out!("rtl: varianta necunoscuta: {} (bloc, rxdv, cache, faracache, pagini, rece, mcu, phy, cplus, ordine)\n", w);
            return;
        }
    };
    let mut nic = crate::drivers::nic::NIC.lock();
    if nic.as_ref().is_some_and(|n| n.name() != "Realtek RTL8168/8111") {
        out!("rtl: placa de retea nu e Realtek\n");
        return;
    }
    // The old card object goes away (its memory stays reserved: the chip may still be writing there until the reset).
    *nic = None;
    out!("rtl: repornesc placa (asteapta legatura pana la 4 s)...\n");
    match crate::drivers::rtl8168::init_with(options) {
        Some(card) => {
            *nic = Some(alloc::boxed::Box::new(card));
            drop(nic);
            out!("rtl: pornita; vezi `net` peste ~30 s si `log rtl`\n");
        }
        None => out!("rtl: nicio placa Realtek pornita (vezi log rtl)\n"),
    }
}

/// The last lines of the kernel log, optionally only those containing a word (any case): `log`, `log rtl`, `log usb 50`.
fn show_log(args: &[&str]) {
    let mut count = 30usize;
    let mut word: Option<String> = None;
    for a in args {
        match a.parse::<usize>() {
            Ok(n) => count = n.clamp(1, 500),
            Err(_) => word = Some(a.to_ascii_lowercase()),
        }
    }
    let lines: Vec<String> = crate::log::recent()
        .into_iter()
        .filter(|l| word.as_ref().is_none_or(|w| l.to_ascii_lowercase().contains(w.as_str())))
        .collect();
    if lines.is_empty() {
        out!("(niciun rand)\n");
    }
    for l in &lines[lines.len().saturating_sub(count)..] {
        out!("{}\n", l);
    }
}

/// One-page hardware report: processor, memory, PCI devices with the driver that handles them, USB, disks, network.
fn hw() {
    out!("== raport hardware ==\n");
    out!("procesor: {}\n", cpu_name());
    mem();
    uptime();
    out!("PCI ([driver] = il folosim, [-] = fara driver):\n");
    for d in crate::pci::devices() {
        out!("  {:02x}:{:02x}.{} {:04x}:{:04x} {} [{}]\n", d.bus, d.dev, d.func, d.vendor, d.device, d.kind(), driver_of(&d));
    }
    out!("USB:\n");
    lsusb();
    for path in ["/disk", "/usb"] {
        match fs::list(path) {
            Ok(entries) => out!("{}: montat ({} intrari)\n", path, entries.len()),
            Err(_) => match fs::failure(path) {
                Some(why) => out!("{}: nemontat ({})\n", path, why),
                None => out!("{}: nemontat\n", path),
            },
        }
    }
    out!("retea:\n");
    net();
}

/// The processor's name from CPUID (leaves 0x8000_0002..4), or its vendor if the name is missing.
fn cpu_name() -> String {
    use core::arch::x86_64::__cpuid;
    let words = |r: core::arch::x86_64::CpuidResult| [r.eax, r.ebx, r.ecx, r.edx];
    let mut bytes = Vec::new();
    // CPUID exists on every x86_64 processor; extended leaves are read only if reported.
    if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            for w in words(__cpuid(leaf)) {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
    }
    if bytes.iter().all(|&b| b == 0 || b == b' ') {
        let r = __cpuid(0);
        for w in [r.ebx, r.edx, r.ecx] {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
    }
    let text: String = bytes.iter().filter(|&&b| b != 0).map(|&b| b as char).collect();
    String::from(text.trim())
}

/// Which of our drivers handles a PCI function ("-" if none).
fn driver_of(d: &crate::pci::Device) -> &'static str {
    match (d.class, d.subclass, d.prog_if) {
        (0x01, 0x06, _) => "ahci",
        (0x01, 0x08, 0x02) => "nvme",
        (0x0C, 0x03, 0x30) => "xhci",
        (0x02, _, _) => crate::drivers::nic::driver_for(d.vendor, d.device).unwrap_or("-"),
        _ => "-",
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
