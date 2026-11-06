//! gh-trimite <fisier> <proprietar/depozit> <cale> [mesaj...]: puts a file from Cyber Universe into a GitHub repository
//! (P6-09) through the REST API "contents": GET the file first (if it exists: its `sha`, and nothing is sent when the
//! content is already the same), then PUT with the content in base64, the commit message and that `sha`. The commit is
//! made by the owner of the token.
//!
//! The token (fine-grained, "Contents: Read and write" for the chosen repository) is one line in /usb/github.token; it is
//! never printed. /etc/github.conf (optional, `name = value`) can change: host, port, address (connect there instead of
//! resolving the host — tests only), token (the token file), hub (where `--rapoarte` sends). Exit code 0 = on GitHub (sent
//! or already the same), 1 = not.
//!
//! `gh-trimite --rapoarte [dosar]` (C-84 item 1, P6-61): sends every report from /usb/raport that is not yet in its list
//! `trimise.txt` to the hub (`Adriancu91/Cyber-hub`, `rezultate/os-automat/<date>/<name>`), so Adrian no longer copies
//! files in Windows. Without a token it says so and the reports stay on the stick (exit code 2).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::{Value, base64};
use cyber_user::println;

cyber_user::entry!(main);
cyber_user::heap!(12 * 1024 * 1024);

#[path = "../http_client.rs"]
mod http_client;
use http_client::read_file;
#[path = "../github.rs"]
mod github;
use github::{Config, config, encode_path, valid_path, valid_repo};

/// The largest file sent (the contents API is meant for files up to about 1 MiB).
const MAX_FILE: usize = 1024 * 1024;
const MAX_ANSWER: usize = 4 * 1024 * 1024;

fn main() -> i64 {
    let args: Vec<&str> = cyber_user::args().skip(1).collect();
    if args.first() == Some(&"--rapoarte") {
        return reports(&config(), args.get(1).copied().unwrap_or(REPORTS_DIR));
    }
    if args.len() < 3 {
        println!("folosire: gh-trimite <fisier> <proprietar/depozit> <cale in depozit> [mesaj]");
        println!("          gh-trimite --rapoarte [dosar]   (rapoartele noi din {} -> hub, rezultate/os-automat/<data>/)", REPORTS_DIR);
        println!("  exemplu: gh-trimite /disk/note.txt Adriancu91/cyber-universe note/azi.txt   (token in /usb/github.token)");
        return 1;
    }
    let message = if args.len() > 3 { args[3..].join(" ") } else { format!("Din Cyber Universe: {}", args[2]) };
    match run(&config(), args[0], args[1], args[2], &message) {
        Ok(()) => 0,
        Err(e) => {
            println!("gh-trimite: {}", e);
            1
        }
    }
}

/// Where `--rapoarte` looks by default: the reports written on the stick (`raport`, `gpu lant`, updates).
const REPORTS_DIR: &str = "/usb/raport";
/// The list of reports already in the hub (one `name -> path in hub` per line), kept next to them.
const LEDGER: &str = "trimise.txt";

/// C-84 item 1 (P6-61): every report in `dir` not yet in its ledger goes to the hub, `rezultate/os-automat/<date>/<name>`.
/// Exit code 0 = nothing left to send, 1 = some did not go, 2 = no token (the reports stay on the stick).
fn reports(c: &Config, dir: &str) -> i64 {
    let mut buf = alloc::vec![0u8; 64 * 1024];
    let n = cyber_user::list(dir, &mut buf);
    if n < 0 {
        println!("gh-trimite: {}: {} (niciun raport de trimis)", dir, cyber_user::error_text(n));
        return 0;
    }
    let ledger_path = format!("{}/{}", dir, LEDGER);
    let ledger = read_file(&ledger_path, 1024 * 1024).map(|t| String::from_utf8_lossy(&t).into_owned()).unwrap_or_default();
    let sent: Vec<&str> = ledger.lines().filter_map(|l| l.split(" -> ").next()).collect();
    let names = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    let pending: Vec<&str> =
        names.lines().filter(|l| !l.is_empty() && !l.ends_with('/') && *l != LEDGER && !sent.contains(l)).collect();
    if pending.is_empty() {
        println!("gh-trimite: nimic nou de trimis din {} ({} trimise deja)", dir, sent.len());
        return 0;
    }
    let Some(token) = read_token(c) else {
        println!(
            "gh-trimite: lipseste tokenul GitHub (un rand in {}): {} rapoarte raman pe stick in {} (vezi docs/USB.md)",
            c.token,
            pending.len(),
            dir
        );
        return 2;
    };
    let day = cyber_user::time()
        .map(|t| format!("{:04}-{:02}-{:02}", t.year, t.month, t.day))
        .unwrap_or_else(|| String::from("fara-data"));
    let mut ledger = ledger;
    let (mut ok, mut failed) = (0, 0);
    for name in pending {
        let file = format!("{}/{}", dir, name);
        let dest = format!("rezultate/os-automat/{}/{}", day, name);
        let message = format!("cyber-universe: raport din OS ({}, trimis automat)", name);
        let result = load(&file).and_then(|data| put(c, &token, &c.hub, &dest, &data, &message));
        match result {
            Ok(_) => {
                ok += 1;
                ledger.push_str(&format!("{} -> {}\n", name, dest));
                if let Err(e) = write_whole(&ledger_path, ledger.as_bytes()) {
                    println!("gh-trimite: {} (lista celor trimise nu s-a scris; la data viitoare se retrimit)", e);
                }
            }
            Err(e) => {
                failed += 1;
                println!("gh-trimite: {}: NU s-a trimis: {} (ramane pe stick)", name, e);
            }
        }
    }
    println!("gh-trimite: rapoarte: {} trimise in {} (rezultate/os-automat/{}/), {} netrimise", ok, c.hub, day, failed);
    if failed == 0 { 0 } else { 1 }
}

