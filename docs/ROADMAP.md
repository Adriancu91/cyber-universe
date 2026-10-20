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

## Faza 6 — Auto-găzduire (1–2 ani de la start)
**Scop:** Cyber Universe se dezvoltă din interiorul lui.
- Compilator și instrumente care rulează în OS.
- Editor de cod și integrare cu Claude Code.

## Faza 7 — Compatibilitate Windows (continuu)
- Reconstruire clean-room a funcțiilor Windows necesare, după documentația publică.
- Încărcare de programe Windows simple (format PE), extinsă treptat.

---

## Principii permanente
1. Minim: fiecare funcție trebuie să aducă un plus clar.
2. Modular: tot ce nu e esențial pentru pornire e modul.
3. Rapid pe hardware slab: măsurăm timpul de pornire și memoria folosită la fiecare fază.
4. Testat automat: nimic nu intră în `main` fără test.
