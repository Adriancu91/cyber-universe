# Studiu pentru Faza 0 și Faza 1

Data: 2026-10-20. Tot ce e aici a fost verificat practic sau din surse oficiale în această dată.

## 1. Rezultatul principal: prototipul pornește

În `docs/research/prototip-p0/` există un nucleu minim care a fost **construit și pornit în QEMU**:

```
BdsDxe: starting Boot0001 "UEFI QEMU DVD-ROM ..."
CYBER UNIVERSE OS: BOOT OK
FRAMEBUFFER OK
```
- Timp de la pornirea QEMU până la ieșire: ~4 secunde (inclusiv firmware-ul UEFI).
- QEMU s-a închis singur prin `isa-debug-exit` cu codul **33** (= `(0x10 << 1) | 1`). Testele trebuie să considere 33 = succes.
- Prototipul e **referință de studiu**, nu cod final. Sarcinile P0-02…P0-09 îl rescriu curat în `kernel/` și `tools/`.

## 2. Versiuni verificate (fixate)

| Componentă | Versiune | Observație |
|---|---|---|
| Rust | nightly (testat: 1.101.0-nightly 2026-10-19) | necesar pentru `rust-src` / `build-std` |
| Țintă Rust | `x86_64-unknown-none` | ținta oficială pentru nuclee x86_64 |
| Limine (binar) | **v12.9.4**, `limine-binary.tar.gz` din GitHub Releases | conține `BOOTX64.EFI`, `limine-uefi-cd.bin` |
| crate `limine` | **0.5.0** | repo oficial `limine-bootloader/limine-rs`; API: `BaseRevision::new()`, `FramebufferRequest`, `RequestsStartMarker/EndMarker`. Există și 0.6.x (alt depozit, API diferit) — NU se folosește fără decizie nouă |
| QEMU | 8.2.2 (Ubuntu 24.04) | `qemu-system-x86_64 -M q35` |
| OVMF | pachetul Ubuntu `ovmf` | `/usr/share/OVMF/OVMF_CODE_4M.fd` + copie a `OVMF_VARS_4M.fd` |
| xorriso | pachetul Ubuntu | creează ISO-ul UEFI |
| Crate-uri utile (opționale) | `x86_64` 0.15.5, `uart_16550` 0.9.0, `spin` 0.12.3 | se pot folosi; se notează în DECISIONS dacă se adoptă |

## 3. Detalii tehnice confirmate

- **Linker script:** adresă de bază `0xffffffff80000000`, secțiuni separate pentru cererile Limine
  (`.requests_start_marker`, `.requests`, `.requests_end_marker`), aliniere la pagină, `ENTRY(kmain)`.
- **Flag-uri Rust:** `relocation-model=static`, `code-model=kernel`, `link-arg=-Tlinker.ld`, `panic = "abort"`.
- **Rust 2024:** atributele trebuie scrise `#[unsafe(no_mangle)]` și `#[unsafe(link_section = "...")]`.
- **Serial COM1:** port `0x3F8`; se așteaptă bitul 5 din registrul `0x3F8+5` înainte de fiecare octet.
  Inițializarea completă (viteză, format) se face la P0-05.
- **Framebuffer Limine:** 32 de biți pe pixel, `pitch` în octeți; testat prin umplerea ecranului.
- **ISO UEFI:** `xorriso -as mkisofs -R -r -J --efi-boot boot/limine/limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label`.
- **Configurare Limine** (`boot/limine/limine.conf`): `timeout: 0`, intrare `/Cyber Universe OS`, `protocol: limine`, `path: boot():/boot/kernel`.

## 4. Mediul de lucru automat (cloud)

Verificat azi: sesiunile Claude din cloud rulează **Ubuntu 24.04** și pot instala singure QEMU, OVMF, xorriso și Rust nightly.
Consecință: **Faza 0 și Faza 1 se pot lucra complet automat, fără laptopul lui Adrian.**
Laptopul (WSL2) e necesar doar pentru: vizualizare cu ecran (`tools/run.sh`) și, din Faza 3, pornire pe hardware real.

## 5. Hardware țintă: ThinkPad L15 Gen 2 (Intel)

Sursa: specificația oficială Lenovo PSREF.

| Componentă | Model | Când ne interesează |
|---|---|---|
| Procesor | Intel i7-1185G7 (Tiger Lake) | Faza 1: APIC, temporizator |
| Stocare | M.2 NVMe (PCIe 3.0 x4) | Faza 3: driver NVMe |
| Rețea cablu | Intel I219-LM / I219-V (pe unele modele lipsește) | Faza 3: driver e1000e |
| Wi-Fi | Intel AX200/AX210, Mediatek MT7921 sau Realtek RTL8852AE | târziu — drivere Wi-Fi sunt foarte complexe |
| USB | USB 3.2, USB-C, Thunderbolt 4 | Faza 3: XHCI |
| Grafică | Intel Iris Xe | framebuffer-ul UEFI ajunge la început; driver propriu mult mai târziu |
| Audio | Realtek ALC3287 (HD Audio) | Faza 4+ |
| Tastatură/touchpad | probabil PS/2 emulat + I2C-HID | de verificat la P3 (tastatura internă a laptopurilor moderne nu e garantat PS/2) |

**Atenție pentru Faza 3:** pe laptopuri moderne, tastatura internă poate să nu răspundă ca PS/2.
Primul test pe hardware real se face cu o **tastatură USB** sau doar cu jurnal pe ecran.

## 6. Surse
- Limine: github.com/limine-bootloader/limine (Releases, tag v12.9.4)
- crate limine 0.5.0: docs.rs/limine/0.5.0
- OSDev Wiki: „Limine Bare Bones”, „Serial Ports”, „GDT”, „IDT”, „Paging”, „APIC”, „PS/2 Keyboard”, „ACPI”
- Intel® 64 and IA-32 Architectures Software Developer's Manual (Vol. 3A)
- ACPI Specification 6.5 (uefi.org)
- Lenovo PSREF: ThinkPad L15 Gen 2 (Intel)
