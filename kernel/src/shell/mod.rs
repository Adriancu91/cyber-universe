//! The first shell: reads a line from the keyboard and runs a command. Everything it prints goes to
//! the screen and to the serial port (so tests and a serial terminal see it too).

use crate::drivers::keyboard::{Key, wait_key};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

pub mod auto;
pub mod compara;
pub mod raport;
#[cfg(feature = "qemu-test")]
pub use raport::self_test as raport_self_test;

const PROMPT: &str = "cyber> ";
const MAX_LINE: usize = 256;

/// Prints to the screen and to the serial port, or to the terminal window this shell runs for.
macro_rules! out {
    ($($arg:tt)*) => {{
        let text = format!($($arg)*);
        emit(&text);
    }};
}

/// While `raport` runs a command, that shell's output goes into the report instead: (shell thread, text so far).
static CAPTURE: crate::sched::Mutex<Option<(u64, String)>> = crate::sched::Mutex::new(None);

fn emit_capture_begin() {
    *CAPTURE.lock() = Some((crate::sched::current_id(), String::new()));
}

fn emit_capture_end() -> String {
    CAPTURE.lock().take().map(|(_, t)| t).unwrap_or_default()
}

/// While the automatic mode runs a command, what programs started by it print is also kept here (P6-41).
static PROGRAM_CAPTURE: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);

fn program_capture_begin() {
    *PROGRAM_CAPTURE.lock() = Some(String::new());
}

fn program_capture_end() -> String {
    PROGRAM_CAPTURE.lock().take().unwrap_or_default()
}

/// Called by the `write` system call for console output: keeps a copy for the automatic mode's report.
pub fn program_output(text: &str) {
    if let Some(buf) = PROGRAM_CAPTURE.lock().as_mut()
        && buf.len() < 1024 * 1024
    {
        buf.push_str(text);
    }
}

fn emit(text: &str) {
    {
        let mut c = CAPTURE.lock();
        if let Some((id, buf)) = c.as_mut()
            && *id == crate::sched::current_id()
        {
            buf.push_str(text);
            return;
        }
    }
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
        raport::auto_if_errors();
        out!("{}", PROMPT);
        let Some(line) = read_line() else { return }; // the terminal window closed
        // The remote console (P6-49): only the allow-listed commands, each one noted in the log.
        if !line.trim().is_empty() && crate::term::is_restricted(crate::sched::current_id()) {
            if let Err(why) = remote_allowed(&line) {
                crate::info!("remote: REFUZAT: {} ({})", line.trim(), why);
                out!("REFUZAT: {} (prin remote merg doar comenzile din lista alba; `help` le arata)\n", why);
                continue;
            }
            crate::info!("remote: comanda: {}", line.trim());
        }
        if !dispatch(&line) {
            return;
        }
    }
}

/// The remote console's allow-list (P6-49): the automatic mode's read-only list, plus `help`, `exit`,
/// `remote stare` and `remote opreste`. Nothing that writes to the video card, no `reboot`, no other programs.
pub fn remote_allowed(line: &str) -> Result<(), &'static str> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["help"] | ["exit"] | ["remote", "stare"] | ["remote", "opreste"] => Ok(()),
        ["remote", ..] => Err("prin remote merge doar `remote stare` si `remote opreste`"),
        ["timp", "timp" | "exit", ..] => Err("timp masoara o singura comanda obisnuita"),
        ["timp", rest @ ..] if !rest.is_empty() => remote_allowed(&rest.join(" ")),
        _ => auto::allowed(line),
    }
}

