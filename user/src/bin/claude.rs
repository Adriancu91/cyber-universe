//! claude <question>: asks Claude through the Anthropic API (Messages, JSON over HTTPS) and prints the answer.
//! The API key is read from /usb/claude.key (one line) and is never printed. /etc/claude.conf (optional, lines
//! `name = value`) can change: model, max_tokens, host, port, address (connect there instead of resolving the
//! host), key (the key file). Exit code 0 when Claude answered, otherwise 1.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::Value;
use cyber_tls::MIN_BUFFER;
use cyber_user::{println, write};

cyber_user::entry!(main);
cyber_user::heap!(1024 * 1024);

const API_VERSION: &str = "2023-06-01";
/// The largest answer we keep (the program's heap is 1 MiB).
const MAX_ANSWER: usize = 512 * 1024;

static mut TLS_BUF: [u8; MIN_BUFFER] = [0; MIN_BUFFER];

struct Config {
    model: String,
    max_tokens: i64,
    host: String,
    port: u16,
    address: Option<String>,
    key: String,
}

fn read_file(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = cyber_user::open(path);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = cyber_user::read(fd, &mut buf);
        if n <= 0 || out.len() >= max {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    cyber_user::close(fd);
    Some(out)
}

fn config() -> Config {
    let mut c = Config {
        model: "claude-sonnet-5-5".to_string(),
        max_tokens: 1024,
        host: "api.anthropic.com".to_string(),
        port: 443,
        address: None,
        key: "/usb/claude.key".to_string(),
    };
    let Some(text) = read_file("/etc/claude.conf", 4096) else { return c };
    for line in core::str::from_utf8(&text).unwrap_or("").lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim().to_string());
        if line.starts_with('#') || value.is_empty() {
            continue;
        }
        match name {
            "model" => c.model = value,
            "max_tokens" => c.max_tokens = value.parse().unwrap_or(c.max_tokens),
            "host" => c.host = value,
            "port" => c.port = value.parse().unwrap_or(c.port),
            "address" => c.address = Some(value),
            "key" => c.key = value,
            _ => println!("claude: /etc/claude.conf: nume necunoscut: {}", name),
        }
    }
    c
}

/// Removes HTTP/1.1 chunked framing (RFC 9112 §7.1): size in hex, CRLF, data, CRLF … until a size of 0.
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

/// Splits an HTTP response into (status code, body), undoing chunked transfer encoding.
fn http_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = &raw[end + 4..];
    Some((status, if chunked { dechunk(body)? } else { body.to_vec() }))
}

fn main() -> i64 {
    let question: Vec<&str> = cyber_user::args().skip(1).collect();
    if question.is_empty() {
        println!("folosire: claude <intrebare>   (cheia API in /usb/claude.key)");
        return 1;
    }
    let question = question.join(" ");
    let c = config();
    let key = match read_file(&c.key, 512) {
        Some(k) => String::from_utf8(k).unwrap_or_default().trim().to_string(),
        None => String::new(),
    };
    if key.is_empty() || key.contains(['\r', '\n']) {
        println!("claude: nu am gasit cheia API (un singur rand in {})", c.key);
        return 1;
    }
    println!("claude: cheie gasita; intreb {} ...", c.model);

    let body = Value::object([
        ("model", Value::str(&c.model)),
        ("max_tokens", Value::int(c.max_tokens)),
        ("messages", Value::Array(alloc::vec![Value::object([("role", Value::str("user")), ("content", Value::str(&question))])])),
    ])
    .to_string();

    // SAFETY: the program has one thread and this buffer is only borrowed here, once.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(TLS_BUF) };
    let Some((mut io, mut tls)) = cyber_user::https::open("claude", &c.host, c.port, c.address.as_deref(), buf) else {
        return 1;
    };
    let length = body.len().to_string();
    let request = [
        "POST /v1/messages HTTP/1.1\r\nHost: ",
        &c.host,
        "\r\nx-api-key: ",
        &key,
        "\r\nanthropic-version: ",
        API_VERSION,
        "\r\ncontent-type: application/json\r\ncontent-length: ",
        &length,
        "\r\nUser-Agent: CyberUniverse\r\nConnection: close\r\n\r\n",
        &body,
    ]
    .concat();
    if let Err(e) = tls.send(&mut io, request.as_bytes()) {
        println!("claude: trimitere: {:?}", e);
        return 1;
    }
    drop(request);
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tls.recv(&mut io, &mut chunk) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(cyber_tls::Error::Io) => break, // closed without close_notify
            Err(e) => {
                println!("claude: primire: {:?}", e);
                return 1;
            }
        }
        if raw.len() > MAX_ANSWER {
            println!("claude: raspunsul e prea mare (peste {} KiB)", MAX_ANSWER / 1024);
            return 1;
        }
    }
    let _ = tls.close(&mut io);

    let Some((status, body)) = http_response(&raw) else {
        println!("claude: raspuns HTTP de neinteles ({} octeti)", raw.len());
        return 1;
    };
    let json = core::str::from_utf8(&body).ok().and_then(|t| cyber_json::parse(t).ok());
    let message = json.as_ref().and_then(|j| j.get("error")).and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("");
    match status {
        200 => {}
        401 => {
            println!("claude: cheia API e gresita sau a expirat (401)");
            return 1;
        }
        429 => {
            println!("claude: prea multe cereri sau limita de cheltuieli atinsa (429), incearca mai tarziu");
            return 1;
        }
        529 => {
            println!("claude: serviciul e supraincarcat acum (529), incearca mai tarziu");
            return 1;
        }
        s => {
            println!("claude: eroare {}: {}", s, message);
            return 1;
        }
    }
    let Some(content) = json.as_ref().and_then(|j| j.get("content")).and_then(Value::as_array) else {
        println!("claude: raspuns fara text");
        return 1;
    };
    for part in content {
        if part.get("type").and_then(Value::as_str) == Some("text") {
            write(1, part.get("text").and_then(Value::as_str).unwrap_or("").as_bytes());
        }
    }
    println!();
    0
}
