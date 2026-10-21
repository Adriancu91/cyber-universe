//! fetch <name> [port] [path]: downloads a web page over plain HTTP (TCP) and prints it (headers removed).
//! fetch https://<name>[:port][/path] [address]: the same over TLS 1.3 (tls/), trusting the certificates in
//! /etc/certs (looked up by the issuer's name, see tools/roots.py); `address` connects there instead of
//! resolving the name (the name still has to match the certificate).
//! Exit code 0 if the server answered 200, otherwise 1.
#![no_std]
#![no_main]

use cyber_tls::MIN_BUFFER;
use cyber_user::net::{TcpStream, resolve};
use cyber_user::{error_text, println, write};

cyber_user::entry!(main);

fn main() -> i64 {
    let mut args = cyber_user::args().skip(1);
    let Some(name) = args.next() else {
        println!("folosire: fetch <nume> [port] [cale]  sau  fetch https://<nume>[:port][/cale] [adresa]");
        return 1;
    };
    if let Some(url) = name.strip_prefix("https://") {
        return https(url, args.next());
    }
    let port = match args.next().map(|p| p.parse::<u16>()) {
        None => 80,
        Some(Ok(p)) => p,
        Some(Err(_)) => {
            println!("fetch: port invalid");
            return 1;
        }
    };
    let path = args.next().unwrap_or("/");
    let ip = match resolve(name) {
        Ok(ip) => ip,
        Err(e) => {
            println!("fetch: {}: {}", name, error_text(e));
            return 1;
        }
    };
    let mut conn = match TcpStream::connect(ip, port, 5000) {
        Ok(c) => c,
        Err(e) => {
            println!("fetch: {}:{}: {}", name, port, error_text(e));
            return 1;
        }
    };
    for part in ["GET ", path, " HTTP/1.0\r\nHost: ", name, "\r\nUser-Agent: CyberUniverse\r\nConnection: close\r\n\r\n"] {
        if let Err(e) = conn.write_all(part.as_bytes()) {
            println!("fetch: trimitere: {}", error_text(e));
            return 1;
        }
    }
    let mut page = Page::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                println!("\nfetch: primire: {}", error_text(e));
                return 1;
            }
        };
        page.feed(&buf[..n]);
    }
    let (total, ok) = (page.total, page.ok);
    println!("-- fetch: {} octeti de la {}:{}", total, name, port);
    if ok { 0 } else { 1 }
}

/// The answer of a web server: the header is kept (up to 1 KiB) until the first empty line, the body is printed.
struct Page {
    head: [u8; 1024],
    head_len: usize,
    in_body: bool,
    ok: bool,
    total: usize,
}

impl Page {
    fn new() -> Page {
        Page { head: [0; 1024], head_len: 0, in_body: false, ok: false, total: 0 }
    }

    fn feed(&mut self, mut data: &[u8]) {
        if !self.in_body {
            let mut i = 0;
            while i < data.len() && !self.in_body {
                if self.head_len < self.head.len() {
                    self.head[self.head_len] = data[i];
                    self.head_len += 1;
                }
                i += 1;
                self.in_body = self.head_len >= 4 && &self.head[self.head_len - 4..self.head_len] == b"\r\n\r\n";
            }
            if self.in_body {
                self.ok = self.head.starts_with(b"HTTP/1.0 200") || self.head.starts_with(b"HTTP/1.1 200");
                if !self.ok {
                    let line = self.head.iter().position(|&b| b == b'\r').unwrap_or(self.head_len);
                    println!("fetch: raspuns: {}", core::str::from_utf8(&self.head[..line]).unwrap_or("?"));
                }
            }
            data = &data[i..];
        }
        if self.in_body {
            self.total += data.len();
            write(1, data);
        }
    }
}

// The program has no heap and a 64 KiB stack: the TLS buffer lives here.
static mut TLS_BUF: [u8; MIN_BUFFER] = [0; MIN_BUFFER];

fn https(url: &str, address: Option<&str>) -> i64 {
    let (hostport, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) => (h, p),
            Err(_) => {
                println!("fetch: port invalid");
                return 1;
            }
        },
        None => (hostport, 443),
    };
    // SAFETY: the program has one thread and this buffer is only borrowed here, once.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(TLS_BUF) };
    let Some((mut io, mut tls)) = cyber_user::https::open("fetch", host, port, address, buf) else { return 1 };
    println!("fetch: TLS 1.3 cu {} (certificat verificat)", host);
    // The whole request in one TLS record: some servers do not answer a request split over several records.
    let mut req = [0u8; 1024];
    let mut n = 0;
    for part in ["GET ", path, " HTTP/1.0\r\nHost: ", host, "\r\nUser-Agent: CyberUniverse\r\nConnection: close\r\n\r\n"] {
        if n + part.len() > req.len() {
            println!("fetch: adresa e prea lunga");
            return 1;
        }
        req[n..n + part.len()].copy_from_slice(part.as_bytes());
        n += part.len();
    }
    if let Err(e) = tls.send(&mut io, &req[..n]) {
        println!("fetch: trimitere: {:?}", e);
        return 1;
    }
    let mut page = Page::new();
    let mut chunk = [0u8; 2048];
    loop {
        match tls.recv(&mut io, &mut chunk) {
            Ok(0) => break,
            Ok(n) => page.feed(&chunk[..n]),
            Err(cyber_tls::Error::Io) => {
                // Closed without close_notify: the page may be cut.
                if page.total == 0 {
                    println!("fetch: conexiunea s-a inchis fara raspuns");
                }
                break;
            }
            Err(e) => {
                println!("\nfetch: primire: {:?}", e);
                return 1;
            }
        }
    }
    let _ = tls.close(&mut io);
    println!("-- fetch: {} octeti de la https://{}:{}", page.total, host, port);
    if page.ok { 0 } else { 1 }
}
