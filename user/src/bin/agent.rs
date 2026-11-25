//! agent: the on-board agent (C-166) — Claude in the OS through the Anthropic API, paid from the monthly API credit.
//! Step 1 (P6-202): the client and the spending guard.
//!   agent ping          a short question ("pong" back): checks key, network, TLS, model, price counting
//!   agent cheltuieli    today's spending, the daily cap, the settings in use
//!   agent <question>    one question, one answer (no tools yet: they come in step 2)
//! Settings: `agent.setari` (lines `name = value`) looked up in /usb, then /disk, then /etc; the folder it is found in is
//! the agent's root: the key is `<root>/claude.cheie` and the agent's own folder `<root>/agent/` (the spending ledger
//! `cheltuieli.txt`). Without a settings file: /usb and the defaults below. The key is read once, never printed, and
//! the right to see it is given up right after (P6-23). Before each request: today's total >= cap -> refused, and the
//! ledger says why. After each answer: its `usage` (input, cache, output tokens) priced and added to the ledger.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::Value;
use cyber_user::println;

#[allow(dead_code)]
#[path = "../claude_api.rs"]
mod claude_api;

cyber_user::entry!(main);
cyber_user::heap!(2 * 1024 * 1024);

/// Default model: the strongest one (C-166: "implicit: Opus").
const MODEL: &str = "claude-opus-5-5";
/// Default prices, in $ per million tokens, kept on the careful side (the cap stops earlier, never later).
/// The real ones go in agent.setari (`pret_intrare`, `pret_iesire`).
const PRICE_IN: &str = "15";
const PRICE_OUT: &str = "75";
/// Default daily cap, in $ (C-166).
const CAP: &str = "40";
/// The ledger is rewritten whole on every request; past this size it is put aside and a new one starts.
const LEDGER_MAX: usize = 384 * 1024;

struct Settings {
    root: String,
    api: claude_api::Config,
    /// Prices in thousandths of a micro-dollar per token (= $ per million tokens x 1000).
    price_in: u64,
    price_out: u64,
    /// Daily cap in micro-dollars.
    cap: u64,
    found: bool,
}

/// "12.5" -> 12_500 with `digits` = 3. None for anything that is not a plain non-negative decimal.
fn decimal(text: &str, digits: u32) -> Option<u64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() && frac.is_empty() || frac.len() > digits as usize {
        return None;
    }
    if !whole.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let w: u64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let mut f: u64 = if frac.is_empty() { 0 } else { frac.parse().ok()? };
    for _ in frac.len()..digits as usize {
        f *= 10;
    }
    w.checked_mul(10u64.pow(digits))?.checked_add(f)
}

/// Micro-dollars as "$ 1.234567".
fn dollars(u: u64) -> String {
    format!("{}.{:06} $", u / 1_000_000, u % 1_000_000)
}

fn settings() -> Settings {
    let mut root = "/usb";
    let mut text = None;
    for r in ["/usb", "/disk", "/etc"] {
        if let Some(t) = claude_api::read_file(&format!("{}/agent.setari", r), 8192) {
            root = r;
            text = Some(t);
            break;
        }
    }
    let mut s = Settings {
        root: root.to_string(),
        api: claude_api::Config {
            model: MODEL.to_string(),
            max_tokens: 2048,
            host: "api.anthropic.com".to_string(),
            port: 443,
            address: None,
            key: format!("{}/claude.cheie", root),
        },
        price_in: decimal(PRICE_IN, 3).unwrap(),
        price_out: decimal(PRICE_OUT, 3).unwrap(),
        cap: decimal(CAP, 6).unwrap(),
        found: text.is_some(),
    };
    let Some(text) = text else { return s };
    for line in core::str::from_utf8(&text).unwrap_or("").lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim());
        if line.starts_with('#') || value.is_empty() {
            continue;
        }
        let bad = || println!("agent: agent.setari: valoare gresita pentru {}: {}", name, value);
        match name {
            "model" => s.api.model = value.to_string(),
            "max_tokens" => match value.parse() {
                Ok(n) if n > 0 => s.api.max_tokens = n,
                _ => bad(),
            },
            "plafon_zi" => match decimal(value, 6) {
                Some(v) => s.cap = v,
                None => bad(),
            },
            "pret_intrare" => match decimal(value, 3) {
                Some(v) => s.price_in = v,
                None => bad(),
            },
            "pret_iesire" => match decimal(value, 3) {
                Some(v) => s.price_out = v,
                None => bad(),
            },
            "cheie" => s.api.key = value.to_string(),
            "host" => s.api.host = value.to_string(),
            "port" => match value.parse() {
                Ok(p) => s.api.port = p,
                _ => bad(),
            },
            "address" => s.api.address = Some(value.to_string()),
            _ => println!("agent: agent.setari: nume necunoscut: {}", name),
        }
    }
    s
}

