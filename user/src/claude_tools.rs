//! The tools Claude may ask for in the chat (P6-06): read a file, list a folder, write a file, run a shell command.
//! Reading and listing happen at once; writing and running need Adrian's "Da" on screen (the app asks through
//! `confirm`). Paths are limited to /disk, /usb and /tmp (never /bin or /etc), without "..".

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::Value;

/// The largest file Claude can read, and the most output kept from a command.
const READ_MAX: usize = 32 * 1024;
const OUTPUT_MAX: usize = 8 * 1024;
/// How long a command may run before we stop waiting for it.
const COMMAND_MS: u64 = 10_000;

pub const DEFINITIONS: &str = r#"[
 {"name":"read_file","description":"Citeste un fisier text din Cyber Universe OS (doar sub /disk, /usb sau /tmp; cel mult 32 KiB).","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"calea completa, de ex. /disk/note.txt"}},"required":["path"]}},
 {"name":"list_dir","description":"Listeaza un dosar (doar sub /disk, /usb sau /tmp). Dosarele se termina cu /.","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
 {"name":"write_file","description":"Scrie (sau inlocuieste) un fisier text sub /disk, /usb sau /tmp. Adrian trebuie sa confirme pe ecran.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}},
 {"name":"run_command","description":"Ruleaza o comanda in shell-ul Cyber Universe (de ex. ls /disk, hw, net). Adrian trebuie sa confirme pe ecran. Iesirea e taiata la 8 KiB.","input_schema":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}}
]"#;

/// Is `path` one Claude may touch?
pub fn allowed(path: &str) -> bool {
    let inside = ["/disk", "/usb", "/tmp"].iter().any(|root| path == *root || path.starts_with(&alloc::format!("{}/", root)));
    inside && !path.split('/').any(|part| part == "..")
}

/// What a tool wants to do, before it is done.
pub enum Request {
    Read(String),
    List(String),
    Write(String, String),
    Run(String),
}

impl Request {
    /// Reads a tool_use block's name and input.
    pub fn from(name: &str, input: &Value) -> Result<Request, String> {
        let field = |k: &str| input.get(k).and_then(Value::as_str).map(String::from).ok_or(alloc::format!("lipseste campul {}", k));
        Ok(match name {
            "read_file" => Request::Read(field("path")?),
            "list_dir" => Request::List(field("path")?),
            "write_file" => Request::Write(field("path")?, field("content")?),
            "run_command" => Request::Run(field("command")?),
            other => return Err(alloc::format!("unealta necunoscuta: {}", other)),
        })
    }

    /// Writing and running need Adrian's "Da".
    pub fn needs_confirmation(&self) -> bool {
        matches!(self, Request::Write(..) | Request::Run(_))
    }

    /// One line for the conversation and the serial log.
    pub fn describe(&self) -> String {
        match self {
            Request::Read(p) => alloc::format!("citesc {}", p),
            Request::List(p) => alloc::format!("listez {}", p),
            Request::Write(p, c) => alloc::format!("scriu {} ({} octeti)", p, c.len()),
            Request::Run(c) => alloc::format!("rulez: {}", c),
        }
    }

    /// The lines shown in the confirmation box.
    pub fn details(&self) -> Vec<String> {
        match self {
            Request::Write(p, c) => {
                let mut v = alloc::vec![alloc::format!("Claude vrea sa scrie fisierul {}", p), alloc::format!("({} octeti), incepand cu:", c.len())];
                v.extend(c.lines().take(4).map(|l| alloc::format!("  {}", l.chars().take(40).collect::<String>())));
                v
            }
            Request::Run(c) => alloc::vec!["Claude vrea sa ruleze comanda:".to_string(), alloc::format!("  {}", c)],
            _ => Vec::new(),
        }
    }

    /// Does it (after the confirmation, when one is needed). Ok = the result for Claude, Err = an error for Claude.
    pub fn run(&self, term: &mut Option<i64>) -> Result<String, String> {
        match self {
            Request::Read(p) | Request::List(p) | Request::Write(p, _) if !allowed(p) => {
                Err(alloc::format!("calea {} nu e permisa (doar /disk, /usb, /tmp, fara ..)", p))
            }
            Request::Read(p) => {
                let fd = cyber_user::open(p);
                if fd < 0 {
                    return Err(alloc::format!("{}: {}", p, cyber_user::error_text(fd)));
                }
                let mut data = Vec::new();
                let mut buf = [0u8; 2048];
                while data.len() < READ_MAX {
                    let n = cyber_user::read(fd, &mut buf);
                    if n <= 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n as usize]);
                }
                cyber_user::close(fd);
                data.truncate(READ_MAX);
                Ok(String::from_utf8_lossy(&data).into_owned())
            }
            Request::List(p) => {
                let mut buf = alloc::vec![0u8; 8192];
                let n = cyber_user::list(p, &mut buf);
                if n < 0 {
                    return Err(alloc::format!("{}: {}", p, cyber_user::error_text(n)));
                }
                Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
            }
            Request::Write(p, content) => {
                let r = cyber_user::remove(p);
                let r = if r < 0 && r != cyber_user::E_NOT_FOUND { r } else { cyber_user::create(p, false) };
                let fd = if r < 0 { r } else { cyber_user::open(p) };
                if fd < 0 {
                    return Err(alloc::format!("{}: {}", p, cyber_user::error_text(fd)));
                }
                let mut done = 0;
                let bytes = content.as_bytes();
                while done < bytes.len() {
                    let n = cyber_user::write(fd as u64, &bytes[done..bytes.len().min(done + 16 * 1024)]);
                    if n <= 0 {
                        break;
                    }
                    done += n as usize;
                }
                cyber_user::close(fd);
                if done == bytes.len() { Ok(alloc::format!("scris {} ({} octeti)", p, done)) } else { Err("scriere incompleta".to_string()) }
            }
            Request::Run(command) => run_command(command, term),
        }
    }
}

