//! gh-ia <proprietar/depozit> <cale> <fisier>: takes one file from a GitHub repository into Cyber Universe (P6-52, C-68
//! F8: "git simplu", the other half of gh-trimite) through the REST API "contents": GET, base64 content decoded, written to
//! <fisier> (an existing file is replaced only when the download is complete) and read back.
//!
//! The token (one line in /usb/github.token) is sent when it exists (needed for private repositories) and never printed.
//! /etc/github.conf works as for gh-trimite. Files over 1 MiB are refused (the contents API gives no content for them).
//! Exit code 0 = file written (or already the same), 1 = not.
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

const MAX_ANSWER: usize = 4 * 1024 * 1024;

fn main() -> i64 {
    let args: Vec<&str> = cyber_user::args().skip(1).collect();
    if args.len() != 3 {
        println!("folosire: gh-ia <proprietar/depozit> <cale in depozit> <fisier>");
        println!("  exemplu: gh-ia Adriancu91/cyber-universe docs/STATE.md /usb/STATE.md   (token in /usb/github.token, daca e privat)");
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
