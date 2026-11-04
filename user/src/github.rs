//! Shared by gh-trimite and gh-ia (P6-09, P6-52): /etc/github.conf, repository and path checks, URL encoding.
use alloc::format;
use alloc::string::{String, ToString};
use crate::http_client::read_file;
use cyber_user::println;

pub struct Config {
    pub host: String,
    pub port: u16,
    pub address: Option<String>,
    pub token: String,
}

pub fn config() -> Config {
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
            _ => println!("github: /etc/github.conf: nume necunoscut: {}", name),
        }
    }
    c
}

/// A path for the URL: unreserved characters and "/" stay, the rest (spaces, diacritics…) is %XX (RFC 3986).
pub fn encode_path(path: &str) -> String {
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

pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.ends_with('/') && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

