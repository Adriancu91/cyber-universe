//! P6-88 (C-118 step W0): `wifi identifica` — the Intel Wi-Fi adapter (AX201, CNVi on the ASUS TUF F15), READ ONLY.
//!
//! What it does: finds the Intel functions of PCI class 02:80 (other network controller), reports their identity,
//! configuration (Command, power state, BARs, PCIe link, capability list) and — only when the firmware left the
//! function in D0 with Memory Space enabled — five CSR registers through the read-only, uncached register window of
//! the GPU level 0 (no WRITABLE flag in the page tables). Nothing is ever written: not the Command register, not the
//! power state, not a register. A function that is asleep or not decoding memory stays unread, with the reason.
//!
//! Register offsets and field layouts are facts taken from the public iwlwifi driver (`iwl-csr.h`, GPL-2.0 OR
//! BSD-3-Clause) read as a specification; the code is ours. The Intel firmware and the Windows driver are never opened.

use crate::pci::Device;
use alloc::{format, string::String, vec::Vec};

const INTEL: u16 = 0x8086;

/// CSR offsets (iwlwifi `iwl-csr.h`): all inside the first 4 KiB page of BAR0, reading them has no side effect.
pub const CSR_HW_IF_CONFIG: u64 = 0x000;
pub const CSR_RESET: u64 = 0x020;
pub const CSR_GP_CNTRL: u64 = 0x024;
pub const CSR_HW_REV: u64 = 0x028;
pub const CSR_HW_RF_ID: u64 = 0x09C;
const CSR_LIST: [(u64, &str); 5] = [
    (CSR_HW_IF_CONFIG, "HW_IF_CONFIG"),
    (CSR_RESET, "RESET"),
    (CSR_GP_CNTRL, "GP_CNTRL"),
    (CSR_HW_REV, "HW_REV"),
    (CSR_HW_RF_ID, "HW_RF_ID"),
];

/// The five CSR values, in the order of `CSR_LIST`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Csr {
    pub values: [u32; 5],
}

impl Csr {
    pub fn get(&self, offset: u64) -> u32 {
        let i = CSR_LIST.iter().position(|(o, _)| *o == offset).unwrap_or(0);
        self.values[i]
    }
}

/// Everything W0 learns about one adapter.
#[derive(Clone, Debug)]
pub struct Facts {
    pub address: String,
    pub vendor: u16,
    pub device: u16,
    pub subsystem: u32,
    pub revision: u8,
    pub class: (u8, u8, u8),
    /// Bus 0 = inside the chipset (Intel CNVi: the MAC is in the PCH, the radio is the module on M.2).
    pub integrated: bool,
    pub command: u16,
    pub power_state: Option<u32>,
    pub bars: Vec<crate::drivers::gpu::Bar>,
    /// PCIe link (current speed code, width) when the function has a PCI Express capability.
    pub link: Option<(u32, u32)>,
    pub capabilities: Vec<u8>,
    pub csr: Result<Csr, &'static str>,
    /// Test builds only: the QEMU `edu` device standing in for the adapter (`wifi identifica --sim`).
    pub simulated: bool,
}

/// The Intel Wi-Fi functions (class 02:80). `sim` (test builds): QEMU's `edu` device (1234:11e8) stands in.
pub fn adapters(sim: bool) -> Vec<Device> {
    crate::pci::devices()
        .into_iter()
        .filter(|d| (d.vendor == INTEL && d.class == 0x02 && d.subclass == 0x80) || (sim && d.vendor == 0x1234 && d.device == 0x11E8))
        .collect()
}

fn power_state(d: &Device) -> Option<u32> {
    let ptr = d.capability(0x01)?;
    Some(d.read32(ptr + 4) & 3)
}

/// The standard capability list (IDs in order), read only.
fn capability_ids(d: &Device) -> Vec<u8> {
    let mut out = Vec::new();
    if d.read32(0x04) >> 16 & 1 << 4 == 0 {
        return out;
    }
    let mut ptr = (d.read32(0x34) & 0xFC) as u16;
    for _ in 0..48 {
        if ptr == 0 {
            break;
        }
        let cap = d.read32(ptr);
        out.push(cap as u8);
        ptr = ((cap >> 8) & 0xFC) as u16;
    }
    out
}

