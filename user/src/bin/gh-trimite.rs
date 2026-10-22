//! gh-trimite <fisier> <proprietar/depozit> <cale> [mesaj...]: puts a file from Cyber Universe into a GitHub repository
//! (P6-09) through the REST API "contents": GET the file first (if it exists: its `sha`, and nothing is sent when the
//! content is already the same), then PUT with the content in base64, the commit message and that `sha`. The commit is
//! made by the owner of the token.
//!
//! The token (fine-grained, "Contents: Read and write" for the chosen repository) is one line in /usb/github.token; it is
//! never printed. /etc/github.conf (optional, `name = value`) can change: host, port, address (connect there instead of
//! resolving the host — tests only), token (the token file). Exit code 0 = on GitHub (sent or already the same), 1 = not.
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

/// The largest file sent (the contents API is meant for files up to about 1 MiB).
const MAX_FILE: usize = 1024 * 1024;
const MAX_ANSWER: usize = 4 * 1024 * 1024;

struct Config {
    host: String,
    port: u16,
    address: Option<String>,
    token: String,
}

fn config() -> Config {
    let mut c = Config { host: "api.github.com".to_string(), port: 443, address: None, token: "/usb/github.token".to_string() };
    let Some(text) = read_file("/etc/github.conf", 4096) else { return c };
    for line in core::str::from_utf8(&text).unwrap_or("").lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim().to_string());
        if line.starts_with('#') || value.is_empty() {
            continue;
        }
        match name {
            "host" => c.host = value,
            "port" => c.port = value.parse().unwrap_or(c.port),
            "address" => c.address = Some(value),
            "token" => c.token = value,
            _ => println!("gh-trimite: /etc/github.conf: nume necunoscut: {}", name),
        }
    }
    c
}

/// A path for the URL: unreserved characters and "/" stay, the rest (spaces, diacritics…) is %XX (RFC 3986).
fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.ends_with('/') && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

fn main() -> i64 {
    let args: Vec<&str> = cyber_user::args().skip(1).collect();
    if args.len() < 3 {
        println!("folosire: gh-trimite <fisier> <proprietar/depozit> <cale in depozit> [mesaj]");
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
    let size = cyber_user::stat(file);
    if size < 0 {
        return Err(format!("{}: {}", file, cyber_user::error_text(size)));
    }
    if size as usize > MAX_FILE {
        return Err(format!("{} are {} KiB; trimit cel mult {} KiB", file, size >> 10, MAX_FILE >> 10));
    }
    let data = read_file(file, MAX_FILE).ok_or_else(|| format!("nu pot citi {}", file))?;
    let token = read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty());
    let Some(token) = token else {
        return Err(format!("lipseste tokenul GitHub (un rand in {})", c.token));
    };
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
            let same = j.get("content").and_then(Value::as_str).and_then(base64::decode).is_some_and(|old| old == data);
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
        (String::from("content"), Value::String(base64::encode(&data))),
    ];
    drop(data);
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
