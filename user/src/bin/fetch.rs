//! fetch <name> [port] [path]: downloads a web page over plain HTTP (TCP) and prints it (headers removed).
//! fetch https://<name>[:port][/path] [address]: the same over TLS 1.3 (tls/), trusting the certificates in
//! /etc/certs; `address` connects there instead of resolving the name (the name still has to match the certificate).
//! Exit code 0 if the server answered 200, otherwise 1.
#![no_std]
#![no_main]

use cyber_tls::{Io, MIN_BUFFER, Tls};
use cyber_user::net::{TcpStream, parse_ip, resolve};
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

struct Tcp(TcpStream);

impl Io for Tcp {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        self.0.read(buf).map_err(|_| ())
    }
    fn write_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.0.write_all(data).map_err(|_| ())
    }
}

// The program has no heap and a 64 KiB stack: the TLS buffer and the trusted certificates live here.
static mut TLS_BUF: [u8; MIN_BUFFER] = [0; MIN_BUFFER];
static mut CERTS: [u8; 32 * 1024] = [0; 32 * 1024];
const MAX_ROOTS: usize = 16;

/// Reads every file in /etc/certs (DER certificates) into CERTS; returns how many and their places.
fn load_roots(store: &mut [u8], places: &mut [(usize, usize); MAX_ROOTS]) -> usize {
    let mut names = [0u8; 2048];
    let n = cyber_user::list("/etc/certs", &mut names);
    if n <= 0 {
        return 0;
    }
    let (mut count, mut used) = (0, 0);
    for name in core::str::from_utf8(&names[..n as usize]).unwrap_or("").lines() {
        if name.ends_with('/') || count == MAX_ROOTS {
            continue;
        }
        let mut path = [0u8; 128];
        let full = b"/etc/certs/";
        if full.len() + name.len() > path.len() {
            continue;
        }
        path[..full.len()].copy_from_slice(full);
        path[full.len()..full.len() + name.len()].copy_from_slice(name.as_bytes());
        let Ok(p) = core::str::from_utf8(&path[..full.len() + name.len()]) else { continue };
        let fd = cyber_user::open(p);
        if fd < 0 {
            continue;
        }
        let start = used;
        loop {
            let r = cyber_user::read(fd, &mut store[used..]);
            if r <= 0 {
                break;
            }
            used += r as usize;
        }
        cyber_user::close(fd);
        if used > start {
            places[count] = (start, used);
            count += 1;
        }
    }
    count
}

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
    let ip = match address {
        Some(a) => match parse_ip(a) {
            Some(ip) => ip,
            None => {
                println!("fetch: adresa invalida: {}", a);
                return 1;
            }
        },
        None => match resolve(host) {
            Ok(ip) => ip,
            Err(e) => {
                println!("fetch: {}: {}", host, error_text(e));
                return 1;
            }
        },
    };
    // SAFETY: the program has one thread and these buffers are only borrowed here, once.
    let (store, buf) = unsafe { (&mut *core::ptr::addr_of_mut!(CERTS), &mut *core::ptr::addr_of_mut!(TLS_BUF)) };
    let mut places = [(0usize, 0usize); MAX_ROOTS];
    let count = load_roots(store, &mut places);
    let mut roots: [&[u8]; MAX_ROOTS] = [&[]; MAX_ROOTS];
    for i in 0..count {
        roots[i] = &store[places[i].0..places[i].1];
    }
    if count == 0 {
        println!("fetch: niciun certificat de incredere in /etc/certs");
        return 1;
    }
    // The real-time clock (taken as UTC) decides whether certificates are still valid.
    let Some(t) = cyber_user::time() else {
        println!("fetch: ceasul nu e disponibil");
        return 1;
    };
    let now = t.year as u64 * 10_000_000_000
        + t.month as u64 * 100_000_000
        + t.day as u64 * 1_000_000
        + t.hour as u64 * 10_000
        + t.minute as u64 * 100
        + t.second as u64;
    let mut random = [0u8; 64];
    if !cyber_user::random(&mut random) {
        println!("fetch: numere aleatoare indisponibile");
        return 1;
    }
    let mut io = match TcpStream::connect(ip, port, 5000) {
        Ok(c) => Tcp(c),
        Err(e) => {
            println!("fetch: {}:{}: {}", host, port, error_text(e));
            return 1;
        }
    };
    let mut tls = match Tls::connect(&mut io, host, &roots[..count], now, &random, buf) {
        Ok(t) => t,
        Err(e) => {
            println!("fetch: TLS refuzat: {:?}", e);
            return 1;
        }
    };
    println!("fetch: TLS 1.3 cu {} (certificat verificat)", host);
    for part in ["GET ", path, " HTTP/1.0\r\nHost: ", host, "\r\nUser-Agent: CyberUniverse\r\nConnection: close\r\n\r\n"] {
        if let Err(e) = tls.send(&mut io, part.as_bytes()) {
            println!("fetch: trimitere: {:?}", e);
            return 1;
        }
    }
    let mut page = Page::new();
    let mut chunk = [0u8; 2048];
    loop {
        match tls.recv(&mut io, &mut chunk) {
            Ok(0) => break,
            Ok(n) => page.feed(&chunk[..n]),
            Err(cyber_tls::Error::Io) => break, // closed without close_notify: the page may be cut
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
