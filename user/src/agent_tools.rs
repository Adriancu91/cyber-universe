//! The on-board agent's tools (P6-203, C-166 step 2): read-only. Claude may list folders, read files, read the log,
//! see the state of the machine and run shell commands from a white list — nothing that writes, deletes, touches the
//! video card or waits for a "DA". Everything is decided here, in the OS, not by the model: a command outside the list
//! is refused whatever the question said. Secrets (keys, tokens) are hidden by the kernel from every program that did
//! not ask for them (P6-23), and the agent gives up its key right after reading it.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::Value;

use crate::claude_tools;

/// The largest piece of a file Claude gets at once.
const READ_MAX: usize = 32 * 1024;

pub const DEFINITIONS: &str = r#"[
 {"name":"fisiere","description":"Listeaza un dosar din Cyber Universe OS (stick-ul /usb, discul /disk, /tmp si dosarul agentului). Dosarele se termina cu /.","input_schema":{"type":"object","properties":{"cale":{"type":"string","description":"de ex. /usb/raport"}},"required":["cale"]}},
 {"name":"citeste","description":"Citeste un fisier text (aceleasi dosare ca fisiere; cel mult 32 KiB de la `de_la`, in octeti). Cheile si tokenurile nu se vad.","input_schema":{"type":"object","properties":{"cale":{"type":"string"},"de_la":{"type":"integer","description":"octetul de inceput (implicit 0)"}},"required":["cale"]}},
 {"name":"citeste_jurnal","description":"Ultimele randuri din jurnalul OS-ului, optional doar cele cu un cuvant (de ex. gpu, usb, kt).","input_schema":{"type":"object","properties":{"cuvant":{"type":"string"},"n":{"type":"integer","description":"cate randuri (implicit 40, cel mult 400)"}}}},
 {"name":"stare","description":"Starea calculatorului: de cand merge, memoria, lucrarile lungi (pasi, viteza, pierderi), placa de retea.","input_schema":{"type":"object","properties":{}}},
 {"name":"ruleaza","description":"Ruleaza o comanda de citire din lista alba a shell-ului: ls, mem, uptime, lspci, lsusb, discuri, net, cpu, hw, hw lista [--detalii], log ..., usb timp, lucrare stare, porneste (fara argumente), hub stare|comenzi|decizii [proiect]. Orice altceva e refuzat de OS.","input_schema":{"type":"object","properties":{"comanda":{"type":"string"}},"required":["comanda"]}}
]"#;

/// Is `path` one the agent may read? /usb, /disk, /tmp and the agent's own root (the tests keep it in /etc).
pub fn allowed(path: &str, root: &str) -> bool {
    let inside = ["/usb", "/disk", "/tmp", root].iter().any(|r| path == *r || path.starts_with(&format!("{}/", r)));
    inside && !path.split('/').any(|p| p == "..")
}

/// Is `command` on the white list? Only commands that read; no `cat`/`compara` (they run in the kernel, where the
/// secret files are not hidden): files go through `citeste`, which runs in this program.
pub fn white_listed(command: &str) -> bool {
    if command.contains([';', '|', '&', '<', '>', '\n', '\r', '`', '$']) {
        return false;
    }
    let w: Vec<&str> = command.split_whitespace().collect();
    let plain_word = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    match w.as_slice() {
        ["ls"] => true,
        ["ls", p] => !p.split('/').any(|x| x == ".."),
        ["mem"] | ["uptime"] | ["lspci"] | ["lsusb"] | ["discuri"] | ["net"] | ["cpu"] | ["hw"] | ["porneste"] => true,
        ["hw", "lista"] | ["hw", "lista", "--detalii"] | ["usb", "timp"] | ["lucrare", "stare"] => true,
        ["log", rest @ ..] => rest.len() <= 2 && rest.iter().all(|a| plain_word(a)),
        ["hub", what] => matches!(*what, "stare" | "comenzi" | "decizii"),
        ["hub", what, p] => matches!(*what, "stare" | "comenzi" | "decizii") && plain_word(p),
        _ => false,
    }
}

fn list(path: &str) -> Result<String, String> {
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let n = cyber_user::list(path, &mut buf);
    if n < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(n)));
    }
    Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

fn read(path: &str, from: usize) -> Result<String, String> {
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let mut data = Vec::new();
    let mut seen = 0usize;
    let mut buf = [0u8; 4096];
    let mut more = false;
    loop {
        let n = cyber_user::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        let start = from.saturating_sub(seen).min(chunk.len());
        seen += chunk.len();
        data.extend_from_slice(&chunk[start..]);
        if data.len() > READ_MAX {
            data.truncate(READ_MAX);
            more = true;
            break;
        }
    }
    cyber_user::close(fd);
    let mut text = String::from_utf8_lossy(&data).into_owned();
    if more {
        text.push_str(&format!("\n(taiat la 32 KiB; continua cu de_la = {})", from + READ_MAX));
    }
    Ok(text)
}

/// Does one tool call. `log` = one line for the screen and the session file. Ok = result, Err = error for Claude.
pub fn run(name: &str, input: &Value, root: &str, term: &mut Option<i64>, log: &mut String) -> Result<String, String> {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    let number = |k: &str| input.get(k).and_then(Value::as_i64);
    match name {
        "fisiere" | "citeste" => {
            let path = text("cale").ok_or("lipseste campul cale")?;
            *log = format!("{} {}", name, path);
            if !allowed(&path, root) {
                return Err(format!("calea {} nu e permisa (doar /usb, /disk, /tmp, {}; fara ..)", path, root));
            }
            if name == "fisiere" { list(&path) } else { read(&path, number("de_la").unwrap_or(0).max(0) as usize) }
        }
        "citeste_jurnal" => {
            let n = number("n").unwrap_or(40).clamp(1, 400);
            let command = match text("cuvant") {
                Some(w) if !w.is_empty() => format!("log {} {}", w, n),
                _ => format!("log {}", n),
            };
            *log = format!("citeste_jurnal: {}", command);
            if !white_listed(&command) {
                return Err("cuvantul poate avea doar litere, cifre, - _ .".to_string());
            }
            claude_tools::run_command(&command, term)
        }
        "stare" => {
            *log = "stare".to_string();
            let mut out = String::new();
            for c in ["uptime", "mem", "lucrare stare", "net"] {
                out.push_str(&format!("$ {}\n", c));
                match claude_tools::run_command(c, term) {
                    Ok(t) | Err(t) => out.push_str(&t),
                }
                out.push('\n');
            }
            Ok(out)
        }
        "ruleaza" => {
            let command = text("comanda").ok_or("lipseste campul comanda")?;
            let command = command.trim().to_string();
            if !white_listed(&command) {
                *log = format!("ruleaza: {} -> REFUZAT", command);
                return Err(format!("comanda `{}` nu e in lista alba (doar comenzi de citire); OS-ul a refuzat-o", command));
            }
            *log = format!("ruleaza: {}", command);
            claude_tools::run_command(&command, term)
        }
        other => {
            *log = format!("unealta necunoscuta: {}", other);
            Err(format!("unealta necunoscuta: {}", other))
        }
    }
}
