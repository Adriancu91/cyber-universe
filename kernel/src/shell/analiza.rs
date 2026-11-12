//! P6-86 (C-117 step 1): the `analiza` command (also `hw uhl`). Builds the inventory (`crate::hw::uhl`, READ ONLY) and
//! writes it on the stick as `<mount>/raport/uhl/<machine>/<AAAALLZZ-HHMM>/`: `inventory.json` (everything, serials
//! included — it never leaves the stick), `inventory.redacted.json` (serials hidden) and `report.txt`. `trimite` sends
//! only the last two (user/src/bin/gh-trimite.rs).

use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String};

/// `analiza`: build, write, say where. In test builds also reads the files back and checks them.
pub fn run() {
    super::emit("analiza: citesc SMBIOS, CPUID, memoria, configuratia PCI (4 KiB pe functie), ACPI, USB - doar citire...\n");
    let (stamp, session) = match crate::drivers::rtc::now() {
        Some(t) => (
            format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second),
            format!("{:04}{:02}{:02}-{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute),
        ),
        None => (String::from("necunoscut"), String::from("fara-data")),
    };
    let Some(mount) = super::raport::writable_mount() else {
        super::emit("analiza: nu am unde scrie (stick-ul lipseste sau nu are cyber.rw); nimic scris\n");
        return;
    };
    let base = format!("{}/raport/uhl", mount);
    let inv = crate::hw::uhl::build(&super::driver_of, &super::cpu_name(), &stamp, &session);
    let machine_dir = format!("{}/{}", base, inv.machine);
    let mut dir = format!("{}/{}", machine_dir, session);
    let made = (|| -> Result<(), String> {
        for d in [base.as_str(), machine_dir.as_str()] {
            match fs::create(d, Kind::Dir) {
                Ok(()) | Err(FsError::AlreadyExists) => {}
                Err(e) => return Err(format!("{}: {}", d, e.message())),
            }
        }
        // A second run in the same minute gets its own folder.
        let first = dir.clone();
        for nr in 2..100 {
            match fs::create(&dir, Kind::Dir) {
                Ok(()) => return Ok(()),
                Err(FsError::AlreadyExists) => dir = format!("{}-{}", first, nr),
                Err(e) => return Err(format!("{}: {}", dir, e.message())),
            }
        }
        Err(format!("{}: prea multe sesiuni in acelasi minut", first))
    })();
    if let Err(why) = made {
        super::emit(&format!("analiza: nu s-a scris: {}\n", why));
        return;
    }
    // The secrets filter of the reports also runs on the two files that may leave the stick (keys never belong there).
    let secrets = super::raport::secrets();
    let files = [
        ("inventory.json", inv.full.clone()),
        ("inventory.redacted.json", super::raport::hide_secrets(&inv.redacted, &secrets)),
        ("report.txt", super::raport::hide_secrets(&inv.report, &secrets)),
    ];
    for (name, text) in &files {
        let path = format!("{}/{}", dir, name);
        let r = fs::create(&path, Kind::File)
            .and_then(|_| fs::write(&path, 0, text.as_bytes()))
            .and_then(|_| fs::flush(&path));
        match r {
            Ok(_) => crate::info!("analiza: {} scris ({} octeti)", path, text.len()),
            Err(e) => {
                super::emit(&format!("analiza: {}: {}\n", path, e.message()));
                return;
            }
        }
    }
    super::emit(&inv.report);
    super::emit(&format!(
        "analiza: scris {}/ (inventory.json {} KiB - ramane pe stick; inventory.redacted.json {} KiB + report.txt - pleaca cu `trimite`)\n",
        dir, inv.full.len() / 1024, inv.redacted.len() / 1024
    ));
    #[cfg(feature = "qemu-test")]
    check_files(&dir);
}

/// Test builds: the written files are read back, parsed with our JSON reader and compared with what the kernel sees.
#[cfg(feature = "qemu-test")]
fn check_files(dir: &str) {
    let read = |n: &str| String::from_utf8(fs::read_all(&format!("{}/{}", dir, n)).expect("analiza: fisier lipsa")).expect("utf8");
    let full = cyber_json::parse_limited(&read("inventory.json"), 8 << 20).expect("analiza: inventory.json nu e JSON valid");
    let red_text = read("inventory.redacted.json");
    let red = cyber_json::parse_limited(&red_text, 8 << 20).expect("analiza: inventory.redacted.json nu e JSON valid");
    assert!(full.get("schema").and_then(|s| s.as_str()) == Some(crate::hw::uhl::SCHEMA), "analiza: schema");
    assert!(red.get("redacted").and_then(|b| b.as_bool()) == Some(true), "analiza: redacted");
    let comps = full.get("components").and_then(|c| c.as_array()).expect("components");
    let kind = |k: &str| comps.iter().filter(|c| c.get("kind").and_then(|x| x.as_str()) == Some(k)).count();
    let pci = crate::pci::devices().len();
    assert!(kind("pci_function") == pci, "analiza: {} functii PCI in JSON, {} in nucleu", kind("pci_function"), pci);
    assert!(kind("acpi_table") > crate::acpi::tables().len(), "analiza: tabelele ACPI + DSDT");
    let writes = full.get("summary").and_then(|s| s.get("device_writes")).and_then(|w| w.as_i64());
    assert!(writes == Some(0), "analiza: scrieri");
    // Every PCI function carries its whole configuration space.
    for c in comps.iter().filter(|c| c.get("kind").and_then(|x| x.as_str()) == Some("pci_function")) {
        let h = c.get("props").and_then(|p| p.get("config_space_hex")).and_then(|p| p.get("value")).and_then(|v| v.as_str()).unwrap_or("");
        assert!(h.len() == 8192, "analiza: configuratia are {} cifre hex", h.len());
    }
    // The fake serial of the test machine (scenario `analiza`: -smbios type=1,serial=SERIE-TEST-4242).
    let full_text = read("inventory.json");
    assert!(full_text.contains("SERIE-TEST-4242"), "analiza: seria lipseste din inventory.json");
    assert!(!red_text.contains("SERIE-TEST-4242") && red_text.contains(crate::hw::uhl::HIDDEN), "analiza: seria NU e ascunsa");
    assert!(read("report.txt").contains("0 scrieri in dispozitive"), "analiza: report.txt");
    crate::info!(
        "test: analiza: JSON valid, {} componente ({} PCI, {} ACPI), seria ascunsa in varianta de trimis OK",
        comps.len(), pci, kind("acpi_table")
    );
}