/// Reads the five CSR registers, or says why not (never enables anything to make it possible).
fn read_csr(d: &Device, bars: &[crate::drivers::gpu::Bar], power: Option<u32>) -> Result<Csr, &'static str> {
    if power.is_some_and(|p| p != 0) {
        return Err("placa nu e in D0 (nu o trezim: ar fi o scriere)");
    }
    if d.command() & 0b10 == 0 {
        return Err("Memory Space oprit de firmware (nu il pornim: ar fi o scriere)");
    }
    let bar0 = bars.iter().find(|b| b.index == 0 && !b.io).ok_or("BAR0 lipseste sau nu e de memorie")?;
    let values = crate::drivers::gpu::with_readonly_window(bar0.phys, |w| {
        let mut v = [0u32; 5];
        for (i, (off, _)) in CSR_LIST.iter().enumerate() {
            // SAFETY: `w` is the read-only, uncached mapping of BAR0's first page made by `with_readonly_window`;
            // every offset in CSR_LIST is < 4096 and 4-byte aligned; these registers have no read side effects.
            v[i] = unsafe { ((w + off) as *const u32).read_volatile() };
        }
        v
    })?;
    if values.iter().all(|v| *v == u32::MAX) {
        return Err("placa nu raspunde (toate registrele FFFFFFFF)");
    }
    Ok(Csr { values })
}

fn pcie_link(d: &Device) -> Option<(u32, u32)> {
    let ptr = d.capability(0x10)?;
    let status = d.read32(ptr + 0x10) >> 16; // Link Status = high half of the dword at +0x10
    Some((status & 0xF, (status >> 4) & 0x3F))
}

/// Collects the facts of every adapter (reads only).
pub fn facts(sim: bool) -> Vec<Facts> {
    adapters(sim)
        .iter()
        .map(|d| {
            let bars = crate::drivers::gpu::bars(d);
            let power = power_state(d);
            let csr = read_csr(d, &bars, power);
            Facts {
                address: d.address(),
                vendor: d.vendor,
                device: d.device,
                subsystem: d.read32(0x2C),
                revision: d.read32(0x08) as u8,
                class: (d.class, d.subclass, d.prog_if),
                integrated: d.bus == 0,
                command: d.command(),
                power_state: power,
                link: pcie_link(d),
                capabilities: capability_ids(d),
                bars,
                csr,
                simulated: d.vendor != INTEL,
            }
        })
        .collect()
}

/// Known Intel Wi-Fi device IDs (facts: the PCI ID tables of iwlwifi and pci.ids).
pub fn model_name(vendor: u16, device: u16) -> Option<&'static str> {
    if vendor != INTEL {
        return None;
    }
    Some(match device {
        0x43F0 => "Wi-Fi 6 AX201 (CNVi, chipset Tiger Lake-H)",
        0xA0F0 => "Wi-Fi 6 AX201 (CNVi, Tiger Lake)",
        0x02F0 => "Wi-Fi 6 AX201 (CNVi, Comet Lake)",
        0x06F0 => "Wi-Fi 6 AX201 (CNVi, Comet Lake-H)",
        0x34F0 => "Wi-Fi 6 AX201 (CNVi, Ice Lake)",
        0x2723 => "Wi-Fi 6 AX200",
        0x2725 => "Wi-Fi 6E AX210",
        0x51F0 => "Wi-Fi 6E AX211 (CNVi, Alder Lake)",
        0x7AF0 => "Wi-Fi 6E AX211 (CNVi, Alder Lake-S)",
        0x9DF0 => "Wireless-AC 9560 (CNVi)",
        _ => return None,
    })
}

/// MAC type: HW_REV bits 15:4 (iwlwifi `CSR_HW_REV_TYPE`). The names are INFERRED from iwlwifi.
pub fn mac_type(hw_rev: u32) -> (u32, Option<&'static str>) {
    let t = (hw_rev >> 4) & 0xFFF;
    let name = match t {
        0x334 => Some("Qu (B)"),
        0x338 => Some("Qu (C)"),
        0x354 => Some("QuZ"),
        0x340 => Some("Cc (AX200)"),
        0x420 => Some("Ty / So (AX210/AX211)"),
        _ => None,
    };
    (t, name)
}

