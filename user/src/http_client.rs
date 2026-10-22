//! A small HTTPS client for programs with a heap (P6-08, P6-09): one request per connection (`Connection: close`),
//! the answer read whole (Content-Length or chunked), redirect addresses split. Included with `#[path]` by
//! /bin/actualizeaza and /bin/gh-trimite, so the `cyber_user` library itself stays without a heap.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use cyber_tls::MIN_BUFFER;

static mut TLS_BUF: [u8; MIN_BUFFER] = [0; MIN_BUFFER];

#[allow(dead_code)] // `location`: used by /bin/actualizeaza, not by /bin/gh-trimite
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
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

/// One HTTPS request. `headers` are extra header lines, each ending in "\r\n"; `body` is sent with its
/// Content-Length when the method is not GET. `address`: connect there instead of resolving `host` (tests).
/// Problems with the connection are printed by `prog` (see `cyber_user::https::open`).
#[allow(clippy::too_many_arguments)]
pub fn request(
    prog: &str,
    address: Option<&str>,
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
    max_body: usize,
) -> Result<Response, String> {
    // SAFETY: programs have one thread, and this buffer is used for one connection at a time.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(TLS_BUF) };
    let Some((mut io, mut tls)) = cyber_user::https::open(prog, host, port, address, buf) else {
        return Err(format!("nu m-am putut conecta la {}", host));
    };
    let length = if method == "GET" { String::new() } else { format!("Content-Length: {}\r\n", body.len()) };
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: CyberUniverse-{}\r\n{}{}Connection: close\r\n\r\n",
        method, path, host, prog, headers, length
    )
    .into_bytes();
    req.extend_from_slice(body);
    if let Err(e) = tls.send(&mut io, &req) {
        return Err(format!("trimitere: {:?}", e));
    }
    drop(req);
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
                if len > max_body {
                    return Err(format!("raspuns prea mare ({} KiB)", len >> 10));
                }
                raw.reserve_exact((end + 4 + len).saturating_sub(raw.len()));
                expected = Some(end + 4 + len);
            }
        }
        if expected.is_some_and(|e| raw.len() >= e) {
            break;
        }
        if raw.len() > max_body + 64 * 1024 {
            return Err(format!("raspuns prea mare (peste {} KiB)", max_body >> 10));
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
#[allow(dead_code)] // used by /bin/actualizeaza, not by /bin/gh-trimite
pub fn split_url(url: &str) -> Option<(String, u16, String)> {
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

/// Reads up to `max` bytes of a file (a little more may be read; callers check the length).
pub fn read_file(path: &str, max: usize) -> Option<Vec<u8>> {
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
