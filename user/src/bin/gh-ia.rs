//! gh-ia <proprietar/depozit> <cale> <fisier>: takes one file from a GitHub repository into Cyber Universe (P6-52, C-68
//! F8: "git simplu", the other half of gh-trimite) through the REST API "contents": GET, base64 content decoded, written to
//! <fisier> (an existing file is replaced only when the download is complete) and read back.
//!
//! The token (one line in /usb/github.token) is sent when it exists (needed for private repositories) and never printed.
//! /etc/github.conf works as for gh-trimite. Files over 1 MiB are refused (the contents API gives no content for them).
//! Exit code 0 = file written (or already the same), 1 = not.
//!
//! `gh-ia --hub [comenzi [proiect] | stare [proiect] | decizii]` (C-84 item 2, P6-61): reads the Cyber Hub (only reads; the
//! repository is `hub =` in /etc/github.conf, by default Adriancu91/Cyber-hub) and shows on screen Adrian's commands for a
//! project with their state lines, a project's state file, or the latest decisions — no Windows needed to see them.
//!
//! `gh-ia --hub-ia <cale> <fisier>` (C-87 item 2, P6-64): one file of the hub to <fisier>, like the plain form (the kernel's
//! `hub ruleaza` takes `integrare/cyber-universe/os-comenzi.md` this way).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::{Value, base64};
use cyber_user::{print, println};

cyber_user::entry!(main);
cyber_user::heap!(12 * 1024 * 1024);

#[path = "../http_client.rs"]
mod http_client;
use http_client::read_file;
#[path = "../github.rs"]
mod github;
use github::{Config, config, encode_path, valid_path, valid_repo};

const MAX_ANSWER: usize = 4 * 1024 * 1024;

fn main() -> i64 {
    let args: Vec<&str> = cyber_user::args().skip(1).collect();
    if args.first() == Some(&"--lista") {
        // F8 (P6-61 step 4): the files of a folder (of a branch, with `--ramura`).
        let (repo, dir, branch) = match &args[1..] {
            [r] => (*r, "", None),
            [r, d] if *d != "--ramura" => (*r, *d, None),
            [r, "--ramura", b] => (*r, "", Some(*b)),
            [r, d, "--ramura", b] => (*r, *d, Some(*b)),
            _ => {
                println!("folosire: gh-ia --lista <proprietar/depozit> [dosar] [--ramura <ramura>]");
                return 1;
            }
        };
        return match list(&config(), repo, dir, branch) {
            Ok(()) => 0,
            Err(e) => {
                println!("gh-ia: {}", e);
                1
            }
        };
    }
    if args.first() == Some(&"--hub") {
        return match hub(&config(), &args[1..]) {
            Ok(()) => 0,
            Err(e) => {
                println!("hub: {}", e);
                1
            }
        };
    }
    if args.first() == Some(&"--hub-ia") {
        // C-87 item 2 (P6-64): one file of the hub (`hub =` in /etc/github.conf) into Cyber Universe, as `gh-ia` does.
        let c = config();
        return match &args[1..] {
            [path, file] => match run(&c, &c.hub, path, file) {
                Ok(()) => 0,
                Err(e) => {
                    println!("gh-ia: {}", e);
                    1
                }
            },
            _ => {
                println!("folosire: gh-ia --hub-ia <cale in hub> <fisier>");
                1
            }
        };
    }
    if args.len() != 3 {
        println!("folosire: gh-ia <proprietar/depozit> <cale in depozit> <fisier>");
        println!("  exemplu: gh-ia Adriancu91/cyber-universe docs/STATE.md /usb/STATE.md   (token in /usb/github.token, daca e privat)");
        println!("          gh-ia --lista <proprietar/depozit> [dosar] [--ramura <ramura>]   (fisierele unui dosar)");
        println!("          gh-ia --hub [comenzi [proiect] | stare [proiect] | decizii]   (doar citire din Cyber Hub)");
        return 1;
    }
    match run(&config(), args[0], args[1], args[2]) {
        Ok(()) => 0,
        Err(e) => {
            println!("gh-ia: {}", e);
            1
        }
    }
}

