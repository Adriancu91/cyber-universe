//! P6-02: our TLS 1.3 client against real sites, with the Mozilla roots of the OS image (boot/initrd/etc/certs).
//!   cargo run --release --example real -- example.com letsencrypt.org ...
//! Prints one line per site; the exit code is the number of sites that failed.

use cyber_tls::{Io, MIN_BUFFER, Tls};
use std::io::{Read, Write};
use std::net::TcpStream;
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

/// Now as YYYYMMDDhhmmss (UTC), the format the certificate checks use.
fn now() -> u64 {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let (days, rem) = (s / 86400, s % 86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm, public domain description).
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    ((y * 100 + m) * 100 + d) as u64 * 1_000_000 + (rem / 3600) * 10000 + (rem % 3600 / 60) * 100 + rem % 60
}

fn main() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../boot/initrd/etc/certs");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let ders: Vec<Vec<u8>> = files.iter().filter(|p| p.extension().is_some_and(|e| e == "der")).map(|p| std::fs::read(p).unwrap()).collect();
    let roots: Vec<&[u8]> = ders.iter().map(|d| d.as_slice()).collect();
    println!("{} radacini, ora {}", roots.len(), now());
    let mut failed = 0;
    for host in std::env::args().skip(1) {
        let result = (|| -> Result<String, String> {
            let s = TcpStream::connect((host.as_str(), 443)).map_err(|e| format!("conectare: {e}"))?;
            s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            let mut io = Tcp(s);
            let mut random = [0u8; 64];
            std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut random).unwrap();
            let mut buf = vec![0u8; MIN_BUFFER];
            let mut tls = Tls::connect(&mut io, &host, &roots, now(), &random, &mut buf).map_err(|e| format!("{e:?}"))?;
            tls.send(&mut io, format!("GET / HTTP/1.0\r\nHost: {host}\r\nUser-Agent: CyberUniverse/0.1\r\n\r\n").as_bytes()).map_err(|e| format!("{e:?}"))?;
            let mut chunk = [0u8; 200];
            let n = tls.recv(&mut io, &mut chunk).map_err(|e| format!("{e:?}"))?;
            Ok(String::from_utf8_lossy(&chunk[..n]).lines().next().unwrap_or("").to_string())
        })();
        match result {
            Ok(status) => println!("OK    {host}: {status}"),
            Err(e) => {
                failed += 1;
                println!("PICAT {host}: {e}");
            }
        }
    }
    std::process::exit(failed);
}
