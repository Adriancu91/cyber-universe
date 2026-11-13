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
            let plain = crate::hw::uhl::GpuTour {
                address: Some(d.address()),
                simulated: false,
                lines: crate::drivers::gpu_tur::tour(),
                from_awake: None,
                plain_reason: None,
            };
            // P6-90: Memory Space off (the laptop) -> the facts of an earlier `gpu tur --trezire` of this power-on.
            let t = crate::hw::uhl::pick_tour(plain, crate::drivers::gpu_tur::last_awake(Some(&d.address())));
            if let Some(when) = &t.from_awake {
                super::emit(&format!("analiza: placa NVIDIA: faptele din gpu tur --trezire de la {} (analiza nu trezeste placa)\n", when));
            }
            t
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
    Some(crate::hw::uhl::GpuTour { address: None, simulated: true, lines: crate::drivers::gpu_tur::tour_sim(), from_awake: None, plain_reason: None })
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
        check_pick_tour();
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

/// P6-90: the choice between the plain tour of `analiza` and an earlier `gpu tur --trezire` (laptop, Memory Space off).
#[cfg(feature = "qemu-test")]
fn check_pick_tour() {
    use crate::drivers::gpu_tur::AwakeTour;
    use crate::hw::uhl::{pick_tour, tour_facts, GpuTour};
    use alloc::{string::ToString, vec};
    let plain = |addr: &str, lines: &[&str]| GpuTour {
        address: Some(addr.to_string()),
        simulated: false,
        lines: lines.iter().map(|l| l.to_string()).collect(),
        from_awake: None,
        plain_reason: None,
    };
    let off = ["gpu0: 01:00.0 GeForce RTX 3060 Laptop", "gpu0: Memory Space oprit de firmware"];
    let awake = |addr: &str| AwakeTour {
        address: Some(addr.to_string()),
        when: "07:38:10".to_string(),
        lines: vec![
            "fapt chip.nume = GA106".to_string(),
            "fapt i6.vram = 6144 MiB".to_string(),
            "gpu0: scrieri: 2 in configuratia PCI (registrul Command, trezirea C-116), 0 in registrele placii; 0 cereri refuzate de adaptor".to_string(),
        ],
    };
    // Memory Space off + an awake tour of the same board -> its facts, marked.
    let t = pick_tour(plain("01:00.0", &off), Some(awake("01:00.0")));
    assert!(t.from_awake.as_deref() == Some("07:38:10") && tour_facts(&t.lines).len() == 2, "analiza: turul treaz nefolosit");
    assert!(t.plain_reason.as_deref().is_some_and(|r| r.contains("Memory Space")), "analiza: motivul turului simplu");
    // Another board's awake tour, or no awake tour -> the plain one stays.
    assert!(pick_tour(plain("02:00.0", &off), Some(awake("01:00.0"))).from_awake.is_none(), "analiza: alta placa");
    assert!(pick_tour(plain("01:00.0", &off), None).from_awake.is_none(), "analiza: fara tur treaz");
    // The plain tour has facts (the workstation) -> it wins.
    let t = pick_tour(plain("01:00.0", &["fapt chip.nume = GA104"]), Some(awake("01:00.0")));
    assert!(t.from_awake.is_none(), "analiza: turul simplu cu fapte trebuie pastrat");
    // The component: the source and the writes of the wake-up, in the JSON and the report.
    let t = pick_tour(plain("01:00.0", &off), Some(awake("01:00.0")));
    let inv = crate::hw::uhl::build(&|_| "-", "test", "2026-11-13T07:41:00", "20261113-0741", Some(&t), &[]);
    assert!(inv.full.contains("gpu tur --trezire din aceeasi pornire (07:38:10)"), "analiza: sursa faptelor");
    assert!(inv.full.contains("0 in registrele placii; 2 in registrul Command PCI"), "analiza: scrierile trezirii");
    assert!(inv.report.contains("2 fapte | cip GA106 | VRAM 6144 MiB") && inv.report.contains("faptele placii: din gpu tur --trezire"), "analiza: raport {}", inv.report);
    crate::info!("test: analiza: faptele din gpu tur --trezire (Memory Space oprit) in inventar OK");
}