/// Runs one command line. `false` = this shell must end (`exit` in a terminal window, `test-gata`).
fn dispatch(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some((&cmd, args)) = words.split_first() else { return true };
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
        "usb" => usb(args),
        "net" => net(),
        "rtl" => rtl(args),
        "hw" => match args {
            [] => hw(),
            ["lista"] => hw_lista(false),
            ["lista", "--detalii"] => hw_lista(true),
            _ => out!("folosire: hw sau hw lista [--detalii]\n"),
        },
        "raport" => raport::manual(),
        "trimite" => raport::send_to_hub(),
        "compara" => match args {
            [a, b] => out!("{}", compara::compare_files(a, b)),
            _ => out!("folosire: compara <fisier1> <fisier2>\n"),
        },
        "gpu" => gpu(args),
        "cpu" => cpu(args),
        "log" => show_log(args),
        "timp" => time_command(args),
        "uptime" => uptime(),
        "remote" => remote(args),
        "reboot" => reboot(args),
        "exit" => match crate::term::of_shell(crate::sched::current_id()) {
            Some(t) => {
                crate::term::close(t);
                return false;
            }
            None => out!("exit: shell-ul consolei nu se inchide (merge cat calculatorul)\n"),
        },
        #[cfg(feature = "qemu-test")]
        "test-gata" => return false,
        // Test builds: put a line in the log (scenario `raport`: a token must not reach the report).
        #[cfg(feature = "qemu-test")]
        "test-jurnal" => crate::info!("test-jurnal: {}", args.join(" ")),
        // Test builds: a command that never ends (scenario `auto-ceas`: the safety clock must restart the machine).
        #[cfg(feature = "test-auto")]
        "test-blocheaza" => {
            crate::info!("test-blocheaza: ma blochez intentionat");
            loop {
                core::hint::spin_loop();
            }
        }
        _ => out!("comanda necunoscuta: {} (scrie help)\n", cmd),
    }
    true
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
    out!("  gpu pregatire-i6 pregatirea pasului I6 (doar citire); gpu i6 --scrie = prima scriere (doar stick-ul de test)\n");
    out!("  gpu tur          turul de citire (doar citire, un rand `fapt` pe fapt); gpu lant = pasii ferestrei in lant (cere DA)\n");
    out!("  hw               raport hardware pe o pagina (pentru o singura poza)\n");
    out!("  hw lista [--detalii]  tot hardware-ul: procesor, memorie, PCI, Wi-Fi, USB, ACPI (doar citire)\n");
    out!("  raport           scrie pe stick /usb/raport/<data>-<nr>.txt: hw, lspci, net, jurnalele (fara chei)\n");
    out!("  trimite          rapoartele noi din /usb/raport -> hub (rezultate/os-automat/<data>/), doar cu /usb/github.token\n");
    out!("  log [cuvant] [n] ultimele n randuri din jurnal (implicit 30), doar cele cu cuvantul\n");
    out!("  log --avertismente [n]  doar avertismentele si erorile; log --sumar = cate randuri pe nivel si pe modul\n");
    out!("  timp <comanda>   cat a durat comanda (ms) si cum s-a schimbat memoria\n");
    out!("  uptime           de cat timp merge sistemul\n");
    out!("  compara <f1> <f2>  randurile care difera intre doua rapoarte sau jurnale\n");
    out!("  remote porneste [port]|opreste|stare  consola text prin retea (cheia din remote.cheie; implicit portul 2323)\n");
    out!("  cpu [porneste]   nucleele procesorului (porneste = porneste-le pe celelalte)\n");
    out!("  usb citire [mare|mica]  citirea de pe stick: 64 KiB pe comanda (varianta noua) sau 4 KiB (implicit)\n");
    out!("  reboot [acpi|cf9|8042|triplu]  reporneste (implicit: toate metodele, pe rand)\n");
    out!("  exit             inchide terminalul (doar in fereastra Terminal)\n");
}

