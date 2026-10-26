//! The first user program: says hello and exits with 0.
//! `hello drepturi` (P6-23, scenario `drepturi`): checks the program permissions from the inside — secrets, visible
//! paths, promises — printing one line per check; exit code 0 only if every check gave the expected answer.
#![no_std]
#![no_main]

use cyber_user::{E_DENIED, E_NOT_FOUND, close, error_text, open, println, rights};

cyber_user::entry!(main);

fn main() -> i64 {
    if cyber_user::args().nth(1) == Some("drepturi") {
        return permissions();
    }
    println!("hello din ELF");
    0
}

/// Opens `path`, prints the result, returns the descriptor or the error (the file is closed again).
fn try_open(what: &str, path: &str) -> i64 {
    let fd = open(path);
    if fd >= 0 {
        println!("drepturi: {}: {} se deschide", what, path);
        close(fd);
    } else {
        println!("drepturi: {}: {}: {}", what, path, error_text(fd));
    }
    fd
}

fn permissions() -> i64 {
    let mut ok = true;
    let mut check = |good: bool| ok &= good;
    // 1. A secret not asked for is invisible; asked for, readable; given up, invisible again for good.
    check(try_open("fara cerere", "/etc/claude.key") == E_NOT_FOUND);
    check(try_open("ocolire", "/ETC//./x/../claude.key") == E_NOT_FOUND);
    check(rights::ask_secret("/etc/claude.key") == 0);
    check(try_open("dupa cerere", "/etc/claude.key") >= 0);
    check(try_open("alt secret", "/etc/github.token") == E_NOT_FOUND);
    check(rights::drop_secrets() == 0);
    check(try_open("dupa renuntare", "/etc/claude.key") == E_NOT_FOUND);
    check(rights::ask_secret("/etc/claude.key") == E_DENIED);
    // 2. Visible paths: only /etc/motd, then the list is closed.
    check(rights::unveil("/etc/motd", rights::READ) == 0 && rights::unveil_close() == 0);
    check(try_open("cale vizibila", "/etc/motd") >= 0);
    check(try_open("cale ascunsa", "/bin/hello") == E_NOT_FOUND);
    check(rights::unveil("/", rights::READ) == E_DENIED);
    // 3. Promises: without `rpath`, opening fails even for a visible path; the mask cannot grow back.
    check(rights::promise(rights::STDIO) == rights::STDIO as i64);
    check(try_open("fara rpath", "/etc/motd") == E_DENIED);
    check(rights::promise(u64::MAX) == rights::STDIO as i64);
    println!("drepturi: {}", if ok { "toate verificarile OK" } else { "GRESIT" });
    if ok { 0 } else { 1 }
}
