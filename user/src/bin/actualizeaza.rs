//! actualizeaza [--forteaza]: updates the OS on the stick it started from (P6-08). Asks GitHub for the latest
//! release (`os-<number>`, P6-07), downloads its files over HTTPS (following GitHub's redirect to its file storage),
//! checks each one against SHA256SUMS and only when every file is good puts them in place: first each is written as
//! `<name>.nou` and read back, then the old file becomes `<name>.vechi`, the new one takes its name and the old one
//! is deleted. If anything fails before that, the old files are untouched and the stick still starts.
//!
//! The repository is private, so the request needs a GitHub token (one line in /usb/github.token, read access is
//! enough); without the file the request is made without one (works if the repository is public). The token is never
//! printed. The stick must be marked for writing (an empty file `cyber.rw` in its root, see kernel/src/fs/fat32.rs).
//! /etc/update.conf (optional, `name = value`) can change: repo, host, port, address (connect there for every host
//! instead of resolving it — tests only), token (the token file), dir (where the stick is mounted).
//! Exit code 0 = updated or already up to date, 1 = nothing changed because of an error.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::sync::atomic::{AtomicBool, Ordering};
use cyber_crypto::sha2::Sha256;
use cyber_json::Value;
use cyber_tls::MIN_BUFFER;
use cyber_user::println;

cyber_user::entry!(main);
cyber_user::heap!(28 * 1024 * 1024);

/// The largest response we keep in memory (the biggest file of a release, the kernel, is ~4 MiB).
const MAX_BODY: usize = 12 * 1024 * 1024;
/// Release file -> place on the stick (docs/USB.md, variant B). VERSIUNE goes last, so it changes only when the
/// rest did.
const FILES: [(&str, &str); 6] = [
    ("BOOTX64.EFI", "EFI/BOOT/BOOTX64.EFI"),
    ("kernel", "boot/kernel"),
    ("initrd.tar", "boot/initrd.tar"),
    ("limine.conf", "boot/limine/limine.conf"),
    ("limine-uefi-cd.bin", "boot/limine/limine-uefi-cd.bin"),
    ("VERSIUNE", "VERSIUNE"),
];

static mut TLS_BUF: [u8; MIN_BUFFER] = [0; MIN_BUFFER];
/// Set once files start being put in place: an error after that no longer means "nothing changed".
static SWAPPING: AtomicBool = AtomicBool::new(false);

struct Config {
    repo: String,
    host: String,
    port: u16,
    address: Option<String>,
    token: String,
    dir: String,
}

fn read_file(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = cyber_user::open(path);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = cyber_user::read(fd, &mut buf);
        if n <= 0 || out.len() > max {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    cyber_user::close(fd);
    Some(out)
}

fn config() -> Config {
    let mut c = Config {
        repo: "Adriancu91/cyber-universe".to_string(),
        host: "api.github.com".to_string(),
        port: 443,
        address: None,
        token: "/usb/github.token".to_string(),
        dir: "/usb".to_string(),
    };
    let Some(text) = read_file("/etc/update.conf", 4096) else { return c };
    for line in core::str::from_utf8(&text).unwrap_or("").lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim().to_string());
        if line.starts_with('#') || value.is_empty() {
            continue;
        }
        match name {
            "repo" => c.repo = value,
            "host" => c.host = value,
            "port" => c.port = value.parse().unwrap_or(c.port),
            "address" => c.address = Some(value),
            "token" => c.token = value,
            "dir" => c.dir = value,
            _ => println!("actualizeaza: /etc/update.conf: nume necunoscut: {}", name),
        }
    }
    c
}

struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

