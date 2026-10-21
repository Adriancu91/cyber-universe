//! Our TLS 1.3 client against real web sites, trusting only the Mozilla roots from boot/certs/ (looked up by
//! file name, as /bin/fetch does). Needs internet: it runs when CYBER_NET=1 (set in CI); otherwise it is skipped.

use cyber_crypto::x509::{TrustStore, subject_id};
use cyber_tls::{Io, MIN_BUFFER, Tls};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

struct Tcp(TcpStream);

impl Io for Tcp {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        self.0.read(buf).map_err(|_| ())
    }
    fn write_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.0.write_all(data).map_err(|_| ())
    }
}

/// The roots in boot/certs, opened by name: <subject id>.0, .1 …
struct Dir;

impl TrustStore for Dir {
    fn find(&mut self, subject: &[u8], f: &mut dyn FnMut(&[u8]) -> bool) -> bool {
        let id = String::from_utf8(subject_id(subject).to_vec()).unwrap();
        for n in 0.. {
            let path = format!("{}/../boot/certs/{id}.{n}", env!("CARGO_MANIFEST_DIR"));
            let Ok(der) = std::fs::read(&path) else { return false };
            if f(&der) {
                return true;
            }
        }
        false
    }
}

fn now() -> u64 {
    let out = std::process::Command::new("date").args(["-u", "+%Y%m%d%H%M%S"]).output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().parse().unwrap()
}

/// Opens TLS to host:443, sends a GET and returns the status line.
fn get(host: &str) -> Result<String, String> {
    let addr = (host, 443).to_socket_addrs().map_err(|e| e.to_string())?.find(|a| a.is_ipv4()).ok_or("fara IPv4")?;
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let mut io = Tcp(s);
    let mut random = [0u8; 64];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut random).unwrap();
    let mut buf = vec![0u8; MIN_BUFFER];
    let mut tls = Tls::connect(&mut io, host, &mut Dir, now(), &random, &mut buf).map_err(|e| format!("{e:?}"))?;
    tls.send(&mut io, format!("GET / HTTP/1.0\r\nHost: {host}\r\nUser-Agent: CyberUniverse\r\n\r\n").as_bytes())
        .map_err(|e| format!("{e:?}"))?;
    let mut page = Vec::new();
    let mut chunk = [0u8; 2048];
    while page.len() < 512 {
        match tls.recv(&mut io, &mut chunk) {
            Ok(0) => break,
            Ok(n) => page.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(format!("primire: {e:?}")),
        }
    }
    let text = String::from_utf8_lossy(&page);
    let line = text.lines().next().unwrap_or("").to_string();
    if line.starts_with("HTTP/1.") { Ok(line) } else { Err(format!("raspuns ciudat: {line:?}")) }
}

/// Sites the next tasks need (P6-04 Claude API, P6-08/P6-09 GitHub) plus common certificate authorities.
const REQUIRED: [&str; 4] = ["api.anthropic.com", "github.com", "api.github.com", "example.com"];
const OTHERS: [&str; 6] = ["www.google.com", "letsencrypt.org", "www.cloudflare.com", "www.wikipedia.org", "www.digicert.com", "objects.githubusercontent.com"];

#[test]
fn real_sites() {
    if std::env::var("CYBER_NET").as_deref() != Ok("1") {
        println!("sarit: fara internet (CYBER_NET=1 il porneste)");
        return;
    }
    let mut failed = Vec::new();
    for host in REQUIRED.iter().chain(OTHERS.iter()) {
        let r = get(host);
        println!("{host}: {r:?}");
        if r.is_err() && REQUIRED.contains(host) {
            failed.push(*host);
        }
    }
    assert!(failed.is_empty(), "site-uri obligatorii refuzate: {failed:?}");
}
