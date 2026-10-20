# Coada de sarcini — Cyber Universe OS

Reguli:
- `[ ]` de făcut · `[~]` în lucru · `[x]` gata · `BLOCAT` = vezi `BLOCKERS.md`
- Următoarea sarcină = **prima `[ ]` nemarcată BLOCAT, cu dependențele `[x]`**.
- „Dep:” = sarcini care trebuie terminate înainte. „Test:” = cum se verifică că e gata.
- Claude Code adaugă sarcini noi la finalul fazei curente când descoperă muncă necesară (cu ID nou).
- **Fiecare sarcină are plan detaliat în `docs/plans/<ID>.md`** — citește-l înainte să începi.

---

## Faza 0 — Fundația

- [x] **P0-01** Verifică mediul: `rustup`, `cargo`, `qemu-system-x86_64`, `git`, `xorriso`, `make`. Scrie versiunile în STATE.md.
  Test: toate comenzile răspund la `--version`. Dacă lipsește ceva → BLOCKERS.md cu comanda de instalare pentru Adrian.
- [x] **P0-02** Configurează Rust nightly + `rust-src` + ținta `x86_64-unknown-none` (`rust-toolchain.toml`).
  Dep: P0-01. Test: `cargo build` pe un crate gol `no_std` reușește.
- [x] **P0-03** Nucleu minim `kernel/`: `no_std`, `no_main`, punct de intrare, buclă `hlt`, handler de panică.
  Dep: P0-02. Test: se construiește fără erori.
- [x] **P0-04** `tools/build.sh`: construiește nucleul, descarcă Limine (versiune fixată în DECISIONS), creează `build/cyber-universe.iso`.
  Dep: P0-03. Test: fișierul ISO există și are peste 1 MB.
- [x] **P0-05** Ieșire pe portul serial COM1: nucleul scrie `CYBER UNIVERSE OS: BOOT OK`.
  Dep: P0-04.
- [x] **P0-06** `tools/run.sh` (QEMU cu ecran, firmware OVMF/UEFI) și `tools/test.sh` (fără ecran, timeout 30 s, caută textul așteptat; cod ieșire 0/1).
  Dep: P0-05. Test: `tools/test.sh` returnează 0.
- [x] **P0-07** Ieșire QEMU automată după test (dispozitiv `isa-debug-exit`), ca testele să nu aștepte timeout-ul.
  Dep: P0-06.
- [x] **P0-08** GitHub Actions: la fiecare push → build + `tools/test.sh`.
  Dep: P0-07. Test: insigna verde pe GitHub.
- [x] **P0-09** Text pe ecran prin framebuffer-ul Limine (font bitmap propriu 8x16): „Cyber Universe OS”.
  Dep: P0-05. Test: captură de ecran QEMU (`screendump`) salvată în `tests/screens/` + verificare că nu e neagră.

## Faza 1 — Nucleul de bază

- [x] **P1-01** Jurnal (logger) unificat: serial + ecran, niveluri INFO/WARN/ERR. Dep: P0-09.
- [x] **P1-02** GDT + TSS. Dep: P1-01.
- [x] **P1-03** IDT + excepții (div zero, page fault, double fault cu stivă separată) cu mesaj clar. Dep: P1-02.
  Test: test care provoacă intenționat o excepție și verifică mesajul.
- [x] **P1-04** Harta memoriei de la Limine afișată în jurnal. Dep: P1-01.
- [x] **P1-05** Alocator de cadre fizice (bitmap). Dep: P1-04. Test: alocă/eliberează 10.000 de cadre fără scurgeri.
- [x] **P1-06** Paginare: mapare/demapare pagini, tabele proprii. Dep: P1-05.
- [x] **P1-07** Heap pentru nucleu (`alloc`: `Vec`, `Box`). Dep: P1-06. Test: `Vec` cu 1 milion de elemente.
- [x] **P1-08** ACPI: găsire tabele (RSDP, MADT). Dep: P1-06.
- [x] **P1-09** APIC local + temporizator, ceas al sistemului (ticks). Dep: P1-03, P1-08.
- [x] **P1-10** Tastatură PS/2 (scancode → caractere, aranjament US). Dep: P1-03, P1-08.
  Test: QEMU `sendkey` trimite taste, nucleul le afișează pe serial.
