//! claude <question>: asks Claude through the Anthropic API (Messages, JSON over HTTPS) and prints the answer.
//! The API key is read from /usb/claude.key (one line) and is never printed. /etc/claude.conf (optional, lines
//! `name = value`) can change: model, max_tokens, host, port, address (connect there instead of resolving the
//! host), key (the key file). Exit code 0 when Claude answered, otherwise 1. The client is in src/claude_api.rs.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use cyber_user::{println, write};

#[path = "../claude_api.rs"]
mod claude_api;

cyber_user::entry!(main);
cyber_user::heap!(1024 * 1024);

fn main() -> i64 {
    let question: Vec<&str> = cyber_user::args().skip(1).collect();
    if question.is_empty() {
        println!("folosire: claude <intrebare>   (cheia API in /usb/claude.key)");
        return 1;
    }
    let question = question.join(" ");
    let c = claude_api::config();
    let key = match claude_api::key(&c) {
        Ok(k) => k,
        Err(e) => {
            println!("claude: {}", e);
            return 1;
        }
    };
    println!("claude: cheie gasita; intreb {} ...", c.model);
    match claude_api::ask(&c, &key, &[("user", &question)]) {
        Ok(answer) => {
            write(1, answer.as_bytes());
            println!();
            0
        }
        Err(e) => {
            println!("claude: {}", e);
            1
        }
    }
}
