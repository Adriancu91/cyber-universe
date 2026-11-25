//! P6-223 (C-166 step 6, part 2): a board command from the agent's channel (`live/os-agent-comenzi.md` in the hub),
//! signed by the command centre with the live channel's key. The agent (a program) never sees the key: it passes the
//! line it read to the shell as
//!
//! ```text
//! agent-placa <nr> <semnatura> test-placa …
//! ```
//!
//! and the kernel checks it here, the same way as a `comanda` of the live channel (`live.rs`):
//!   - the signature: HMAC-SHA-256 with `/usb/hub/live.cheie` over `cyber-agent-1\n<nr>\n<comanda>` (64 hex digits);
//!   - the board key `/usb/hub/permite-live-placa` (put only with Adrian's yes) must be there;
//!   - the number must be larger than every number already taken: the stick journal `/usb/hub/agent-placa.txt`
//!     (kernel-only, like the key) gets `<nr> incepe …` BEFORE the command runs, so a signed line copied again, or put
//!     back to `nou`, never runs twice;
//!   - only `test-placa …` (the forms of `auto/teste.txt`), run by the same `board_test` as the list and the live
//!     channel: Adrian's keys, the slot, the temperature and time guards, the safe stop; the report goes to the hub.
//!
//! Idea: the live channel's gate (C-159), reused for the agent's channel; our own code.

use super::{auto, ciclu, live};
use crate::fs::{self, Kind};
use alloc::{format, string::String};
use core::sync::atomic::Ordering;
use cyber_crypto::sha2::Sha256;

/// The version tag at the start of the signed text.
pub const FORM: &str = "cyber-agent-1";
/// The stick journal of the numbers taken (kernel-only: `proc::rights::KERNEL_ONLY_NAMES`).
pub const JOURNAL: &str = "agent-placa.txt";

/// The hub folder of the stick (`/etc/hub` in the QEMU tests of the agent and of the live channel).
fn dir() -> &'static str {
    #[cfg(feature = "test-agent")]
    return "/etc/hub";
    #[cfg(not(feature = "test-agent"))]
    return super::hubcmd::dir();
}

/// The text that is signed.
pub fn canonical(nr: u32, command: &str) -> String {
    format!("{}\n{}\n{}", FORM, nr, command)
}

/// The signature of a channel line: HMAC-SHA-256 with `key`, 64 lowercase hex digits.
pub fn sign(key: &[u8], nr: u32, command: &str) -> String {
    let mut tag = [0u8; 32];
    cyber_crypto::hmac::hmac::<Sha256>(key, canonical(nr, command).as_bytes(), &mut tag);
    tag.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Equal without an early exit.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The largest number in the journal (lines `<nr> …`), 0 for none.
pub fn last_taken(journal: &str) -> u32 {
    journal.lines().filter_map(|l| l.split_whitespace().next()?.parse::<u32>().ok()).max().unwrap_or(0)
}

/// The gate (no file touched): `Ok` = the parsed board test, ready for `auto::board_test`.
pub fn check(key: &[u8], board_key: bool, journal: &str, nr: &str, sig: &str, command: &str) -> Result<Result<String, &'static str>, String> {
    if key.len() < 32 {
        return Err(String::from("cheia live.cheie lipseste sau e prea scurta (cel putin 32 de octeti)"));
    }
    let Some(nr) = nr.parse::<u32>().ok().filter(|&n| n > 0) else {
        return Err(String::from("numarul comenzii trebuie sa fie un intreg pozitiv"));
    };
    let command = command.trim();
    if !same(sign(key, nr, command).as_bytes(), sig.to_ascii_lowercase().as_bytes()) {
        return Err(String::from("semnatura gresita"));
    }
    let last = last_taken(journal);
    if nr <= last {
        return Err(format!("comanda repetata sau veche: nr {} (ultimul luat: {}); o comanda semnata ruleaza o singura data", nr, last));
    }
    let Some(parsed) = ciclu::board_command(command) else {
        return Err(String::from("prin canalul agentului, semnat, merge doar `test-placa …`"));
    };
    if !board_key {
        return Err(String::from("comanda scrie in placa video: fara cheia permite-live-placa (pusa doar cu da-ul lui Adrian) nu ruleaza"));
    }
    Ok(parsed)
}

fn journal_add(path: &str, line: &str) {
    if fs::kind(path).is_err() {
        let _ = fs::create(path, Kind::File);
    }
    let at = fs::size(path).unwrap_or(0);
    let _ = fs::write(path, at, format!("{}\n", line).as_bytes());
    let _ = fs::flush(path);
}

