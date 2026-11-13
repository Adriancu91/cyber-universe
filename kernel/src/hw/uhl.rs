//! P6-86 (C-117 step 1): `analiza` — the hardware inventory in the shape of Adrian's UHL Portable (`uhl-inventory/0.1`),
//! seen from under every driver: SMBIOS, CPUID, the memory map, the WHOLE configuration space of every PCI function
//! (BARs by index, capabilities and extended capabilities, Resizable BAR, ASPM, D-states, PCIe links), every ACPI table
//! instance, the enumerated USB devices. READ ONLY: no configuration write, no BAR sizing, no MMIO, no power change.
//!
//! Every value is a property `{value, status, source_refs}`; serial numbers and UUIDs carry `sensitive: true` and are
//! replaced with `[ascuns]` in the redacted variant (the only one that leaves the stick). The NVIDIA registers
//! (BOOT_0, GFW, VBIOS headers, VRAM, WPR2) are step 2 (P6-87) and are marked NOT_TESTED here.
//!
//! Written from: PCI Local Bus 3.0 and PCI Express Base 4.0/5.0 (configuration header, capability and extended
//! capability lists, PCI Express capability, Link Capabilities/Control/Status, Resizable BAR, Power Management,
//! Device Serial Number), ACPI 6.5 chapter 5 (table header, FADT DSDT pointers), DMTF SMBIOS DSP0134 3.x (types 0, 1, 2,
//! 3, 4, 17), the public Intel/AMD CPUID documentation. The schema fields follow C-117 (UHL is Adrian's own tool).

use crate::pci::Device;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use alloc::string::ToString;
use cyber_json::Value as J;


/// Builds the props list with less noise: `props!(v; "key" => prop, ...)`.
macro_rules! props {
    ($v:ident; $($k:expr => $e:expr),* $(,)?) => {{ $( $v.push((String::from($k), $e)); )* }};
}

pub const SCHEMA: &str = "uhl-inventory/0.1";
pub const HIDDEN: &str = "[ascuns]";

/// The statuses of UHL, with what they mean here.
const STATUSES: &[(&str, &str)] = &[
    ("CONFIRMED", "citit si verificat de a doua sursa (ex. nucleele din MADT = nucleele pornite)"),
    ("MEASURED", "citit direct din hardware sau din tabelele firmware-ului, de sub orice driver"),
    ("DOCUMENTED", "din documentatie publica (tabelul nostru de nume, specificatii)"),
    ("INFERRED", "dedus din alte valori"),
    ("UNKNOWN", "nu s-a putut afla"),
    ("UNSUPPORTED", "nu exista pe acest calculator / dispozitiv"),
    ("NOT_TESTED", "nu s-a citit intentionat (ar cere o scriere sau e pasul urmator)"),
];

const SOURCES: &[(&str, &str, &str)] = &[
    ("smbios", "firmware-table", "Tabelele SMBIOS ale firmware-ului (DMTF DSP0134 3.x), prin Limine"),
    ("acpi", "firmware-table", "Tabelele ACPI (RSDP -> XSDT/RSDT, FADT -> DSDT), ACPI 6.5 cap. 5"),
    ("pci-config", "hardware", "Spatiul de configurare PCI prin ECAM (MCFG), 4 KiB pe functie, doar citire"),
    ("pcie-spec", "specification", "PCI Local Bus 3.0 / PCI Express Base 4.0-5.0 (decodarea registrelor)"),
    ("cpuid", "hardware", "Instructiunea CPUID (documentatia publica Intel / AMD)"),
    ("madt", "firmware-table", "ACPI MADT (procesoarele) si procesoarele pornite de nucleu"),
    ("limine-memmap", "bootloader", "Harta de memorie data de Limine"),
    ("xhci", "hardware", "Dispozitivele USB enumerate de driverul XHCI al Cyber Universe"),
    ("cu-names", "documentation", "Tabelul de nume al Cyber Universe (kernel/src/hw/names.rs), scris dupa paginile publice"),
    ("cu-drivers", "software", "Driverele Cyber Universe pornite la boot"),
    ("cyber-gpu", "hardware", "Turul de citire Cyber-GPU (lista sigura C-54): registrele placii NVIDIA prin adaptorul care refuza orice scriere"),
    ("gpu-sim", "simulation", "Modelele gpu-sim ale Cyber-GPU (doar in build-urile de test; NU e placa reala)"),
    ("iwlwifi-csr", "documentation", "Registrele CSR ale placilor Wi-Fi Intel (iwl-csr.h din iwlwifi, GPL-2.0 OR BSD-3, citit ca specificatie)"),
    ("qemu-edu", "simulation", "Dispozitivul QEMU edu in locul placii Wi-Fi (doar in build-urile de test; NU e placa reala)"),
];

/// P6-87 (C-117 step 2): the read-only tour of the NVIDIA board (`gpu tur`), given by the shell.
pub struct GpuTour {
    /// PCI address of the board ("00:01.0"); None for the simulation.
    pub address: Option<String>,
    pub simulated: bool,
    pub lines: Vec<String>,
}

/// `fapt <key> = <value>` lines -> (key, value); the order is kept.
pub fn tour_facts(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|l| {
            let rest = &l[l.find("fapt ")? + 5..];
            let (k, v) = rest.split_once(" = ")?;
            Some((String::from(k.trim()), String::from(v.trim())))
        })
        .collect()
}

fn gpu_component(t: &GpuTour) -> J {
    let refs: &[&str] = if t.simulated { &["gpu-sim"] } else { &["cyber-gpu"] };
    let facts = tour_facts(&t.lines);
    let state = if facts.is_empty() { "UNKNOWN" } else { "MEASURED" };
    // The tour's own count: "0 scrieri in placa (..." or "... cerut N scrieri, toate REFUZATE de adaptor".
    let refused = t.lines.iter().find(|l| l.contains("0 scrieri in placa")).map(|l| {
        l.split_whitespace().zip(l.split_whitespace().skip(1)).find(|(_, b)| *b == "scrieri,").and_then(|(a, _)| a.parse::<u64>().ok()).unwrap_or(0)
    });
    let mut v = Vec::new();
    props!(v;
        "simulated" => p(J::Bool(t.simulated), "MEASURED", refs),
        "facts" => p(J::Object(facts.iter().map(|(k, x)| (k.clone(), J::String(x.clone()))).collect()), state, refs),
        "fact_count" => p(int(facts.len() as u64), state, refs),
        "writes_to_board" => match refused {
            Some(r) => p(J::String(format!("0 (adaptorul a refuzat {} cereri de scriere)", r)), "CONFIRMED", refs),
            None => p(J::Null, "UNKNOWN", refs),
        },
        "tour_lines" => p(J::Array(t.lines.iter().map(|l| J::String(l.clone())).collect()), "MEASURED", refs),
        "vbios_image" => not_tested("imaginea VBIOS nu se copiaza in inventar (doar antetele, din fapte)"),
    );
    if facts.is_empty() {
        props!(v; "why_no_facts" => p(J::String(t.lines.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default()), "MEASURED", refs));
    }
    let id = match &t.address {
        Some(a) => format!("gpu:0000:{}", a),
        None => String::from("gpu:simulare"),
    };
    let name = facts.iter().find(|(k, _)| k == "chip.nume").map(|(_, x)| format!("NVIDIA {}", x)).unwrap_or_else(|| String::from("NVIDIA"));
    component(id, "gpu_registers", if t.simulated { format!("{} (SIMULARE gpu-sim)", name) } else { name }, v)
}

