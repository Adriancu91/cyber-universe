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
- [x] **P3-09** Test pe un calculator real, `docs/HARDWARE.md`. Dep: P3-02, P3-03, P3-04, P3-06.
  Rezultat (2026-10-21, comanda C-2): Lenovo ThinkPad AMD — pornește până la desktop; ecran, touchpad, tastatură, editor merg; rețeaua nu (Realtek → P6-01).
- [x] **P3-11** Testul 2 pe hardware real: comanda `hw` (raport pe o pagină) + lista pentru Adrian. Dep: P3-09.
  Test: scenariul `shell` (`hw`). Pozele de la Adrian completează `HARDWARE.md` (fără ele P3-10 așteaptă).
- [ ] **P3-10** Închiderea Fazei 3: măsurători, rezumat. Dep: P3-11 + pozele testului 2 de la Adrian. **Are nevoie de Adrian.**

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
- [x] **P5-16** HTTPS în OS: `fetch https://…`, rădăcini în initrd, ora din RTC, numere aleatoare. Dep: P5-05, P5-06.
- [x] **P5-10** Criptografie 1: SHA-256, SHA-384, HMAC, HKDF (crate `crypto/`), cu vectorii NIST/RFC. Dep: —.
- [x] **P5-11** Criptografie 2: X25519 (schimb de chei, RFC 7748). Dep: P5-10.
- [x] **P5-12** Criptografie 3: ChaCha20-Poly1305 (RFC 8439). Dep: P5-10.
- [x] **P5-13** Criptografie 4: numere mari și semnături RSA (PKCS#1 v1.5 și PSS). Dep: P5-10.
- [x] **P5-14** Criptografie 5: semnături ECDSA P-256. Dep: P5-10.
- [x] **P5-15** Criptografie 6: certificate X.509 (DER) și lanțul de încredere. Dep: P5-13, P5-14.
- [x] **P5-09** Închiderea Fazei 5 (fără P5-07/P5-08, mutate după suită — decizia lui Adrian, 21 oct 22:25). Dep: P5-01 … P5-06, P5-10 … P5-16.

## Faza 6 — Loc de lucru cu Claude înăuntru

Generate la închiderea Fazei 5 (P5-09, 2026-10-21), după comanda C-3 din hub. Ordinea e cea cerută de Adrian.
Fiecare are plan în `docs/plans/`.

Reparații după testul 2 pe Lenovo (comanda C-4, înaintea restului fazei):
- [x] **P6-01b** Recepția Realtek: PHY pornit, legătura așteptată, ISR șters, registrele citite înapoi în jurnal, `net` cu legătura și contoarele RX. Dep: P6-01.
  Test: `cd rtl8168 && cargo test --release` (15 teste, modelul imită PHY oprit, fără cablu, FIFO plin). Pe laptop: testul 3.
- [x] **P6-01c** Montarea stick-urilor formatate în Windows (MBR cu orice tip, fără tabelă) + comenzile SCSI de „trezire”. Dep: P3-07.
  Test: scenariul `usb-floppy` (stick fără tabelă → `/usb`), `usb-disk` (GPT) neschimbat.
- [x] **P6-01d** Comanda `log [cuvânt] [n]` (jurnalul nucleului ținut în memorie) și `net` „fără adresă” când DHCP nu răspunde. Dep: P2-09.
  Test: scenariul `shell` (`log E1000E 2`, `log nimicnimic`).

- [x] **P6-01** Driver Realtek RTL8168/8111 (`10ec:8168`): crate `rtl8168/` + model software al cipului (teste pe gazdă), placa în nucleu. Dep: P3-08.
  Test: `cd rtl8168 && cargo test --release` (11 teste față de model) + toate scenariile de rețea QEMU (e1000 prin interfața nouă `nic::Card`). Pe laptop: de verificat la testul 2 (cablu, `net`, `ping`).
- [x] **P6-02** Rădăcinile reale de încredere (Mozilla) în imagine + `fetch https://` cu un site real. Dep: P5-16.
  Test: `cd crypto && cargo test --release` (117 rădăcini citite, P-384), `cd tls && CYBER_NET=1 cargo test` (10 site-uri reale, în CI), scenariul `https-real` (example.com, api.github.com; doar cu internet).
- [x] **P6-03** Parser și generator JSON propriu (crate `json/`). Dep: —.
  Test: `cd json && cargo test --release` (RFC 8259, 25 de cazuri greșite, 200 de documente față de Python).
- [x] **P6-04** Clientul API Claude (`/bin/claude` în linia de comandă; cheia din `/usb/claude.key`). Dep: P6-02, P6-03.
  Test: scenariul `claude-cli` (imitație a API-ului: cerere verificată, răspuns chunked; cheie greșită → 401; fără cheie).
- [x] **P6-05** Aplicația „Claude”: chat în fereastră. Dep: P6-04.
  Test: scenariul `claude-app` (două mesaje, conversația întreagă trimisă, captura `tests/screens/claude-app.png`).
- [x] **P6-06** Uneltele lui Claude în OS (citește/scrie fișier, listează dosar, rulează comandă) cu confirmare pe ecran. Dep: P6-05.
  Test: scenariul `claude-tools` (Nu respectat, Da executat, scriere în /etc refuzată; discul verificat pe gazdă).
- [x] **P6-07** CI publică un GitHub Release cu fișierele stick-ului și sumele SHA-256. Dep: —.
  Test: jobul `release` din CI pe `main`; `GH_TOKEN=… tools/check-release.sh` (depozitul e privat → descărcarea cere token).
- [x] **P6-08** Comanda „Actualizează”: descarcă ultimul Release prin HTTPS, verifică sumele, rescrie stick-ul. Dep: P6-02, P6-07.
  Test: scenariul `update` (imitație GitHub: fără token, sumă greșită, actualizare întreruptă reparată, actualizare, „la zi”) + `tools/after-update.sh`.
- [x] **P6-09** Trimitere pe GitHub din OS (API `contents`, token din `/usb/github.token`). Dep: P6-02, P6-03.
  Test: scenariul `ghpush` (imitația API-ului contents) + unealta `github_push` în `claude-tools`; `cd json && cargo test --release` (base64).
Reparații după testul 3 pe Lenovo (comanda C-5, înaintea lui P6-10):
- [x] **P6-10a** Recepția Realtek după inel plin: inel RX de 256, re-pornirea recepției după RDU, re-sincronizare, `net` cu inelul și primele cadre. Dep: P6-01b.
  Test: `cd rtl8168 && cargo test --release` (20 de teste; modelul imită blocarea după RDU și indexul păstrat). Pe laptop: testul 4.
- [x] **P6-10b** Configurarea stick-ului pe xHCI real: `log usb` pas cu pas cu codurile TRB, verificările din plan. Dep: P6-01c.
  Test: scenariile `usb-disk` (stick USB 3, burst 15), `usb-disk-hs` nou (stick USB 2 pe port USB 2), `usb-enum`, `usb-floppy`. Pe laptop: testul 4.

Reparații după testul 4 pe Lenovo (comanda C-6, înaintea lui P6-10):
- [x] **P6-10c** Realtek: cadrele 2+ citite ca zero (DHCP 0): golirea cache-ului înainte de citire, No Snoop oprit, diagnostic `log rx` pe cadru. Dep: P6-10a.
  Test: `cd rtl8168 && cargo test --release` (23 de teste: model cu memorie ne-coerentă, 12 cadre consecutive verificate); `tools/test.sh` (48 de scenarii). Pe laptop: testul 5.

Reparații după testul 5 pe Lenovo (comanda C-8, înaintea lui P6-10):
- [~] **P6-10d** Realtek: revenire după regresia 0/0 (fără scrieri în Device Control, configurarea PCI doar bit cu bit, în jurnal), buffere RX într-un bloc contiguu sub 4 GB, adrese pe 64 de biți, diagnostic `log rx`/`log rtl` cu adrese complete, scenariul `net-high` (-m 6G). Dep: P6-10c.
  Test: `cd rtl8168 && cargo test --release`; `tools/test.sh` (cu `net-high`). Pe laptop: testul 6.

- [ ] **P6-10** Închiderea Fazei 6 (testul pe laptop cu Adrian). Dep: P6-01 … P6-09.

## Mai târziu (mutate din Faza 5 — după ce suita office își atinge ținta)

- [ ] BLOCAT **P5-07** Suita office ca modul (după ce citim proiectul din Cyber Hub). Dep: P4-11.
- [ ] **P5-08** Kronos ca modul. Dep: P5-07.

## Faza 7 și următoarele

Sarcinile se generează de Claude Code din `ROADMAP.md` la închiderea fazei anterioare (P6-10),
în același format, cu ID-uri `P7-xx` etc.