/// Removes HTTP/1.1 chunked framing (RFC 9112 §7.1).
fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = body.windows(2).position(|w| w == b"\r\n")?;
        let line = core::str::from_utf8(&body[..eol]).ok()?;
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        body = &body[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if body.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// One HTTPS GET with `Connection: close`; the whole answer is read (at most MAX_BODY bytes of body).
fn get(c: &Config, host: &str, port: u16, path: &str, headers: &str) -> Result<Response, String> {
    // SAFETY: the program has one thread and uses this buffer for one connection at a time.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(TLS_BUF) };
    let Some((mut io, mut tls)) = cyber_user::https::open("actualizeaza", host, port, c.address.as_deref(), buf) else {
        return Err(format!("nu m-am putut conecta la {}", host));
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: CyberUniverse-actualizeaza\r\n{}Connection: close\r\n\r\n",
        path, host, headers
    );
    if let Err(e) = tls.send(&mut io, request.as_bytes()) {
        return Err(format!("trimitere: {:?}", e));
    }
    drop(request);
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let mut head_end = None;
    let mut expected = None; // header + body, when Content-Length is known
    loop {
        match tls.recv(&mut io, &mut chunk) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(cyber_tls::Error::Io) => break, // closed without close_notify
            Err(e) => return Err(format!("primire: {:?}", e)),
        }
        if head_end.is_none()
            && let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n")
        {
            head_end = Some(end);
            let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
            if let Some(len) = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()) {
                if len > MAX_BODY {
                    return Err(format!("fisier prea mare ({} MiB)", len >> 20));
                }
                raw.reserve_exact((end + 4 + len).saturating_sub(raw.len()));
                expected = Some(end + 4 + len);
            }
        }
        if expected.is_some_and(|e| raw.len() >= e) {
            break;
        }
        if raw.len() > MAX_BODY + 64 * 1024 {
            return Err(format!("raspuns prea mare (peste {} MiB)", MAX_BODY >> 20));
        }
    }
    let _ = tls.close(&mut io);
    let end = head_end.ok_or_else(|| format!("raspuns HTTP de neinteles ({} octeti)", raw.len()))?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).ok_or("raspuns HTTP fara cod")?;
    let mut location = None;
    let mut chunked = false;
    for l in lines {
        let Some((name, value)) = l.split_once(':') else { continue };
        let name = name.trim().to_ascii_lowercase();
        if name == "location" {
            location = Some(value.trim().to_string());
        } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    raw.drain(..end + 4);
    if let Some(e) = expected {
        if raw.len() < e - end - 4 {
            return Err(format!("conexiune intrerupta ({} din {} octeti)", raw.len(), e - end - 4));
        }
        raw.truncate(e - end - 4);
    }
    let body = if chunked { dechunk(&raw).ok_or("corp chunked stricat")? } else { raw };
    Ok(Response { status, location, body })
}

/// "https://host[:port]/path" -> (host, port, path).
fn split_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (authority, 443),
    };
    (!host.is_empty()).then(|| (host.to_string(), port, path.to_string()))
}