/// P6-88 (C-118 step W0): the Intel Wi-Fi adapter, read only (`wifi identifica`).
fn wifi_component(f: &crate::drivers::wifi::Facts) -> J {
    use crate::drivers::wifi::{self, CSR_GP_CNTRL, CSR_HW_IF_CONFIG, CSR_HW_REV, CSR_HW_RF_ID, CSR_RESET};
    let hw: &[&str] = if f.simulated { &["qemu-edu"] } else { &["pci-config"] };
    let csr_refs: &[&str] = if f.simulated { &["qemu-edu"] } else { &["iwlwifi-csr"] };
    let mut v = Vec::new();
    props!(v;
        "simulated" => p(J::Bool(f.simulated), "MEASURED", hw),
        "pci_address" => p(J::String(f.address.clone()), "MEASURED", hw),
        "model" => match wifi::model_name(f.vendor, f.device) {
            Some(m) => p(s(m), "DOCUMENTED", &["cu-names"]),
            None => p(J::Null, "UNKNOWN", &["cu-names"]),
        },
        "integrated_cnvi" => p(J::Bool(f.integrated), "INFERRED", hw),
        "command" => p(hex(f.command as u64, 4), "MEASURED", hw),
        "power_state" => match f.power_state {
            Some(d) => p(J::String(format!("D{}", d)), "MEASURED", hw),
            None => p(J::Null, "UNKNOWN", hw),
        },
        "capability_ids" => p(J::Array(f.capabilities.iter().map(|c| hex(*c as u64, 2)).collect()), "MEASURED", hw),
    );
    match &f.csr {
        Ok(c) => {
            let (mt, mn) = wifi::mac_type(c.get(CSR_HW_REV));
            let (rt, rn) = wifi::rf_type(c.get(CSR_HW_RF_ID));
            props!(v;
                "csr_hw_if_config" => p(hex(c.get(CSR_HW_IF_CONFIG) as u64, 8), "MEASURED", csr_refs),
                "csr_reset" => p(hex(c.get(CSR_RESET) as u64, 8), "MEASURED", csr_refs),
                "csr_gp_cntrl" => p(hex(c.get(CSR_GP_CNTRL) as u64, 8), "MEASURED", csr_refs),
                "csr_hw_rev" => p(hex(c.get(CSR_HW_REV) as u64, 8), "MEASURED", csr_refs),
                "csr_hw_rf_id" => p(hex(c.get(CSR_HW_RF_ID) as u64, 8), "MEASURED", csr_refs),
                "mac_type" => p(J::String(format!("{:#05x} {}", mt, mn.unwrap_or("?"))), "INFERRED", csr_refs),
                "rf_type" => p(J::String(format!("{:#05x} {}", rt, rn.unwrap_or("?"))), "INFERRED", csr_refs),
                "gp_cntrl_bits" => p(J::String(wifi::gp_cntrl_text(c.get(CSR_GP_CNTRL))), "INFERRED", csr_refs),
            );
        }
        Err(why) => {
            props!(v; "csr_registers" => not_tested(why));
        }
    }
    props!(v;
        "firmware" => not_tested("pasul W1 (firmware-ul il descarca Adrian; nu se desface)"),
        "writes_to_board" => p(int(0), "CONFIRMED", hw),
    );
    let name = format!("{:04x}:{:04x} {}", f.vendor, f.device, wifi::model_name(f.vendor, f.device).unwrap_or("Wi-Fi"));
    component(format!("wifi:0000:{}", f.address), "wifi_adapter", if f.simulated { format!("{} (SIMULARE qemu edu)", name) } else { name }, v)
}

/// One property.
fn p(value: J, status: &str, refs: &[&str]) -> J {
    J::object([
        ("value", value),
        ("status", J::str(status)),
        ("source_refs", J::Array(refs.iter().map(|r| J::str(r)).collect())),
    ])
}

/// A sensitive property (serial numbers, UUID): hidden in the redacted variant. `masked` = what the redacted
/// variant shows instead of `[ascuns]` (e.g. the configuration space with the serial number bytes zeroed).
fn sensitive(value: J, status: &str, refs: &[&str], masked: Option<J>) -> J {
    let J::Object(mut pairs) = p(value, status, refs) else { unreachable!() };
    pairs.push((String::from("sensitive"), J::Bool(true)));
    if let Some(m) = masked {
        pairs.push((String::from("redacted_value"), m));
    }
    J::Object(pairs)
}

fn s(v: &str) -> J {
    J::str(v)
}

fn hex(v: u64, digits: usize) -> J {
    J::String(format!("0x{:0w$x}", v, w = digits))
}

fn int(v: u64) -> J {
    J::int(v as i64)
}

fn not_tested(why: &str) -> J {
    p(s(why), "NOT_TESTED", &[])
}

fn component(id: String, kind: &str, name: String, props: Vec<(String, J)>) -> J {
    J::Object(vec![
        (String::from("id"), J::String(id)),
        (String::from("kind"), s(kind)),
        (String::from("name"), J::String(name)),
        (String::from("props"), J::Object(props)),
    ])
}


/// What the shell writes: the full inventory, the redacted one, the short report, and the folder name of the machine.
pub struct Inventory {
    pub full: String,
    pub redacted: String,
    pub report: String,
    pub machine: String,
}

/// Builds the inventory. `driver_of` = which of our drivers handles a function; `stamp` = "AAAA-LL-ZZTHH:MM:SS".
pub fn build(
    driver_of: &dyn Fn(&Device) -> &'static str,
    cpu_name: &str,
    stamp: &str,
    session: &str,
    gpu: Option<&GpuTour>,
    wifi: &[crate::drivers::wifi::Facts],
) -> Inventory {
    let smb = smbios_structs();
    let machine = machine_name(&smb);
    let mut comps: Vec<J> = Vec::new();
    smbios_components(&mut comps, &smb);
    comps.push(cpu_component(cpu_name));
    comps.push(memory_component());
    let pci = crate::pci::devices();
    for d in &pci {
        let toured = gpu.is_some_and(|g| g.address.as_deref() == Some(d.address().as_str()));
        comps.push(pci_component(d, driver_of, toured));
    }
    if let Some(g) = gpu {
        comps.push(gpu_component(g));
    }
    for w in wifi {
        comps.push(wifi_component(w));
    }
    let acpi_count = acpi_components(&mut comps);
    let usb = crate::drivers::usb::devices();
    for u in &usb {
        let mut v = Vec::new();
        props!(v;
            "vendor_id" => p(hex(u.vendor as u64, 4), "MEASURED", &["xhci"]),
            "product_id" => p(hex(u.product as u64, 4), "MEASURED", &["xhci"]),
            "vendor_name" => match super::names::usb_vendor(u.vendor) {
                Some(n) => p(s(n), "DOCUMENTED", &["cu-names"]),
                None => p(J::Null, "UNKNOWN", &["cu-names"]),
            },
            "type" => p(s(u.kind()), "MEASURED", &["xhci"]),
            "class" => p(J::String(format!("{:02x}/{:02x}/{:02x}", u.class.0, u.class.1, u.class.2)), "MEASURED", &["xhci"]),
            "speed" => p(s(super::names::usb_speed(u.speed)), "MEASURED", &["xhci"]),
            "port" => p(J::String(u.location()), "MEASURED", &["xhci"]),
            "serial_number" => not_tested("descriptorul de text al seriei nu se citeste"),
        );
        comps.push(component(format!("usb:{}", u.location()), "usb_device", format!("{:04x}:{:04x} {}", u.vendor, u.product, u.kind()), v));
    }

    let mut doc = J::Object(vec![
        (String::from("schema"), s(SCHEMA)),
        (String::from("tool"), J::object([
            ("name", s("Cyber Universe OS - analiza (C-117)")),
            ("version", s(env!("CARGO_PKG_VERSION"))),
            ("cyber_gpu", s(crate::drivers::gpu::CYBER_GPU_REV)),
            ("environment", s("Cyber Universe OS, de sub orice driver (fara Windows), doar citire")),
        ])),
        (String::from("generated_at"), s(stamp)),
        (String::from("clock_note"), s("ceasul calculatorului (RTC), fara fus orar")),
        (String::from("session"), s(session)),
        (String::from("machine"), J::String(machine.clone())),
        (String::from("redacted"), J::Bool(false)),
        (String::from("statuses"), J::Array(STATUSES.iter().map(|(k, m)| J::object([("status", s(k)), ("meaning", s(m))])).collect())),
        (String::from("sources"), J::Array(SOURCES.iter().map(|(id, kind, t)| J::object([("id", s(id)), ("kind", s(kind)), ("title", s(t))])).collect())),
        (String::from("components"), J::Array(comps)),
    ]);
    let mut counts = [0usize; 7];
    count_statuses(&doc, &mut counts);
    let device_writes = 0;
    if let J::Object(pairs) = &mut doc {
        pairs.push((String::from("summary"), J::object([
            ("pci_functions", int(pci.len() as u64)),
            ("acpi_tables", int(acpi_count as u64)),
            ("usb_devices", int(usb.len() as u64)),
            ("smbios_structures", int(smb.as_ref().map_or(0, |t| t.1.len()) as u64)),
            ("status_counts", J::Object(STATUSES.iter().zip(counts).map(|((k, _), c)| (String::from(*k), int(c as u64))).collect())),
            ("device_writes", int(device_writes)),
        ])));
    }
    let full = strip_redacted_values(&doc).to_string();
    let redacted = redact(&doc).to_string();
    let report = report_text(&doc, &machine, stamp, session, &counts);
    Inventory { full, redacted, report, machine }
}

