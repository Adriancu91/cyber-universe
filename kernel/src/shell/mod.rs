//! The first shell: reads a line from the keyboard and runs a command. Everything it prints goes to
//! the screen and to the serial port (so tests and a serial terminal see it too).

use crate::drivers::keyboard::{Key, wait_key};
use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

mod analiza;
pub mod antrenare;
pub mod antrenare_mini;
pub mod auto;
pub mod ciclu;
pub mod compara;
pub mod date_bucati;
pub mod punct_control;
pub mod hubcmd;
pub mod live;
pub mod lucrare;
pub mod raport;
mod testtot;
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

/// P6-109: the exit code of the last program `run` started (0 before every live command; 127 = unknown command).
static LAST_EXIT: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);

/// Called by the `write` system call for console output: keeps a copy for the automatic mode's report.
pub fn program_output(text: &str) {
    if let Some(buf) = PROGRAM_CAPTURE.lock().as_mut()
        && buf.len() < 1024 * 1024
    {
        buf.push_str(text);
    }
}

/// C-99: a line printed at once by a long-running command (the GPU wait), on the shell that runs it.
pub(crate) fn live_line(text: &str) {
    emit(text);
    emit("\n");
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
        // P6-111 (C-159 step 2): a line typed through the live channel (`scrie` / `tasta`) never writes to the video card
        // and never starts another live session; board commands go only as `comanda`, through the gate and its keys.
        if !line.trim().is_empty() && crate::term::is_live(crate::sched::current_id()) {
            if let Err(why) = live::typed_line(&line) {
                crate::info!("live: rand tastat REFUZAT: {} ({})", line.trim(), why);
                out!("REFUZAT: {}\n", why);
                continue;
            }
            crate::info!("live: rand tastat: {}", line.trim());
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
        "antrenare" => match args {
            // P6-121 (C-161 point 3): Kronos's golden step checked in the OS (Cyber-GPU's CPU oracle; the board untouched).
            ["proba"] => antrenare::proba(None, None).iter().for_each(|r| out!("{}\n", r)),
            ["proba", dir] => antrenare::proba(Some(dir), None).iter().for_each(|r| out!("{}\n", r)),
            // P6-151 (C-161 point 5): the sgd weights after 10 steps saved as a checkpoint (safe rewrite), read back.
            ["proba", "--salveaza", path] => antrenare::proba(None, Some(path)).iter().for_each(|r| out!("{}\n", r)),
            ["proba", dir, "--salveaza", path] => antrenare::proba(Some(dir), Some(path)).iter().for_each(|r| out!("{}\n", r)),
            ["control", path] => match punct_control::load(path) {
                Ok(c) => {
                    let elems: usize = c.tensors.iter().map(|(_, t)| t.len()).sum();
                    out!("antrenare control: {}: CYBKCK01, SHA-256 verificat; {} tensori, {} valori F32\n", path, c.tensors.len(), elems);
                    for (k, v) in &c.meta {
                        out!("antrenare control:   {} = {}\n", k, v);
                    }
                }
                Err(e) => out!("antrenare control: REFUZAT: {}\n", e),
            },
            // P6-150 (C-161 point 5): Kronos's training data (chunks + index.json), checked as Kronos does at start-up.
            ["date", rest @ ..] => date_bucati::command(rest).iter().for_each(|r| out!("{}\n", r)),
            // P6-157 (C-161 points 3 + 5): a whole small training run on Kronos's data, on the processor, with checkpoint.
            ["mini", rest @ ..] => antrenare_mini::command(rest).iter().for_each(|r| out!("{}\n", r)),
            _ => out!("folosire: antrenare proba [dosar] [--salveaza <cale>] (pasul de aur al lui Kronos, pe procesor; implicit {} sau {}) | antrenare control <cale> | antrenare date [dosar] [--tokenizer <cale>] [--felie <inceput> <numar>] (datele lui Kronos pe bucati) | antrenare mini [dosar] [--pasi N] [--lot B T] [--samanta s] [--lr x] [--salveaza <cale>] [--din <cale>] (o antrenare mica pe datele lui Kronos, pe procesor)\n", antrenare::DIRS[0], antrenare::DIRS[1]),
        },
        "discuri" => {
            // P6-100 (C-148 point 5): the computer's own disks as seen at start-up (reads only).
            out!("discurile vazute de OS (doar citire, la pornire):\n");
            for row in crate::block::inventory() {
                out!("{}\n", row);
            }
        }
        "disc" => match args {
            // P6-146 (C-158 point 7): chunked data and safe rewrites on the own disk marked cyber.rw.
            ["bucati", base] => {
                let c = crate::fs::bucati::chunks(base);
                if c.is_empty() {
                    out!("disc bucati: nu exista {}.000\n", base);
                }
                for (p, n) in &c {
                    out!("  {}  {} octeti\n", p, n);
                }
                if !c.is_empty() {
                    out!("disc bucati: {} bucati, {} octeti in total\n", c.len(), c.iter().map(|(_, n)| n).sum::<u64>());
                }
            }
            ["goleste"] => out!("{}\n", crate::fs::flush_all()),
            ["proba"] => crate::fs::bucati::self_check().0.iter().for_each(|r| out!("{}\n", r)),
            _ => out!("folosire: disc bucati <cale> | disc goleste | disc proba   (proba scrie doar pe /disk cu cyber.rw)\n"),
        },
        "lsusb" => lsusb(),
        "usb" => usb(args),
        "net" => net(),
        "rtl" => rtl(args),
        "hw" => match args {
            [] => hw(),
            ["lista"] => hw_lista(false),
            ["lista", "--detalii"] => hw_lista(true),
            ["uhl"] => analiza::run(false),
            _ => out!("folosire: hw sau hw lista [--detalii] sau hw uhl (= analiza)\n"),
        },
        "raport" => raport::manual(),
        "analiza" => match args {
            [] => analiza::run(false), // C-117 (P6-86, P6-87)
            #[cfg(feature = "test-gpu-sim")]
            ["--sim"] => analiza::run(true),
            _ => out!("folosire: analiza   (fara optiuni; inventarul ca UHL pe stick, doar citire)\n"),
        },
        "trimite" => raport::send_to_hub(),
        "ora" => ora(args),
        "test" => match args {
            ["tot"] => testtot::run(),
            _ => out!("folosire: test tot   (autotestul OS-ului pe calculatorul acesta, raport PASS/FAIL pe stick)\n"),
        },
        "hub" if args.first() == Some(&"ruleaza") => hubcmd::command(&args[1..]), // C-87 item 2 (P6-64)
        "hub" if args.first() == Some(&"asculta") => hubcmd::listen_command(&args[1..]), // C-104 (P6-73)
        "hub" if args.first() == Some(&"opreste") && args.len() == 1 => hubcmd::stop_command(),
        "hub" => {
            // C-84 item 2 (P6-61): Adrian's commands, a project's state, the decisions — read from the Cyber Hub.
            let mut all = alloc::vec!["gh-ia", "--hub"];
            all.extend_from_slice(args);
            run_program(&all)
        }
        "compara" => match args {
            [a, b] => out!("{}", compara::compare_files(a, b)),
            _ => out!("folosire: compara <fisier1> <fisier2>\n"),
        },
        "gpu" => gpu(args),
        "wifi" => match args {
            // P6-88 (C-118 step W0): the Intel Wi-Fi adapter, read only.
            [] | ["identifica"] => {
                for l in crate::drivers::wifi::identify(false) {
                    out!("{}\n", l);
                }
            }
            #[cfg(feature = "qemu-test")]
            ["identifica", "--sim"] => {
                for l in crate::drivers::wifi::identify(true) {
                    out!("{}\n", l);
                }
            }
            _ => out!("folosire: wifi [identifica]   (placa Wi-Fi Intel, doar citire: pasul W0 din C-118)\n"),
        },
        "cpu" => cpu(args),
        "log" => show_log(args),
        "timp" => time_command(args),
        "uptime" => uptime(),
        "remote" => remote(args),
        "live" => live::command(args), // C-159 step 1 (P6-108, P6-109)
        "telefon" => out!("{}", live::phone()), // C-158 point 9 (P6-144): the state on the phone
        "lucrare" => lucrare::command(args), // C-158 point 8 (P6-140): long jobs
        "reboot" => reboot(args),
        "porneste" => porneste(args),
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
        _ => {
            LAST_EXIT.store(127, core::sync::atomic::Ordering::Relaxed);
            out!("comanda necunoscuta: {} (scrie help)\n", cmd)
        }
    }
    true
}