- [x] **P1-11** Test de stabilitate: nucleul rulează 5 minute în QEMU, temporizator activ, fără erori. Dep: P1-09.
  Test: `tools/test.sh --stress` (5 min, zilnic în `stress.yml`); varianta de 15 s `stress-quick` la fiecare push.
- [x] **P1-12** Măsurători: timp de pornire și memorie folosită, scrise în `docs/METRICS.md`. Dep: P1-11.
  `tools/metrics.sh` (5 porniri, media); sarcinile Fazei 2 generate (P2-01…P2-11, cu planuri).

- [x] **P1-13** Tabele de pagini proprii (în locul celor Limine) + recuperarea memoriei bootloader-ului. Dep: P1-07. Plan: `docs/plans/P1-13.md`.
  Test: scenariile `log` (recuperat ~43 MiB, toate cadrele scrise) și `rodata`.

## Faza 2 — Procese și sistem

Scop (ROADMAP): mai multe programe rulând în paralel, separate de nucleu. Gata când: din shell se poate porni
un program separat care citește un fișier. Fiecare sarcină are plan în `docs/plans/`.

- [x] **P2-01** Fire de execuție ale nucleului + comutare de context (cooperativ). Dep: P1-11.
  Test: scenariul `log` — `test: fire A1 B1 C1 A2 B2 C2 A3 B3 C3, stive eliberate`.
- [x] **P2-02** Planificator preemptiv round-robin pe temporizator. Dep: P2-01.
  Test: scenariul `log` — `test: preemptiune OK` (două fire care nu cedează procesorul primesc amândouă timp).
- [x] **P2-03** Așteptare și sincronizare: `sleep_ms`, mutex, cozi de așteptare (și pentru tastatură). Dep: P2-02.
  Test: scenariul `log` — `sleep_ms(500) = 50 ticks; mutex: 4 fire x 10000 = 40000`; `keyboard` așteaptă tastele dormind.
- [x] **P2-04** Spații de adrese separate pentru procese. Dep: P1-13, P2-01.
  Test: scenariul `log` — `test: spatii de adrese separate OK`.
- [x] **P2-05** Mod utilizator (ring 3) + apeluri de sistem (`syscall`): `write`, `exit`, `yield`, `sleep_ms`. Dep: P2-02, P2-04.
  Test: scenariul `log` — `salut din ring 3`, cod 7; programul cu eroare e oprit, nucleul continuă.
- [x] **P2-06** Încărcarea programelor ELF64 (programe în `user/`, puse în ISO). Dep: P2-05.
  Test: scenariul `log` — `hello din ELF`, `test: ELF OK` (fișiere stricate refuzate, memoria eliberată).
- [x] **P2-07** VFS + ramfs, fișiere inițiale dintr-o arhivă `tar` (initrd). Dep: P1-07.
  Test: scenariul `log` — `test: ramfs OK`; programele se încarcă din `/bin` (initrd).
- [x] **P2-08** Apeluri de sistem pentru fișiere (`open`/`read`/`close`) + pornirea programelor din ramfs. Dep: P2-06, P2-07.
  Test: scenariul `log` — `/bin/cat /etc/motd` afișează fișierul; fișiere lipsă → cod 1.
- [x] **P2-09** Shell: `help`, `ls`, `cat`, `run`, `mem`, `uptime`, `reboot`. Dep: P2-03, P2-08, P1-10.
  Test: scenariul `shell` (taste trimise prin QEMU: help, ls, mem, run cat, comandă greșită, uptime).
- [x] **P2-10** Citire FAT32 (doar citire) peste o interfață de disc; imagine de test în memorie. Dep: P2-07.
  Test: scenariul `log` — `test: FAT32 OK` (GPT și MBR, nume lungi, 100 KiB cu suma corectă); `shell` face `ls /disk`.