// ---------------------------------------------------------------- SMBIOS

/// One SMBIOS structure: type, handle, the formatted area and its strings (raw, printable ASCII).
pub struct Smb {
    pub kind: u8,
    pub f: Vec<u8>,
    pub strings: Vec<String>,
}

impl Smb {
    fn byte(&self, o: usize) -> Option<u8> {
        self.f.get(o).copied()
    }
    fn word(&self, o: usize) -> Option<u16> {
        Some(u16::from_le_bytes([*self.f.get(o)?, *self.f.get(o + 1)?]))
    }
    fn dword(&self, o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.f.get(o..o + 4)?.try_into().ok()?))
    }
    /// The string whose index is at offset `o` (None when absent or 0).
    fn text(&self, o: usize) -> Option<&str> {
        let i = self.byte(o)? as usize;
        (i > 0).then(|| self.strings.get(i - 1).map(|s| s.as_str())).flatten().filter(|s| !s.trim().is_empty())
    }
}

/// Walks every structure of a raw table (pure: also used by the self-test).
pub fn smbios_walk(t: &[u8]) -> Vec<Smb> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 4 <= t.len() && out.len() < 1024 {
        let kind = t[off];
        let len = t[off + 1] as usize;
        if len < 4 || off + len > t.len() {
            break;
        }
        let f = t[off..off + len].to_vec();
        let mut end = off + len;
        let mut strings = Vec::new();
        let mut complete = false;
        while end < t.len() {
            let start = end;
            while end < t.len() && t[end] != 0 {
                end += 1;
            }
            if end >= t.len() {
                break;
            }
            if end == start {
                end += 1;
                if strings.is_empty() && end < t.len() && t[end] == 0 {
                    end += 1;
                }
                complete = true;
                break;
            }
            strings.push(t[start..end].iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' }).collect::<String>().trim().into());
            end += 1;
        }
        if !complete {
            break;
        }
        out.push(Smb { kind, f, strings });
        if kind == 127 {
            break;
        }
        off = end;
    }
    out
}

fn smbios_structs() -> Option<((u8, u8), Vec<Smb>)> {
    let (version, raw) = super::smbios::raw()?;
    Some((version, smbios_walk(&raw)))
}

/// Folder-safe name of the machine: "<manufacturer>-<product>", letters, digits and '-', at most 48 characters.
fn machine_name(smb: &Option<((u8, u8), Vec<Smb>)>) -> String {
    let mut name = String::new();
    if let Some((_, list)) = smb {
        if let Some(t1) = list.iter().find(|x| x.kind == 1) {
            let parts: Vec<&str> = [t1.text(4), t1.text(5)].into_iter().flatten().collect();
            name = parts.join(" ");
        }
    }
    folder_name(&name)
}

/// Letters and digits kept, every other run of characters becomes one '-'; empty -> "necunoscut".
pub fn folder_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = String::from(out.trim_end_matches('-'));
    if out.is_empty() { String::from("necunoscut") } else { out }
}

fn smbios_components(comps: &mut Vec<J>, smb: &Option<((u8, u8), Vec<Smb>)>) {
    let Some((version, list)) = smb else {
        comps.push(component(String::from("system"), "system", String::from("calculator (SMBIOS absent)"), vec![
            (String::from("smbios"), p(J::Null, "UNSUPPORTED", &["smbios"])),
        ]));
        return;
    };
    let txt = |x: &Smb, o: usize| match x.text(o) {
        Some(t) => p(s(t), "MEASURED", &["smbios"]),
        None => p(J::Null, "UNKNOWN", &["smbios"]),
    };
    let secret = |x: &Smb, o: usize| match x.text(o) {
        Some(t) => sensitive(s(t), "MEASURED", &["smbios"], None),
        None => p(J::Null, "UNKNOWN", &["smbios"]),
    };
    // Type 1: the system.
    let mut v = Vec::new();
    props!(v; "smbios_version" => p(J::String(format!("{}.{}", version.0, version.1)), "MEASURED", &["smbios"]));
    let mut name = String::from("calculator");
    if let Some(t1) = list.iter().find(|x| x.kind == 1) {
        name = [t1.text(4), t1.text(5)].into_iter().flatten().collect::<Vec<_>>().join(" ");
        props!(v;
            "manufacturer" => txt(t1, 4),
            "product_name" => txt(t1, 5),
            "version" => txt(t1, 6),
            "serial_number" => secret(t1, 7),
            "sku" => txt(t1, 0x19),
            "family" => txt(t1, 0x1A),
        );
        if let Some(u) = t1.f.get(8..24) {
            let uuid = u.iter().map(|b| format!("{:02x}", b)).collect::<String>();
            let state = if u.iter().all(|&b| b == 0) || u.iter().all(|&b| b == 0xFF) { "UNKNOWN" } else { "MEASURED" };
            props!(v; "uuid" => sensitive(J::String(uuid), state, &["smbios"], None));
        }
    }
    comps.push(component(String::from("system"), "system", name, v));
    // Type 0: the firmware.
    if let Some(t0) = list.iter().find(|x| x.kind == 0) {
        let mut v = Vec::new();
        props!(v; "vendor" => txt(t0, 4), "version" => txt(t0, 5), "release_date" => txt(t0, 8));
        if let (Some(a), Some(b)) = (t0.byte(0x14), t0.byte(0x15)) {
            props!(v; "bios_release" => p(J::String(format!("{}.{}", a, b)), "MEASURED", &["smbios"]));
        }
        if let Some(e) = t0.byte(0x13) {
            props!(v; "uefi" => p(J::Bool(e & 1 << 3 != 0), "MEASURED", &["smbios"]));
        }
        comps.push(component(String::from("firmware"), "firmware", String::from(t0.text(5).unwrap_or("BIOS")), v));
    }
    // Type 2: the baseboards.
    for (i, t2) in list.iter().filter(|x| x.kind == 2).enumerate() {
        let mut v = Vec::new();
        props!(v; "manufacturer" => txt(t2, 4), "product" => txt(t2, 5), "version" => txt(t2, 6), "serial_number" => secret(t2, 7));
        comps.push(component(format!("baseboard:{}", i), "baseboard", String::from(t2.text(5).unwrap_or("placa de baza")), v));
    }
    // Type 3: the chassis.
    for (i, t3) in list.iter().filter(|x| x.kind == 3).enumerate() {
        let mut v = Vec::new();
        props!(v; "manufacturer" => txt(t3, 4));
        if let Some(k) = t3.byte(5) {
            props!(v; "type" => p(s(chassis(k & 0x7F)), "MEASURED", &["smbios"]));
        }
        props!(v; "serial_number" => secret(t3, 7));
        comps.push(component(format!("chassis:{}", i), "chassis", String::from(chassis(t3.byte(5).unwrap_or(2) & 0x7F)), v));
    }
    // Type 4: the processor sockets.
    for (i, t4) in list.iter().filter(|x| x.kind == 4).enumerate() {
        let mut v = Vec::new();
        props!(v; "socket" => txt(t4, 4), "manufacturer" => txt(t4, 7), "version" => txt(t4, 0x10));
        if let Some(m) = t4.word(0x14) {
            props!(v; "max_speed_mhz" => p(int(m as u64), "MEASURED", &["smbios"]));
        }
        if let Some(m) = t4.word(0x16) {
            props!(v; "current_speed_mhz" => p(int(m as u64), "MEASURED", &["smbios"]));
        }
        if let (Some(c), Some(t)) = (t4.byte(0x23), t4.byte(0x25)) {
            props!(v; "core_count" => p(int(c as u64), "MEASURED", &["smbios"]), "thread_count" => p(int(t as u64), "MEASURED", &["smbios"]));
        }
        props!(v; "serial_number" => secret(t4, 0x20));
        comps.push(component(format!("cpu_socket:{}", i), "cpu_socket", String::from(t4.text(0x10).unwrap_or("procesor")), v));
    }
    // Type 17: the memory devices (empty slots kept, size 0).
    for (i, t17) in list.iter().filter(|x| x.kind == 17).enumerate() {
        let mut v = Vec::new();
        let size_mib = match t17.word(0x0C) {
            Some(0) | None => 0u64,
            Some(0x7FFF) => t17.dword(0x1C).map_or(0, |e| (e & 0x7FFF_FFFF) as u64),
            Some(w) if w & 0x8000 != 0 => ((w & 0x7FFF) as u64) / 1024, // KiB units
            Some(w) => w as u64,
        };
        props!(v; "locator" => txt(t17, 0x10), "bank" => txt(t17, 0x11), "size_mib" => p(int(size_mib), "MEASURED", &["smbios"]));
        if let Some(t) = t17.byte(0x12) {
            props!(v; "memory_type" => p(s(memory_type(t)), "MEASURED", &["smbios"]));
        }
        if let Some(sp) = t17.word(0x15) {
            props!(v; "speed_mts" => p(int(sp as u64), "MEASURED", &["smbios"]));
        }
        if let Some(sp) = t17.word(0x20) {
            props!(v; "configured_speed_mts" => p(int(sp as u64), "MEASURED", &["smbios"]));
        }
        props!(v; "manufacturer" => txt(t17, 0x17), "serial_number" => secret(t17, 0x18), "part_number" => txt(t17, 0x1A));
        let label = String::from(t17.text(0x10).unwrap_or("memorie"));
        comps.push(component(format!("memory_device:{}", i), "memory_device", label, v));
    }
    // Everything else: how many structures of each type (the raw table stays on the machine).
    let mut kinds: Vec<(u8, usize)> = Vec::new();
    for x in list {
        match kinds.iter_mut().find(|k| k.0 == x.kind) {
            Some(k) => k.1 += 1,
            None => kinds.push((x.kind, 1)),
        }
    }
    comps.push(component(String::from("smbios"), "firmware_table_set", String::from("SMBIOS"), vec![
        (String::from("structure_types"), p(J::Object(kinds.iter().map(|(k, n)| (format!("{}", k), int(*n as u64))).collect()), "MEASURED", &["smbios"])),
    ]));
}