/// P6-118 (C-161 point 1): the answer to a `DA` question. During a live session nobody is at the keyboard and keys sent
/// through the channel must never answer it: the answer comes at once and is never `DA` (the command stops as without DA).
fn read_da() -> Option<String> {
    if live::in_session() {
        out!("\ncere DA de la tastatura: doar cu Adrian de fata (in modul viu intrebarea primeste raspuns pe loc, fara DA)\n");
        crate::info!("live: o intrebare DA a primit raspuns pe loc, fara DA");
        return Some(String::new());
    }
    read_line()
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
    out!("  discuri          discurile calculatorului (controler, model, marime, partitii, cyber.rw); doar citire\n");
    out!("  disc             bucati <cale> | goleste | proba: date in bucati de cel mult 4 GiB, golirea discurilor\n");
    out!("  lsusb            dispozitivele USB conectate\n");
    out!("  net              placa de retea (adresa MAC, pachete)\n");
    out!("  rtl porneste [variante]  reporneste placa Realtek cu variantele date (fara: ca la pornire; `rtl` le arata)\n");
    out!("  gpu [identifica|plan]  placile video (nivel 0: doar citire; NVIDIA: registrele de identitate)\n");
    out!("  gpu pregatire-i6 pregatirea pasului I6 (doar citire); gpu i6 --scrie = prima scriere (doar stick-ul de test)\n");
    out!("  wifi [identifica] placa Wi-Fi Intel (AX201): configuratia PCI si registrele de identitate, doar citire (W0)\n");
    out!("  gpu tur          turul de citire (doar citire, un rand `fapt` pe fapt); gpu lant = pasii ferestrei in lant (cere DA)\n");
    out!("  gpu tur --trezire  laptopul (GA106): Memory Space pornit pentru tur, repus la final (cere DA; doar stick-ul de test)\n");
    out!("  gpu recupereaza  dupa un lant oprit: Booter Unload coboara WPR2 fara repornire (cere DA; doar stick-ul de test)\n");
    out!("  hw               raport hardware pe o pagina (pentru o singura poza)\n");
    out!("  hw lista [--detalii]  tot hardware-ul: procesor, memorie, PCI, Wi-Fi, USB, ACPI (doar citire)\n");
    out!("  analiza          inventarul hardware ca UHL (uhl-inventory/0.1) pe stick: raport/uhl/<calculator>/<sesiune>/ (doar citire)\n");
    out!("  raport           scrie pe stick /usb/raport/<data>-<nr>.txt: hw, lspci, net, jurnalele (fara chei)\n");
    out!("  ora [server [port]]  ora din retea (SNTP, implicit pool.ntp.org) fata de ceasul calculatorului (nu-l schimba)\n");
    out!("  test tot         autotestul OS-ului (procesor, ceas, stick, retea, placa video): PASS/FAIL + raport pe stick\n");
    out!("  hub [comenzi|stare|decizii] [proiect]  citeste din Cyber Hub comenzile lui Adrian, starea, deciziile\n");
    out!("  hub ruleaza [--arata]  ruleaza comenzile lui Adrian din hub (os-comenzi.md), doar din lista alba; raport -> hub\n");
    out!("  hub asculta [--la N]   la fiecare N minute (implicit 2) ia si ruleaza comenzile noi din hub; Esc opreste\n");
    out!("  hub opreste      opreste bucla `hub asculta` pornita la pornire (/usb/hub/asculta)\n");
    out!("  live <minute>    canalul viu (C-159): comenzi semnate din hub, live/<sesiune>/ pe ramura os/live; cere /usb/hub/live.cheie\n");
    out!("  lucrare <nume>   lucrare lunga din /usb/lucrari/<nume>.txt (ora, rapoarte, oprire, reluare); lucrare stare | opreste <nume>\n");
    out!("  telefon          starea pe telefon (C-158 p.9): rezumat + ecranul urcate in hub, live/telefon/ (ramura os/live)\n");
    out!("  trimite          rapoartele noi din /usb/raport -> hub (rezultate/os-automat/<data>/), doar cu /usb/github.token\n");
    out!("  log [cuvant] [n] ultimele n randuri din jurnal (implicit 30), doar cele cu cuvantul\n");
    out!("  log --avertismente [n]  doar avertismentele si erorile; log --sumar = cate randuri pe nivel si pe modul\n");
    out!("  timp <comanda>   cat a durat comanda (ms) si cum s-a schimbat memoria\n");
    out!("  uptime           de cat timp merge sistemul\n");
    out!("  compara <f1> <f2>  randurile care difera intre doua rapoarte sau jurnale\n");
    out!("  remote porneste [port]|opreste|stare  consola text prin retea (cheia din remote.cheie; implicit portul 2323)\n");
    out!("  cpu [porneste]   nucleele procesorului (porneste = porneste-le pe celelalte)\n");
    out!("  usb citire [mare|mica]  citirea de pe stick: 64 KiB pe comanda (varianta noua) sau 4 KiB (implicit)\n");
    out!("  usb timp                cat au durat scrierile si sincronizarile pe stick de la pornire\n");
    out!("  porneste [windows|os [--o-data|--fara-garda]]  ce porneste stick-ul data viitoare (fara: arata)\n");
    out!("  reboot [acpi|cf9|8042|triplu]  reporneste (implicit: toate metodele, pe rand)\n");
    out!("  exit             inchide terminalul (doar in fereastra Terminal)\n");
}

