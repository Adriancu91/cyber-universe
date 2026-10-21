//! Our TLS 1.3 client against a real OpenSSL server (tests/server.py, Python's ssl), on localhost.

use cyber_tls::{Error, Io, MIN_BUFFER, Tls};
use cyber_crypto::x509;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

struct Tcp(TcpStream);

impl Io for Tcp {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        self.0.read(buf).map_err(|_| ())
    }
    fn write_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.0.write_all(data).map_err(|_| ())
    }
}

const NOW: u64 = 20261020120000;
const ROOT: &[u8] = include_bytes!("data/root.der");
const OTHER_ROOT: &[u8] = include_bytes!("data/other_root.der");

fn server(name: &str) -> (Child, u16) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let mut child = Command::new("python3")
        .arg(format!("{dir}/server.py"))
        .arg(format!("{dir}/data/{name}.pem"))
        .arg(format!("{dir}/data/{name}.key"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    (child, line.trim().parse().unwrap())
}

/// Connects, sends a GET, returns the whole response (or the error).
fn fetch(name: &str, host: &str, roots: &[&[u8]], now: u64) -> Result<Vec<u8>, Error> {
    let (mut child, port) = server(name);
    let mut io = Tcp(TcpStream::connect(("127.0.0.1", port)).unwrap());
    let mut random = [0u8; 64];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut random).unwrap();
    let mut buf = vec![0u8; MIN_BUFFER];
    let result = (|| {
        let mut store = roots;
        let mut tls = Tls::connect(&mut io, host, &mut store, now, &random, &mut buf)?;
        tls.send(&mut io, format!("GET / HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())?;
        let mut page = Vec::new();
        let mut chunk = [0u8; 1000];
        loop {
            let n = tls.recv(&mut io, &mut chunk)?;
            if n == 0 {
                break;
            }
            page.extend_from_slice(&chunk[..n]);
        }
        tls.close(&mut io)?;
        Ok(page)
    })();
    drop(io);
    let _ = child.wait();
    result
}

fn check_page(page: &[u8]) {
    let text = String::from_utf8_lossy(&page[..60]);
    assert!(text.starts_with("HTTP/1.0 200 OK"), "{text}");
    assert!(page.windows(10).any(|w| w == b"Salut prin"));
    assert!(page.ends_with(&(0..=255u8).collect::<Vec<_>>()));
    assert!(page.len() > 25600);
}

#[test]
fn ecdsa_server() {
    check_page(&fetch("ec", "test.cyber.local", &[ROOT], NOW).unwrap());
}

#[test]
fn rsa_pss_server() {
    check_page(&fetch("rsa", "test.cyber.local", &[OTHER_ROOT, ROOT], NOW).unwrap());
}

#[test]
fn refused() {
    assert_eq!(fetch("ec", "test.cyber.local", &[OTHER_ROOT], NOW), Err(Error::Certificate(x509::Error::UnknownIssuer)));
    assert_eq!(fetch("ec", "alt.cyber.local", &[ROOT], NOW), Err(Error::Certificate(x509::Error::NameMismatch)));
    assert_eq!(fetch("expired", "test.cyber.local", &[ROOT], NOW), Err(Error::Certificate(x509::Error::Expired)));
}
