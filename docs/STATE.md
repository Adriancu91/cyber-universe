# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 14:40–18:05 UTC (17:40–21:05 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (12 sarcini):** P4-04 … P4-13 (Faza 4 închisă: criteriul „se instalează și dezinstalează un modul din
  interfață” e îndeplinit și testat cu mouse-ul) · P5-01, P5-02. Adăugate: sarcina **P4-13** și sarcinile **Fazei 5**
  (P5-01 … P5-09), toate cu planuri.
- **Blocat:** **P3-09** (test pe calculatoare reale) — are nevoie de Adrian, vezi `BLOCKERS.md` B-001. P3-10 depinde de el.
- **Ce merge acum (rezumat pentru Adrian):**
  - Interfața grafică: ferestre care se mută, se suprapun și se închid cu mouse-ul; programele își deschid ferestre;
    **desktopul „Cyber Universe” pornește singur** după nucleu (bară jos cu Start și ceas; Esc → consola text).
  - Aplicații: Terminal (shell-ul în fereastră), Fișiere (navigare, copiere, ștergere), Editor de text (salvare),
    Monitor (programe, memorie, procesor, oprirea unui program), Setări (fus orar, tastatură), Module (instalare,
    pornire, oprire, ștergere), demo.
  - Discuri: scriere FAT32 (fișiere, directoare, ștergere) — DOAR pe partițiile cu fișierul `cyber.rw` în rădăcină.
  - Module: pachete `.cum`, managerul de module (shell `mod ...` și aplicația Module), dependențe.
  - Tastatura română standard (ă î â ș ț, AltGr), săgeți, Ctrl; fontul are literele cu coadă (g j p q y).
  - Rețea: DHCP (adresă automată), ARP, IPv4, ping (`ping 10.0.2.2` în shell).
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **36 de scenarii trecute** (noi: windows, appwindow, desktop, fatwrite, modules, sysapps,
  editor, phase4, terminal, keyboard-ro, keyboard-ro-usb, layout-boot; net/net-e1000 verifică acum și DHCP + ping).
  `tools/test.sh --stress` (5 min) trecut după schimbările din planificator. CI verde pe toate commit-urile până la P4-12
  (ultimele două erau în curs la final).
- **Reparate pe drum:** o cursă la ieșirea proceselor (tabelele de pagini eliberate puteau fi reîncărcate la o comutare),
  o cursă în testul de spații de adrese, împărțirea inegală a procesorului când un fir se trezește la fiecare tick
  (felie minimă de 5 ms). Măsurători: `docs/METRICS.md` (rând nou la sfârșitul Fazei 4; desktopul: 10 MiB în total).

## Următoarea sarcină
**P5-03** — DNS. Plan: `docs/plans/P5-03.md` (atenție la serverul DNS de test: rețeaua „user” a QEMU trimite DNS-ul
către serverul gazdei; încearcă întâi aranjamentul din plan). Apoi P5-04 (TCP), P5-05, P5-07.
**P5-06 cere o decizie de la Adrian** (criptografie proprie sau o bibliotecă open-source ca dependență).
Când Adrian trimite rezultatele de pe calculatoarele reale: P3-09, apoi P3-10.

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
- Scenariile pot trimite taste (`keys=`) și comenzi de monitor QEMU (`mon=cmd:arg,cmd2`, `:` = spațiu). Când le ai pe
  amândouă, scrie `mon=` ÎNAINTEA lui `keys=` (keys ia restul rândului). Tastele pleacă primele, apoi comenzile.
- Rândurile așteptate nu pot conține `|` (e separatorul coloanelor din `scenarios.txt`).
- `disk=NUME.img` în coloana QEMU: discul de test e o copie nouă, marcată pentru scriere (`cyber.rw`), fără snapshot;
  `tests/disk/NUME/` (dacă există) se copiază în ea; după QEMU rulează `tools/after-<scenariu>.sh` (ex. `fsck.fat`).
- Captura la alt moment decât TEST READY: coloana captură `fișier.png@rândul de pe serial`.
- Testele de interfață presupun ecran 1280×800 (QEMU) și ferestre centrate; cursorul pornește în mijlocul ecranului.
  Clicurile simulate din teste: `input::push_mouse`, tastele: `keyboard::push_key`; `ui::cursor()`/`ui::top_window()` ajută.
- Meniul Start listează tot ce e în `/bin` (fără `desktop`): un program nou schimbă numărul de rânduri — actualizează
  `items` în `test_desktop` și rândul `/bin = ...` din scenariul `log`.
- Programele n-au heap: stiva lor e 64 KiB; imaginile ferestrelor sunt `static mut` în program.
- Build-ul normal pornește desktopul; build-ul de test nu (testele pornesc ce le trebuie).
- **Atenție: la 17:41 UTC a pornit o ALTĂ sesiune programată în paralel** (a marcat P5-02 „în lucru” în timp ce
  această sesiune îl termina; conflictul din TASKS.md a fost rezolvat prin merge). Dacă două sesiuni se suprapun,
  fă `git pull` des și verifică TASKS.md înainte să începi o sarcină.
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~4 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