/// Runs `command` in the app's own shell (a terminal, as in /bin/terminal) and returns what it printed.
fn run_command(command: &str, term: &mut Option<i64>) -> Result<String, String> {
    const PROMPT: &str = "cyber> ";
    let mut buf = [0u8; 2048];
    let id = match *term {
        Some(id) => id,
        None => {
            let id = cyber_user::term::open();
            if id < 0 {
                return Err("nu pot porni un shell".to_string());
            }
            // Wait for the first prompt and drop the greeting.
            let mut seen = String::new();
            for _ in 0..200 {
                let n = cyber_user::term::read(id, &mut buf);
                if n > 0 {
                    seen.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
                }
                if seen.ends_with(PROMPT) {
                    break;
                }
                cyber_user::sleep_ms(10);
            }
            *term = Some(id);
            id
        }
    };
    if command.contains('\n') {
        return Err("comanda trebuie sa fie pe un singur rand".to_string());
    }
    cyber_user::term::write(id, &alloc::format!("{}\n", command));
    let mut out = String::new();
    let mut waited = 0;
    loop {
        let n = cyber_user::term::read(id, &mut buf);
        if n < 0 {
            *term = None;
            break;
        }
        if n > 0 {
            out.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            if out.len() > 4 * OUTPUT_MAX {
                out.drain(..out.len() - 2 * OUTPUT_MAX);
            }
        }
        if out.ends_with(PROMPT) {
            out.truncate(out.len() - PROMPT.len());
            break;
        }
        if waited >= COMMAND_MS {
            out.push_str("\n(comanda inca ruleaza dupa 10 s; nu mai astept)");
            break;
        }
        cyber_user::sleep_ms(20);
        waited += 20;
    }
    // The shell echoes the command: drop that first line.
    let out = match out.split_once('\n') {
        Some((first, rest)) if first.trim_end() == command.trim_end() => rest.to_string(),
        _ => out,
    };
    let mut out = out.trim_end().to_string();
    if out.len() > OUTPUT_MAX {
        let mut cut = OUTPUT_MAX;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n(iesire taiata la 8 KiB)");
    }
    Ok(out)
}

/// A write outside the allowed folders is refused without asking Adrian (there is nothing to confirm).
pub fn allowed_request(r: &Request) -> bool {
    match r {
        Request::Write(p, _) => allowed(p),
        _ => true,
    }
}