fn remote(args: &[&str]) {
    use crate::net::remote;
    match args {
        ["porneste"] | ["porneste", _] => {
            let port = match args.get(1).map(|p| p.parse::<u16>()) {
                None => remote::PORT_DEFAULT,
                Some(Ok(p)) if p != 0 => p,
                Some(_) => {
                    out!("remote: port gresit (1-65535)\n");
                    return;
                }
            };
            match remote::start(port) {
                Ok(msg) | Err(msg) => out!("{}\n", msg),
            }
        }
        ["opreste"] => out!("{}\n", if remote::stop() { "remote: opresc serviciul" } else { "remote: serviciul nu merge" }),
        ["stare"] | [] => out!("{}\n", remote::status()),
        _ => out!("folosire: remote porneste [port] | remote opreste | remote stare\n"),
    }
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
        // P6-23: the key and the token never go on the screen (a terminal window may belong to the Claude app).
        if crate::proc::rights::canonical(path).is_some_and(|c| crate::proc::rights::is_secret(&c)) {
            out!("cat: {}: fisier secret (il citesc doar programele care il cer)\n", path);
            continue;
        }
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
        #[cfg(feature = "test-gpu-sim")]
        Some("identifica") if args.get(1) == Some(&"--sim") => {
            for l in crate::drivers::gpu::identify_sim() {
                out!("{}\n", l);
            }
        }
        #[cfg(not(feature = "test-gpu-sim"))]
        Some("identifica") if args.get(1) == Some(&"--sim") => {
            out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        }
        None | Some("identifica") => {
            for l in crate::drivers::gpu::identify() {
                out!("{}\n", l);
            }
        }
        Some(w @ ("gfw" | "vbios")) => {
            use crate::drivers::gpu::InitStep;
            let rest = &args[1..];
            let details = w == "vbios" && rest.contains(&"--detalii");
            let sim = rest.contains(&"--sim");
            if let Some(bad) = rest.iter().find(|a| **a != "--sim" && !(w == "vbios" && **a == "--detalii")) {
                out!("gpu {}: optiune necunoscuta: {} (folosire: gpu {} [--sim]{})\n", w, bad, w, if w == "vbios" { " [--detalii]" } else { "" });
                return;
            }
            let step = match (w, details) {
                ("gfw", _) => InitStep::Gfw,
                (_, false) => InitStep::Vbios,
                (_, true) => InitStep::VbiosDetails,
            };
            if sim {
                #[cfg(feature = "test-gpu-sim")]
                {
                    for l in crate::drivers::gpu::init_step_sim(step) {
                        out!("{}\n", l);
                    }
                }
                #[cfg(not(feature = "test-gpu-sim"))]
                {
                    let _ = step;
                    out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
                }
            } else {
                for l in crate::drivers::gpu::init_step(step) {
                    out!("{}\n", l);
                }
            }
        }
        Some("fwsec") => {
            // P6-35 (C-38): step I3b, read only.
            let rest = &args[1..];
            if let Some(bad) = rest.iter().find(|a| **a != "--sim") {
                out!("gpu fwsec: optiune necunoscuta: {} (folosire: gpu fwsec [--sim])\n", bad);
                return;
            }
            if rest.contains(&"--sim") {
                #[cfg(feature = "test-gpu-sim")]
                for l in crate::drivers::gpu::init_step_sim(crate::drivers::gpu::InitStep::Fwsec) {
                    out!("{}\n", l);
                }
                #[cfg(not(feature = "test-gpu-sim"))]
                out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
            } else {
                for l in crate::drivers::gpu::init_step(crate::drivers::gpu::InitStep::Fwsec) {
                    out!("{}\n", l);
                }
            }
        }
        Some("firmware") => {
            // P6-35 (C-38): step I4 — the GSP files on the stick; no access to the card at all.
            let rest = &args[1..];
            if let Some(bad) = rest.iter().find(|a| **a != "--sim") {
                out!("gpu firmware: optiune necunoscuta: {} (folosire: gpu firmware [--sim])\n", bad);
                return;
            }
            if rest.contains(&"--sim") {
                #[cfg(feature = "test-gpu-sim")]
                for l in crate::drivers::gpu::firmware_check_sim() {
                    out!("{}\n", l);
                }
                #[cfg(not(feature = "test-gpu-sim"))]
                out!("gpu: --sim exista doar in build-ul de test (fisierele de proba gpu-sim nu intra in sistemul pornit de pe stick)\n");
            } else {
                out!("gpu firmware: citesc fisierele de pe stick (cel mai mare ~64 MB, poate dura cateva secunde)...\n");
                for l in crate::drivers::gpu::firmware_check() {
                    out!("{}\n", l);
                }
            }
        }
        Some("pregatire-i6") => {
            // P6-40 (C-52): step 1 of the I6 window, read only (16 MiB read-only window), journal live on the stick.
            let rest = &args[1..];
            if let Some(bad) = rest.iter().find(|a| **a != "--sim") {
                out!("gpu pregatire-i6: optiune necunoscuta: {} (folosire: gpu pregatire-i6 [--sim])\n", bad);
                return;
            }
            if rest.contains(&"--sim") {
                #[cfg(feature = "test-gpu-sim")]
                for l in crate::drivers::gpu_i6::prepare_i6_sim() {
                    out!("{}\n", l);
                }
                #[cfg(not(feature = "test-gpu-sim"))]
                out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
            } else {
                for l in crate::drivers::gpu_i6::prepare_i6() {
                    out!("{}\n", l);
                }
            }
        }
        Some("i6") => i6(&args[1..]),
        Some("tur") => {
            // P6-42 (C-55): the read-only tour, one `fapt` line per fact; allowed in the automatic loop.
            let rest = &args[1..];
            if let Some(bad) = rest.iter().find(|a| **a != "--sim") {
                out!("gpu tur: optiune necunoscuta: {} (folosire: gpu tur [--sim])\n", bad);
                return;
            }
            if rest.contains(&"--sim") {
                #[cfg(feature = "test-gpu-sim")]
                for l in crate::drivers::gpu_tur::tour_sim() {
                    out!("{}\n", l);
                }
                #[cfg(not(feature = "test-gpu-sim"))]
                out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
            } else {
                for l in crate::drivers::gpu_tur::tour() {
                    out!("{}\n", l);
                }
            }
        }
        Some("lant") => lant(&args[1..]),
        Some("plan") => {
            for l in crate::drivers::gpu::PLAN {
                out!("{}\n", l);
            }
        }
        Some(other) => out!("gpu: necunoscut: {} (folosire: gpu identifica | gpu gfw | gpu vbios [--detalii] | gpu fwsec | gpu firmware | gpu pregatire-i6 | gpu i6 --scrie | gpu tur | gpu lant | gpu plan)\n", other),
    }
}

