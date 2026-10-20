//! The first user program: says hello and exits with 0.
#![no_std]
#![no_main]

use cyber_user::println;

cyber_user::entry!(main);

fn main() -> i64 {
    println!("hello din ELF");
    0
}