/// `agent-placa <nr> <semnatura> test-placa …`: prints the board test's report, or `REFUZAT: …` (code 1).
pub fn command(args: &[&str]) {
    let (Some(nr), Some(sig)) = (args.first(), args.get(1)) else {
        super::LAST_EXIT.store(2, Ordering::Relaxed);
        super::emit(&format!("folosire: agent-placa <nr> <semnatura> test-placa … (doar de la agentul de bord, linii semnate de centru)\n"));
        return;
    };
    let text = args[2..].join(" ");
    let d = dir();
    let key = fs::read_all(&format!("{}/live.cheie", d)).unwrap_or_default();
    let board_key = fs::kind(&format!("{}/permite-live-placa", d)).is_ok();
    let path = format!("{}/{}", d, JOURNAL);
    let journal = String::from_utf8(fs::read_all(&path).unwrap_or_default()).unwrap_or_default();
    let parsed = match check(&key, board_key, &journal, nr, sig, &text) {
        Ok(p) => p,
        Err(why) => {
            crate::info!("agent-placa: nr {} REFUZAT: {} ({})", nr, why, text.trim());
            super::LAST_EXIT.store(1, Ordering::Relaxed);
            super::emit(&format!("REFUZAT: {} (nimic nu s-a scris in placa)\n", why));
            return;
        }
    };
    // Taken BEFORE it runs: a crash in the middle never repeats it.
    journal_add(&path, &format!("{} incepe {} {}", nr, auto::now_text(), text.trim()));
    crate::info!("agent-placa: nr {} incepe: {}", nr, text.trim());
    super::LAST_EXIT.store(0, Ordering::Relaxed);
    let report = live::board_command(parsed);
    let refused = super::LAST_EXIT.load(Ordering::Relaxed) != 0;
    journal_add(&path, &format!("{} {} {}", nr, if refused { "refuz" } else { "gata" }, auto::now_text()));
    super::emit(&report);
}

/// The pure checks (QEMU, `test-agent`): the signature as in Python, every refusal, the gate passed.
#[cfg_attr(not(feature = "test-agent"), allow(dead_code))]
pub fn self_test() {
    let key = [7u8; 32];
    let t = "test-placa --bariera";
    // The same signature as tools/agent-semneaza.py (Python's hmac).
    assert!(sign(&key, 6, t) == "ce001163fee895c8e4130d2f382971c605ac4ebcf17f9c64ae05a1c146e05b39", "agent-placa: semnatura ca in Python");
    let good = sign(&key, 6, t);
    let why = |r: Result<Result<String, &'static str>, String>| match r {
        Ok(_) => String::from("TRECE"),
        Err(w) => w,
    };
    assert!(why(check(&key, true, "", "6", &good, t)) == "TRECE", "agent-placa: semnatura buna");
    assert!(why(check(&key, true, "", "6", &good.to_ascii_uppercase(), t)) == "TRECE", "agent-placa: hex mare");
    assert!(why(check(&key, true, "", "6", &good, " test-placa --bariera ")) == "TRECE", "agent-placa: spatiile de la capete");
    assert!(why(check(&key[..16], true, "", "6", &good, t)).starts_with("cheia live.cheie"), "agent-placa: cheie scurta");
    assert!(why(check(&key, true, "", "6", &"0".repeat(64), t)) == "semnatura gresita", "agent-placa: semnatura gresita");
    assert!(why(check(&key, true, "", "7", &good, t)) == "semnatura gresita", "agent-placa: alt numar");
    assert!(why(check(&key, true, "", "6", &good, "test-placa --p8")) == "semnatura gresita", "agent-placa: alt text");
    assert!(why(check(&key, true, "", "x", &good, t)).starts_with("numarul"), "agent-placa: numar");
    assert!(why(check(&key, true, "6 incepe 2026-11-26 t\n6 gata x\n", "6", &good, t)).starts_with("comanda repetata"), "agent-placa: repetata");
    assert!(why(check(&key, true, "9 incepe x\n", "6", &good, t)).starts_with("comanda repetata sau veche"), "agent-placa: veche");
    assert!(why(check(&key, false, "", "6", &good, t)).contains("permite-live-placa"), "agent-placa: fara cheia placii");
    let m = sign(&key, 6, "gpu lant");
    assert!(why(check(&key, true, "", "6", &m, "gpu lant")).contains("doar `test-placa"), "agent-placa: alta comanda");
    let bad = sign(&key, 6, "test-placa --nu-exista");
    assert!(matches!(check(&key, true, "", "6", &bad, "test-placa --nu-exista"), Ok(Err(_))), "agent-placa: forma gresita trece poarta, o refuza board_test");
    assert!(last_taken("3 incepe\n# x\n12 gata\n5 refuz\n") == 12);
    crate::info!("test: agent-placa: poarta (semnatura, numarul, cheia placii, doar test-placa) OK");
}
