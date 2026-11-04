//! Our own small name tables for common PCI and USB hardware (F4, `hw lista`).
//!
//! Written by hand from the vendors' public product documentation and the ids the devices report; only devices we
//! meet often (QEMU, Adrian's laptop and workstation, the usual Wi-Fi cards). Anything else is shown by its numbers.

/// PCI vendor names (PCI-SIG vendor ids).
pub fn pci_vendor(vendor: u16) -> Option<&'static str> {
    Some(match vendor {
        0x8086 | 0x8087 => "Intel",
        0x10DE => "NVIDIA",
        0x10EC => "Realtek",
        0x1022 => "AMD",
        0x1002 => "AMD/ATI",
        0x14E4 => "Broadcom",
        0x168C => "Qualcomm Atheros",
        0x17CB => "Qualcomm",
        0x14C3 => "MediaTek",
        0x1B36 => "Red Hat (QEMU)",
        0x1AF4 => "Red Hat (virtio)",
        0x1234 => "QEMU/Bochs",
        0x15AD => "VMware",
        0x80EE => "VirtualBox",
        0x144D => "Samsung",
        0x15B7 => "SanDisk/WD",
        0x1987 => "Phison",
        0x1C5C => "SK hynix",
        0x1344 => "Micron",
        0x126F => "Silicon Motion",
        0x2646 => "Kingston",
        0x1B21 => "ASMedia",
        0x1912 => "Renesas",
        0x1106 => "VIA",
        _ => return None,
    })
}

/// PCI device names for the ids we meet often.
pub fn pci_device(vendor: u16, device: u16) -> Option<&'static str> {
    Some(match (vendor, device) {
        // QEMU q35 machine
        (0x8086, 0x29C0) => "82G33/P35 (Q35) punte gazda",
        (0x8086, 0x2918) => "ICH9 LPC",
        (0x8086, 0x2922) => "ICH9 SATA AHCI",
        (0x8086, 0x2930) => "ICH9 SMBus",
        (0x8086, 0x10D3) => "82574L Gigabit (e1000e)",
        (0x8086, 0x100E) => "82540EM Gigabit (e1000)",
        (0x1234, 0x1111) => "VGA standard (QEMU)",
        (0x1B36, 0x000D) => "XHCI (QEMU)",
        (0x1B36, 0x0010) => "NVMe (QEMU)",
        // Wired network
        (0x10EC, 0x8168) => "RTL8111/8168 Gigabit",
        (0x10EC, 0x8125) => "RTL8125 2.5 Gigabit",
        (0x8086, 0x15F3) => "I225-V 2.5 Gigabit",
        // Intel Wi-Fi
        (0x8086, 0x2723) => "Wi-Fi 6 AX200",
        (0x8086, 0x2725) => "Wi-Fi 6E AX210",
        (0x8086, 0x272B) => "Wi-Fi 7 BE200",
        (0x8086, 0x06F0) | (0x8086, 0x43F0) | (0x8086, 0xA0F0) => "Wi-Fi 6 AX201 (CNVi)",
        (0x8086, 0x51F0) | (0x8086, 0x7AF0) => "Wi-Fi 6E AX211 (CNVi)",
        (0x8086, 0x2526) => "Wireless-AC 9260",
        (0x8086, 0x9DF0) => "Wireless-AC 9560 (CNVi)",
        // Other Wi-Fi
        (0x10EC, 0xC821) => "RTL8821CE Wi-Fi",
        (0x10EC, 0xC822) => "RTL8822CE Wi-Fi",
        (0x10EC, 0xB852) => "RTL8852BE Wi-Fi 6",
        (0x14C3, 0x7961) => "MT7921 Wi-Fi 6",
        (0x14C3, 0x0616) => "MT7922 Wi-Fi 6E",
        (0x168C, 0x003E) => "QCA6174 Wi-Fi 5",
        // NVIDIA (the workstation's and the laptop's)
        (0x10DE, 0x2482) => "GeForce RTX 3070 Ti (GA104)",
        (0x10DE, 0x2520) | (0x10DE, 0x2560) => "GeForce RTX 3060 Laptop (GA106)",
        _ => return None,
    })
}