/// Radio type: HW_RF_ID bits 23:12 (iwlwifi `CSR_HW_RFID_TYPE`). INFERRED names.
pub fn rf_type(rf_id: u32) -> (u32, Option<&'static str>) {
    let t = (rf_id >> 12) & 0xFFF;
    let name = match t {
        0x105 => Some("JF1"),
        0x106 => Some("JF2"),
        0x10A => Some("HR (Hrkn)"),
        0x10C => Some("HR1"),
        0x10D => Some("GF"),
        _ => None,
    };
    (t, name)
}

/// GP_CNTRL bits (iwlwifi): 0 = MAC clock ready, 2 = init done, 3 = MAC access request, 27 = radio switch (1 = allowed).
pub fn gp_cntrl_text(v: u32) -> String {
    format!(
        "ceasul MAC gata {}, init gata {}, cerere de acces MAC {}, comutatorul radio: {}",
        if v & 1 != 0 { "da" } else { "nu" },
        if v & 1 << 2 != 0 { "da" } else { "nu" },
        if v & 1 << 3 != 0 { "da" } else { "nu" },
        if v & 1 << 27 != 0 { "permite radio" } else { "oprit (sau MAC adormit)" }
    )
}

fn link_speed(code: u32) -> &'static str {
    match code {
        1 => "2,5 GT/s",
        2 => "5 GT/s",
        3 => "8 GT/s",
        4 => "16 GT/s",
        _ => "?",
    }
}

/// The report lines of one adapter.
pub fn lines_of(n: usize, f: &Facts) -> Vec<String> {
    let mut l = Vec::new();
    let tag = if f.simulated { "SIMULARE (dispozitivul QEMU edu): " } else { "" };
    l.push(format!(
        "{}wifi{}: {} {:04x}:{:04x} {} subsistem {:08x} rev {:02x} (clasa {:02x}:{:02x}:{:02x})",
        tag, n, f.address, f.vendor, f.device,
        model_name(f.vendor, f.device).unwrap_or("model necunoscut"),
        f.subsystem, f.revision, f.class.0, f.class.1, f.class.2
    ));
    l.push(format!(
        "wifi{}: {}; Command {:04x} (memorie {}, bus master {}), stare {}",
        n,
        if f.integrated { "integrata in chipset (magistrala 0: CNVi, radioul pe M.2)" } else { "pe magistrala PCIe (placa separata)" },
        f.command,
        if f.command & 2 != 0 { "da" } else { "nu" },
        if f.command & 4 != 0 { "da" } else { "nu" },
        f.power_state.map(|s| format!("D{}", s)).unwrap_or_else(|| String::from("necunoscuta (fara capabilitatea PM)"))
    ));
    for b in &f.bars {
        l.push(format!(
            "wifi{}: BAR{} {:#x} {}{}{}", n, b.index, b.phys,
            if b.io { "I/O" } else { "memorie" },
            if b.is64 { ", 64 biti" } else { "" },
            if b.prefetch { ", prefetch" } else { "" }
        ));
    }
    match f.link {
        Some((s, w)) => l.push(format!("wifi{}: legatura PCIe {} x{}", n, link_speed(s), w)),
        None => l.push(format!("wifi{}: fara capabilitatea PCI Express (normal pentru CNVi)", n)),
    }
    let caps: Vec<String> = f.capabilities.iter().map(|c| format!("{:02x}", c)).collect();
    l.push(format!("wifi{}: capabilitati PCI: {}", n, if caps.is_empty() { String::from("niciuna") } else { caps.join(" ") }));
    match &f.csr {
        Ok(c) => {
            let regs: Vec<String> = CSR_LIST.iter().zip(c.values).map(|((o, name), v)| format!("{}@{:03x}={:08x}", name, o, v)).collect();
            l.push(format!("wifi{}: registre CSR (doar citire): {}", n, regs.join(" ")));
            let (mt, mn) = mac_type(c.get(CSR_HW_REV));
            let (rt, rn) = rf_type(c.get(CSR_HW_RF_ID));
            l.push(format!(
                "wifi{}: MAC tip {:#05x} {} ; radio tip {:#05x} {} (nume deduse din iwlwifi)",
                n, mt, mn.unwrap_or("(necunoscut)"), rt,
                if c.get(CSR_HW_RF_ID) == 0 { "(0: citit doar dupa trezirea MAC-ului, W1)" } else { rn.unwrap_or("(necunoscut)") }
            ));
            l.push(format!("wifi{}: GP_CNTRL: {}", n, gp_cntrl_text(c.get(CSR_GP_CNTRL))));
        }
        Err(why) => l.push(format!("wifi{}: registrele CSR nu s-au citit: {}", n, why)),
    }
    l
}