/// Writes `data` as the whole content of `path` (the ledger is small; written again each time).
fn write_whole(path: &str, data: &[u8]) -> Result<(), String> {
    let _ = cyber_user::remove(path);
    let r = cyber_user::create(path, false);
    if r < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(r)));
    }
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let w = cyber_user::write(fd as u64, data);
    cyber_user::close(fd);
    if w != data.len() as i64 {
        return Err(format!("{}: scriere incompleta", path));
    }
    Ok(())
}

/// P6-23: the token file is visible only to a program that asks for it; given up right after reading.
fn read_token(c: &Config) -> Option<String> {
    let _ = cyber_user::rights::ask_secret(&c.token);
    let token = read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty());
    let _ = cyber_user::rights::drop_secrets();
    token
}

fn load(file: &str) -> Result<Vec<u8>, String> {
    let size = cyber_user::stat(file);
    if size < 0 {
        return Err(format!("{}: {}", file, cyber_user::error_text(size)));
    }
    if size as usize > MAX_FILE {
        return Err(format!("{} are {} KiB; trimit cel mult {} KiB", file, size >> 10, MAX_FILE >> 10));
    }
    read_file(file, MAX_FILE).ok_or_else(|| format!("nu pot citi {}", file))
}

fn error_for(status: u16, body: &[u8]) -> String {
    let text = core::str::from_utf8(body).ok().and_then(|t| cyber_json::parse(t).ok());
    let msg = text.as_ref().and_then(|j| j.get("message")).and_then(Value::as_str).unwrap_or("").to_string();
    match status {
        401 => "tokenul GitHub e gresit sau a expirat (401)".to_string(),
        403 => format!("GitHub a refuzat (403): tokenul nu are drept de scriere pe depozit sau limita de cereri ({})", msg),
        404 => "depozitul nu exista sau tokenul nu are acces la el (404)".to_string(),
        409 => "fisierul s-a schimbat pe GitHub intre timp (409); incearca din nou".to_string(),
        s => format!("eroare {} de la GitHub: {}", s, msg),
    }
}

fn run(c: &Config, file: &str, repo: &str, path: &str, message: &str) -> Result<(), String> {
    if !valid_repo(repo) {
        return Err(format!("depozit invalid: {} (forma: proprietar/depozit)", repo));
    }
    if !valid_path(path) {
        return Err(format!("cale invalida in depozit: {} (fara / la inceput, fara ..)", path));
    }
    let data = load(file)?;
    let Some(token) = read_token(c) else {
        return Err(format!("lipseste tokenul GitHub (un rand in {})", c.token));
    };
    put(c, &token, repo, path, &data, message)
}

/// GET (is it there, is it the same?) then PUT. Prints what happened.
fn put(c: &Config, token: &str, repo: &str, path: &str, data: &[u8], message: &str) -> Result<(), String> {
    if !valid_repo(repo) || !valid_path(path) {
        return Err(format!("depozit sau cale invalida: {} {}", repo, path));
    }
    let api = format!("/repos/{}/contents/{}", repo, encode_path(path));
    let headers = format!("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\nAuthorization: Bearer {}\r\n", token);
    let ask = |method: &str, extra: &str, body: &[u8]| {
        http_client::request("gh-trimite", c.address.as_deref(), &c.host, c.port, method, &api, &format!("{}{}", headers, extra), body, MAX_ANSWER)
    };

    // 1. Is it there already?
    let r = ask("GET", "", &[])?;
    let old_sha = match r.status {
        200 => {
            let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
            if j.as_array().is_some() || j.get("type").and_then(Value::as_str) == Some("dir") {
                return Err(format!("{} e un dosar in {}", path, repo));
            }
            let sha = j.get("sha").and_then(Value::as_str).ok_or("raspuns fara sha")?.to_string();
            let same = j.get("content").and_then(Value::as_str).and_then(base64::decode).is_some_and(|old| old == *data);
            if same {
                println!("gh-trimite: {} e deja la fel in {} (nimic de trimis)", path, repo);
                return Ok(());
            }
            Some(sha)
        }
        404 => None,
        s => return Err(error_for(s, &r.body)),
    };

    // 2. Send it.
    let mut fields = alloc::vec![
        (String::from("message"), Value::str(message)),
        (String::from("content"), Value::String(base64::encode(data))),
    ];
    if let Some(sha) = &old_sha {
        fields.push((String::from("sha"), Value::str(sha)));
    }
    let body = Value::Object(fields).to_string();
    let r = ask("PUT", "Content-Type: application/json\r\n", body.as_bytes())?;
    drop(body);
    if r.status != 200 && r.status != 201 {
        return Err(error_for(r.status, &r.body));
    }
    let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok());
    let commit = j.as_ref().and_then(|j| j.get("commit")).and_then(|c| c.get("sha")).and_then(Value::as_str).unwrap_or("");
    println!(
        "gh-trimite: {} {} in {} (commit {})",
        if old_sha.is_some() { "actualizat" } else { "creat" },
        path,
        repo,
        &commit[..commit.len().min(7)]
    );
    Ok(())
}