/// "AAAA-LL-ZZ HH:MM" from the computer's clock ("0000-00-00 00:00" without one).
fn now() -> String {
    match cyber_user::time() {
        Some(t) => format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute),
        None => "0000-00-00 00:00".to_string(),
    }
}

fn ledger_path(s: &Settings) -> String {
    format!("{}/agent/cheltuieli.txt", s.root)
}

/// Today's total, in micro-dollars: the `cost_u=` of every ledger line of today.
fn spent_today(ledger: &str, day: &str) -> u64 {
    let mut total = 0u64;
    for line in ledger.lines().filter(|l| l.starts_with(day)) {
        if let Some(v) = line.split_whitespace().find_map(|w| w.strip_prefix("cost_u=")) {
            total = total.saturating_add(v.parse().unwrap_or(0));
        }
    }
    total
}

fn read_ledger(s: &Settings) -> String {
    claude_api::read_file(&ledger_path(s), LEDGER_MAX * 2).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

fn write_whole(path: &str, data: &[u8]) -> Result<(), String> {
    let r = cyber_user::remove(path);
    let r = if r < 0 && r != cyber_user::E_NOT_FOUND { r } else { cyber_user::create(path, false) };
    let fd = if r < 0 { r } else { cyber_user::open(path) };
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let mut done = 0;
    while done < data.len() {
        let n = cyber_user::write(fd as u64, &data[done..data.len().min(done + 16 * 1024)]);
        if n <= 0 {
            break;
        }
        done += n as usize;
    }
    cyber_user::close(fd);
    if done == data.len() { Ok(()) } else { Err(format!("{}: scriere incompleta", path)) }
}

/// Adds one line to the ledger (the whole file is written again: FAT has no append here). A ledger past
/// LEDGER_MAX is kept under a dated name and the new one starts with today's total carried over.
fn add_to_ledger(s: &Settings, line: &str) {
    let dir = format!("{}/agent", s.root);
    let _ = cyber_user::create(&dir, true); // already there: fine
    let mut ledger = read_ledger(s);
    if ledger.len() > LEDGER_MAX {
        let stamp = now();
        let day = &stamp[..10];
        let old = format!("{}/cheltuieli-{}{}.txt", dir, day.replace('-', ""), stamp[11..].replace(':', ""));
        if cyber_user::rename(&ledger_path(s), &old) >= 0 {
            ledger = format!("{} reportat din {}: cost_u={}\n", stamp, old, spent_today(&ledger, day));
        }
    }
    ledger.push_str(line);
    ledger.push('\n');
    match write_whole(&ledger_path(s), ledger.as_bytes()) {
        Ok(()) => {
            let _ = cyber_user::sync(&dir);
        }
        Err(e) => println!("agent: registrul cheltuielii NU s-a scris: {}", e),
    }
}

/// Today's total, or a refusal (also written to the ledger) when the cap is reached.
fn check_cap(s: &Settings) -> Result<u64, String> {
    let stamp = now();
    let spent = spent_today(&read_ledger(s), &stamp[..10]);
    if spent >= s.cap {
        let why = format!("plafonul zilei atins: {} cheltuiti din {}; cererea refuzata", dollars(spent), dollars(s.cap));
        add_to_ledger(s, &format!("{} PLAFON {}", stamp, why));
        return Err(why);
    }
    Ok(spent)
}

struct Usage {
    input: u64,
    cache: u64,
    output: u64,
    cost: u64,
}

fn usage(s: &Settings, answer: &Value) -> Usage {
    let u = answer.get("usage");
    let n = |k: &str| u.and_then(|u| u.get(k)).and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
    let (input, output) = (n("input_tokens"), n("output_tokens"));
    // Cache writes cost more than plain input and cache reads less; both are priced as plain input here, which
    // is close and keeps one price per direction in the settings.
    let cache = n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    let milli = (input + cache) * s.price_in + output * s.price_out;
    Usage { input, cache, output, cost: milli.div_ceil(1000) }
}

/// One question: cap check, request, cost into the ledger. Returns the answer's text.
fn ask(s: &Settings, kind: &str, question: &str) -> Result<String, String> {
    let before = check_cap(s)?;
    let key = claude_api::key(&s.api)?;
    let messages = [Value::object([("role", Value::str("user")), ("content", Value::str(question))])];
    let answer = claude_api::send(&s.api, &key, &messages, None);
    drop(key);
    let answer = answer?;
    let u = usage(s, &answer);
    let model = answer.get("model").and_then(Value::as_str).unwrap_or(&s.api.model).to_string();
    add_to_ledger(
        s,
        &format!("{} {} intrare={} cache={} iesire={} cost_u={} model={}", now(), kind, u.input, u.cache, u.output, u.cost, model),
    );
    println!(
        "agent: {} simboluri intrare (+{} cache), {} iesire = {}; azi {} din plafonul de {}",
        u.input,
        u.cache,
        u.output,
        dollars(u.cost),
        dollars(before + u.cost),
        dollars(s.cap)
    );
    Ok(claude_api::text_of(&answer))
}

fn main() -> i64 {
    let words: Vec<&str> = cyber_user::args().skip(1).collect();
    let s = settings();
    match words.first().copied() {
        None => {
            println!("folosire: agent ping | agent cheltuieli | agent <intrebare>");
            println!("  cheia in claude.cheie, setarile in agent.setari (pe stick)");
            1
        }
        Some("cheltuieli") => {
            let stamp = now();
            let spent = spent_today(&read_ledger(&s), &stamp[..10]);
            println!("agent: azi ({}) {} din plafonul de {}", &stamp[..10], dollars(spent), dollars(s.cap));
            println!(
                "agent: model {}, preturi {} / {} pe milion de simboluri (intrare / iesire), setari {}",
                s.api.model,
                dollars(s.price_in * 1000),
                dollars(s.price_out * 1000),
                if s.found { format!("{}/agent.setari", s.root) } else { "implicite".to_string() }
            );
            println!("agent: registrul: {}", ledger_path(&s));
            0
        }
        Some("ping") => {
            println!("agent: ping catre {} ({}) ...", s.api.model, s.api.host);
            match ask(&s, "ping", "ping: raspunde doar cu cuvantul pong") {
                Ok(answer) => {
                    println!("agent: raspuns: {}", answer.trim());
                    println!("agent: ping OK");
                    0
                }
                Err(e) => {
                    println!("agent: ping ESUAT: {}", e);
                    1
                }
            }
        }
        Some(_) => {
            // The shell splits at spaces and keeps the quotes: glue the words back and drop the outer quotes.
            let question = words.join(" ");
            let question = question.trim().trim_matches('"').trim();
            if question.is_empty() {
                println!("agent: intrebarea e goala");
                return 1;
            }
            match ask(&s, "intrebare", question) {
                Ok(answer) => {
                    cyber_user::write(1, answer.as_bytes());
                    println!();
                    0
                }
                Err(e) => {
                    println!("agent: {}", e);
                    1
                }
            }
        }
    }
}