fn run(c: &Config, repo: &str, path: &str, file: &str) -> Result<(), String> {
    let data = fetch(c, repo, path)?;
    if read_file(file, data.len() + 1).as_deref() == Some(&data[..]) {
        println!("gh-ia: {} e deja la fel ({} octeti)", file, data.len());
        return Ok(());
    }
    // Written next to the old file first; the old one is replaced only when the new one reads back right.
    let tmp = format!("{}.nou", file);
    write_checked(&tmp, &data)?;
    let _ = cyber_user::remove(file);
    if cyber_user::rename(&tmp, file) < 0 {
        // A file system without renaming (the memory one, /etc): the checked bytes written straight to the name.
        write_checked(file, &data)?;
        let _ = cyber_user::remove(&tmp);
    }
    println!("gh-ia: {} din {} -> {} ({} octeti)", path, repo, file, data.len());
    Ok(())
}

/// How many commands `hub comenzi` shows (the latest), and how many lines of decisions.
const HUB_COMMANDS: usize = 8;
const HUB_DECISIONS: usize = 40;

/// `--hub ...`: reads one file of the hub and shows what Adrian needs from it. Nothing is written anywhere.
fn hub(c: &Config, args: &[&str]) -> Result<(), String> {
    let what = args.first().copied().unwrap_or("comenzi");
    let project = args.get(1).copied().unwrap_or("cyber-universe");
    if args.len() > 2 || !project.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("folosire: hub [comenzi [proiect] | stare [proiect] | decizii]".to_string());
    }
    let text = |path: &str| fetch(c, &c.hub, path).map(|d| String::from_utf8_lossy(&d).into_owned());
    match what {
        "comenzi" => {
            let all = text("COMENZI.md")?;
            let picked = commands_for(&all, project);
            let fresh = picked.iter().filter(|(_, states)| states.iter().any(|s| s.trim_end() == "- stare: nou")).count();
            println!("hub: {} comenzi pentru {} in COMENZI.md ({} cu stare nou); ultimele {}:", picked.len(), project, fresh, HUB_COMMANDS.min(picked.len()));
            for (head, states) in picked.iter().skip(picked.len().saturating_sub(HUB_COMMANDS)) {
                println!("{}", head.trim_start_matches("### "));
                for s in states {
                    println!("   {}", s.trim_start_matches("- "));
                }
            }
        }
        "stare" => print!("{}", text(&format!("stare/{}.md", project))?),
        "decizii" => {
            let all = text("DECIZII.md")?;
            let lines: Vec<&str> = all.lines().collect();
            println!("hub: DECIZII.md, ultimele {} randuri din {}:", HUB_DECISIONS.min(lines.len()), lines.len());
            for l in &lines[lines.len().saturating_sub(HUB_DECISIONS)..] {
                println!("{}", l);
            }
        }
        other => return Err(format!("necunoscut: {} (folosire: hub [comenzi [proiect] | stare [proiect] | decizii])", other)),
    }
    Ok(())
}

/// The commands (`### C-NN · proiecte · titlu`) whose project list names `project`, each with its `- stare...` lines.
fn commands_for<'a>(all: &'a str, project: &str) -> Vec<(&'a str, Vec<&'a str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut current = false;
    for line in all.lines() {
        if line.starts_with("### ") {
            let projects = line.split(" · ").nth(1).unwrap_or("");
            current = line.starts_with("### C-")
                && projects.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')).any(|w| w == project);
            if current {
                out.push((line, Vec::new()));
            }
        } else if line.starts_with("## ") {
            current = false;
        } else if current && line.starts_with("- stare") {
            if let Some(last) = out.last_mut() {
                last.1.push(line);
            }
        }
    }
    out
}

