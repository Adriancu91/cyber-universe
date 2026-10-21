# Roadmap — Cyber Universe OS

Fazele se fac în ordine. O fază se consideră terminată doar când toate criteriile ei sunt îndeplinite
și testele automate trec. Duratele sunt estimative.

---

## Faza 0 — Fundația proiectului (1–2 săptămâni)
**Scop:** mediu de lucru complet, build și test automat.
- Toolchain Rust nightly pentru țintă `x86_64-unknown-none`.
- Bootloader Limine, imagine ISO/UEFI generată automat.
- Scripturi: build, rulare QEMU cu ecran, test QEMU fără ecran.
- Nucleul pornește și scrie „Cyber Universe OS” pe portul serial și pe ecran.
- CI pe GitHub Actions: fiecare push construiește și rulează testul de pornire.

**Gata când:** `tools/test.sh` trece local și în GitHub Actions.

## Faza 1 — Nucleul de bază (1–3 luni)
**Scop:** un nucleu stabil care gestionează procesorul și memoria.
- Afișare text pe ecran (framebuffer) și jurnal pe serial.
- Tabele GDT/IDT, gestionarea excepțiilor (nucleul nu „îngheață” fără mesaj).
- Întreruperi: temporizator (APIC), tastatură.
- Memorie: harta memoriei, alocator de cadre, paginare, heap pentru nucleu.
- Teste automate pentru fiecare componentă.

**Gata când:** nucleul rulează minute în șir în QEMU, primește tastatura, alocă și eliberează memorie fără erori.

## Faza 2 — Procese și sistem (2–4 luni)
**Scop:** mai multe programe rulând în paralel, separate de nucleu.
- Planificator de sarcini (multitasking).
- Mod utilizator (ring 3), apeluri de sistem.
- Încărcare programe (format ELF).
- Sistem de fișiere în memorie (ramfs), apoi citire FAT32 de pe disc.
- Consolă/shell simplu: comenzi `help`, `ls`, `cat`, `run`, `mem`, `reboot`.

**Gata când:** din shell se poate porni un program separat care citește un fișier.

## Faza 3 — Hardware real (2–4 luni, în paralel cu faza 4)
**Scop:** pornire de pe stick USB pe calculatoare reale.
- Pornire UEFI pe ThinkPad L15 Gen 2 și pe calculatoare de test.
- Drivere: disc (AHCI/NVMe), USB (XHCI: tastatură, stick), rețea pe cablu (Intel e1000/e1000e).
- Jurnal de compatibilitate hardware: `docs/HARDWARE.md`.

**Gata când:** pornește de pe stick pe cel puțin 2 calculatoare reale și citește un fișier de pe disc.

## Faza 4 — Interfața și sistemul de module (3–6 luni)
**Scop:** un sistem folosibil de un om, nu doar de programator.
- Server grafic simplu: ferestre, mouse, fonturi.
- Desktop „Cyber Universe”, gândit pentru hardware slab.
- Manager de module: instalare, pornire, oprire, ștergere module/aplicații.
- Primele aplicații de sistem: fișiere, editor text, setări, monitor sistem.

**Gata când:** se poate instala și dezinstala un modul din interfață.

## Faza 5 — Rețea și aplicațiile proprii (3–6 luni)
- Stivă TCP/IP, DHCP, DNS, HTTPS de bază.
- Mutarea treptată a aplicațiilor proprii ca module: suita office, apoi Kronos.

## Faza 6 — Loc de lucru cu Claude înăuntru (decizia lui Adrian, 2026-10-21 22:25)
**Scop:** pe laptopul de test (Lenovo AMD, rețea Realtek `10ec:8168`), Cyber Universe devine un loc de lucru: rețea pe cablu,
HTTPS cu site-uri adevărate, aplicația „Claude” (chat prin API, cu unelte în OS), actualizare din OS, trimitere pe GitHub.
- Driver Realtek RTL8168/8111 (clean-room, după documentația publică), testat pe gazdă cu un model software al cipului.
- Rădăcinile reale de încredere (lista Mozilla, MPL 2.0, doar ca date) + `fetch https://` cu un site real.
- Parser JSON propriu; aplicația „Claude”: cheia din `/usb/claude.key` (niciodată în repo), chat în fereastră, unelte
  (citește/scrie fișier, listează dosar, rulează comandă) — scrierea și rularea doar cu confirmarea lui Adrian pe ecran.
- Auto-actualizare: CI publică un GitHub Release cu fișierele stick-ului + sume SHA-256; OS-ul le descarcă și rescrie stick-ul.
- Trimitere pe GitHub din OS (API `contents`, token din fișier de pe stick).
- **Nu e obiectiv acum:** compilarea OS-ului în interiorul OS-ului (construirea rămâne în CI). Wi-Fi (Intel AX200): mai târziu.

**Gata când:** pe laptopul de test, din OS: Adrian vorbește cu Claude în fereastră, Claude citește un fișier și (cu confirmare)
scrie unul, iar OS-ul se actualizează singur de pe GitHub.

## Faza 7 — Compatibilitate Windows (continuu)
- Reconstruire clean-room a funcțiilor Windows necesare, după documentația publică.
- Încărcare de programe Windows simple (format PE), extinsă treptat.

## Mai târziu (din fazele anterioare)
- Aplicațiile proprii ca module: suita office (P5-07), apoi Kronos (P5-08) — după ce suita își atinge ținta (decizia din 20 oct).
- Auto-găzduirea (compilator și instrumente în OS) — nu e obiectiv acum (decizia din 21 oct 22:25).
- Wi-Fi (Intel AX200 — cere firmware de la producător).

---

## Principii permanente
1. Minim: fiecare funcție trebuie să aducă un plus clar.
2. Modular: tot ce nu e esențial pentru pornire e modul.
3. Rapid pe hardware slab: măsurăm timpul de pornire și memoria folosită la fiecare fază.
4. Testat automat: nimic nu intră în `main` fără test.