fn chassis(k: u8) -> &'static str {
    match k {
        3 => "desktop",
        4 => "low-profile desktop",
        6 => "mini tower",
        7 => "tower",
        8 => "portable",
        9 => "laptop",
        10 => "notebook",
        13 => "all-in-one",
        14 => "sub-notebook",
        17 => "main server chassis",
        23 => "rack mount",
        30 => "tablet",
        31 => "convertible",
        32 => "detachable",
        35 => "mini PC",
        _ => "other",
    }
}

fn memory_type(t: u8) -> &'static str {
    match t {
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x02 => "unknown",
        _ => "other",
    }
}

// ---------------------------------------------------------------- CPU and memory

fn cpu_component(name: &str) -> J {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    let l0 = __cpuid(0);
    let max = l0.eax;
    let max_ext = __cpuid(0x8000_0000).eax;
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&l0.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&l0.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&l0.ecx.to_le_bytes());
    let vendor = String::from(core::str::from_utf8(&vendor).unwrap_or("?"));
    let l1 = __cpuid(1);
    let base_family = (l1.eax >> 8) & 0xF;
    let family = if base_family == 0xF { base_family + ((l1.eax >> 20) & 0xFF) } else { base_family };
    let model = if base_family == 0x6 || base_family == 0xF { ((l1.eax >> 16) & 0xF) << 4 | (l1.eax >> 4) & 0xF } else { (l1.eax >> 4) & 0xF };
    let l7 = if max >= 7 { __cpuid_count(7, 0) } else { __cpuid(0) };
    let e1 = if max_ext >= 0x8000_0001 { __cpuid(0x8000_0001) } else { __cpuid(0) };
    let mut flags: Vec<J> = Vec::new();
    let table: &[(bool, &str)] = &[
        (l1.edx & 1 << 0 != 0, "fpu"), (l1.edx & 1 << 4 != 0, "tsc"), (l1.edx & 1 << 5 != 0, "msr"), (l1.edx & 1 << 9 != 0, "apic"),
        (l1.edx & 1 << 25 != 0, "sse"), (l1.edx & 1 << 26 != 0, "sse2"), (l1.edx & 1 << 28 != 0, "htt"),
        (l1.ecx & 1 << 0 != 0, "sse3"), (l1.ecx & 1 << 1 != 0, "pclmulqdq"), (l1.ecx & 1 << 5 != 0, "vmx"), (l1.ecx & 1 << 9 != 0, "ssse3"),
        (l1.ecx & 1 << 12 != 0, "fma"), (l1.ecx & 1 << 13 != 0, "cx16"), (l1.ecx & 1 << 19 != 0, "sse4.1"), (l1.ecx & 1 << 20 != 0, "sse4.2"),
        (l1.ecx & 1 << 21 != 0, "x2apic"), (l1.ecx & 1 << 22 != 0, "movbe"), (l1.ecx & 1 << 23 != 0, "popcnt"), (l1.ecx & 1 << 25 != 0, "aes"),
        (l1.ecx & 1 << 26 != 0, "xsave"), (l1.ecx & 1 << 28 != 0, "avx"), (l1.ecx & 1 << 29 != 0, "f16c"), (l1.ecx & 1 << 30 != 0, "rdrand"),
        (l1.ecx & 1 << 31 != 0, "hypervisor"),
        (max >= 7 && l7.ebx & 1 << 3 != 0, "bmi1"), (max >= 7 && l7.ebx & 1 << 5 != 0, "avx2"), (max >= 7 && l7.ebx & 1 << 8 != 0, "bmi2"),
        (max >= 7 && l7.ebx & 1 << 16 != 0, "avx512f"), (max >= 7 && l7.ebx & 1 << 17 != 0, "avx512dq"), (max >= 7 && l7.ebx & 1 << 18 != 0, "rdseed"),
        (max >= 7 && l7.ebx & 1 << 29 != 0, "sha"), (max >= 7 && l7.ebx & 1 << 30 != 0, "avx512bw"), (max >= 7 && l7.ebx & 1 << 31 != 0, "avx512vl"),
        (max >= 7 && l7.ecx & 1 << 11 != 0, "avx512vnni"), (max >= 7 && l7.ecx & 1 << 8 != 0, "gfni"), (max >= 7 && l7.ecx & 1 << 9 != 0, "vaes"),
        (max >= 7 && l7.edx & 1 << 22 != 0, "amx-bf16"), (max >= 7 && l7.edx & 1 << 24 != 0, "amx-tile"),
        (e1.edx & 1 << 11 != 0, "syscall"), (e1.edx & 1 << 20 != 0, "nx"), (e1.edx & 1 << 26 != 0, "pdpe1gb"), (e1.edx & 1 << 27 != 0, "rdtscp"),
        (e1.edx & 1 << 29 != 0, "lm"), (e1.ecx & 1 << 5 != 0, "lzcnt"), (e1.ecx & 1 << 2 != 0, "svm"),
    ];
    for (on, n) in table {
        if *on {
            flags.push(s(n));
        }
    }
    // Caches: leaf 4 (Intel) or 0x8000_001D (AMD), same layout.
    let cache_leaf = if vendor == "GenuineIntel" && max >= 4 {
        Some(4)
    } else if max_ext >= 0x8000_001D && vendor == "AuthenticAMD" {
        Some(0x8000_001D)
    } else {
        None
    };
    let mut caches = Vec::new();
    if let Some(leaf) = cache_leaf {
        for sub in 0..16 {
            let r = __cpuid_count(leaf, sub);
            let kind = r.eax & 0x1F;
            if kind == 0 {
                break;
            }
            let ways = (r.ebx >> 22) + 1;
            let parts = ((r.ebx >> 12) & 0x3FF) + 1;
            let line = (r.ebx & 0xFFF) + 1;
            let sets = r.ecx + 1;
            let bytes = ways as u64 * parts as u64 * line as u64 * sets as u64;
            caches.push(J::object([
                ("level", int(((r.eax >> 5) & 7) as u64)),
                ("type", s(match kind { 1 => "data", 2 => "instruction", 3 => "unified", _ => "other" })),
                ("size_kib", int(bytes / 1024)),
                ("ways", int(ways as u64)),
                ("line_bytes", int(line as u64)),
            ]));
        }
    }
    // Raw leaves (subleaf 0), so a later comparison can check every bit.
    let mut leaves = Vec::new();
    for leaf in (0..=max.min(0x1F)).chain(0x8000_0000..=max_ext.clamp(0x8000_0000, 0x8000_0008)) {
        let r = __cpuid_count(leaf, 0);
        leaves.push(J::object([
            ("leaf", hex(leaf as u64, 8)),
            ("eax", hex(r.eax as u64, 8)), ("ebx", hex(r.ebx as u64, 8)), ("ecx", hex(r.ecx as u64, 8)), ("edx", hex(r.edx as u64, 8)),
        ]));
    }
    let madt = crate::acpi::madt::get().cpu_count as u64;
    let online = crate::arch::x86_64::smp::online() as u64;
    let mut v = Vec::new();
    props!(v;
        "vendor" => p(J::String(vendor.clone()), "MEASURED", &["cpuid"]),
        "brand" => p(s(name), "MEASURED", &["cpuid"]),
        "family" => p(hex(family as u64, 2), "MEASURED", &["cpuid"]),
        "model" => p(hex(model as u64, 2), "MEASURED", &["cpuid"]),
        "stepping" => p(int((l1.eax & 0xF) as u64), "MEASURED", &["cpuid"]),
        "signature" => p(hex(l1.eax as u64, 8), "MEASURED", &["cpuid"]),
        "max_leaf" => p(hex(max as u64, 8), "MEASURED", &["cpuid"]),
        "max_extended_leaf" => p(hex(max_ext as u64, 8), "MEASURED", &["cpuid"]),
        "features" => p(J::Array(flags), "MEASURED", &["cpuid"]),
        "caches" => if caches.is_empty() { p(J::Null, "UNKNOWN", &["cpuid"]) } else { p(J::Array(caches), "MEASURED", &["cpuid"]) },
        "logical_processors_madt" => p(int(madt), if madt == online { "CONFIRMED" } else { "MEASURED" }, &["madt"]),
        "logical_processors_started" => p(int(online), if madt == online { "CONFIRMED" } else { "MEASURED" }, &["madt"]),
        "cpuid_leaves" => p(J::Array(leaves), "MEASURED", &["cpuid"]),
        "msr" => not_tested("registrele MSR nu se citesc in pasul 1 (o citire nesuportata opreste procesorul)"),
    );
    component(String::from("cpu:0"), "cpu", String::from(name), v)
}

