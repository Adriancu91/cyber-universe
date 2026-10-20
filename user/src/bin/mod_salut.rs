//! Program of the test module `salut` (modules/salut): says it started, then keeps running (a
//! module that works in the background) until the module manager stops it.
#![no_std]
#![no_main]

use cyber_user::println;

cyber_user::entry!(main);

fn main() -> i64 {
    println!("modul salut: pornit");
    loop {
        cyber_user::sleep_ms(100);
    }
}
