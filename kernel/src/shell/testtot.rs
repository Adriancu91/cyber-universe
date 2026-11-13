//! `test tot` (P6-61 step 5, C-84 item 5, plan F9): the OS's self-test on the real computer, by reads only (plus one
//! small file written and read back on the stick): processor, clock, timer, stick, network, video card (PCI ids only).
//! One line per check, `PASS` / `FAIL` / `LIPSA` (not present: not counted as a failure), a final verdict, and the
//! whole text as one file on the stick (`raport/test-tot-AAAALLZZ-<nr>.txt`), then sent to the hub when there is a token.
//! Nothing is written to the computer's internal disks and nothing to the video card.

use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};

#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Pass,
    Fail,
    Missing,
}

struct Check {
    name: &'static str,
    verdict: Verdict,
    detail: String,
}

fn pass(name: &'static str, detail: String) -> Check {
    Check { name, verdict: Verdict::Pass, detail }
}
fn fail(name: &'static str, detail: String) -> Check {
    Check { name, verdict: Verdict::Fail, detail }
}

/// The processor: its name, and the official crypto test vectors (integer arithmetic, SHA-2, ChaCha20, X25519...).
fn processor() -> Check {
    let name = super::cpu_name();
    let start = crate::time::uptime_ms().unwrap_or(0);
    match cyber_crypto::vectors::self_test() {
        Ok(n) => pass("procesor", format!("{}; {} vectori oficiali de calcul trecuti in {} ms", name, n, crate::time::uptime_ms().unwrap_or(0) - start)),
        Err(v) => fail("procesor", format!("{}; vectorul {} a dat alt rezultat", name, v)),
    }
}

/// The clock of the mainboard: present, a plausible date.
fn clock() -> Check {
    match crate::drivers::rtc::now() {
        Some(t) if (2025..=2100).contains(&t.year) => {
            pass("ceas", format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} (ceasul placii de baza)", t.year, t.month, t.day, t.hour, t.minute, t.second))
        }
        Some(t) => fail("ceas", format!("data {:04}-{:02}-{:02} nu e plauzibila (bateria ceasului?)", t.year, t.month, t.day)),
        None => fail("ceas", String::from("ceasul placii de baza nu raspunde")),
    }
}

/// The timer: 200 ms of sleep must take 150..1000 ms of uptime.
fn timer() -> Check {
    let a = crate::time::uptime_ms().unwrap_or(0);
    crate::sched::sleep_ms(200);
    let b = crate::time::uptime_ms().unwrap_or(0);
    let d = b.saturating_sub(a);
    if (150..=1000).contains(&d) {
        pass("temporizator", format!("200 ms de asteptare = {} ms masurati", d))
    } else {
        fail("temporizator", format!("200 ms de asteptare = {} ms masurati (asteptat 150..1000)", d))
    }
}

/// The stick (the first writable report destination): 4 KiB written, read back, removed.
fn stick(mount: Option<&str>) -> Check {
    let Some(m) = mount else {
        return fail("stick", String::from("niciun stick montat cu scriere (lipseste cyber.rw?)"));
    };
    let path = format!("{}/raport/.test-tot.tmp", m);
    let data: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
    let _ = fs::remove(&path);
    let r = fs::create(&path, Kind::File)
        .and_then(|_| fs::write(&path, 0, &data))
        .and_then(|_| fs::flush(&path))
        .and_then(|_| fs::read_all(&path));
    let _ = fs::remove(&path);
    match r {
        Ok(back) if back == data => pass("stick", format!("{}: 4 KiB scrisi, cititi inapoi identic, stersi", m)),
        Ok(_) => fail("stick", format!("{}: ce am citit inapoi nu e ce am scris", m)),
        Err(e) => fail("stick", format!("{}: {}", m, e.message())),
    }
}

/// The network: a card, an address from DHCP (10 s), a name resolved through DNS.
fn network() -> Check {
    if crate::net::config().is_none() {
        return Check { name: "retea", verdict: Verdict::Missing, detail: String::from("nicio placa de retea cu driver") };
    }
    if !crate::net::wait_ready(10_000) {
        return fail("retea", String::from("placa exista, dar DHCP nu a dat adresa in 10 s (cablul?)"));
    }
    let c = crate::net::config().unwrap_or_else(|| unreachable!());
    let ip = format!("{}.{}.{}.{}", c.ip[0], c.ip[1], c.ip[2], c.ip[3]);
    match crate::net::dns::resolve("api.github.com") {
        Ok((a, _)) => pass("retea", format!("adresa {}; api.github.com = {}.{}.{}.{}", ip, a[0], a[1], a[2], a[3])),
        Err(e) => fail("retea", format!("adresa {}, dar DNS nu raspunde: {}", ip, e.message())),
    }
}

/// The video card: NVIDIA on PCI (ids only, from the configuration space already read at start-up).
fn video() -> Check {
    let cards: Vec<String> = crate::pci::devices()
        .iter()
        .filter(|d| d.vendor == 0x10de && d.class == 0x03)
        .map(|d| format!("{:02x}:{:02x}.{} 10de:{:04x}", d.bus, d.dev, d.func, d.device))
        .collect();
    if cards.is_empty() {
        Check { name: "placa video", verdict: Verdict::Missing, detail: String::from("nicio placa NVIDIA pe PCI") }
    } else {
        pass("placa video", format!("NVIDIA {} (doar ID-urile PCI, nimic scris)", cards.join(", ")))
    }
}

/// P6-88 (C-118: "at every test on the laptop, Wi-Fi too"): the Intel Wi-Fi adapter, read only (step W0).
fn wifi() -> Check {
    match crate::drivers::wifi::summary() {
        Some(d) => pass("wifi", d),
        None => Check { name: "wifi", verdict: Verdict::Missing, detail: String::from("nicio placa Wi-Fi Intel pe PCI") },
    }
}

/// `test tot`: runs every check, prints, writes the report on the stick and sends it to the hub.
pub fn run() {
    super::emit("test tot: autotestul OS-ului (doar citiri; un fisier mic de proba pe stick)\n");
    let mount = super::raport::writable_mount();
    let checks = [processor(), clock(), timer(), stick(mount.as_deref()), network(), video(), wifi()];
    let mut text = String::from("== test tot: autotestul Cyber Universe OS ==\n");
    for c in &checks {
        let v = match c.verdict {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Missing => "LIPSA",
        };
        let line = format!("test tot: {} {}: {}\n", v, c.name, c.detail);
        super::emit(&line);
        text.push_str(&line);
    }
    let failed = checks.iter().filter(|c| c.verdict == Verdict::Fail).count();
    let passed = checks.iter().filter(|c| c.verdict == Verdict::Pass).count();
    let verdict = if failed == 0 {
        format!("test tot: REZULTAT: PASS ({} trecute, {} lipsa, 0 esuate)\n", passed, checks.len() - passed)
    } else {
        format!("test tot: REZULTAT: FAIL ({} esuate din {})\n", failed, checks.len())
    };
    super::emit(&verdict);
    text.push_str(&verdict);
    crate::info!("{}", verdict.trim_end());
    match super::raport::write_named("test-tot-", &text) {
        Ok(path) => {
            super::emit(&format!("test tot: raport scris {}\n", path));
            super::raport::send_to_hub();
        }
        Err(e) => super::emit(&format!("test tot: raportul nu s-a scris: {}\n", e)),
    }
}