fn memory_component() -> J {
    use crate::mem::map;
    let regions = map::regions();
    let ram: u64 = regions
        .iter()
        .filter(|r| matches!(r.kind, map::Kind::Usable | map::Kind::BootloaderReclaimable | map::Kind::Kernel))
        .map(|r| r.length)
        .sum();
    let list: Vec<J> = regions
        .iter()
        .map(|r| J::object([("base", hex(r.base, 12)), ("length", int(r.length)), ("type", s(r.kind.name()))]))
        .collect();
    let mut v = Vec::new();
    props!(v;
        "usable_ram_mib" => p(int(ram >> 20), "MEASURED", &["limine-memmap"]),
        "regions" => p(J::Array(list), "MEASURED", &["limine-memmap"]),
    );
    component(String::from("memory"), "memory", format!("RAM {} MiB", ram >> 20), v)
}

// ---------------------------------------------------------------- PCI

fn speed_name(code: u32) -> &'static str {
    match code {
        1 => "2.5 GT/s (Gen1)",
        2 => "5 GT/s (Gen2)",
        3 => "8 GT/s (Gen3)",
        4 => "16 GT/s (Gen4)",
        5 => "32 GT/s (Gen5)",
        6 => "64 GT/s (Gen6)",
        _ => "necunoscuta",
    }
}

fn port_type(t: u32) -> &'static str {
    match t {
        0 => "endpoint",
        1 => "legacy endpoint",
        4 => "root port",
        5 => "upstream switch port",
        6 => "downstream switch port",
        7 => "PCIe to PCI bridge",
        8 => "PCI to PCIe bridge",
        9 => "root complex integrated endpoint",
        10 => "root complex event collector",
        _ => "necunoscut",
    }
}

fn aspm(bits: u32) -> &'static str {
    match bits & 3 {
        0 => "niciunul",
        1 => "L0s",
        2 => "L1",
        _ => "L0s+L1",
    }
}

pub fn ext_capability_name(id: u16) -> &'static str {
    match id {
        0x01 => "AER",
        0x02 | 0x09 => "VC",
        0x03 => "Device Serial Number",
        0x04 => "Power Budgeting",
        0x05 => "Root Complex Link Declaration",
        0x0B => "Vendor-Specific (VSEC)",
        0x0D => "ACS",
        0x0E => "ARI",
        0x0F => "ATS",
        0x10 => "SR-IOV",
        0x13 => "PRI",
        0x15 => "Resizable BAR",
        0x16 => "DPA",
        0x17 => "TPH",
        0x18 => "LTR",
        0x19 => "Secondary PCIe",
        0x1B => "PASID",
        0x1D => "DPC",
        0x1E => "L1 PM Substates",
        0x1F => "PTM",
        0x23 => "DVSEC",
        0x25 => "Data Link Feature",
        0x26 => "Physical Layer 16 GT/s",
        0x27 => "Lane Margining",
        0x2A => "Physical Layer 32 GT/s",
        _ => "alta",
    }
}

/// (offset, id, version) of the extended capabilities (from 0x100; at most 64, against loops).
fn ext_capabilities(d: &Device) -> Vec<(u16, u16, u8)> {
    let mut out = Vec::new();
    let mut ptr = 0x100u16;
    while out.len() < 64 {
        let h = d.read32(ptr);
        if h == 0 || h == 0xFFFF_FFFF {
            break;
        }
        out.push((ptr, h as u16, ((h >> 16) & 0xF) as u8));
        let next = ((h >> 20) & 0xFFC) as u16;
        if next < 0x100 || out.iter().any(|&(o, _, _)| o == next) {
            break;
        }
        ptr = next;
    }
    out
}