/// GET of an API address (with the token), following up to 5 redirects; the token is sent only to the API host,
/// never to the file storage the API redirects to (its address is already signed).
fn get_api(c: &Config, token: Option<&str>, path: &str, accept: &str) -> Result<Vec<u8>, String> {
    let auth = token.map(|t| format!("Authorization: Bearer {}\r\n", t)).unwrap_or_default();
    let mut target = (c.host.clone(), c.port, path.to_string());
    for _ in 0..6 {
        let headers = if target.0 == c.host && target.1 == c.port {
            format!("Accept: {}\r\nX-GitHub-Api-Version: 2022-11-28\r\n{}", accept, auth)
        } else {
            format!("Accept: {}\r\n", accept)
        };
        let r = get(c, &target.0, target.1, &target.2, &headers)?;
        match r.status {
            200 => return Ok(r.body),
            301 | 302 | 303 | 307 | 308 => {
                let loc = r.location.ok_or("redirectionare fara adresa")?;
                target = split_url(&loc).ok_or_else(|| format!("redirectionare catre o adresa necunoscuta: {}", loc))?;
            }
            401 => return Err("tokenul GitHub e gresit sau a expirat (401)".to_string()),
            403 => return Err("GitHub a refuzat cererea (403: limita de cereri sau token fara drept de citire)".to_string()),
            404 if token.is_none() => {
                return Err(format!("nu gasesc versiunea (404): depozitul e privat, pune tokenul GitHub in {}", c.token));
            }
            s => return Err(format!("eroare {} la {}", s, target.2)),
        }
    }
    Err("prea multe redirectionari".to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn path_of(c: &Config, rel: &str) -> String {
    format!("{}/{}", c.dir.trim_end_matches('/'), rel)
}

fn exists(path: &str) -> bool {
    cyber_user::stat(path) >= 0
}

/// Creates the folders above `path` that are missing.
fn make_parents(path: &str) -> Result<(), String> {
    let mut at = 0;
    while let Some(i) = path[at + 1..].find('/') {
        let dir = &path[..at + 1 + i];
        let mut probe = [0u8; 1];
        if cyber_user::list(dir, &mut probe) < 0 && cyber_user::create(dir, true) < 0 {
            return Err(format!("nu pot crea dosarul {}", dir));
        }
        at += 1 + i;
    }
    Ok(())
}

/// Writes `data` as a new file `path` (an old one with that name is deleted first) and reads it back.
fn write_checked(path: &str, data: &[u8], sum: &[u8; 32]) -> Result<(), String> {
    let _ = cyber_user::remove(path);
    make_parents(path)?;
    let r = cyber_user::create(path, false);
    if r < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(r)));
    }
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    for part in data.chunks(64 * 1024) {
        let n = cyber_user::write(fd as u64, part);
        if n != part.len() as i64 {
            cyber_user::close(fd);
            return Err(format!("{}: scriere: {}", path, if n < 0 { cyber_user::error_text(n) } else { "incompleta" }));
        }
    }
    cyber_user::close(fd);
    match read_file(path, data.len()) {
        Some(back) if back.len() == data.len() && Sha256::digest(&back) == *sum => Ok(()),
        _ => Err(format!("{}: ce am citit inapoi nu e ce am scris", path)),
    }
}

/// If an earlier update stopped in the middle of putting files in place, the old file waits as `<name>.vechi`:
/// it gets its name back, so the stick starts as before.
fn repair(c: &Config) {
    for (_, rel) in FILES {
        let (dest, old) = (path_of(c, rel), format!("{}.vechi", path_of(c, rel)));
        if exists(&old) {
            if exists(&dest) {
                let _ = cyber_user::remove(&old);
            } else if cyber_user::rename(&old, &dest) == 0 {
                println!("actualizeaza: {} refacut din copia veche", rel);
            }
        }
    }
}

fn cleanup(c: &Config) {
    for (_, rel) in FILES {
        let _ = cyber_user::remove(&format!("{}.nou", path_of(c, rel)));
    }
}

fn main() -> i64 {
    let force = cyber_user::args().skip(1).any(|a| a == "--forteaza");
    let c = config();
    match run(&c, force) {
        Ok(()) => 0,
        Err(e) => {
            cleanup(&c);
            println!("actualizeaza: {}", e);
            if SWAPPING.load(Ordering::Relaxed) {
                println!("actualizeaza: ATENTIE: doar o parte din fisiere au fost inlocuite; ruleaza din nou actualizeaza");
            } else {
                println!("actualizeaza: nimic schimbat pe stick (fisierele vechi au ramas)");
            }
            1
        }
    }
}

