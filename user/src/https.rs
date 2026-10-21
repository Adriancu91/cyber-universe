//! HTTPS for programs (P5-16, P6-02, P6-04): opening a TLS 1.3 connection with the certificate checked against the
//! trusted roots in /etc/certs. Used by /bin/fetch and /bin/claude.

use crate::net::{TcpStream, parse_ip, resolve};
use crate::{error_text, println};
use cyber_crypto::x509::{self, TrustStore, subject_id};
use cyber_tls::{Io, Tls};

/// A TCP connection as the "pipe" under TLS.
pub struct Tcp(pub TcpStream);

impl Io for Tcp {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        self.0.read(buf).map_err(|_| ())
    }
    fn write_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.0.write_all(data).map_err(|_| ())
    }
}

/// The trusted roots in /etc/certs (tools/roots.py): only the files named after the issuer we look for are read,
/// so the ~120 roots of the Mozilla list never need to be in memory together.
pub struct EtcCerts;

impl TrustStore for EtcCerts {
    fn find(&mut self, subject: &[u8], f: &mut dyn FnMut(&[u8]) -> bool) -> bool {
        let id = subject_id(subject);
        let mut path = *b"/etc/certs/0123456789abcdef.0";
        path[11..27].copy_from_slice(&id);
        for n in 0..10u8 {
            path[28] = b'0' + n;
            let Ok(p) = core::str::from_utf8(&path) else { return false };
            let fd = crate::open(p);
            if fd < 0 {
                return false;
            }
            let mut der = [0u8; 4096];
            let mut used = 0;
            while used < der.len() {
                let r = crate::read(fd, &mut der[used..]);
                if r <= 0 {
                    break;
                }
                used += r as usize;
            }
            crate::close(fd);
            if f(&der[..used]) {
                return true;
            }
        }
        false
    }
}

/// The real-time clock (taken as UTC) as YYYYMMDDHHMMSS, which decides whether certificates are still valid.
pub fn now() -> Option<u64> {
    let t = crate::time()?;
    Some(
        t.year as u64 * 10_000_000_000
            + t.month as u64 * 100_000_000
            + t.day as u64 * 1_000_000
            + t.hour as u64 * 10_000
            + t.minute as u64 * 100
            + t.second as u64,
    )
}

/// Connects to `host`:`port` (or to `address`, when given; the name must still match the certificate) and opens
/// TLS in `buf` (at least `cyber_tls::MIN_BUFFER` bytes). Problems are printed as "`prog`: …" and give None.
pub fn open<'b>(prog: &str, host: &str, port: u16, address: Option<&str>, buf: &'b mut [u8]) -> Option<(Tcp, Tls<'b>)> {
    let ip = match address {
        Some(a) => match parse_ip(a) {
            Some(ip) => ip,
            None => {
                println!("{}: adresa invalida: {}", prog, a);
                return None;
            }
        },
        None => match resolve(host) {
            Ok(ip) => ip,
            Err(e) => {
                println!("{}: {}: {}", prog, host, error_text(e));
                return None;
            }
        },
    };
    let Some(now) = now() else {
        println!("{}: ceasul nu e disponibil", prog);
        return None;
    };
    let mut random = [0u8; 64];
    if !crate::random(&mut random) {
        println!("{}: numere aleatoare indisponibile", prog);
        return None;
    }
    let mut io = match TcpStream::connect(ip, port, 5000) {
        Ok(c) => Tcp(c),
        Err(e) => {
            println!("{}: {}:{}: {}", prog, host, port, error_text(e));
            return None;
        }
    };
    match Tls::connect(&mut io, host, &mut EtcCerts, now, &random, buf) {
        Ok(t) => Some((io, t)),
        Err(cyber_tls::Error::Certificate(x509::Error::UnknownIssuer)) => {
            println!("{}: TLS refuzat: emitent necunoscut (nicio radacina potrivita in /etc/certs)", prog);
            None
        }
        Err(e) => {
            println!("{}: TLS refuzat: {:?}", prog, e);
            None
        }
    }
}