fn pci_component(d: &Device, driver_of: &dyn Fn(&Device) -> &'static str, toured: bool) -> J {
    const R: &[&str] = &["pci-config", "pcie-spec"];
    let mut config = [0u8; 4096];
    for o in (0..4096u16).step_by(4) {
        config[o as usize..o as usize + 4].copy_from_slice(&d.read32(o).to_le_bytes());
    }
    let dword = |o: usize| u32::from_le_bytes(config[o..o + 4].try_into().unwrap());
    let cmd = dword(4) as u16;
    let status = (dword(4) >> 16) as u16;
    let header = config[0x0E];
    let mut v = Vec::new();
    props!(v;
        "address" => p(J::String(format!("0000:{}", d.address())), "MEASURED", R),
        "vendor_id" => p(hex(d.vendor as u64, 4), "MEASURED", R),
        "device_id" => p(hex(d.device as u64, 4), "MEASURED", R),
        "vendor_name" => match super::names::pci_vendor(d.vendor) {
            Some(n) => p(s(n), "DOCUMENTED", &["cu-names"]),
            None => p(J::Null, "UNKNOWN", &["cu-names"]),
        },
        "device_name" => match super::names::pci_device(d.vendor, d.device) {
            Some(n) => p(s(n), "DOCUMENTED", &["cu-names"]),
            None => p(J::Null, "UNKNOWN", &["cu-names"]),
        },
        "class_code" => p(J::String(format!("{:02x}{:02x}{:02x}", d.class, d.subclass, d.prog_if)), "MEASURED", R),
        "class_name" => p(s(super::names::pci_class(d.class, d.subclass, d.prog_if)), "DOCUMENTED", &["cu-names", "pcie-spec"]),
        "revision" => p(hex(config[8] as u64, 2), "MEASURED", R),
        "header_type" => p(hex(header as u64, 2), "MEASURED", R),
        "multifunction" => p(J::Bool(header & 0x80 != 0), "MEASURED", R),
        "command" => p(hex(cmd as u64, 4), "MEASURED", R),
        "status" => p(hex(status as u64, 4), "MEASURED", R),
        "io_space_enabled" => p(J::Bool(cmd & 1 != 0), "MEASURED", R),
        "memory_space_enabled" => p(J::Bool(cmd & 2 != 0), "MEASURED", R),
        "bus_master_enabled" => p(J::Bool(cmd & 4 != 0), "MEASURED", R),
        "interrupt_line" => p(int(config[0x3C] as u64), "MEASURED", R),
        "interrupt_pin" => p(int(config[0x3D] as u64), "MEASURED", R),
        "cu_driver" => p(s(driver_of(d)), "MEASURED", &["cu-drivers"]),
    );
    match header & 0x7F {
        0 => props!(v;
            "subsystem_vendor_id" => p(hex(u16::from_le_bytes([config[0x2C], config[0x2D]]) as u64, 4), "MEASURED", R),
            "subsystem_id" => p(hex(u16::from_le_bytes([config[0x2E], config[0x2F]]) as u64, 4), "MEASURED", R),
            "expansion_rom" => p(hex(dword(0x30) as u64, 8), "MEASURED", R),
        ),
        1 => props!(v;
            "primary_bus" => p(int(config[0x18] as u64), "MEASURED", R),
            "secondary_bus" => p(int(config[0x19] as u64), "MEASURED", R),
            "subordinate_bus" => p(int(config[0x1A] as u64), "MEASURED", R),
        ),
        _ => {}
    }
    // Capabilities (classic list).
    let mut caps: Vec<(u16, u8)> = Vec::new();
    if status & 1 << 4 != 0 {
        let mut ptr = (config[0x34] & 0xFC) as u16;
        while ptr >= 0x40 && caps.len() < 48 && !caps.iter().any(|c| c.0 == ptr) {
            caps.push((ptr, config[ptr as usize]));
            ptr = (config[ptr as usize + 1] & 0xFC) as u16;
        }
    }
    let ext = if caps.iter().any(|c| c.1 == 0x10) { ext_capabilities(d) } else { Vec::new() };
    // Resizable BAR: current size per BAR index (readable without any write).
    let mut rebar: Vec<(u32, u64, u32)> = Vec::new(); // (bar index, current MiB, supported mask)
    if let Some(&(e, _, _)) = ext.iter().find(|c| c.1 == 0x15) {
        let e = e as usize;
        let n = ((dword(e + 8) >> 5) & 7).clamp(1, 6);
        for i in 0..n as usize {
            let capw = dword(e + 4 + i * 8);
            let ctrl = dword(e + 8 + i * 8);
            rebar.push((ctrl & 7, 1u64 << ((ctrl >> 8) & 0x3F), capw >> 4));
        }
    }
    // BARs by index, as the firmware programmed them.
    let count = match header & 0x7F {
        0 => 6,
        1 => 2,
        _ => 0,
    };
    let mut bars = Vec::new();
    let mut i = 0usize;
    while i < count {
        let low = dword(0x10 + i * 4);
        let mut step = 1;
        let mut b = vec![(String::from("index"), int(i as u64))];
        if low == 0 {
            b.push((String::from("type"), s("neprogramat")));
        } else if low & 1 != 0 {
            b.push((String::from("type"), s("io")));
            b.push((String::from("address"), hex((low & !3) as u64, 4)));
        } else {
            let is64 = (low >> 1) & 3 == 2;
            let mut addr = (low & !0xF) as u64;
            if is64 && i + 1 < count {
                addr |= (dword(0x14 + i * 4) as u64) << 32;
                step = 2;
            }
            b.push((String::from("type"), s(if is64 { "memory64" } else { "memory32" })));
            b.push((String::from("address"), hex(addr, 12)));
            b.push((String::from("prefetchable"), J::Bool(low & 8 != 0)));
        }
        if low != 0 {
            let size = match rebar.iter().find(|r| r.0 as usize == i) {
                Some(r) => p(int(r.1), "MEASURED", &["pci-config", "pcie-spec"]),
                None => not_tested("marimea cere o scriere in BAR (nefacuta)"),
            };
            b.push((String::from("size_mib"), size));
        }
        bars.push(J::Object(b));
        i += step;
    }
    props!(v; "bars" => p(J::Array(bars), "MEASURED", R));
    let cap_list: Vec<J> = caps
        .iter()
        .map(|&(o, id)| J::object([("offset", hex(o as u64, 2)), ("id", hex(id as u64, 2)), ("name", s(super::names::pci_capability(id)))]))
        .collect();
    props!(v; "capabilities" => p(J::Array(cap_list), "MEASURED", R));
    // Power management.
    if let Some(&(pm, _)) = caps.iter().find(|c| c.1 == 0x01) {
        let pmc = (dword(pm as usize) >> 16) as u16;
        let csr = dword(pm as usize + 4);
        let mut sup = vec![s("D0")];
        if pmc & 1 << 9 != 0 {
            sup.push(s("D1"));
        }
        if pmc & 1 << 10 != 0 {
            sup.push(s("D2"));
        }
        sup.push(s("D3hot"));
        if pmc & 1 << 15 != 0 {
            sup.push(s("D3cold (PME)"));
        }
        props!(v;
            "power_state" => p(J::String(format!("D{}", csr & 3)), "MEASURED", R),
            "power_states_supported" => p(J::Array(sup), "MEASURED", R),
            "no_soft_reset" => p(J::Bool(csr & 1 << 3 != 0), "MEASURED", R),
        );
    } else {
        props!(v; "power_state" => p(J::Null, "UNSUPPORTED", R));
    }
    // PCI Express.
    if let Some(&(pe, _)) = caps.iter().find(|c| c.1 == 0x10) {
        let pe = pe as usize;
        let pcap = (dword(pe) >> 16) as u16 as u32;
        let ver = pcap & 0xF;
        let ty = (pcap >> 4) & 0xF;
        let devctl = dword(pe + 8) as u16 as u32;
        let lcap = dword(pe + 0x0C);
        let lctl = dword(pe + 0x10) as u16 as u32;
        let lsta = dword(pe + 0x10) >> 16;
        props!(v;
            "pcie_version" => p(int(ver as u64), "MEASURED", R),
            "pcie_port_type" => p(s(port_type(ty)), "MEASURED", R),
            "max_payload_bytes" => p(int(128u64 << ((devctl >> 5) & 7)), "MEASURED", R),
            "max_read_request_bytes" => p(int(128u64 << ((devctl >> 12) & 7)), "MEASURED", R),
        );
        if ty != 9 && ty != 10 {
            props!(v;
                "link_max_speed" => p(s(speed_name(lcap & 0xF)), "MEASURED", R),
                "link_max_width" => p(int(((lcap >> 4) & 0x3F) as u64), "MEASURED", R),
                "link_speed" => p(s(speed_name(lsta & 0xF)), "MEASURED", R),
                "link_width" => p(int(((lsta >> 4) & 0x3F) as u64), "MEASURED", R),
                "aspm_supported" => p(s(aspm(lcap >> 10)), "MEASURED", R),
                "aspm_enabled" => p(s(aspm(lctl)), "MEASURED", R),
                "port_number" => p(int((lcap >> 24) as u64), "MEASURED", R),
            );
            if ver >= 2 {
                let vec2 = (dword(pe + 0x2C) >> 1) & 0x7F;
                let list: Vec<J> = (0..7).filter(|b| vec2 & 1 << b != 0).map(|b| s(speed_name(b + 1))).collect();
                props!(v; "link_speeds_supported" => p(J::Array(list), "MEASURED", R));
            }
        }
    }
    let ext_list: Vec<J> = ext
        .iter()
        .map(|&(o, id, ver)| J::object([("offset", hex(o as u64, 3)), ("id", hex(id as u64, 4)), ("version", int(ver as u64)), ("name", s(ext_capability_name(id)))]))
        .collect();
    props!(v; "extended_capabilities" => p(J::Array(ext_list), "MEASURED", R));
    if !rebar.is_empty() {
        let list: Vec<J> = rebar
            .iter()
            .map(|&(bar, cur, mask)| {
                let sizes: Vec<J> = (0..28).filter(|b| mask & 1 << b != 0).map(|b| int(1u64 << b)).collect();
                J::object([("bar", int(bar as u64)), ("current_mib", int(cur)), ("supported_mib", J::Array(sizes))])
            })
            .collect();
        props!(v; "resizable_bar" => p(J::Array(list), "MEASURED", R));
    }
    // The configuration space itself; a Device Serial Number capability is a serial: zeroed in the redacted variant.
    let full_hex: String = config.iter().map(|b| format!("{:02x}", b)).collect();
    match ext.iter().find(|c| c.1 == 0x03) {
        Some(&(o, _, _)) => {
            let o = o as usize;
            let serial = u64::from_le_bytes(config[o + 4..o + 12].try_into().unwrap());
            let mut masked = config;
            masked[o + 4..o + 12].fill(0);
            let masked_hex: String = masked.iter().map(|b| format!("{:02x}", b)).collect();
            props!(v;
                "device_serial_number" => sensitive(hex(serial, 16), "MEASURED", R, None),
                "config_space_hex" => sensitive(J::String(full_hex), "MEASURED", &["pci-config"], Some(J::String(masked_hex))),
            );
        }
        None => props!(v; "config_space_hex" => p(J::String(full_hex), "MEASURED", &["pci-config"])),
    }
    props!(v; "config_space_bytes" => p(int(4096), "MEASURED", &["pci-config"]));
    if d.vendor == 0x10DE && d.class == 0x03 {
        props!(v; "nvidia_registers" => if toured {
            p(J::String(format!("componenta gpu:0000:{} (turul de citire cyber-gpu)", d.address())), "MEASURED", &["cyber-gpu"])
        } else {
            not_tested("turul de citire cyber-gpu nu a rulat pentru aceasta placa")
        });
    }
    let name = match (super::names::pci_vendor(d.vendor), super::names::pci_device(d.vendor, d.device)) {
        (Some(a), Some(b)) => format!("{} {}", a, b),
        _ => format!("{:04x}:{:04x} {}", d.vendor, d.device, super::names::pci_class(d.class, d.subclass, d.prog_if)),
    };
    component(format!("pci:0000:{}", d.address()), "pci_function", name, v)
}