/// Class name from the PCI class code (PCI Code and ID Assignment specification).
pub fn pci_class(class: u8, subclass: u8, prog_if: u8) -> &'static str {
    match (class, subclass, prog_if) {
        (0x01, 0x01, _) => "controler IDE",
        (0x01, 0x06, _) => "controler SATA (AHCI)",
        (0x01, 0x08, _) => "controler NVMe",
        (0x01, _, _) => "controler de stocare",
        (0x02, 0x00, _) => "placa de retea Ethernet",
        (0x02, 0x80, _) => "alt controler de retea",
        (0x02, _, _) => "placa de retea",
        (0x03, 0x00, _) => "placa video (VGA)",
        (0x03, 0x02, _) => "placa video 3D",
        (0x03, _, _) => "placa video",
        (0x04, 0x01, _) => "audio",
        (0x04, 0x03, _) => "audio HD",
        (0x04, _, _) => "multimedia",
        (0x05, _, _) => "controler de memorie",
        (0x06, 0x00, _) => "punte gazda",
        (0x06, 0x01, _) => "punte ISA (LPC)",
        (0x06, 0x04, _) => "punte PCI",
        (0x06, _, _) => "punte",
        (0x07, _, _) => "comunicatii (serial)",
        (0x08, _, _) => "periferic de sistem",
        (0x09, _, _) => "dispozitiv de intrare",
        (0x0C, 0x03, 0x00) => "controler USB (UHCI)",
        (0x0C, 0x03, 0x10) => "controler USB (OHCI)",
        (0x0C, 0x03, 0x20) => "controler USB (EHCI)",
        (0x0C, 0x03, 0x30) => "controler USB (XHCI)",
        (0x0C, 0x03, _) => "controler USB",
        (0x0C, 0x05, _) => "SMBus",
        (0x0C, 0x80, _) => "controler de magistrala",
        (0x0C, _, _) => "magistrala seriala",
        (0x0D, _, _) => "controler fara fir",
        (0x10, _, _) => "criptare",
        (0x11, _, _) => "achizitie de semnal",
        (0x12, _, _) => "acceleratoare de procesare",
        (0x13, _, _) => "instrumentatie",
        _ => "alt dispozitiv",
    }
}

/// Is this PCI function a Wi-Fi card? Class 02/80 ("other network", how Intel, Realtek, MediaTek and Qualcomm
/// Wi-Fi cards report themselves), class 0D/20-21 (802.11 controller), or an id from our Wi-Fi table.
pub fn is_wifi(vendor: u16, device: u16, class: u8, subclass: u8) -> bool {
    matches!((class, subclass), (0x02, 0x80) | (0x0D, 0x20) | (0x0D, 0x21))
        || pci_device(vendor, device).is_some_and(|n| n.contains("Wi-Fi") || n.contains("Wireless"))
}

/// USB vendor names (USB-IF vendor ids) for the devices we meet often.
pub fn usb_vendor(vendor: u16) -> Option<&'static str> {
    Some(match vendor {
        0x0627 | 0x46F4 => "QEMU",
        0x1D6B => "Linux Foundation (hub)",
        0x8087 => "Intel",
        0x0BDA => "Realtek",
        0x046D => "Logitech",
        0x045E => "Microsoft",
        0x05AC => "Apple",
        0x0781 => "SanDisk",
        0x0951 => "Kingston",
        0x090C => "Silicon Motion",
        0x13D3 => "IMC Networks",
        0x0B05 => "ASUS",
        0x1532 => "Razer",
        0x1038 => "SteelSeries",
        0x04F2 => "Chicony",
        0x0930 => "Toshiba",
        _ => return None,
    })
}

/// USB speed from the xHCI Port Speed ID (xHCI 1.2, default speed ids).
pub fn usb_speed(speed: u8) -> &'static str {
    match speed {
        1 => "12 Mb/s (full)",
        2 => "1,5 Mb/s (low)",
        3 => "480 Mb/s (high)",
        4 => "5 Gb/s (super)",
        5 => "10 Gb/s (super+)",
        _ => "viteza necunoscuta",
    }
}

/// PCI capability names (PCI Local Bus spec, capability ids).
pub fn pci_capability(id: u8) -> &'static str {
    match id {
        0x01 => "PM",
        0x05 => "MSI",
        0x09 => "furnizor",
        0x0D => "punte-subsistem",
        0x10 => "PCIe",
        0x11 => "MSI-X",
        0x12 => "SATA",
        0x13 => "AF",
        _ => "?",
    }
}

/// Checks the tables on known ids (test builds, at boot).
pub fn self_test() -> bool {
    pci_vendor(0x8086) == Some("Intel")
        && pci_vendor(0x10DE) == Some("NVIDIA")
        && pci_vendor(0x10EC) == Some("Realtek")
        && pci_vendor(0xABCD).is_none()
        && pci_device(0x8086, 0x2922) == Some("ICH9 SATA AHCI")
        && pci_device(0x10DE, 0x2482) == Some("GeForce RTX 3070 Ti (GA104)")
        // Wi-Fi: by class (any vendor) and by id; wired cards and other classes are not Wi-Fi.
        && is_wifi(0x8086, 0xA0F0, 0x02, 0x80)
        && is_wifi(0x14C3, 0x7961, 0x02, 0x80)
        && is_wifi(0x1234, 0x5678, 0x0D, 0x20)
        && is_wifi(0x8086, 0x2725, 0x02, 0x00)
        && !is_wifi(0x8086, 0x10D3, 0x02, 0x00)
        && !is_wifi(0x10EC, 0x8168, 0x02, 0x00)
        && !is_wifi(0x10DE, 0x2482, 0x03, 0x00)
        && pci_class(0x0C, 0x03, 0x30) == "controler USB (XHCI)"
        && usb_vendor(0x0627) == Some("QEMU")
        && usb_speed(3) == "480 Mb/s (high)"
}
