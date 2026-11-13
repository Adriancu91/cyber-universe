//! P6-86 (C-117 step 1): the `analiza` command (also `hw uhl`). Builds the inventory (`crate::hw::uhl`, READ ONLY) and
//! writes it on the stick as `<mount>/raport/uhl/<machine>/<AAAALLZZ-HHMM>/`: `inventory.json` (everything, serials
//! included — it never leaves the stick), `inventory.redacted.json` (serials hidden) and `report.txt`. `trimite` sends
//! only the last two (user/src/bin/gh-trimite.rs).

use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String};

/// `analiza`: build, write, say where. In test builds also reads the files back and checks them.
/// `sim` (`analiza --sim`, test builds with gpu-sim only): the NVIDIA part comes from the gpu-sim model, marked SIMULARE.
pub fn run(sim: bool) {
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
    // P6-87 (C-117 step 2): the NVIDIA board through the read-only tour of Cyber-GPU (`gpu tur`, list C-54, 0 writes).
    let gpu = if sim {
        gpu_sim_tour()
    } else {
        crate::drivers::gpu::display_devices().into_iter().find(|d| d.vendor == 0x10DE).map(|d| {
            super::emit(&format!("analiza: placa NVIDIA {}: turul de citire cyber-gpu (gpu tur, 0 scrieri)...\n", d.address()));
            crate::hw::uhl::GpuTour { address: Some(d.address()), simulated: false, lines: crate::drivers::gpu_tur::tour() }
        })
    };
    // P6-88 (C-118 W0): the Intel Wi-Fi adapter, read only (with --sim: QEMU's edu device stands in).
    let wifi = crate::drivers::wifi::facts(sim);
    let inv = crate::hw::uhl::build(&super::driver_of, &super::cpu_name(), &stamp, &session, gpu.as_ref(), &wifi);
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
    check_files(&dir, sim);
}

#[cfg(feature = "test-gpu-sim")]
fn gpu_sim_tour() -> Option<crate::hw::uhl::GpuTour> {
    Some(crate::hw::uhl::GpuTour { address: None, simulated: true, lines: crate::drivers::gpu_tur::tour_sim() })
}

#[cfg(not(feature = "test-gpu-sim"))]
fn gpu_sim_tour() -> Option<crate::hw::uhl::GpuTour> {
    None
}

/// Test builds: the written files are read back, parsed with our JSON reader and compared with what the kernel sees.
#[cfg(feature = "qemu-test")]
fn check_files(dir: &str, sim: bool) {
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
    if sim {
        let g = comps.iter().find(|c| c.get("kind").and_then(|x| x.as_str()) == Some("gpu_registers")).expect("analiza: componenta gpu");
        let facts = g.get("props").and_then(|p| p.get("facts")).and_then(|p| p.get("value")).expect("analiza: fapte");
        assert!(facts.get("chip.nume").and_then(|x| x.as_str()) == Some("GA104"), "analiza: chip.nume");
        assert!(facts.get("pci.id").and_then(|x| x.as_str()) == Some("10de:2482"), "analiza: pci.id");
        // P6-89 (cyber-gpu 835f24d): the VBIOS version string of the model, as a property of its own.
        let ver = g.get("props").and_then(|p| p.get("vbios_version")).and_then(|p| p.get("value")).and_then(|v| v.as_str()).unwrap_or("");
        assert!(ver.contains('.'), "analiza: vbios_version {:?}", ver);
        crate::info!("test: analiza --sim: vbios_version = {}", ver);
        crate::info!("test: analiza --sim: faptele placii (gpu-sim) in inventar OK");
    }
    // P6-88: one wifi_adapter component per adapter (with --sim: QEMU's edu device, its identification register read).
    let wifi_n = crate::drivers::wifi::adapters(sim).len();
    assert!(kind("wifi_adapter") == wifi_n, "analiza: {} placi Wi-Fi in JSON, {} in nucleu", kind("wifi_adapter"), wifi_n);
    if let Some(w) = comps.iter().find(|c| c.get("kind").and_then(|x| x.as_str()) == Some("wifi_adapter")) {
        let reg = w.get("props").and_then(|p| p.get("csr_hw_if_config")).and_then(|p| p.get("value")).and_then(|v| v.as_str());
        assert!(reg.is_some(), "analiza: registrele placii Wi-Fi lipsesc");
        crate::info!("test: analiza: placa Wi-Fi in inventar OK (HW_IF_CONFIG {})", reg.unwrap_or(""));
    }
    crate::info!(
        "test: analiza: JSON valid, {} componente ({} PCI, {} ACPI), seria ascunsa in varianta de trimis OK",
        comps.len(), pci, kind("acpi_table")
    );
}