// ---------------------------------------------------------------- ACPI

fn acpi_components(comps: &mut Vec<J>) -> usize {
    let mut tables = crate::acpi::tables();
    // The DSDT is not in the XSDT: the FADT points at it (X_DSDT at 140 when present, else DSDT at 40).
    if let Some((p_fadt, len)) = crate::acpi::find(b"FACP") {
        // SAFETY: `find` returned a mapped, checksummed table of `len` bytes.
        let t = unsafe { core::slice::from_raw_parts(p_fadt, len) };
        let x = (len >= 148).then(|| u64::from_le_bytes(t[140..148].try_into().unwrap())).filter(|&a| a != 0);
        let dsdt = x.or_else(|| (len >= 44).then(|| u32::from_le_bytes(t[40..44].try_into().unwrap()) as u64).filter(|&a| a != 0));
        if let Some(a) = dsdt {
            if let Some(h) = crate::acpi::header_at(a) {
                tables.push((a, h));
            }
        }
    }
    let mut seen: Vec<([u8; 4], usize)> = Vec::new();
    for (phys, h) in &tables {
        let sig = h.signature;
        let n = match seen.iter_mut().find(|x| x.0 == sig) {
            Some(x) => {
                x.1 += 1;
                x.1
            }
            None => {
                seen.push((sig, 1));
                1
            }
        };
        let text = |b: &[u8]| -> String { b.iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' }).collect::<String>().trim_end().into() };
        let sig_t = text(&sig);
        let creator = h.creator_id.to_le_bytes();
        let (len, rev, oem_rev, cr_rev) = (h.length, h.revision, h.oem_revision, h.creator_revision);
        const A: &[&str] = &["acpi"];
        let mut v = Vec::new();
        props!(v;
            "signature" => p(J::String(sig_t.clone()), "MEASURED", A),
            "instance" => p(int(n as u64), "MEASURED", A),
            "revision" => p(int(rev as u64), "MEASURED", A),
            "length" => p(int(len as u64), "MEASURED", A),
            "oem_id" => p(J::String(text(&h.oem_id)), "MEASURED", A),
            "oem_table_id" => p(J::String(text(&h.oem_table_id)), "MEASURED", A),
            "oem_revision" => p(hex(oem_rev as u64, 8), "MEASURED", A),
            "creator_id" => p(J::String(text(&creator)), "MEASURED", A),
            "creator_revision" => p(hex(cr_rev as u64, 8), "MEASURED", A),
            "address" => p(hex(*phys, 12), "MEASURED", A),
            "checksum_ok" => p(J::Bool(true), "CONFIRMED", A),
        );
        if let Some(extra) = super::table_details(&sig) {
            props!(v; "decoded" => p(J::String(extra), "MEASURED", &["acpi", "madt"]));
        }
        comps.push(component(format!("acpi:{}#{}", sig_t, n), "acpi_table", format!("{} {}", sig_t, text(&h.oem_table_id)), v));
    }
    tables.len()
}

// ---------------------------------------------------------------- variants and report

fn count_statuses(v: &J, counts: &mut [usize; 7]) {
    match v {
        J::Object(pairs) => {
            if let (Some(J::String(st)), true) = (v.get("status"), pairs.iter().any(|(k, _)| k == "source_refs")) {
                if let Some(i) = STATUSES.iter().position(|(k, _)| k == st) {
                    counts[i] += 1;
                }
            }
            for (_, x) in pairs {
                count_statuses(x, counts);
            }
        }
        J::Array(items) => items.iter().for_each(|x| count_statuses(x, counts)),
        _ => {}
    }
}

/// The full document: the `redacted_value` helper fields removed.
fn strip_redacted_values(v: &J) -> J {
    match v {
        J::Object(pairs) => J::Object(pairs.iter().filter(|(k, _)| k != "redacted_value").map(|(k, x)| (k.clone(), strip_redacted_values(x))).collect()),
        J::Array(items) => J::Array(items.iter().map(strip_redacted_values).collect()),
        x => x.clone(),
    }
}

/// The variant that may leave the stick: every sensitive value hidden (or masked), `redacted: true`.
pub fn redact(v: &J) -> J {
    match v {
        J::Object(pairs) => {
            let is_sensitive = v.get("sensitive").and_then(|b| b.as_bool()).unwrap_or(false);
            let masked = v.get("redacted_value").cloned();
            J::Object(
                pairs
                    .iter()
                    .filter(|(k, _)| k != "redacted_value")
                    .map(|(k, x)| {
                        let nv = if k == "redacted" && matches!(x, J::Bool(_)) {
                            J::Bool(true)
                        } else if is_sensitive && k == "value" {
                            if x.is_null() { J::Null } else { masked.clone().unwrap_or(J::str(HIDDEN)) }
                        } else {
                            redact(x)
                        };
                        (k.clone(), nv)
                    })
                    .collect(),
            )
        }
        J::Array(items) => J::Array(items.iter().map(redact).collect()),
        x => x.clone(),
    }
}

fn prop_value<'a>(c: &'a J, key: &str) -> Option<&'a J> {
    c.get("props")?.get(key)?.get("value")
}