- [x] **P2-11** Închiderea Fazei 2: test complet, stabilitate extinsă, măsurători, generarea sarcinilor Fazei 3. Dep: P2-09, P2-10.
  Test: scenariul `phase2` (shell → `run cat /disk/salut.txt`), stres 5 min cu 300 de procese, rând nou în METRICS.md, sarcinile P3 generate.

## Faza 3 — Hardware real

Scop (ROADMAP): pornire de pe stick USB pe calculatoare reale. Gata când: pornește de pe stick pe cel puțin
2 calculatoare reale și citește un fișier de pe disc. Toate driverele se testează întâi în QEMU (care emulează
AHCI, NVMe, XHCI, e1000). Fiecare sarcină are plan în `docs/plans/`.

- [x] **P3-01** PCI: enumerarea dispozitivelor (ECAM din ACPI MCFG), comanda `lspci`. Dep: P2-11.
  Test: scenariile `log`, `pci-usb` (cu `-device qemu-xhci`) și `shell` (`lspci`).
- [x] **P3-02** Driver AHCI (SATA), doar citire; `/disk` de pe disc real. Dep: P3-01.
  Test: scenariul `ahci` (discul de test ca SATA → `/disk` prin AHCI); `shell` și `phase2` citesc tot prin AHCI.
- [x] **P3-03** Driver NVMe, doar citire. Dep: P3-01.
  Test: scenariul `nvme` (discul de test ca NVMe → `/disk`).
- [x] **P3-04** Imagine pentru stick USB (`tools/build.sh --usb`) + ghid `docs/USB.md`. Dep: P2-11.
  Test: scenariul `usb` (QEMU pornește de pe imaginea de stick prin XHCI + usb-storage); imaginea se descarcă din CI (artefactul `stick-usb`).
- [x] **P3-05** Controler USB XHCI: pornire și enumerarea dispozitivelor, `lsusb`. Dep: P3-01.
  Test: scenariul `usb-enum` (tastatură + stick USB găsite cu clasa corectă); `shell` (`lsusb`).
- [x] **P3-06** Tastatură USB (HID boot protocol). Dep: P3-05.
  Test: scenariul `usb-kbd` (QEMU tastează pe tastatura USB; nucleul primește h, i, Enter).
- [x] **P3-07** Stick USB (Mass Storage, Bulk-Only + SCSI) ca dispozitiv de blocuri, montat la `/usb`. Dep: P3-05.
  Test: scenariul `usb-disk` (discul de test ca stick USB → `/usb`, suma fișierului de 100 KiB corectă).
- [x] **P3-08** Placă de rețea Intel e1000/e1000e: cadre Ethernet, test ARP. Dep: P3-01.
  Test: scenariile `net` (e1000e) și `net-e1000` (cerere ARP → răspuns de la 10.0.2.2); `shell` (`net`).
- [ ] BLOCAT **P3-09** Test pe calculatoare reale (ThinkPad L15 Gen 2 + încă unul), `docs/HARDWARE.md`. **Are nevoie de Adrian.** Dep: P3-02, P3-03, P3-04, P3-06.
- [ ] **P3-10** Închiderea Fazei 3: măsurători, generarea sarcinilor Fazei 4. Dep: P3-09.

## Faza 4 — Interfața și sistemul de module

Scop (ROADMAP): un sistem folosibil de un om, nu doar de programator. Gata când: se poate instala și
dezinstala un modul din interfață. ROADMAP: Faza 4 merge în paralel cu Faza 3 (sarcinile au fost generate mai devreme,
cât P3-09 așteaptă testul pe hardware real). Fiecare sarcină are plan în `docs/plans/`.

- [x] **P4-01** Mouse (PS/2 și USB). Dep: P3-06.
  Test: scenariile `mouse` (PS/2) și `usb-mouse` (QEMU mișcă mouse-ul și face clic).