/// The entries of a folder through the contents API (an array of `{name, type, size}`), one per line.
fn list(c: &Config, repo: &str, dir: &str, branch: Option<&str>) -> Result<(), String> {
    if !valid_repo(repo) {
        return Err(format!("depozit invalid: {} (forma: proprietar/depozit)", repo));
    }
    if !dir.is_empty() && !valid_path(dir) {
        return Err(format!("cale invalida in depozit: {} (fara / la inceput, fara ..)", dir));
    }
    if let Some(b) = branch {
        if b.is_empty() || b.contains("..") || !b.bytes().all(|x| x.is_ascii_alphanumeric() || b"-_./".contains(&x)) {
            return Err(format!("ramura invalida: {}", b));
        }
    }
    let _ = cyber_user::rights::ask_secret(&c.token);
    let token = read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty());
    let _ = cyber_user::rights::drop_secrets();
    let auth = token.map(|t| format!("Authorization: Bearer {}\r\n", t)).unwrap_or_default();
    let mut api = format!("/repos/{}/contents/{}", repo, encode_path(dir));
    if let Some(b) = branch {
        api += &format!("?ref={}", encode_path(b));
    }
    let headers = format!("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n{}", auth);
    let r = http_client::request("gh-ia", c.address.as_deref(), &c.host, c.port, "GET", &api, &headers, &[], MAX_ANSWER)?;
    match r.status {
        200 => {}
        401 => return Err("tokenul GitHub e gresit sau a expirat (401)".to_string()),
        404 => return Err(format!("{} nu exista in {} (sau depozitul e privat si lipseste tokenul)", if dir.is_empty() { "/" } else { dir }, repo)),
        s => return Err(format!("eroare {} de la GitHub", s)),
    }
    let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
    let Some(items) = j.as_array() else {
        return Err(format!("{} e un fisier, nu un dosar", dir));
    };
    println!("gh-ia: {}/{}{}: {} intrari", repo, dir, branch.map(|b| format!(" (ramura {})", b)).unwrap_or_default(), items.len());
    for it in items {
        let name = it.get("name").and_then(Value::as_str).unwrap_or("?");
        match it.get("type").and_then(Value::as_str) {
            Some("dir") => println!("  {}/", name),
            _ => println!("  {}  ({} octeti)", name, it.get("size").and_then(Value::as_f64).unwrap_or(0.0) as u64),
        }
    }
    Ok(())
}

/// GET of one file through the contents API (base64 decoded, size checked).
fn fetch(c: &Config, repo: &str, path: &str) -> Result<Vec<u8>, String> {
    if !valid_repo(repo) {
        return Err(format!("depozit invalid: {} (forma: proprietar/depozit)", repo));
    }
    if !valid_path(path) {
        return Err(format!("cale invalida in depozit: {} (fara / la inceput, fara ..)", path));
    }
    // P6-23: the token file is visible only to a program that asks for it; given up right after reading.
    let _ = cyber_user::rights::ask_secret(&c.token);
    let token = read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty());
    let _ = cyber_user::rights::drop_secrets();
    let auth = token.map(|t| format!("Authorization: Bearer {}\r\n", t)).unwrap_or_default();
    let api = format!("/repos/{}/contents/{}", repo, encode_path(path));
    let headers = format!("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n{}", auth);
    let r = http_client::request("gh-ia", c.address.as_deref(), &c.host, c.port, "GET", &api, &headers, &[], MAX_ANSWER)?;
    match r.status {
        200 => {}
        401 => return Err("tokenul GitHub e gresit sau a expirat (401)".to_string()),
        403 => return Err("GitHub a refuzat (403): limita de cereri sau token fara drept de citire".to_string()),
        404 => return Err(format!("{} nu exista in {} (sau depozitul e privat si lipseste tokenul)", path, repo)),
        s => return Err(format!("eroare {} de la GitHub", s)),
    }
    let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
    if j.as_array().is_some() || j.get("type").and_then(Value::as_str) != Some("file") {
        return Err(format!("{} nu e un fisier in {}", path, repo));
    }
    let content = j.get("content").and_then(Value::as_str).unwrap_or("");
    let size = j.get("size").and_then(Value::as_f64).unwrap_or(0.0) as usize;
    if content.is_empty() && size > 0 {
        return Err(format!("{} are {} KiB; iau cel mult 1024 KiB", path, size >> 10));
    }
    let data = base64::decode(content).ok_or("continut base64 stricat")?;
    if data.len() != size {
        return Err(format!("am primit {} octeti, GitHub spune {}", data.len(), size));
    }
    Ok(data)
}

/// Writes `data` as a new file `path` and reads it back; on any failure the half-written file is removed.
fn write_checked(path: &str, data: &[u8]) -> Result<(), String> {
    let _ = cyber_user::remove(path);
    let r = cyber_user::create(path, false);
    if r < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(r)));
    }
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    for part in data.chunks(64 * 1024) {
        if cyber_user::write(fd as u64, part) != part.len() as i64 {
            cyber_user::close(fd);
            let _ = cyber_user::remove(path);
            return Err(format!("{}: scriere incompleta", path));
        }
    }
    cyber_user::close(fd);
    if read_file(path, data.len() + 1).as_deref() != Some(data) {
        let _ = cyber_user::remove(path);
        return Err(format!("{}: ce am citit inapoi nu e ce am scris", path));
    }
    Ok(())
}