fn run(c: &Config, force: bool) -> Result<(), String> {
    if !exists(&path_of(c, "cyber.rw")) {
        return Err(format!(
            "stick-ul ({}) nu e marcat pentru scriere: pune pe el, in radacina, un fisier gol numit cyber.rw",
            c.dir
        ));
    }
    repair(c);
    let token = read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty());
    println!("actualizeaza: intreb GitHub de ultima versiune ({}{})", c.repo, if token.is_some() { ", cu token" } else { "" });
    let json = get_api(c, token.as_deref(), &format!("/repos/{}/releases/latest", c.repo), "application/vnd.github+json")?;
    let release = core::str::from_utf8(&json).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
    drop(json);
    let tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("?").to_string();
    let assets = release.get("assets").and_then(Value::as_array).ok_or("versiunea nu are fisiere")?;
    let url_of = |name: &str| -> Result<String, String> {
        let a = assets.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(name));
        let url = a.and_then(|a| a.get("url")).and_then(Value::as_str).ok_or_else(|| format!("lipseste fisierul {} din {}", name, tag))?;
        split_url(url).map(|(_, _, p)| p).ok_or_else(|| format!("adresa ciudata pentru {}: {}", name, url))
    };
    let download = |name: &str| get_api(c, token.as_deref(), &url_of(name)?, "application/octet-stream");

    let sums = download("SHA256SUMS")?;
    let sums = String::from_utf8(sums).map_err(|_| "SHA256SUMS nu e text")?;
    let sum_of = |name: &str| -> Result<[u8; 32], String> {
        let line = sums.lines().find(|l| l.split_whitespace().nth(1).map(|n| n.trim_start_matches('*')) == Some(name));
        let h = line.and_then(|l| l.split_whitespace().next()).filter(|h| h.len() == 64).ok_or_else(|| format!("{} lipseste din SHA256SUMS", name))?;
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).map_err(|_| "SHA256SUMS stricat")?;
        }
        Ok(out)
    };
    let version = download("VERSIUNE")?;
    if Sha256::digest(&version) != sum_of("VERSIUNE")? {
        return Err("VERSIUNE nu se potriveste cu SHA256SUMS".to_string());
    }
    let commit = |v: &[u8]| String::from_utf8_lossy(v).lines().next().unwrap_or("").to_string();
    let current = read_file(&path_of(c, "VERSIUNE"), 4096);
    println!("actualizeaza: ultima versiune: {} ({})", tag, commit(&version));
    if !force && current.as_deref() == Some(&version[..]) {
        println!("actualizeaza: stick-ul are deja aceasta versiune, nimic de facut");
        return Ok(());
    }

    // 1. Every file downloaded, checked and written next to the old one as <name>.nou.
    for (name, rel) in FILES {
        let sum = sum_of(name)?;
        let data = if name == "VERSIUNE" { version.clone() } else { download(name)? };
        let got = Sha256::digest(&data);
        if got != sum {
            return Err(format!("{}: suma SHA-256 nu se potriveste ({}…, asteptat {}…)", name, &hex(&got)[..12], &hex(&sum)[..12]));
        }
        write_checked(&format!("{}.nou", path_of(c, rel)), &data, &sum)?;
        println!("actualizeaza: {} ({} KiB) descarcat, verificat, scris", rel, data.len().div_ceil(1024));
    }
    if cyber_user::sync(&c.dir) < 0 {
        println!("actualizeaza: atentie: stick-ul nu a confirmat golirea memoriei");
    }
    // 2. Into place: old -> .vechi, .nou -> name, .vechi deleted. Each step leaves a startable stick or one that
    //    `repair` fixes at the next run.
    SWAPPING.store(true, Ordering::Relaxed);
    for (_, rel) in FILES {
        let dest = path_of(c, rel);
        let (new, old) = (format!("{}.nou", dest), format!("{}.vechi", dest));
        if exists(&dest) {
            let _ = cyber_user::remove(&old);
            let r = cyber_user::rename(&dest, &old);
            if r < 0 {
                return Err(format!("{}: redenumire: {}", rel, cyber_user::error_text(r)));
            }
        }
        let r = cyber_user::rename(&new, &dest);
        if r < 0 {
            let _ = cyber_user::rename(&old, &dest);
            return Err(format!("{}: redenumire: {}", rel, cyber_user::error_text(r)));
        }
        let _ = cyber_user::remove(&old);
    }
    if cyber_user::sync(&c.dir) < 0 {
        println!("actualizeaza: atentie: stick-ul nu a confirmat golirea memoriei; asteapta 10 secunde inainte sa-l scoti");
    }
    println!("actualizeaza: gata - stick-ul are acum {}. Reporneste calculatorul.", tag);
    Ok(())
}