fn report_text(doc: &J, machine: &str, stamp: &str, session: &str, counts: &[usize; 7]) -> String {
    let comps = doc.get("components").and_then(|c| c.as_array()).unwrap_or(&[]);
    fn kind_of<'a>(comps: &'a [J], k: &'a str) -> impl Iterator<Item = &'a J> + 'a {
        comps.iter().filter(move |c| c.get("kind").and_then(|x| x.as_str()) == Some(k))
    }
    let of = |k: &'static str| kind_of(comps, k);
    let mut t = String::new();
    t.push_str("== Cyber Universe: analiza (C-117, schema uhl-inventory/0.1) ==\n");
    t.push_str(&format!("calculator: {} | sesiunea {} | ceasul calculatorului {}\n", machine, session, stamp));
    t.push_str(&format!("nucleu: cyber-universe {} (cyber-gpu {})\n", env!("CARGO_PKG_VERSION"), crate::drivers::gpu::CYBER_GPU_REV));
    for c in of("system").chain(of("firmware")).chain(of("baseboard")) {
        t.push_str(&format!("{}: {}\n", c.get("kind").and_then(|x| x.as_str()).unwrap_or("?"), c.get("name").and_then(|x| x.as_str()).unwrap_or("?")));
    }
    for c in of("cpu") {
        let n = |k: &str| prop_value(c, k).and_then(|x| x.as_i64()).unwrap_or(0);
        t.push_str(&format!(
            "procesor: {} ({} fire in MADT, {} pornite)\n",
            c.get("name").and_then(|x| x.as_str()).unwrap_or("?"), n("logical_processors_madt"), n("logical_processors_started")
        ));
    }
    let dimms: Vec<&J> = of("memory_device").filter(|c| prop_value(c, "size_mib").and_then(|x| x.as_i64()).unwrap_or(0) > 0).collect();
    for c in of("memory") {
        t.push_str(&format!("memorie: {} ({} module in SMBIOS)\n", c.get("name").and_then(|x| x.as_str()).unwrap_or("?"), dimms.len()));
    }
    t.push_str(&format!("PCI: {} functii\n", of("pci_function").count()));
    for c in of("pci_function") {
        let str_of = |k: &str| prop_value(c, k).and_then(|x| x.as_str()).unwrap_or("-");
        let mut line = format!("  {} {}:{} {}", str_of("address"), str_of("vendor_id"), str_of("device_id"), c.get("name").and_then(|x| x.as_str()).unwrap_or("?"));
        if let Some(sp) = prop_value(c, "link_speed").and_then(|x| x.as_str()) {
            line.push_str(&format!(" | legatura {} x{}", sp, prop_value(c, "link_width").and_then(|x| x.as_i64()).unwrap_or(0)));
        }
        if let Some(ps) = prop_value(c, "power_state").and_then(|x| x.as_str()) {
            line.push_str(&format!(" | {}", ps));
        }
        if let Some(J::Array(r)) = prop_value(c, "resizable_bar") {
            for x in r {
                line.push_str(&format!(" | ReBAR BAR{} = {} MiB", x.get("bar").and_then(|b| b.as_i64()).unwrap_or(0), x.get("current_mib").and_then(|b| b.as_i64()).unwrap_or(0)));
            }
        }
        t.push_str(&line);
        t.push('\n');
    }
    let acpi: Vec<String> = of("acpi_table").filter_map(|c| c.get("id").and_then(|x| x.as_str()).map(|x| String::from(x.trim_start_matches("acpi:")))).collect();
    t.push_str(&format!("ACPI: {} tabele: {}\n", acpi.len(), acpi.join(" ")));
    t.push_str(&format!("USB: {} dispozitive\n", of("usb_device").count()));
    let st: Vec<String> = STATUSES.iter().zip(counts).map(|((k, _), c)| format!("{} {}", k, c)).collect();
    t.push_str(&format!("valori pe statusuri: {}\n", st.join(", ")));
    t.push_str("serii si UUID: doar in inventory.json (ramane pe stick); inventory.redacted.json le are ascunse\n");
    let gpus: Vec<&J> = kind_of(comps, "gpu_registers").collect();
    if gpus.is_empty() {
        t.push_str("placa NVIDIA: niciuna (sau turul nu a rulat)\n");
    }
    for c in gpus {
        let facts = prop_value(c, "facts");
        let f = |k: &str| facts.and_then(|x| x.get(k)).and_then(|x| x.as_str()).unwrap_or("-");
        t.push_str(&format!(
            "placa NVIDIA: {} | {} fapte | cip {} | VRAM {} | GFW {} | scrieri: {}\n",
            c.get("name").and_then(|x| x.as_str()).unwrap_or("?"),
            prop_value(c, "fact_count").and_then(|x| x.as_i64()).unwrap_or(0),
            f("chip.nume"), f("i6.vram"), f("gfw.progres"),
            prop_value(c, "writes_to_board").and_then(|x| x.as_str()).unwrap_or("necunoscut"),
        ));
    }
    t.push_str("0 scrieri in dispozitive\n");
    t
}

/// Boot self-test: the SMBIOS walker, the folder name, the redaction (every test build).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let mut t: Vec<u8> = Vec::new();
    let mut f = vec![0u8; 0x1B];
    f[0] = 1;
    f[1] = 0x1B;
    f[4] = 1;
    f[5] = 2;
    f[7] = 3;
    t.extend_from_slice(&f);
    t.extend_from_slice(b"ASUSTeK COMPUTER INC.\0TUF Gaming F15/FX506HM\0SERIE-TEST-123\0\0");
    t.extend_from_slice(&[17, 0x28, 1, 0]);
    let mut m = vec![0u8; 0x28 - 4];
    m[0x0C - 4] = 0xFF; // 0x7FFF -> extended size
    m[0x0D - 4] = 0x7F;
    m[0x1C - 4..0x20 - 4].copy_from_slice(&32768u32.to_le_bytes());
    t.extend_from_slice(&m);
    t.extend_from_slice(b"\0\0");
    t.extend_from_slice(&[127, 4, 2, 0, 0, 0]);
    let list = smbios_walk(&t);
    assert!(list.len() == 3 && list[0].text(7) == Some("SERIE-TEST-123") && list[1].kind == 17, "uhl: smbios_walk");
    assert!(list[1].word(0x0C) == Some(0x7FFF) && list[1].dword(0x1C) == Some(32768), "uhl: tipul 17");
    assert!(smbios_walk(&t[..30]).len() == 0, "uhl: tabel trunchiat");
    assert!(folder_name("ASUSTeK COMPUTER INC. TUF Gaming F15/FX506HM") == "ASUSTeK-COMPUTER-INC-TUF-Gaming-F15-FX506HM", "uhl: nume dosar");
    assert!(folder_name("  ") == "necunoscut", "uhl: nume gol");
    let doc = J::object([
        ("redacted", J::Bool(false)),
        ("a", sensitive(s("SERIE"), "MEASURED", &["smbios"], None)),
        ("b", sensitive(s("ff00"), "MEASURED", &["pci-config"], Some(s("0000")))),
        ("c", p(s("vizibil"), "MEASURED", &[])),
    ]);
    let r = redact(&doc).to_string();
    assert!(!r.contains("SERIE") && r.contains(HIDDEN) && r.contains("\"0000\"") && !r.contains("ff00") && r.contains("vizibil"), "uhl: redact {}", r);
    assert!(r.contains("\"redacted\":true") && !r.contains("redacted_value"), "uhl: redact marcaj");
    let full = strip_redacted_values(&doc).to_string();
    assert!(full.contains("SERIE") && !full.contains("redacted_value"), "uhl: varianta completa");
    let f = tour_facts(&[String::from("SIMULARE: fapt chip.nume = GA104"), String::from("gpu0: fapt i6.vram = 8192 MiB"), String::from("fara fapt")]);
    assert!(f.len() == 2 && f[0] == (String::from("chip.nume"), String::from("GA104")) && f[1].1 == "8192 MiB", "uhl: fapte {:?}", f);
    crate::info!("test: analiza (uhl) OK (SMBIOS, nume de dosar, ascunderea seriilor)");
}