/// P6-102 (C-152 point 2): the boot switch on the stick (`kernel/src/pornire.rs`).
fn porneste(args: &[&str]) {
    use crate::pornire::{choose, status, Choice};
    let choice = match args {
        [] => {
            out!("{}\n", status("/usb"));
            return;
        }
        ["windows"] => Choice::Windows,
        ["os"] => Choice::OsGuarded,
        ["os", "--o-data"] => Choice::OsOnce,
        ["os", "--fara-garda"] => Choice::OsUnguarded,
        _ => {
            out!("folosire: porneste [windows | os [--o-data | --fara-garda]]\n");
            return;
        }
    };
    match choose("/usb", choice) {
        Ok(msg) => out!("{}\n", msg),
        Err(e) => out!("porneste: {}\n", e),
    }
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
    // P6-62 step 3: `kronos --gpu` runs the whole window chain (it writes to the board): the same `DA` as `gpu lant`.
    let gpu = path == "/bin/kronos" && args.iter().any(|a| matches!(*a, "--gpu" | "--gpu-sim"));
    if gpu {
        for l in crate::drivers::gpu::LANT_WARNING {
            out!("{}\n", l);
        }
        out!("kronos --gpu: lantul de mai sus ruleaza o data, apoi textul tau pe placa (fiecare simbol comparat cu procesorul);\n");
        out!("la orice eroare kronos continua pe procesor.\n");
        out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva = doar procesorul: ");
        let Some(answer) = read_da() else { return };
        if answer.trim() == "DA" {
            crate::info!("gpu: kronos --gpu: confirmat cu DA");
            crate::drivers::gpu_tur::KRONOS_ARMED.store(true, core::sync::atomic::Ordering::SeqCst);
        } else {
            out!("kronos --gpu: fara DA, placa nu e atinsa; kronos merge pe procesor\n");
        }
    }
    let spawned = crate::proc::spawn_path(&path, args);
    if gpu {
        // Whatever happens, the permission never outlives this one program.
        if let Ok(id) = &spawned {
            let code = crate::proc::wait(*id);
            LAST_EXIT.store(code, core::sync::atomic::Ordering::Relaxed);
            crate::drivers::gpu_tur::KRONOS_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
            if code != 0 {
                out!("({} a iesit cu codul {})\n", name, code);
            }
            return;
        }
        crate::drivers::gpu_tur::KRONOS_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    }
    match spawned {
        Ok(id) => {
            let code = crate::proc::wait(id);
            LAST_EXIT.store(code, core::sync::atomic::Ordering::Relaxed);
            if code != 0 {
                out!("({} a iesit cu codul {})\n", name, code);
            }
        }
        Err(e) => {
            LAST_EXIT.store(126, core::sync::atomic::Ordering::Relaxed);
            out!("run: {}: {}\n", path, e.message())
        }
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
            if let Some(bad) = rest.iter().find(|a| !matches!(**a, "--sim" | "--trezire")) {
                out!("gpu tur: optiune necunoscuta: {} (folosire: gpu tur [--trezire])\n", bad);
                return;
            }
            if rest.contains(&"--trezire") {
                return tur_trezire(rest.contains(&"--sim"));
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
        Some("variante") => {
            // P6-78 (C-108): the numbered I8 variants; no card access.
            for l in crate::drivers::gpu_tur::variants_list() {
                out!("{}\n", l);
            }
        }
        Some("recupereaza") => recupereaza(&args[1..]),
        Some("dieta-sim") => {
            // P6-107 (C-154): test build only: the driver's diet for the next `kronos --gpu-sim` (in the stick's system
            // only the cycle turns it on, around its ordinary test).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::DIET_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu dieta-sim: pornita (SIMULARE: pushbuffer prin umbra, inregistrari nezerorizate)\n");
                }
                Some("2") if args.len() == 2 => {
                    // P6-119 (C-157 point 3): the second diet (on top of the first one, as in the cycle).
                    crate::drivers::gpu_tur::DIET_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu dieta-sim: dieta 2 pornita (SIMULARE: doua inregistrari, asteptarea planificata, amprenta dupa ceas, argmax pe biti)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::DIET_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::DIET2_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu dieta-sim: oprita\n");
                }
                _ => out!("folosire: gpu dieta-sim on|2|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu dieta-sim: exista doar in build-ul de test (pe stick dieta o porneste doar ciclul)\n");
        }
        Some("argmax-sim") => {
            // P6-123 (C-155 point 3): test build only: the argmax program after the copy, checked, for the next
            // `kronos --gpu-sim` (in the stick's system only the cycle turns it on, with `test-placa … --argmax-placa`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu argmax-sim: pornita (SIMULARE: copia logit-urilor + programul argmax in acelasi simbol, verificat)\n");
                }
                // P6-126 (C-155 point 4): the argmax without the copy, the token chosen on the board.
                Some("fara-copie") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu argmax-sim: fara copie (SIMULARE: doar programul argmax, simbolul ales pe placa, verificat dupa placa)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ARGMAX_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::ARGMAX_TOP_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu argmax-sim: oprita\n");
                }
                _ => out!("folosire: gpu argmax-sim on|fara-copie|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu argmax-sim: exista doar in build-ul de test (pe stick argmax-ul pe placa il porneste doar ciclul, test-placa --argmax-placa)\n");
        }
        Some("colsum-sim") => {
            // P6-134 (C-161 point 4): test build only: the `colsum` probe (5 sets) after the next `kronos --gpu-sim`'s
            // texts (in the stick's system only the cycle turns it on, with `test-placa … --colsum`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu colsum-sim: pornita (SIMULARE: dupa texte, colsum de 5 ori, judecat pe procesor)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::COLSUM_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu colsum-sim: oprita\n");
                }
                _ => out!("folosire: gpu colsum-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu colsum-sim: exista doar in build-ul de test (pe stick proba colsum o porneste doar ciclul, test-placa --colsum)\n");
        }
        Some("mm-tc-sim") => {
            // P6-137 (C-161 point 4): test build only: the `mm_tc` probe (5 small sets; qkv-513 uploaded and read back,
            // its launches skipped in SIMULARE) after the next `kronos --gpu-sim`'s texts (in the stick's system only the
            // cycle turns it on, `test-placa … --mm-tc`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu mm-tc-sim: pornita (SIMULARE: dupa texte, mm_tc pe 5 seturi, judecat pe procesor; qkv-513 doar urcat si recitit)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::MM_TC_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu mm-tc-sim: oprita\n");
                }
                _ => out!("folosire: gpu mm-tc-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu mm-tc-sim: exista doar in build-ul de test (pe stick proba mm_tc o porneste doar ciclul, test-placa --mm-tc)\n");
        }
        Some("lot-sim") => {
            // P6-145 (C-157 point 7): test build only: after the texts of the next `kronos --gpu-sim` with the argmax without
            // the copy, the same generation in batches on one doorbell (in the stick's system only the cycle turns it on,
            // `test-placa … --argmax-fara-copie --lot`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::BATCH_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lot-sim: pornit (SIMULARE: dupa texte, aceeasi generare pe loturi: embed_dev, mai multe pozitii pe o sonerie)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::BATCH_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lot-sim: oprit\n");
                }
                _ => out!("folosire: gpu lot-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu lot-sim: exista doar in build-ul de test (pe stick lotul il porneste doar ciclul, test-placa --argmax-fara-copie --lot)\n");
        }
        Some("atentie-placi-sim") => {
            // P6-141 (C-161 point 4): test build only: the tiled attention probe (5 small sets; atentie-513-b4 uploaded
            // and read back, its launches skipped in SIMULARE) after the next `kronos --gpu-sim`'s texts (in the stick's
            // system only the cycle turns it on, `test-placa … --atentie-placi`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu atentie-placi-sim: pornita (SIMULARE: dupa texte, atentia pe placi pe 5 seturi, judecata pe procesor; atentie-513-b4 doar urcat si recitit)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ATTN_TILE_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu atentie-placi-sim: oprita\n");
                }
                _ => out!("folosire: gpu atentie-placi-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu atentie-placi-sim: exista doar in build-ul de test (pe stick proba o porneste doar ciclul, test-placa --atentie-placi)\n");
        }
        Some("atentie2-sim") => {
            // P6-149 (cyber-gpu note C-157_P2m): test build only: with `gpu atentie-placi-sim on`, the new programs
            // (`attn_*_2`) beside the old ones on the same buffers (in the stick's system only the cycle turns it on,
            // `test-placa … --atentie-placi --atentie2`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ATTN_TILE2_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu atentie2-sim: pornita (SIMULARE: in proba atentiei pe placi, dupa lantul vechi, lantul nou attn_*_2 pe acelasi buffer, comparat cuvant cu cuvant)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::ATTN_TILE2_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu atentie2-sim: oprita\n");
                }
                _ => out!("folosire: gpu atentie2-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu atentie2-sim: exista doar in build-ul de test (pe stick o porneste doar ciclul, test-placa --atentie-placi --atentie2)\n");
        }
        Some("pas-aur-sim") => {
            // P6-136 (C-161 point 4): test build only: the small golden step (10 steps of sgd and of adamw_513) after
            // the next `kronos --gpu-sim`'s texts (in the stick's system only the cycle turns it on, `test-placa … --pas-aur`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu pas-aur-sim: pornit (SIMULARE: dupa texte, pasul de aur mic de 10 ori pe sgd si pe adamw_513)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::GOLDEN_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu pas-aur-sim: oprit\n");
                }
                _ => out!("folosire: gpu pas-aur-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu pas-aur-sim: exista doar in build-ul de test (pe stick pasul de aur il porneste doar ciclul, test-placa --pas-aur)\n");
        }
        Some("p8-sim") => {
            // P6-117 (C-161 point 2): test build only: P8 (`mma16_f16`, 6 sets) after the next `kronos --gpu-sim`'s texts
            // (in the stick's system only the cycle turns it on, with `test-placa … --p8`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::P8_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu p8-sim: pornita (SIMULARE: dupa texte, mma16_f16 de 6 ori, judecat pe procesor)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::P8_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu p8-sim: oprita\n");
                }
                _ => out!("folosire: gpu p8-sim on|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu p8-sim: exista doar in build-ul de test (pe stick P8 il porneste doar ciclul, test-placa --p8)\n");
        }
        Some("lipit-sim") => {
            // P6-110 (C-155 point 1): test build only: P3's comparison (plain list, then glued) for the next
            // `kronos --gpu-sim` (in the stick's system only the cycle turns it on, with `test-placa … --lipit`).
            #[cfg(feature = "test-gpu-sim")]
            match args.get(1).copied() {
                Some("on") if args.len() == 2 => {
                    crate::drivers::gpu_tur::FUSED_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: pornita (SIMULARE: textul de doua ori, lista simpla apoi cea lipita)\n");
                }
                Some("singur") if args.len() == 2 => {
                    // P6-112: the glued list alone, as the cycle's default.
                    crate::drivers::gpu_tur::FUSED_ONLY_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: singura (SIMULARE: doar lista lipita, ca implicit in ciclu)\n");
                }
                Some("ln") if args.len() == 2 => {
                    // P6-114 (C-155 point 2): the glued list with the old LayerNorm, then with the new one, compared.
                    crate::drivers::gpu_tur::LN_NEW_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: ln nou (SIMULARE: textul de doua ori, lista lipita cu LayerNorm vechi apoi cu layernorm256)\n");
                }
                Some("ln-singur") if args.len() == 2 => {
                    // P6-120: the new LayerNorm alone, as the cycle's default from os-336 on.
                    crate::drivers::gpu_tur::LN_ONLY_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: ln singur (SIMULARE: doar lista lipita cu layernorm256, ca implicit in ciclu)\n");
                }
                Some("sferturi") if args.len() == 2 => {
                    // P6-124 (C-155 point 4): the glued list, then the glued list on quarters (`Fused32`), compared.
                    crate::drivers::gpu_tur::QUARTERS_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: sferturi (SIMULARE: textul de doua ori, lista lipita apoi lista lipita pe sferturi, q8_fused32)\n");
                }
                Some("ln-32") if args.len() == 2 => {
                    // P6-128 (C-157 point 2): `lipita-ln`, then `lipita-ln-32` (every product on quarters), compared.
                    crate::drivers::gpu_tur::LN32_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: ln-32 (SIMULARE: textul de doua ori, lipita-ln apoi lipita-ln-32, toate inmultirile pe sferturi)\n");
                }
                Some("ln-32-singur") if args.len() == 2 => {
                    // P6-135 (note C-157_P2i): `lipita-ln-32` alone, as the cycle's default from this Release on.
                    crate::drivers::gpu_tur::LN32_ONLY_IN_CYCLE.store(true, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: ln-32 singur (SIMULARE: doar lipita-ln-32, toate inmultirile pe sferturi, ca implicit in ciclu)\n");
                }
                Some("off") if args.len() == 2 => {
                    crate::drivers::gpu_tur::LN32_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::LN32_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::FUSED_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::LN_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::FUSED_ONLY_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::LN_NEW_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    crate::drivers::gpu_tur::QUARTERS_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
                    out!("gpu lipit-sim: oprita\n");
                }
                _ => out!("folosire: gpu lipit-sim on|singur|ln|ln-singur|sferturi|ln-32|ln-32-singur|off\n"),
            }
            #[cfg(not(feature = "test-gpu-sim"))]
            out!("gpu lipit-sim: exista doar in build-ul de test (pe stick lista lipita o porneste doar ciclul, test-placa --lipit)\n");
        }
        Some("stare") => {
            // P6-105 (C-152 point 5): the last dashboard read at the end of a chain; the board is not touched.
            if let Some(bad) = args[1..].first() {
                out!("gpu stare: optiune necunoscuta: {} (folosire: gpu stare)\n", bad);
                return;
            }
            for l in crate::drivers::gpu_tur::dashboard_status() {
                out!("{}\n", l);
            }
        }
        Some("plan") => {
            for l in crate::drivers::gpu::PLAN {
                out!("{}\n", l);
            }
        }
        Some(other) => out!("gpu: necunoscut: {} (folosire: gpu identifica | gpu gfw | gpu vbios [--detalii] | gpu fwsec | gpu firmware | gpu pregatire-i6 | gpu i6 --scrie | gpu tur [--trezire] | gpu lant [--varianta N] [--i9 c125|c133|c135] | gpu variante | gpu recupereaza | gpu stare | gpu plan)\n", other),
    }
}