/// `wifi identifica`: one line per entry; every line also goes to the log (prefix "wifi:").
pub fn identify(sim: bool) -> Vec<String> {
    let all = facts(sim);
    let mut lines = Vec::new();
    if all.is_empty() {
        lines.push(String::from("nicio placa Wi-Fi Intel gasita in PCI (clasa 02:80)"));
    }
    for (n, f) in all.iter().enumerate() {
        lines.extend(lines_of(n, f));
    }
    lines.push(String::from("zero scrieri in placa Wi-Fi (configuratie PCI si registre: doar citire; pasul W0 din C-118)"));
    for l in &lines {
        crate::info!("wifi: {}", l);
    }
    lines
}

/// One line for `test tot`: `None` = no adapter.
pub fn summary() -> Option<String> {
    let all = facts(false);
    let f = all.first()?;
    let regs = match &f.csr {
        Ok(c) => format!("HW_REV {:08x}, HW_RF_ID {:08x}", c.get(CSR_HW_REV), c.get(CSR_HW_RF_ID)),
        Err(why) => format!("registre necitite: {}", why),
    };
    Some(format!(
        "{} {:04x}:{:04x} {}; {}; nimic scris (W0)",
        f.address, f.vendor, f.device, model_name(f.vendor, f.device).unwrap_or("model necunoscut"), regs
    ))
}

/// Boot self-test of the decoding (every test build).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    assert!(mac_type(0x0000_3340 | 0x8) == (0x334, Some("Qu (B)")), "wifi: tipul MAC");
    assert!(mac_type(0x0000_3544).1 == Some("QuZ"), "wifi: QuZ");
    assert!(mac_type(0xFFFF_0000).1.is_none(), "wifi: tip necunoscut");
    assert!(rf_type(0x0010_A100) == (0x10A, Some("HR (Hrkn)")), "wifi: radio HR");
    let g = gp_cntrl_text(1 | 1 << 27);
    assert!(g.contains("ceasul MAC gata da") && g.contains("init gata nu") && g.contains("permite radio"), "wifi: GP_CNTRL {}", g);
    assert!(model_name(0x8086, 0x43F0).is_some_and(|m| m.contains("AX201")) && model_name(0x10EC, 0x43F0).is_none(), "wifi: model");
    let f = Facts {
        address: String::from("00:14.3"), vendor: 0x8086, device: 0x43F0, subsystem: 0x0074_8086, revision: 0x11,
        class: (2, 0x80, 0), integrated: true, command: 0x0406, power_state: Some(0), bars: Vec::new(), link: None,
        capabilities: alloc::vec![0x01, 0x05, 0x11],
        csr: Ok(Csr { values: [0, 0, 1 | 1 << 27, 0x0000_3540, 0] }), simulated: false,
    };
    let text = lines_of(0, &f).join("\n");
    assert!(text.contains("AX201") && text.contains("CNVi") && text.contains("QuZ") && text.contains("W1") && text.contains("01 05 11"), "wifi: raport {}", text);
    let off = Facts { command: 0x0400, csr: Err("Memory Space oprit de firmware (nu il pornim: ar fi o scriere)"), ..f };
    assert!(lines_of(0, &off).join("\n").contains("nu s-au citit: Memory Space oprit"), "wifi: refuz");
    crate::info!("test: wifi (W0) decodare OK (AX201, Qu/QuZ, HR, GP_CNTRL)");
}
