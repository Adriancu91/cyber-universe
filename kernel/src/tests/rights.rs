//! P6-23: program permissions, end to end. A secret file (`/etc/claude.key`, a test copy) is invisible to a program
//! that did not ask for it (`/bin/cat`), and `hello drepturi` checks secrets, visible paths and promises from inside.

use crate::fs;

pub fn run() {
    let _ = fs::create("/etc/claude.key", fs::Kind::File);
    fs::write("/etc/claude.key", 0, b"cheie-de-test-p623\n").expect("test: /etc/claude.key");
    let _ = fs::create("/etc/github.token", fs::Kind::File);
    fs::write("/etc/github.token", 0, b"token-de-test-p623\n").expect("test: /etc/github.token");
    let cat = crate::proc::spawn_path("/bin/cat", &["cat", "/etc/claude.key"]).expect("test: /bin/cat");
    let code = crate::proc::wait(cat);
    assert!(code == 1, "test: cat a citit cheia (cod {})", code);
    let hello = crate::proc::spawn_path("/bin/hello", &["hello", "drepturi"]).expect("test: /bin/hello");
    let code = crate::proc::wait(hello);
    assert!(code == 0, "test: hello drepturi a iesit cu {}", code);
    crate::info!("test: drepturi pe program: cheia ascunsa pentru cat, hello drepturi OK");
}