- [x] **P4-02** Bibliotecă grafică: buffer dublu și desen rapid. Dep: P2-11.
  Test: scenariul `log` — `test: grafica OK` (pixeli, zone schimbate, copiere pe ecran; ~290 cadre/s pe tot ecranul în QEMU).
- [x] **P4-03** Fonturi: diacritice românești și text UTF-8. Dep: P4-02.
  Test: scenariul `log` (pixelii lui Ș, ă și ai chenarului pentru caractere necunoscute); captura `boot` arată diacriticele. Aranjamentul RO → P4-12.
- [x] **P4-04** Server grafic: ferestre și cursor. Dep: P4-01, P4-02, P4-03.
- [x] **P4-05** Ferestre pentru programe (apeluri de sistem, memorie partajată). Dep: P4-04, P2-08.
- [x] **P4-06** Desktop „Cyber Universe”. Dep: P4-05.
- [x] **P4-07** FAT32: scriere (fișiere noi, modificare, ștergere). Dep: P2-10.
- [x] **P4-08** Manager de module (modmgr). Dep: P4-07, P2-08.
- [x] **P4-09** Aplicații de sistem: monitor sistem și fișiere. Dep: P4-06, P4-08.
- [x] **P4-10** Aplicații de sistem: editor de text și setări. Dep: P4-09.
- [x] **P4-11** Module din interfață și închiderea Fazei 4. Dep: P4-10.
- [x] **P4-12** Aranjamentul românesc de tastatură + literele cu „coadă” (g, j, p, q, y). Dep: P4-03.
- [x] **P4-13** Terminal (shell-ul) ca aplicație în fereastră; desktopul pornește automat după nucleu. Dep: P4-06.

## Faza 5 — Rețea și aplicațiile proprii

Generate la închiderea Fazei 4 (P4-11, 2026-10-20). Fiecare are plan în `docs/plans/`.

- [x] **P5-01** Stiva de rețea: Ethernet, ARP, IPv4, ICMP (ping). Dep: P3-08.
- [x] **P5-02** UDP și DHCP. Dep: P5-01.
- [x] **P5-03** DNS. Dep: P5-02.
- [x] **P5-04** TCP (client). Dep: P5-01.
- [x] **P5-05** Rețea pentru programe (socket-uri) și aplicația „Rețea”. Dep: P5-03, P5-04.
- [x] **P5-06** Client TLS 1.3 (biblioteca `tls/`), testat pe gazdă cu un server OpenSSL. Criptografie proprie (D-011). Dep: P5-10 … P5-15.
- [~] **P5-16** HTTPS în OS: `fetch https://…`, rădăcini în initrd, ora din RTC, numere aleatoare. Dep: P5-05, P5-06.
- [ ] BLOCAT **P5-07** Suita office ca modul (după ce citim proiectul din Cyber Hub). Dep: P4-11.
- [ ] **P5-08** Kronos ca modul. Dep: P5-07.
- [x] **P5-10** Criptografie 1: SHA-256, SHA-384, HMAC, HKDF (crate `crypto/`), cu vectorii NIST/RFC. Dep: —.
- [x] **P5-11** Criptografie 2: X25519 (schimb de chei, RFC 7748). Dep: P5-10.
- [x] **P5-12** Criptografie 3: ChaCha20-Poly1305 (RFC 8439). Dep: P5-10.
- [x] **P5-13** Criptografie 4: numere mari și semnături RSA (PKCS#1 v1.5 și PSS). Dep: P5-10.
- [x] **P5-14** Criptografie 5: semnături ECDSA P-256. Dep: P5-10.
- [x] **P5-15** Criptografie 6: certificate X.509 (DER) și lanțul de încredere. Dep: P5-13, P5-14.
- [ ] **P5-09** Închiderea Fazei 5. Dep: P5-01 … P5-08, P5-16.

## Faza 6 și următoarele

Sarcinile se generează de Claude Code din `ROADMAP.md` la închiderea fazei anterioare (P5-09),
în același format, cu ID-uri `P6-xx`, `P7-xx` etc.
