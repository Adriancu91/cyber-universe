# Decizii tehnice

> O decizie nu se șterge și nu se modifică. Dacă se schimbă, se adaugă una nouă: „Înlocuiește D-00X”.

### D-001 — Nucleu propriu, de la zero (2026-10-20)
Cyber Universe are nucleu propriu, nu Linux. Decizia lui Adrian: control total, sistem minim.
Consecință acceptată: driverele pentru hardware real vin încet; dezvoltarea începe în QEMU.

### D-002 — Limbaj: Rust (2026-10-20)
Rust `no_std` pentru nucleu și module. Motiv: siguranța memoriei elimină o clasă mare de erori
(cele mai frecvente în nuclee scrise în C), iar Claude Code îl scrie bine. Assembly doar unde e obligatoriu.

### D-003 — Arhitectură: x86_64, pornire UEFI (2026-10-20)
Ținta: ThinkPad L15 Gen 2 (Intel i7-1185G7) și PC-uri x86_64 din ultimii ~12 ani. Fără BIOS vechi (legacy) la început.

### D-004 — Bootloader: Limine (2026-10-20)
Bootloader existent, open source (licență BSD), care pornește nucleul direct în 64 de biți și oferă
harta memoriei și framebuffer. Nu scriem bootloader propriu acum; poate deveni o fază ulterioară.
Versiunea exactă se fixează la P0-04 și se notează aici ca D-004a.

### D-004a — Versiunea Limine fixată (2026-10-20)
Limine **v12.9.4**, arhiva `limine-binary.tar.gz` din GitHub Releases,
SHA-256 `847334bc4bc142de996fbd132806b89ed73ac9ab34dbc1e90cfebf1fcc68c2b1` (verificată de `tools/build.sh`).
Crate Rust `limine = "=0.5.0"`. Confirmă D-009.

### D-005 — Licență: MIT (2026-10-20)
Codul propriu e MIT. Nu se copiază cod GPL în proiect (vezi `CLEAN_ROOM.md`).

### D-006 — Mediu de dezvoltare: WSL2 Ubuntu pe Windows 11 (2026-10-20)
Build-ul și testele rulează în Ubuntu sub WSL2 pe ThinkPad, plus GitHub Actions (Ubuntu).
Motiv: aceleași scripturi `.sh` funcționează local, în cloud și în CI.

### D-007 — Emulator: QEMU + OVMF (2026-10-20)
Testarea se face în QEMU cu firmware UEFI OVMF. Teste fără ecran prin portul serial.

### D-008 — Lucru automat în cloud, înainte de laptop (2026-10-20)
Verificat: sesiunile Claude din cloud (Ubuntu 24.04) pot instala QEMU, OVMF, xorriso și Rust nightly
și au pornit cu succes un prototip de nucleu. Fazele 0–2 se lucrează automat în cloud.
Laptopul (WSL2, D-006) rămâne pentru rulare cu ecran și pentru hardware real (Faza 3).

### D-009 — Versiuni propuse după studiu (2026-10-20)
Limine **v12.9.4** (arhiva binară din GitHub Releases) și crate-ul Rust `limine` **0.5.0** (depozitul oficial
`limine-bootloader/limine-rs`), testate împreună în prototip. Se confirmă definitiv în D-004a la P0-04, cu suma SHA-256.

### D-010 — Tratarea întreruperilor cu `extern "x86-interrupt"` (2026-10-20)
Handler-ele de excepții folosesc convenția `x86-interrupt` din Rust nightly (`#![feature(abi_x86_interrupt)]`),
nu stub-uri scrise în assembly. Motiv: mai puțin cod `unsafe`, compilatorul salvează registrele corect.
GDT, TSS și IDT sunt scrise de noi (fără crate-ul `x86_64`), după Intel SDM. Dacă vreodată convenția dispare
din nightly, înlocuirea e un set de stub-uri în assembly (o sarcină nouă).
