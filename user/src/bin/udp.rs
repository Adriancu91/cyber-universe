//! udp <address> <port> <text>: sends the text in one UDP datagram and prints the answer (waits 3 s).
#![no_std]
#![no_main]

use cyber_user::net::{UdpSocket, resolve};
use cyber_user::{error_text, println};

cyber_user::entry!(main);

fn main() -> i64 {
    let mut args = cyber_user::args().skip(1);
    let (Some(name), Some(port), Some(text)) = (args.next(), args.next(), args.next()) else {
        println!("folosire: udp <adresa> <port> <text>");
        return 1;
    };
    let Ok(port) = port.parse::<u16>() else {
        println!("udp: port invalid");
        return 1;
    };
    let ip = match resolve(name) {
        Ok(ip) => ip,
        Err(e) => {
            println!("udp: {}: {}", name, error_text(e));
            return 1;
        }
    };
    let sock = match UdpSocket::bind(0) {
        Ok(s) => s,
        Err(e) => {
            println!("udp: {}", error_text(e));
            return 1;
        }
    };
    if let Err(e) = sock.send_to(text.as_bytes(), ip, port) {
        println!("udp: trimitere: {}", error_text(e));
        return 1;
    }
    let mut buf = [0u8; 1500];
    match sock.recv_from(&mut buf, 3000) {
        Ok((n, from, from_port)) => {
            let reply = core::str::from_utf8(&buf[..n]).unwrap_or("<date binare>");
            println!("udp: raspuns de la {}.{}.{}.{}:{}: {}", from[0], from[1], from[2], from[3], from_port, reply);
            0
        }
        Err(e) => {
            println!("udp: {}", error_text(e));
            1
        }
    }
}