/// P6-42 (C-55): `gpu lant [--sim] [--eroare]` — the window steps in a chain. Asks for `DA` before anything; refused by
/// the automatic loop (shell/auto.rs). Writes only in the test-stick build; elsewhere the chain runs read only.
fn lant(rest: &[&str]) {
    // P6-78 (C-108): `--varianta N` picks one of Cyber-GPU's numbered I8 variants (`gpu variante`); checked here,
    // before the warning and the `DA`, so a wrong number writes nothing.
    let mut variant: Option<u8> = None;
    // P6-91 (C-125): `--i9 c125` = Cyber-GPU's new I9 try (V2 + V3), off by default; checked here, before the `DA`.
    let mut i9: Option<crate::drivers::gpu_golden::I9Choice> = None;
    let mut opts: Vec<&str> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if *a == "--varianta" {
            let n = it.next().and_then(|v| v.parse::<u8>().ok());
            match n.filter(|n| nvidia_ga104::gsp::variants::get(*n).is_some()) {
                Some(n) if variant.is_none() => variant = Some(n),
                _ => {
                    out!(
                        "gpu lant: --varianta cere un numar din `gpu variante` (1-{}); nimic scris in placa\n",
                        nvidia_ga104::gsp::variants::list().len()
                    );
                    return;
                }
            }
        } else if *a == "--i9" {
            match it.next().and_then(|v| crate::drivers::gpu_golden::i9_option(v)) {
                Some(o) if i9.is_none() => i9 = Some(o),
                _ => {
                    out!("gpu lant: --i9 cere c125 (varianta C-125), c133 (varianta C-133), c135 (varianta C-135) sau os302 (implicita); nimic scris in placa\n");
                    return;
                }
            }
        } else {
            opts.push(a);
        }
    }
    let i9 = i9.unwrap_or_default();
    let rest = &opts[..];
    if let Some(bad) = rest.iter().find(|a| !matches!(**a, "--sim" | "--eroare" | "--doar-citire" | "--date-vechi" | "--flr-proba" | "--ciclu-viteza" | "--ciclu-viteza-refuz" | "--tablou" | "--tablou-refuz" | "--tablou-viteza")) {
        out!("gpu lant: optiune necunoscuta: {} (folosire: gpu lant [--varianta N] [--i9 c125|c133|c135] [--flr-proba] [--tablou])\n", bad);
        return;
    }
    let sim = rest.contains(&"--sim");
    if sim && !cfg!(feature = "test-gpu-sim") {
        out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        return;
    }
    // P6-94 (C-139): `--date-vechi` = the GPU model serves stale data until an invalidation (the os-305 hypothesis), so
    // the I14 ladder and its journal rows can be seen in SIMULATION. Only with `--sim`.
    // P6-106: `--ciclu-viteza[-refuz]` = the chain as the cycle runs it (the boost asked), only in SIMULATION.
    if rest.iter().any(|a| a.starts_with("--ciclu-viteza")) && !sim {
        out!("gpu lant: --ciclu-viteza exista doar cu --sim (viteza maxima o cere doar ciclul); nimic scris in placa\n");
        return;
    }
    // P6-142: `--tablou-viteza` = the dashboard before the max-speed request (the cycle's chain), only in SIMULATION here.
    if rest.contains(&"--tablou-viteza") && !sim {
        out!("gpu lant: --tablou-viteza exista doar cu --sim (pe placa il cere ciclul: test-placa --tablou-viteza); nimic scris in placa\n");
        return;
    }
    if rest.contains(&"--date-vechi") && !sim {
        out!("gpu lant: --date-vechi exista doar cu --sim (ipoteza C-139 in modelul gpu-sim); nimic scris in placa\n");
        return;
    }
    for l in crate::drivers::gpu::LANT_WARNING {
        out!("{}\n", l);
    }
    out!("{}\n", crate::drivers::gpu_tur::variant_line(variant));
    out!("{}\n", crate::drivers::gpu_golden::i9_line(i9));
    if !sim && !cfg!(feature = "gpu-scrie") {
        out!("(acest sistem nu poate scrie in placa: lantul va rula doar citire, oprit inainte de primul pas care scrie)\n");
    }
    out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva opreste: ");
    let Some(answer) = read_da() else { return };
    if answer.trim() != "DA" {
        out!("gpu lant: oprit, nimic scris in placa\n");
        crate::info!("gpu: lant: oprit la confirmare (nimic scris)");
        return;
    }
    crate::info!(
        "gpu: lant: confirmat cu DA{}; {}; {}",
        if sim { " (SIMULARE)" } else { "" },
        crate::drivers::gpu_tur::variant_line(variant),
        crate::drivers::gpu_golden::i9_line(i9)
    );
    #[cfg(feature = "test-gpu-sim")]
    if sim {
        let stale = rest.contains(&"--date-vechi");
        crate::drivers::gpu_i10::SIM_STALE_READS.store(stale, core::sync::atomic::Ordering::Relaxed);
        if stale {
            out!("SIMULARE: modelul gpu-sim serveste date vechi pana la o invalidare (ipoteza C-139, linii de 128 B)\n");
        }
        let flr_trial = rest.contains(&"--flr-proba");
        let cycle = rest.iter().any(|a| a.starts_with("--ciclu-viteza"));
        crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(cycle, core::sync::atomic::Ordering::SeqCst);
        *crate::drivers::gpu_tur::SIM_BOOST_RULE.lock() =
            rest.contains(&"--ciclu-viteza-refuz").then_some(gpu_sim::fake_rm::BoostRule::Refuse(0x56));
        // P6-105: the dashboard at the end of the chain; `--tablou-refuz` = the RM model refuses the start.
        crate::drivers::gpu_tur::RUSD_ARMED
            .store(rest.iter().any(|a| a.starts_with("--tablou")), core::sync::atomic::Ordering::SeqCst);
        *crate::drivers::gpu_tur::SIM_RUSD_RULE.lock() =
            rest.contains(&"--tablou-refuz").then_some(gpu_sim::fake_rm::RusdRule::RefuseInit(0x56));
        crate::drivers::gpu_tur::RUSD_EARLY.store(rest.contains(&"--tablou-viteza"), core::sync::atomic::Ordering::SeqCst);
        for l in crate::drivers::gpu_tur::chain_sim(rest.contains(&"--eroare"), !rest.contains(&"--doar-citire"), variant, i9, flr_trial) {
            out!("{}\n", l);
        }
        crate::drivers::gpu_i10::SIM_STALE_READS.store(false, core::sync::atomic::Ordering::Relaxed);
        crate::drivers::gpu_tur::BOOST_IN_CYCLE.store(false, core::sync::atomic::Ordering::SeqCst);
        crate::drivers::gpu_tur::RUSD_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
        crate::drivers::gpu_tur::RUSD_EARLY.store(false, core::sync::atomic::Ordering::SeqCst);
        return;
    }
    // P6-105: `--tablou` = the dashboard at the end of this chain only (not the FLR trial's second chain).
    crate::drivers::gpu_tur::RUSD_ARMED.store(rest.contains(&"--tablou"), core::sync::atomic::Ordering::SeqCst);
    let first = crate::drivers::gpu_tur::chain(variant, i9);
    crate::drivers::gpu_tur::RUSD_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    for l in first {
        out!("{}\n", l);
    }
    // P6-101 (C-152 point 1): `--flr-proba` = the chain a second time in this power-on (it starts with the FLR).
    if rest.contains(&"--flr-proba") {
        out!("-- proba FLR: lantul a doua oara, cu resetul din interior la inceput\n");
        crate::drivers::gpu_tur::FLR_ARMED.store(true, core::sync::atomic::Ordering::SeqCst);
        for l in crate::drivers::gpu_tur::chain(variant, i9) {
            out!("{}\n", l);
        }
        crate::drivers::gpu_tur::FLR_ARMED.store(false, core::sync::atomic::Ordering::SeqCst);
    }
    // C-84 item 1: the chain's journal (/usb/raport/gpu-jurnal-NNN.txt) goes to the hub when there is a token.
    raport::send_to_hub();
}

