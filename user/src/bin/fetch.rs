//! fetch <name> [port] [path]: downloads a web page over plain HTTP (TCP) and prints it (headers removed).
//! Exit code 0 if the server answered 200, otherwise 1.
#![no_std]
#![no_main]

use cyber_user::net::{TcpStream, resolve};
use cyber_user::{error_text, println, write};

cyber_user::entry!(main);

fn main() -> i64 {
    let mut args = cyber_user::args().skip(1);
    let Some(name) = args.next() else {
        println!("folosire: fetch <nume> [port] [cale]");
        return 1;
    };
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
    // The header ends at the first empty line; it is read byte by byte into a small buffer.
    let mut head = [0u8; 1024];
    let mut head_len = 0;
    let mut buf = [0u8; 2048];
    let mut total = 0usize;
    let mut in_body = false;
    let mut ok = false;
    loop {
        let n = match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                println!("\nfetch: primire: {}", error_text(e));
                return 1;
            }
        };
        let mut data = &buf[..n];
        if !in_body {
            let mut i = 0;
            while i < data.len() && !in_body {
                if head_len < head.len() {
                    head[head_len] = data[i];
                    head_len += 1;
                }
                i += 1;
                in_body = head_len >= 4 && &head[head_len - 4..head_len] == b"\r\n\r\n";
            }
            if in_body {
                ok = head.starts_with(b"HTTP/1.0 200") || head.starts_with(b"HTTP/1.1 200");
                if !ok {
                    let line = head.iter().position(|&b| b == b'\r').unwrap_or(head_len);
                    println!("fetch: raspuns: {}", core::str::from_utf8(&head[..line]).unwrap_or("?"));
                }
            }
            data = &data[i..];
        }
        if in_body {
            total += data.len();
            write(1, data);
        }
    }
    println!("-- fetch: {} octeti de la {}:{}", total, name, port);
    if ok { 0 } else { 1 }
}