/// P6-42 (C-55): `gpu lant [--sim] [--eroare]` — the window steps in a chain. Asks for `DA` before anything; refused by
/// the automatic loop (shell/auto.rs). Writes only in the test-stick build; elsewhere the chain runs read only.
fn lant(rest: &[&str]) {
    if let Some(bad) = rest.iter().find(|a| !matches!(**a, "--sim" | "--eroare" | "--doar-citire")) {
        out!("gpu lant: optiune necunoscuta: {} (folosire: gpu lant)\n", bad);
        return;
    }
    let sim = rest.contains(&"--sim");
    if sim && !cfg!(feature = "test-gpu-sim") {
        out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        return;
    }
    for l in crate::drivers::gpu::LANT_WARNING {
        out!("{}\n", l);
    }
    if !sim && !cfg!(feature = "gpu-scrie") {
        out!("(acest sistem nu poate scrie in placa: lantul va rula doar citire, oprit inainte de primul pas care scrie)\n");
    }
    out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva opreste: ");
    let Some(answer) = read_line() else { return };
    if answer.trim() != "DA" {
        out!("gpu lant: oprit, nimic scris in placa\n");
        crate::info!("gpu: lant: oprit la confirmare (nimic scris)");
        return;
    }
    crate::info!("gpu: lant: confirmat cu DA{}", if sim { " (SIMULARE)" } else { "" });
    #[cfg(feature = "test-gpu-sim")]
    if sim {
        for l in crate::drivers::gpu_tur::chain_sim(rest.contains(&"--eroare"), !rest.contains(&"--doar-citire")) {
            out!("{}\n", l);
        }
        return;
    }
    for l in crate::drivers::gpu_tur::chain() {
        out!("{}\n", l);
    }
    // C-84 item 1: the chain's journal (/usb/raport/gpu-jurnal-NNN.txt) goes to the hub when there is a token.
    raport::send_to_hub();
}

/// P6-40 (C-52): `gpu i6 --scrie [--sim]` — the first writes to the card (step I6). Asks for `DA` before anything.
fn i6(rest: &[&str]) {
    if let Some(bad) = rest.iter().find(|a| !matches!(**a, "--scrie" | "--sim" | "--eroare")) {
        out!("gpu i6: optiune necunoscuta: {} (folosire: gpu i6 --scrie)\n", bad);
        return;
    }
    if !rest.contains(&"--scrie") {
        out!("gpu i6: scrie in placa video; ruleaza doar cu `gpu i6 --scrie`, in fereastra cu Adrian (vezi `gpu plan`)\n");
        return;
    }
    let sim = rest.contains(&"--sim");
    if !sim && !cfg!(feature = "gpu-scrie") {
        for l in crate::drivers::gpu_i6::i6_write() {
            out!("{}\n", l);
        }
        return;
    }
    if sim && !cfg!(feature = "test-gpu-sim") {
        out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        return;
    }
    for l in crate::drivers::gpu::I6_WARNING {
        out!("{}\n", l);
    }
    out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva opreste: ");
    let Some(answer) = read_line() else { return };
    if answer.trim() != "DA" {
        out!("gpu i6: oprit, nimic scris in placa\n");
        crate::info!("gpu: i6 --scrie: oprit la confirmare (nimic scris)");
        return;
    }
    crate::info!("gpu: i6 --scrie: confirmat cu DA{}", if sim { " (SIMULARE)" } else { "" });
    #[cfg(feature = "test-gpu-sim")]
    if sim {
        for l in crate::drivers::gpu_i6::i6_write_sim(rest.contains(&"--eroare")) {
            out!("{}\n", l);
        }
        return;
    }
    for l in crate::drivers::gpu_i6::i6_write() {
        out!("{}\n", l);
    }
}