/// P6-85 (C-116): `gpu tur --trezire [--sim]` — the read tour on the laptop's GA106 with the minimal wake-up (one
/// write in the PCI Command register, put back at the end). Asks for `DA`; refused by the automatic loop and the hub;
/// outside the test-stick build it refuses before the question.
fn tur_trezire(sim: bool) {
    if sim && !cfg!(feature = "test-gpu-sim") {
        out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        return;
    }
    if !sim && !cfg!(feature = "gpu-scrie") {
        out!("{}\n", crate::drivers::gpu_tur::TREZIRE_REFUSED);
        crate::info!("gpu: {}", crate::drivers::gpu_tur::TREZIRE_REFUSED);
        return;
    }
    for l in crate::drivers::gpu_tur::TREZIRE_WARNING {
        out!("{}\n", l);
    }
    out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva opreste: ");
    let Some(answer) = read_da() else { return };
    if answer.trim() != "DA" {
        out!("gpu tur --trezire: oprit, nimic scris in placa\n");
        crate::info!("gpu: tur --trezire: oprit la confirmare (nimic scris)");
        return;
    }
    crate::info!("gpu: tur --trezire: confirmat cu DA{}", if sim { " (SIMULARE)" } else { "" });
    #[cfg(feature = "test-gpu-sim")]
    if sim {
        for l in crate::drivers::gpu_tur::tour_awake_sim() {
            out!("{}\n", l);
        }
        return;
    }
    for l in crate::drivers::gpu_tur::tour_awake() {
        out!("{}\n", l);
    }
    raport::send_to_hub();
}

