//! cat <file>...: prints the files one after another. Exit code 1 if one of them cannot be read.
#![no_std]
#![no_main]

use cyber_user::{close, error_text, open, println, read, write};

cyber_user::entry!(main);

fn main() -> i64 {
    let mut status = 0;
    for path in cyber_user::args().skip(1) {
        let fd = open(path);
        if fd < 0 {
            println!("cat: {}: {}", path, error_text(fd));
            status = 1;
            continue;
        }
        let mut buf = [0u8; 512];
        loop {
            let n = read(fd, &mut buf);
            if n <= 0 {
                if n < 0 {
                    println!("cat: {}: {}", path, error_text(n));
                    status = 1;
                }
                break;
            }
            write(1, &buf[..n as usize]);
        }
        close(fd);
    }
    status
}