fn lsusb() {
    let devices = crate::drivers::usb::devices();
    if devices.is_empty() {
        out!("(niciun dispozitiv USB)\n");
    }
    for d in devices {
        out!("  port {}  {:04x}:{:04x}  {}\n", d.location(), d.vendor, d.product, d.kind());
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
    if args == ["--sumar"] {
        out!("{}", log_summary(&crate::log::recent()));
        return;
    }
    let mut count = 30usize;
    let mut word: Option<String> = None;
    let mut warnings_only = false;
    for a in args {
        match a.parse::<usize>() {
            Ok(n) => count = n.clamp(1, 500),
            Err(_) if *a == "--avertismente" => warnings_only = true,
            Err(_) => word = Some(a.to_ascii_lowercase()),
        }
    }
    let lines: Vec<String> = crate::log::recent()
        .into_iter()
        .filter(|l| !warnings_only || matches!(log_fields(l), Some(("WARN" | "ERR", _))))
        .filter(|l| word.as_ref().is_none_or(|w| l.to_ascii_lowercase().contains(w.as_str())))
        .collect();
    if lines.is_empty() {
        out!("(niciun rand)\n");
    }
    for l in &lines[lines.len().saturating_sub(count)..] {
        out!("{}\n", l);
    }
}

/// Level and module of one log line (`[  12.345] INFO  net: ...` -> `("INFO", "net")`); `None` if it has another shape.
fn log_fields(line: &str) -> Option<(&str, &str)> {
    let rest = line.split_once("] ")?.1;
    let (level, rest) = rest.trim_start().split_once(' ')?;
    if !matches!(level, "DEBUG" | "INFO" | "WARN" | "ERR") {
        return None;
    }
    let module = rest.trim_start().split_once(':').map(|(m, _)| m).filter(|m| !m.is_empty() && !m.contains(' '));
    Some((level, module.unwrap_or("(altele)")))
}

/// `log --sumar`: how many lines per level and the modules that wrote the most (F9, P6-52).
fn log_summary(lines: &[String]) -> String {
    let mut levels = [("ERR", 0usize), ("WARN", 0), ("INFO", 0), ("DEBUG", 0)];
    let mut modules: Vec<(String, usize, usize)> = Vec::new(); // name, lines, warnings + errors
    let mut other = 0usize;
    for l in lines {
        let Some((level, module)) = log_fields(l) else {
            other += 1;
            continue;
        };
        if let Some(e) = levels.iter_mut().find(|e| e.0 == level) {
            e.1 += 1;
        }
        let bad = matches!(level, "WARN" | "ERR") as usize;
        match modules.iter_mut().find(|m| m.0 == module) {
            Some(m) => {
                m.1 += 1;
                m.2 += bad;
            }
            None => modules.push((String::from(module), 1, bad)),
        }
    }
    modules.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
    let mut s = format!("jurnal: {} randuri", lines.len());
    for (name, n) in levels {
        s += &format!(", {} {}", n, name);
    }
    if other > 0 {
        s += &format!(", {} fara nivel", other);
    }
    s += "\nmodule (cele cu avertismente/erori primele):\n";
    for (name, n, bad) in modules.iter().take(12) {
        s += &format!("  {:<12} {:>4} randuri, {} avertismente/erori\n", name, n, bad);
    }
    if modules.len() > 12 {
        s += &format!("  ... inca {} module\n", modules.len() - 12);
    }
    s
}

/// `timp <comanda>`: runs one command and says how long it took and how the memory changed (F9, P6-52).
fn time_command(args: &[&str]) {
    match args.first() {
        None => {
            out!("folosire: timp <comanda>   (de ex. timp run kronos --n 16 Buna ziua)\n");
            return;
        }
        Some(&c @ ("timp" | "exit")) => {
            out!("timp: `{}` nu se masoara\n", c);
            return;
        }
        _ => {}
    }
    use crate::mem::{frame, heap, map::PAGE_SIZE};
    let line = args.join(" ");
    let free_before = frame::free_frames() * PAGE_SIZE / 1024;
    let heap_before = heap::stats().0 / 1024;
    let start = crate::time::uptime_ms();
    dispatch(&line);
    let end = crate::time::uptime_ms();
    let free_after = frame::free_frames() * PAGE_SIZE / 1024;
    let heap_after = heap::stats().0 / 1024;
    match (start, end) {
        (Some(a), Some(b)) => out!("timp: `{}` a durat {} ms", line, b.saturating_sub(a)),
        _ => out!("timp: `{}` (ceasul nu merge inca)", line),
    }
    out!("; memorie libera {} -> {} KiB, heap {} -> {} KiB\n", free_before, free_after, heap_before, heap_after);
    crate::info!("timp: {} = {} ms", line, match (start, end) { (Some(a), Some(b)) => b.saturating_sub(a), _ => 0 });
}

/// One-page hardware report: processor, memory, PCI devices with the driver that handles them, USB, disks, network.
fn hw() {
    out!("== raport hardware ==\n");
    out!("procesor: {}\n", cpu_name());
    match crate::gfx::framebuffer::boot_info() {
        Some((phys, size, w, h, pitch, bpp)) => out!(
            "ecran: framebuffer la adresa fizica {:#x}, {} octeti ({} MiB), {}x{}, pitch {} B, {} bpp\n",
            phys, size, size.div_ceil(1 << 20), w, h, pitch, bpp
        ),
        None => out!("ecran: fara framebuffer de la Limine\n"),
    }
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

/// `hw lista`: the full hardware list from `crate::hw` (F4), by reads only.
fn hw_lista(detalii: bool) {
    for line in crate::hw::lista(detalii, &driver_of, &cpu_name()) {
        out!("{}\n", line);
    }
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

/// `cpu`: how many processors the MADT lists and how many run; `cpu porneste` starts the others (K6-02; since K6-02b threads run on all of them).
fn cpu(args: &[&str]) {
    let listed = crate::acpi::madt::get().cpu_count;
    match args.first().copied() {
        None => out!("procesor: {}; nuclee in MADT: {}; pornite: {}\n", cpu_name(), listed, crate::arch::x86_64::smp::online()),
        Some("porneste") => {
            out!("cpu: pornesc celelalte nuclee (detalii: log smp)...\n");
            let n = crate::arch::x86_64::smp::start_all();
            out!("cpu: {} nuclee pornite din {} (firele programelor ruleaza pe toate)\n", n, listed);
        }
        Some(other) => out!("cpu: necunoscut: {} (cpu, cpu porneste)\n", other),
    }
}

/// `usb citire [mare|mica]` (P6-29): the size of one stick read; without an argument, the current one.
fn usb(args: &[&str]) {
    match args {
        ["citire"] => out!(
            "usb: citire {} pe comanda (`usb citire mare` = 64 KiB, `usb citire mica` = 4 KiB)\n",
            if crate::drivers::usb::storage::big_reads() { "64 KiB" } else { "4 KiB" }
        ),
        ["citire", "mare"] => {
            crate::drivers::usb::storage::set_big_reads(true);
            out!("usb: citire 64 KiB pe comanda (varianta noua; `usb citire mica` revine)\n");
        }
        ["citire", "mica"] => {
            crate::drivers::usb::storage::set_big_reads(false);
            out!("usb: citire 4 KiB pe comanda (ca pana acum)\n");
        }
        _ => out!("usb: folosire: usb citire [mare|mica]\n"),
    }
}

/// Restarts the machine: every method in turn (ACPI, 0xCF9, 8042, triple fault), or only the one named.
fn reboot(args: &[&str]) {
    let arg = args.first().copied().unwrap_or("");
    let only = if arg.is_empty() {
        None
    } else {
        match crate::power::Method::parse(arg) {
            Some(m) => Some(m),
            None => {
                out!("reboot: metoda necunoscuta: {} (acpi, cf9, 8042, triplu)\n", arg);
                return;
            }
        }
    };
    out!("repornesc...\n");
    crate::power::reboot(only)
}