/// P6-74 (C-103): `gpu recupereaza [--sim] [--eroare]` — Booter Unload brings WPR2 down without a restart, so the chain
/// can run again in the same power-on. Asks for `DA` before anything; refused by the automatic loop and the hub.
fn recupereaza(rest: &[&str]) {
    if let Some(bad) = rest.iter().find(|a| !matches!(**a, "--sim" | "--eroare")) {
        out!("gpu recupereaza: optiune necunoscuta: {} (folosire: gpu recupereaza)\n", bad);
        return;
    }
    let sim = rest.contains(&"--sim");
    if sim && !cfg!(feature = "test-gpu-sim") {
        out!("gpu: --sim exista doar in build-ul de test (modelul gpu-sim nu intra in sistemul pornit de pe stick)\n");
        return;
    }
    if !sim && !cfg!(feature = "gpu-scrie") {
        for l in crate::drivers::gpu_rec::recover() {
            out!("{}\n", l);
        }
        return;
    }
    for l in crate::drivers::gpu::RECUPEREAZA_WARNING {
        out!("{}\n", l);
    }
    out!("Scrie DA (litere mari) si Enter ca sa continui; orice altceva opreste: ");
    let Some(answer) = read_da() else { return };
    if answer.trim() != "DA" {
        out!("gpu recupereaza: oprit, nimic scris in placa\n");
        crate::info!("gpu: recupereaza: oprit la confirmare (nimic scris)");
        return;
    }
    crate::info!("gpu: recupereaza: confirmat cu DA{}", if sim { " (SIMULARE)" } else { "" });
    #[cfg(feature = "test-gpu-sim")]
    if sim {
        for l in crate::drivers::gpu_rec::recover_sim(rest.contains(&"--eroare")) {
            out!("{}\n", l);
        }
        return;
    }
    for l in crate::drivers::gpu_rec::recover() {
        out!("{}\n", l);
    }
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
    let Some(answer) = read_da() else { return };
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

/// P6-61 step 7 (C-84): `ora [server [port]]` — the time from the network (SNTP), in UTC and in Romania's time, and how
/// far the computer's battery clock is from it. The battery clock is only read (Windows keeps it in local time).
fn ora(args: &[&str]) {
    let (server, port) = match args {
        [] => ("pool.ntp.org", 123),
        [s] => (*s, 123),
        [s, p] => match p.parse::<u16>() {
            Ok(p) if p > 0 => (*s, p),
            _ => {
                out!("ora: portul trebuie sa fie un numar 1..65535\n");
                return;
            }
        },
        _ => {
            out!("folosire: ora [server [port]]\n");
            return;
        }
    };
    if crate::net::config().is_none() {
        out!("ora: nicio placa de retea\n");
        return;
    }
    if !crate::net::wait_ready(5000) {
        out!("ora: reteaua nu are inca adresa (DHCP)\n");
        return;
    }
    let ip = match crate::net::parse_ip(server) {
        Some(ip) => ip,
        None => match crate::net::dns::resolve(server) {
            Ok((ip, _)) => ip,
            Err(e) => {
                out!("ora: {}: {}\n", server, e.message());
                return;
            }
        },
    };
    use crate::net::sntp;
    let a = match sntp::query(ip, port) {
        Ok(a) => a,
        Err(e) => {
            out!("ora: {} ({}.{}.{}.{}:{}): {}\n", server, ip[0], ip[1], ip[2], ip[3], port, e);
            return;
        }
    };
    let (y, mo, d, h, mi, se) = sntp::civil(a.unix);
    let (off, zone) = sntp::romania_offset(a.unix);
    let local = (a.unix as i64 + off) as u64;
    let (ly, lmo, ld, lh, lmi, lse) = sntp::civil(local);
    out!(
        "ora: din retea ({}.{}.{}.{}:{}, strat {}, {} ms): {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC = {:04}-{:02}-{:02} {:02}:{:02}:{:02} ora Romaniei ({})\n",
        ip[0], ip[1], ip[2], ip[3], port, a.stratum, a.rtt_ms, y, mo, d, h, mi, se, ly, lmo, ld, lh, lmi, lse, zone
    );
    match crate::drivers::rtc::now() {
        Some(t) => {
            let rtc = sntp::days_from_civil(t.year as i64, t.month as u32, t.day as u32) * 86_400
                + t.hour as i64 * 3600
                + t.minute as i64 * 60
                + t.second as i64;
            let vs_local = rtc - local as i64;
            let vs_utc = rtc - a.unix as i64;
            let (diff, what) = if vs_local.abs() <= vs_utc.abs() { (vs_local, "ora Romaniei (ca Windows)") } else { (vs_utc, "UTC") };
            out!(
                "ora: ceasul calculatorului {:04}-{:02}-{:02} {:02}:{:02}:{:02} = {} {} {} s{}\n",
                t.year, t.month, t.day, t.hour, t.minute, t.second, what,
                if diff >= 0 { "+" } else { "-" },
                diff.abs(),
                if diff.abs() <= 120 { " (bun)" } else { " (DIFERENTA MARE: bateria ceasului sau fusul orar)" }
            );
        }
        None => out!("ora: ceasul calculatorului nu raspunde\n"),
    }
    crate::info!("ora: din retea {} (UTC {}), {}", server, a.unix, zone);
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
    if args.first() == Some(&"mmio") {
        out!("{}\n", crate::mem::paging::mmio_summary()); // P6-12 diagnostic; the matching log lines follow
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
    for l in crate::hw::smbios::lines() {
        out!("{}\n", l);
    }
    out!("procesor: {}\n", cpu_name());
    out!("{}\n", crate::hw::temp::line());
    out!("{}\n", crate::hw::tco::line());
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
        // P6-83 (C-113): how long the stick's writes and syncs took since power-on.
        ["timp"] => out!("{}\n", crate::drivers::usb::storage::timing_line()),
        _ => out!("usb: folosire: usb citire [mare|mica] | usb timp\n"),
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
    out!("{}\n", crate::fs::flush_all());
    out!("repornesc...\n");
    crate::power::reboot(only)
}
