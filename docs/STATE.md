# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 11:40–14:30 UTC (14:40–17:30 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (22 de sarcini):** P1-11, P1-12, P1-13 (Faza 1 închisă) · P2-01 … P2-11 (Faza 2 închisă) ·
  P3-01 … P3-08 · P4-01, P4-02, P4-03. Adăugată sarcina nouă **P4-12** (cu plan).
- **Blocat:** **P3-09** (test pe calculatoare reale) — are nevoie de Adrian, vezi `BLOCKERS.md` B-001. P3-10 depinde de el.
- **Ce merge acum (rezumat pentru Adrian):**
  - Faza 1: nucleul rezistă 5 minute de stres fără pierderi; are tabele de pagini proprii (recuperează ~43 MiB de la bootloader).
  - Faza 2: mai multe programe rulează în paralel (planificator cu ceas), programe separate în modul utilizator (ELF),
    fișiere în memorie (initrd) și pe disc FAT32, shell cu `help`, `ls`, `cat`, `run`, `mem`, `uptime`, `lspci`, `lsusb`, `net`, `reboot`.
  - Faza 3 (în QEMU): PCI, discuri SATA (AHCI) și NVMe, USB (controler XHCI, tastatură, mouse, stick), placă de rețea
    Intel e1000/e1000e (ARP), imagine de stick USB care pornește (artefactul CI `stick-usb`, ghid `docs/USB.md`).
  - Faza 4 (început): mouse PS/2 și USB, desen în memorie cu copiere rapidă (~290 cadre/s pe 1280×800 în emulare),
    fontul are diacriticele românești.
- **Build:** OK, fără avertismente (normal, `--usb` și toate variantele de test).
- **Teste:** `tools/test.sh` → **24 de scenarii trecute** (boot, log, panic, divzero, pagefault, doublefault, unmapped, rodata,
  ahci, nvme, usb, usb-enum, usb-kbd, usb-disk, usb-mouse, mouse, net, net-e1000, pci-usb, smp, keyboard, stress-quick,
  shell, phase2). `tools/test.sh --stress` (5 min, 300 de procese) trecut. CI verde (un singur roșu, reparat imediat: P2-10).
- **Măsurători:** `docs/METRICS.md` (Faza 1 și Faza 2).

## Următoarea sarcină
**P4-04** — server grafic: ferestre și cursor. Plan: `docs/plans/P4-04.md`. Apoi P4-05 (ferestre pentru programe),
P4-07 (scriere FAT32, independentă), P4-12 (aranjament RO).
Când Adrian trimite rezultatele de pe calculatoarele reale: P3-09 (repararea problemelor găsite), apoi P3-10.

## Mediu (verificat la P0-01, sesiune cloud)
| Unealtă | Versiune |
|---|---|
| rustc (nightly) | 1.101.0-nightly (2026-10-19) |
| cargo (stable) | 1.97.0 |
| qemu-system-x86_64 | 8.2.2 |
| git | 2.43.0 |
| xorriso | 1.5.6 |
| OVMF | /usr/share/OVMF/OVMF_CODE_4M.fd |

## Cum funcționează testele (important pentru sesiunile următoare)
- Scenariile sunt în `tests/scenarios.txt`: nume | feature-uri cargo | `success`/`failed` | rânduri așteptate (`;;`) | captură | argumente QEMU sau `keys=...`.
- Un scenariu care trebuie să „pice” (excepție, panică) se termină cu codul QEMU 35 și are `failed`.
- În build-ul de test nucleul rulează `run_tests()` din `main.rs` și se termină cu `qemu::finish_test()`.
- Funcții noi folosite doar de teste: `#[cfg(feature = "qemu-test")]`; API pentru mai târziu: `#[allow(dead_code)]` cu motiv.

## Avertismente pentru sesiunea următoare
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install` (acum instalează și dosfstools, mtools, fdisk).
- **Cyber Hub inaccesibil din această sesiune:** unealta `add_repo` nu există aici și accesul GitHub e limitat la `cyber-universe`;
  clonarea `Adriancu91/cyber-hub` cere autentificare. `stare/` și `jurnal/` din hub NU au fost actualizate azi —
  sesiunea următoare (cu acces) trebuie să le scrie pe baza acestui fișier.
- Nu edita `tools/test.sh` cât rulează un test în fundal (bash citește scriptul pe bucăți).
- Discul de test (`build/disk.img`, 40 MiB) e atașat de QEMU ca SATA la orice rulare (`tools/qemu-common.sh`); în ISO intră
  doar cu feature-ul `test-disk` (Limine l-ar încărca ~13 s în emulare). `{BUILD}` în coloana QEMU din `scenarios.txt` = directorul build.
- Scenariile pot trimite taste (`keys=`) și comenzi de monitor QEMU (`mon=cmd:arg,cmd2`, `:` = spațiu).
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~4 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
