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
- [x] **P6-10d** Realtek: revenire după regresia 0/0 (fără scrieri în Device Control, configurarea PCI doar bit cu bit, în jurnal), buffere RX într-un bloc contiguu sub 4 GB, adrese pe 64 de biți, diagnostic `log rx`/`log rtl` cu adrese complete, scenariul `net-high` (-m 6G). Dep: P6-10c.
  Test: `cd rtl8168 && cargo test --release` (27 de teste: poarta RXDV, bloc sub 4 GB, adrese pe 64 de biți); `tools/test.sh` (50 de scenarii, cu `net-high` la -m 6G). Pe laptop: testul 6 (variantele una câte una).

Reparații după testul 6 pe Lenovo (comanda C-9, înaintea lui P6-10):
- [x] **P6-10e** Realtek: inițializarea completă la pornirea „la rece” (variante `rtl porneste rece|mcu|phy|cplus|ordine`, fiecare pas în `log rtl`). Dep: P6-10d.
  Test: `cd rtl8168 && cargo test --release` (32 de teste: mod „rece” al modelului, pașii în ordine, așteptări cu limită); `tools/test.sh` (50 de scenarii; `shell` verifică `rtl porneste rece`). Pe laptop: testul 7 (R).
- [x] **P6-10f** Realtek: cadrele 2+ citite ca zero — golirea cache-ului devine implicită (RX și TX), confirmată pe laptop la testul 6 G–L; plus alias `fetch` și mesajul lui `host` (C-9, completările 18:20 și 18:30). Dep: P6-10d.
  Test: `cd rtl8168 && cargo test --release`; `tools/test.sh`. Pe laptop: testul 7 (rece, fără `rtl porneste cache`).

Reparații după testul 7 pe Lenovo (comanda C-10, înaintea lui P6-10):
- [x] **P6-10g** Realtek: prima inițializare citește zerouri la cald — paza paginilor DMA (nicio pagină dată de două ori, test în QEMU), jurnalul regiunii din harta memoriei, inelele și bufferele într-un bloc nou peste 16 MiB (ca la repornire). Dep: P6-10f.
  Test: `cd rtl8168 && cargo test --release` (36); `tools/test.sh` (49 trecute; `net` și `net-high` verifică paza DMA). Pe laptop: testul 8 (C) — `net` la prima inițializare → adresă.
- [x] **P6-10h** Realtek: accesul MAC OCP verificat (citire înainte de scriere, bitul 15 din E8DE „se șterge singur”), pașii reci reinterpretați în `log rtl`. Dep: P6-10e.
  Test: `cd rtl8168 && cargo test --release` (39: bitul 15 care se șterge singur, OCP care nu răspunde, C+ bit 3 fix); `tools/test.sh shell net boot`. Pe laptop: testul 8 (R).

Comanda C-12 (Cyber-GPU, fereastra de pe workstation), înaintea lui P6-10:
- [x] **P6-11** Comanda `gpu` (nivel 0, doar citire): plăcile video din PCI, BAR-uri, Resizable BAR, două registre publice NVIDIA (BOOT_0, BOOT_42) printr-o pagină fără drept de scriere; `gpu plan`. Dep: P6-01.
  Test: `tools/test.sh` (51 trecute + `https-real` sărit; `gpu`, `gpu-ro`, `tools/after-gpu.sh`). Pe hardware: fereastra de pe workstation.
- [ ] **P6-12** `map_mmio` garantează registre fără cache (observația Cyber-GPU, adevărată ca posibilitate): întâi diagnostic în jurnal, apoi fereastră MMIO proprie — după testul 8, o schimbare pe Release. Dep: P6-10g.
  Test: scenariile cu drivere + verificarea biților PCD/PWT.

Comanda C-13 (prima pornire pe workstation), înaintea lui P6-12 și P6-10 — o schimbare de comportament pe Release (D-013), în ordine:
- [x] **P6-13** PCI pe toate magistralele-rădăcină din MCFG (după parcurgerea punților, fără dubluri, ordine bus/dev/fn; paginile goale demapate). Dep: P6-11.
  Test: `tools/test.sh` → 52 trecute + `https-real` sărit; `pci-roots` (QEMU `pxb-pcie`: rădăcina 40 cu un dispozitiv la 41:00.0 apare; pe codul vechi testul pică). Release **os-166**. Pe hardware: fereastra a doua pe workstation (`lspci` → `10de:2482` la b3).
- [x] **P6-14** USB: dispozitive în spatele hub-urilor (clasa 09: descriptorul de hub, Get Port Status, Set Port Feature RESET, Route String / TT în contextul slotului xHCI). Dep: P6-13 (Release separat).
  Test: `tools/test.sh` → 53 trecute + `https-real` sărit; `usb-hub` (QEMU `usb-hub` full-speed: tastatura la 5.1, stick-ul la 5.2, `/usb` montat). Hub-urile de mare viteză (TT) și USB 3 (SET_HUB_DEPTH) nu există în QEMU → doar pe workstation (`0bda:0411` / `0bda:5411`).
- [x] **P6-15** `/usb` = stick-ul de pe care am pornit: toate dispozitivele Mass Storage încercate, „Medium Not Present” (2/3A) sărit, preferat discul cu `cyber.rw` și `VERSIUNE`. Dep: P6-14 (Release separat).
  Test: `tools/test.sh` → 54 trecute + `https-real` sărit; `usb-pick` (cititor CD fără mediu sărit, stick-ul cu `cyber.rw` + `VERSIUNE` ales înaintea discului GPT), toate. Pe hardware: workstation.
- [x] **P6-17** `reboot` care chiar repornește pe workstation (C-13 p.5b): pe rând ACPI RESET_REG (FADT), portul 0xCF9 (2, apoi 6), controlerul 8042 (0xFE), triple fault; câte un rând pe ecran și în jurnal; `reboot acpi|cf9|8042|triplu` pentru o singură metodă. Dep: P6-15 (Release separat).
  Test: `tools/test.sh` → 58 trecute + `https-real` sărit; noi: `reboot` (calea implicită → ACPI, FADT-ul OVMF: io 0xCF9 ← 0x0F), `reboot-cf9`, `reboot-8042`, `reboot-triplu` (QEMU `-no-reboot` iese la resetare, cod 0) + decodarea FADT pe octeți cunoscuți. Pe hardware: workstation (`reboot` → repornește; dacă nu, poza arată ultima metodă încercată).

Comanda C-14 (Adrian plecat câteva zile: doar QEMU și CI, câte o sarcină pe rând, Release la fiecare), după C-13; nu se pornesc C-7 și C-11:
- [x] **P6-16** Driver Intel I219-LM (`8086:15e3`, familia e1000e/PCH), din documentația publică Intel: logica în biblioteca `e1000e/` (model + teste pe gazdă), semafor PHY, LANPHYPC, bloc DMA păzit, `log e1000`. Dep: P6-17.
  Test: `cd e1000e && cargo test --release` → 12 (model: PCH cu semafor și PHY la adresa 2, PHY adormit trezit cu LANPHYPC, semafor ținut de firmware, reset blocat, fără MAC, inele lăsate pornite, trimitere/primire); `tools/test.sh` → 58 trecute + `https-real` sărit (`net` citește acum PHY-ul 82574 prin MDIC: `01410cb1`). Pe hardware: workstation (`net`, `log e1000 40`, `ping 1.1.1.1`).
- [x] **K6-01** SSE/AVX în nucleu și în programe: CR0/CR4, XCR0 prin XSETBV (doar ce raportează CPUID), XSAVE/XRSTOR la comutarea firelor (zonă per fir, aliniată la 64, mărimea din CPUID 0Dh). Dep: P6-16.
  Test: `tools/test.sh` → 60 trecute + `https-real` sărit (o dată `https` a picat la testul vechi al temporizatorului — 80 de întreruperi pe secundă sub încărcare —, trecut la reluare); noi: `fpu` (`-cpu max`: XSAVE x87+SSE+AVX, 832 octeți/fir, 3 fire × 3000 de runde, ~3500 de comutări, XMM și YMM păstrate; `/bin/vectori` cu AVX2 = 500500) și `fpu-fx` (fără XSAVE: FXSAVE, SSE2). Fără salvare la comutare, `fpu` pică (3257 de verificări greșite). Meniul desktop: până la 20 de programe (era 16).
- [x] **K6-02** SMP, partea 1: pornirea celorlalte nuclee (MADT, INIT-SIPI-SIPI, trambulină sub 1 MiB, GDT/TSS + stivă + APIC per nucleu, aceeași unitate vectorială), apoi parcate; la cerere (`cpu porneste`), pornirea implicită neschimbată. Dep: K6-01.
  Test: `tools/test.sh` → 62 trecute + `https-real` sărit (o dată `reboot-triplu` a picat la testul vechi al temporizatorului — 89/s, mașină încărcată —, trecut la reluare); noi: `smp-start` (QEMU `-smp 4`: trambulina la 0x3000, nucleele 1–3 pornite în ~10 ms fiecare, parcate, planificatorul merge mai departe), `cpu` (`cpu` / `cpu porneste` în shell, `-smp 2`). Cu `-smp 4 -cpu max` testul vechi al temporizatorului pică des pe mașina de 2 nuclee → scenariul rămâne fără `-cpu max`. Pe hardware: `cpu porneste` pe laptop și workstation.
- [x] **K6-02b** SMP, partea 2: încuietori reale (spinlock cu întreruperile oprite), date per nucleu (GS), planificator pe toate nucleele, temporizator per nucleu. Dep: K6-02.
  Test: QEMU `-smp 4`: fire repartizate pe nuclee, testul de stres trece; nicio regresie cu `-smp 1`.
- [x] **K6-03** Memorie mare pentru programe: apel de sistem de alocare/eliberare de regiuni anonime (MiB–GiB, pagini mari unde se poate). Dep: K6-02b.
  Test: un program alocă 1 GiB, îl scrie și îl verifică (QEMU cu memorie suficientă).
- [x] **K6-04** (opțional) Fire multiple într-un proces: `thread_spawn`/`thread_join`/`thread_exit` (37–39), oprirea tuturor firelor la ieșire, heap comun cu încuietoare, `/bin/fire`. Dep: K6-03.
  Test: un program cu 4 fire care lucrează pe aceeași memorie, rezultat exact.

Comanda C-15 (repo-ul devine public: GitHub Actions gratuit; B-004 = facturarea), după C-14:
- [x] **P6-18** CI mai econom + Release-ul cu munca din C-14: `paths-ignore` (`docs/**`, `**/*.md`, `tests/screens/**`), `concurrency` cu `cancel-in-progress`, testul de stres săptămânal, cache și pentru `target/` din bibliotecile pe gazdă; Release cu K6-02b..K6-04; B-004 închis; `actualizeaza` fără token notat în `docs/USB.md`. Dep: K6-04.
  Test: CI verde pe GitHub pe commit-ul cu cod → Release `os-<n>`; un commit doar cu documente nu mai pornește CI.
  Rezultat: run 186 (primul după 5 ore) a pornit, uneltele se instalează în 22 s (înainte ~20 min), dar `log-smp` și `fire` roșii: cursă în testul ordinii firelor pe 4 nuclee (litera și pasul scrise separat) → reparat în test (`972fa21`); run 187 verde → **Release `os-187`** (K6-02b, K6-03, K6-04). Local: 70 trecute (`cpu` picat o dată la testul vechi al temporizatorului, trecut la reluare).

Comanda C-16 (OS + Cyber-GPU integrate și testate automat, cu punct de revenire), după C-15:
- [x] **P6-19** Dependență pe Cyber-GPU fixată pe `41e44ba` (`gpu-core`, `gpu-pci`, `nvidia-ga104`, `gpu-compute`; `gpu-sim` doar la teste), adaptor PCI fără scrieri; `gpu identifica` prin `nvidia-ga104`, `gpu identifica --sim` (SIMULARE, workstation), `/bin/kronos-calcul` (matmul + operațiile Kronos pe toate nucleele, SSE/AVX, comparat cu referința). Lucru pe `wip/integrare-gpu`; punct de revenire `stabil-2026-10-23`. Dep: P6-18.
  Test: toate scenariile vechi + `gpu`, `gpu-sim`, `kronos-calcul`, `kronos-calcul-1`; CI verde pe ramură, apoi pe `main` → Release.
  Rezultat: local 74 de scenarii (70 + 3 picate la testul vechi al temporizatorului, trecute la reluare; `https-real` sărit); CI verde pe `wip/integrare-gpu` (run 188) → unit în `main`. Bloc Kronos pe 4 fire identic bit cu bit cu referința; matmul AVX2+FMA eroare maximă 3.8e-6 (toleranța 1e-5). Tag-ul `stabil-2026-10-23` nu se poate urca din cloud (403) → ramura `revenire/stabil-2026-10-23`; os-187 păstrat de CI.

Comanda C-17 (Release-urile testate pe hardware nu se mai șterg), după C-16:
- [x] **P6-20** Lista `docs/RELEASES_PASTRATE.md` (tag, commit, motiv); curățarea din CI sare peste tag-urile din listă; os-150 și os-161 republicate ca Release-uri păstrate din fișierele originale ale rulărilor CI 150 și 161 (nemarcate „latest”). Dep: P6-19.
  Test: CI verde pe `main` → Release nou, iar toate tag-urile din listă încă există; os-150 și os-161 apar din nou la Releases, cu aceleași sume SHA-256 ca rularea originală.
  Rezultat: `republica.yml` (manual) a republicat os-150 (din rularea 150, commit af7cd3d) și os-161 (rularea 161, d2da9e0) cu fișierele originale, sumele verificate, fără „latest”. CI run 190 verde → Release `os-190`; curățarea a șters doar os-172 (al 6-lea normal); os-150, os-161, os-187 au rămas. Codul OS-ului neschimbat (testele din CI: verzi).

Comanda C-19 (`gpu gfw` și `gpu vbios`, doar citire), din 26 oct:
- [x] **P6-21** Fixarea Cyber-GPU mutată de pe `41e44ba` pe `dac2e32` (ramura lor `g0-g2-fundatie`, „Stare: sesiunea 5”; doar adăugări); comenzile `gpu gfw` (I2: așteaptă GFW_BOOT, max 4 s) și `gpu vbios` (I3: imaginile din PROM, tabela BIT) prin `nvidia-ga104`, peste o fereastră BAR0 de 4 MiB mapată pagină cu pagină fără drept de scriere; scrierile cerute de bibliotecă refuzate și numărate; `--sim` în build-ul de test. Dep: P6-20.
  Test: scenariul `gpu-sim` extins: `gpu gfw --sim` („gata 0xff”, 0 scrieri), `gpu vbios --sim` (PCI-AT, EFI, FWSEC, BIT 0x70, 0 scrieri); `gpu` pe QEMU: „nicio placa NVIDIA”; CI verde → Release.
  Rezultat: în SIMULARE GFW gata după 9 runde (0xff), VBIOS cu 4 imagini (PCI-AT, EFI, 2×FWSEC), BIT cu 0x70, „complet: da”, 1066 citiri, 0 scrieri. Fereastra de 4 MiB verificată la pornire pe framebuffer-ul QEMU (aceleași valori ca o citire directă). `after-gpu.sh` verifică acum toate adaptoarele (testat și cu o scriere introdusă intenționat: pică). Pe placa reală: netestat (fereastra de pe workstation).

Comanda C-22 („cutia neagră” pe stick + permisiuni pe program), după C-19:
- [x] **P6-22** Comanda `raport`: scrie pe stick `/usb/raport/<data>-<nr>.txt` cu versiunea, `hw`, `lspci`, jurnalele `net`/`usb`/`pci`/`gpu`, registrele driverelor; automat la panică/erori grave (cât se poate); chei și token-uri filtrate. Dep: P6-21.
  Test: QEMU scrie raportul pe discul virtual; testul citește fișierul de pe imagine și caută secțiunile; un token pus intenționat în jurnal nu apare.
  Rezultat: scenariul `raport` + `after-raport.sh`: raportul automat (la eroarea de test de la pornire) și cel manual, ~13 KB fiecare, fsck curat, tokenul din fișier și cuvântul `ghp_…` apar ca `[ascuns]`. Automat = înainte de prompt (max 3/pornire); la panică nu se scrie (ar putea strica stick-ul).
- [x] **P6-23** Permisiuni pe program: apel nou „promisiuni” (grupuri de apeluri) + „căi vizibile”; după declarare nu se mai pot lărgi; `/usb/claude.key` și `/usb/github.token` vizibile doar programelor care le cer explicit. Dep: P6-22.
  Test: în QEMU un program fără permisiune primește eroare la citirea cheii și la un apel nepromis; unul cu permisiune o citește.
  Rezultat: apeluri noi 40 `promise`, 41 `unveil`, 42 `secret`; eroarea nouă -24. Scenariul `drepturi`: `/bin/cat` nu vede cheia, `hello drepturi` trece toate cele 15 verificări (secret cerut/renunțat, ocolire prin `..`, căi vizibile, promisiuni care nu se mai lărgesc). `claude`, `claude-app`, `gh-trimite`, `actualizeaza` cer secretul lor și renunță imediat după citire; `cat` din shell refuză fișierele secrete. Secretele = `claude.key`, `github.token` din `/usb`, `/disk`, `/etc`.

Comenzile C-23 (laptopul AMD: #GP la `kronos-calcul`; laptopul Intel: stick negăsit) și C-26 (adresa ecranului în `hw`), din 27 oct:
- [x] **P6-24** Căderea #GP (cod 0x18) pe AMD la `run kronos-calcul`: cauza = `sysret` pe AMD ia SS din STAR fără RPL 3; STAR are acum baza 0x13. Dep: P6-23.
  Test: rândul „test: sysret: SS=0x1b CS=0x23 pe Intel si pe AMD” în `log` (pică cu valoarea veche: „SS AMD 0x18”) + scenariul `kronos-calcul-amd` (`-cpu EPYC-Rome`).
  Rezultat: RIP-ul din os-191 = `iretq` de la sfârșitul întreruperii de ceas; programul rula cu SS=0x18 după orice apel de sistem, iar prima întoarcere în program după o întrerupere de ceas cădea. Pe Intel procesorul pune singur RPL 3, de aceea acolo mergea.
- [x] **P6-25** Toate controlerele USB (xHCI) pornite, stick-ul căutat pe fiecare (`00:14.0` pe Tiger Lake). Dep: P6-24.
  Test: scenariul `usb-2xhci` (două `qemu-xhci`, stick-ul pe al doilea) + scenariile USB vechi.
  Rezultat: „usb: 2 controlere XHCI pornite”, stick-ul găsit la „c2/1” și montat la `/usb`; un controler care nu pornește nu le mai oprește pe celelalte.
- [x] **P6-26** O linie în `hw` (și în `raport`) cu adresa fizică și mărimea framebuffer-ului de la Limine, doar citire. Dep: P6-25.
  Test: scenariile `shell` și `raport` (`after-raport.sh` caută rândul în fișier).
  Rezultat: în QEMU „ecran: framebuffer la adresa fizica 0x80000000, 4096000 octeti (4 MiB), 1280x800, pitch 5120 B, 32 bpp”; valorile sunt păstrate la pornire (răspunsul Limine e în memorie recuperată mai târziu).

Comenzile C-25 (Kronos adevărat în OS) și C-27 (căutarea automată a variantei de calcul), din 27 oct:
- [x] **P6-27** Motorul KGR1 în Rust + comanda `kronos` (C-25), partea 1: cititorul KGR1 (F32, Q8_0), pasul înainte pe toate nucleele (AVX2+FMA sau scalar), memoria cheilor/valorilor, SHA-256, `kronos --simboluri` / `--verifica`. Dep: P6-26.
  Test: scenariile `kronos-kgr` (`-smp 2 -cpu max`) și `kronos-kgr-1` (1 nucleu, fără AVX): modelul minuscul F32 și Q8_0 din `tools/kgr-mini.py` (referință float64 scrisă de noi din specificație) — logit-uri în `1e-4·(1+|ref|)`, continuarea greedy identică, suma greșită refuzată, nicio pagină pierdută; o greșeală introdusă intenționat (scara atenției ×1,01) pică testul.
  Rezultat: 0 din 1632 logit-uri în afara toleranței (abaterea maximă ~5e-6–1e-5), continuările identice pe F32 și Q8_0, scalar și AVX2 pe 2 și 4 fire. Specificația din hub a sosit (28 oct 02:15); vectorii oficiali Kronos și descrierea tokenizerului încă nu → P6-27b.
- [x] **P6-27b** Partea 2 din C-25: tokenizerul din fișier (`tokenizer.bpe`) + `kronos "<text>"`; testul pe vectorii oficiali din hub (`integrare/kronos/vectori/`). Dep: P6-27.
  Rezultat (2026-10-28): tokenizerul nostru (`user/src/bpe.rs`) = referința pe 23.008 texte (verificare locală) și pe toate exemplele din specificație; vectorii oficiali F32 și Q8_0 trec în QEMU (abaterea maximă 5,2e-6, toleranța 1e-4 / 1e-3; argmax, greedy cu și fără cache, textul generat identice); `kronos Ana are mere.` scrie continuarea. Memoria cheilor/valorilor mutată în afara heap-ului (modelul #459 cere 36 MiB > heap 32 MiB).
- [x] **P6-28** `kronos-calcul --cauta`: căutare automată verificată a celei mai bune variante de calcul, reglaj pe stick (C-27). Dep: P6-27.
  Test: scenariul `kronos-reglaj` (`-smp 2 -cpu max`, disc de test cu scriere): 12 variante (scalar/AVX2+FMA × 1/2 fire × ordinea t-j/j-t/stricată), fiecare verificată față de controlul float64; cele 4 stricate intenționat respinse; fișierul `/disk/kronos/reglaj-<procesor>.txt` scris (fsck curat, citit de `after-kronos-reglaj.sh`) și folosit de `kronos`, care rămâne egal cu referința.
  Rezultat: în QEMU câștigă „AVX2+FMA, 1 fir, j-t” (22× față de scalar pe 1 fir; 2 fire pierd la matricea mică din cauza pornirii firelor la fiecare produs — de urmărit pe workstation). Ceasul fin: TSC calibrat pe ceasul de 10 ms. AVX-512 încă nu e variantă (QEMU nu-l are, deci n-ar avea test).

Comanda C-29 (Kronos în OS corect pe workstation, dar lent), din 28 oct:
- [x] **P6-29** Kronos în OS mai rapid (C-29): calcul cu virgulă mobilă în procesor pentru programe (nu emulat), `kronos --profil` (TSC pe operații), bazin de fire pornit o dată cu barieră fără somn și atenția împărțită pe capete, `kronos-calcul --cauta` pe dimensiunile modelului de pe stick, citire USB cu mai multe sectoare pe cerere, `--repetitie <p>` (opțional, implicit oprit). Dep: P6-27b, P6-28.
- [x] **P6-30** Licență proprietară pentru codul nou (C-32): `LICENSE` „toate drepturile rezervate” cu nota că versiunile până la `0bf1303` au fost MIT, D-015 înlocuiește D-005, `CLAUDE.md` §6, README-uri, `CLEAN_ROOM.md`, `license-file` în `Cargo.toml`; firmware-ul NVIDIA nu se distribuie. Dep: —
- [x] **P6-31** `gpu vbios --detalii` (C-31): Cyber-GPU fixat pe `8313973` (cu `vbios_diag`, doar adăugări față de `dac2e32`), comanda doar citire cu „0 scrieri”, test `gpu-sim` cu lanțul de tip ASUS (imaginile „NV” la `0x24600`) și `gpu`; `cyber-gpu` din copia din hub (`tools/cyber-gpu-local.sh`, B-005). Release: nu se poate din sesiune (CI fără minute). Dep: —
- [x] **P6-32** Stick nou construit în sesiune, livrat prin hub (C-35): `tools/stick-hub.sh` face `stick-cyber-universe-os-<nr>.zip` (fișierele Release-ului, fără model Kronos și fără firmware NVIDIA) + `VERSIUNE.txt` + `README.md` în `integrare/cyber-universe/stick/` din hub, doar ultima arhivă; înainte, suita QEMU. Se repetă la fiecare schimbare care ajunge pe stick, până revine CI-ul. Dep: —
- [x] **P6-33** CI din nou cu `cyber-gpu` privat (B-005, după decizia hub 23:35 „Actions repornite”): pasul `tools/ci-cyber-gpu-acces.sh` în `CI` și `Stress` folosește secretul `CYBER_GPU_TOKEN` (doar citire pe `cyber-gpu`); fără secret, CI se oprește cu un singur mesaj clar. Secretul îl pune Adrian. Dep: —
- [x] **P6-34** CI: `CYBER_GPU_TOKEN` ascuns de credențialele lăsate de `actions/checkout` (C-37): `persist-credentials: false` în `CI` și `Stress`, verificarea accesului fără antetul `extraheader`; plus `phase4` (aplicația Module sortează pachetele, ordinea de pe disc depindea de mașina CI). CI verde (run 205), Release `os-205`. Dep: P6-33
- [x] **P6-35** `gpu fwsec` + `gpu firmware` (C-38), doar citire, pentru fereastra C-36: Cyber-GPU fixat pe `2c9c90c`, comenzile cu `--sim`, 2 rânduri în `gpu plan`, teste QEMU; firmware-ul nu intră în repo/stick. Dep: P6-31
  Rezultat (2026-10-28): `os-1001` (commit `02f2043`, 6,9 MB, SHA-256 `ca0bbc5a…1650`) în hub, `integrare/cyber-universe/stick/`. Suita QEMU 78/80 + 1 sărit la prima trecere; `fpu-smp` și `kronos-calcul` = temporizatorul la `-smp 4` pe gazda cu 2 nuclee, trec la reluare. Arhiva dezarhivată de probă (sume OK) și imaginea pornită în QEMU de pe USB până la desktop.
  Test: vectorii oficiali F32/Q8_0 și modelul minuscul rămân identici (scenariile `kronos-kgr*`), `--profil` scrie toate rândurile, bazinul dă aceleași logit-uri pe 1/2/4 fire, `--cauta` pe un model de pe disc; viteza adevărată = workstation (Adrian).
  Rezultat (2026-10-28): cauzele găsite în cod — virgula mobilă a programelor era emulată (soft-float), fire noi la fiecare produs (și stivele lor nu se eliberau: `mem_unmap` e refuzat cât mai rulează alte fire), FAT32 parcurgea lanțul de la început la fiecare 64 KiB, derularea ecranului citea memoria plăcii video. Toate reparate (ținta `x86_64-cyber-user.json`, D-014; bazin de fire; FAT cu poziție ținută minte; copie a ecranului în RAM). `kronos --profil`, `--repetitie`, `--cauta` pe model, `usb citire mare` (variantă, implicit oprită, D-013). Suita locală: 77/80 la prima trecere (3 = testul vechi al temporizatorului la `-smp 4` pe gazda cu 2 nuclee), toate 3 trec la reluare. În QEMU un model de 2,4 MiB se citește în 80 ms (era 360 ms). Viteza pe workstation = testul lui Adrian.

- [x] **P6-36** Kronos în OS: frânele de pe workstation (C-40): procesoarele libere trezite imediat (IPI) când un fir devine gata, qkv + atenția într-o singură împărțire la generare, GELU pe fire, textul pe ecran pe rânduri (`--fara-ecran`), `--cauta` cu costul sincronizării. Simbolurile identice. Dep: P6-35
  Rezultat (2026-10-29): suita locală 77/80 + 1 sărit (3 = server de test / temporizatorul la `-smp 4`, trec la reluare); testul SMP nou: fir nou pornit pe un nucleu liber în ~40 µs (era până la 10 ms). Viteza reală = workstation (Adrian).

- [x] **P6-37** Fixarea Cyber-GPU după C-39 (C-42): `2c9c90c` → `aee06c4` (kernel + user, doar adăugări), textul fixării în scenarii, teste QEMU (`gpu firmware --sim` → antete citite 4/4), CI verde → Release nou. Dep: P6-35
  Rezultat (2026-10-29): `gpu firmware --sim` → „HS v2: 2 semnaturi de 384 octeti” la ambele Booter, mărimile reale (61.304 / 41.080 / 24.684 octeți), „criteriul I4: TRECE (fisiere oficiale 4/4, antete citite 4/4)”; suita locală 79/80 + 1 sărit (`keyboard-ro-usb` trece la reluare); CI verde (run 40949687259) → Release `os-209`.

- [x] **P6-38** Modelul Kronos #477 pe stick (C-46): tabelul modelelor cunoscute (#477, #459 rezervă) cu SHA-256 și vectorii de control, alegerea implicită 477 > 459 > primul `*.kgr`, `kronos --control`, teste QEMU, Release nou. Dep: P6-37
  Rezultat (2026-10-31): `kronos --control` pe modelul mic al vectorilor oficiali trece în QEMU; alegerea 477 > 459 > primul `*.kgr` și suma din tabel (refuz cod 3) verificate; suita locală 80/80 + 1 sărit. Pe hardware: `run kronos --control` (Adrian).

- [x] **P6-39** Kronos în OS: produsul „proiecție” și afișarea pe ecran (C-48): proiecția la generare pe firele împărțirii pe capete (`fire_proiectie`, căutată de `--cauta`, `--fire-proiectie`), textul pe ecran în bloc cel mult o dată pe secundă, consola mută ecranul o dată pe bloc. Simbolurile identice. Dep: P6-38
  Rezultat (2026-11-01): în QEMU simbolurile identice cu proiecția pe 1, 2, 3 și 4 fire; consola: blocul = aceeași imagine ca rând cu rând; suita locală 80/80 + 1 sărit (`kronos-calcul` = temporizatorul cunoscut la `-smp 4`, trece la reluare). Viteza reală = workstation (Adrian).

- [x] **P6-40** Primele scrieri în placă din OS (C-52, I6): fixarea cyber-gpu `9597863`, `gpu pregatire-i6` (doar citire, fereastra de 16 MiB), jurnalul fiecărui acces pe stick pe loc, `gpu i6 --scrie` doar în stick-ul de test (`gpu-scrie`, „DA” tastat, Bus Master pus la loc și la eroare). Teste doar în QEMU/simulare; placa reală doar cu Adrian. Dep: P6-35
  Rezultat (2026-11-01): în QEMU, pe modelele gpu-sim, `gpu i6 --scrie --sim` TRECE: zona DMA reală a OS-ului (sub 4 GiB) citită de modelul falcon-ului, 84 de scrieri în cele 18 registre din plan, WPR2 = regiunea FRTS, Bus Master pus la loc (și la eroarea FWSEC), jurnalul de pe disc TRECE la `i6_log`. Suita locală 81/83 + 1 sărit (`kronos-calcul` = temporizatorul cunoscut la `-smp 4`, pică și pe `main` în acest mediu cu 2 procesoare). Placa reală: doar cu Adrian.

- [x] **P6-41** Bucla automată, partea OS (C-53): la pornire, dacă există `/usb/auto/teste.txt`, rulează comenzile din lista albă (fără nicio scriere în placa video), scrie `auto/raport-<data>.txt` pe loc, redenumește lista în `teste-facut-<data>.txt` și repornește; ceasul de siguranță (implicit 10 min); scriptul pentru Windows (`tools/windows/`); documentația pregătirii cu Adrian. Dep: P6-40
  Rezultat (2026-11-01): în QEMU, scenariile `auto` (7 comenzi: 5 rulate, `gpu i6 --scrie` și `reboot` refuzate, raport cu ieșirea programelor, lista redenumită, repornire), `auto-ceas` (comanda blocată intenționat → ceasul de siguranță repornește după 4 s, raportul se oprește la „in lucru: test-blocheaza”) și `auto-intrerupt` (lista întreruptă nu se reia, „INTRERUPT” în raport) trec; scriptul `tools/windows/bucla-automata.ps1` (în Release) + `docs/BUCLA_AUTOMATA.md`. Suita locală 84 trecute + 1 sărit; `kronos-calcul` pică doar aici (temporizatorul cu `-smp 4`, cunoscut). Watchdog-ul chipsetului: pas următor.

- [x] **P6-42** `gpu tur` + `gpu lant` (C-55): fixarea cyber-gpu `4e6f747` (≥ `911e6d0`); `gpu tur` doar citire, un rând `fapt …` pe fapt, pe lista albă a modului automat; `gpu lant` = pașii ferestrei în lanț, oprire la primul eșec, Bus Master pus la loc, doar cu „DA”, interzis în modul automat. Teste QEMU/simulare. Dep: P6-41
  Rezultat (2026-11-01): în QEMU, scenariul `gpu-tur`: `gpu tur --sim` dă 41 de fapte ale plăcii observate (`fapt pci.id = 10de:2482`, `fapt chip.nume = GA104`, …) cu 0 scrieri; `gpu lant --sim` cu `DA` trece pregătirea, Bus Master, I6, se oprește singur la I7, pune Bus Master la loc (COMMAND 0x0003) și face turul după (86 de scrieri, toate în fereastra GSP + 2 Bus Master); `--eroare` se oprește la I6 cu Bus Master tot la loc; `--doar-citire` 0 scrieri; `gpu lant` fără `DA` = nimic; jurnalele de pe disc verificate de `tools/after-gpu-tur.sh`; modul automat: `gpu tur` permis, `gpu lant` refuzat. Suita locală 86 trecute + 1 sărit (`fpu-smp` și `kronos-calcul` = temporizatorul cunoscut, trec la reluare).

- [x] **P6-43** Kronos în OS: mai puțină așteptare între fire (C-59): se așteaptă doar firele care au lucru, semnal „gata” pe fir (fără contor comun), firele nu mai cedează procesorul la fiecare ~4000 de pauze când sunt destule procesoare; `--profil` arată barierele (număr, µs) și `--cauta` costul unei bariere. Simbolurile identice. Dep: P6-39
  Rezultat (2026-11-03): în QEMU simbolurile identice (vectorii oficiali F32/Q8_0, `--control`, proiecția pe 1 și 3 fire, 4 fire pe 2 nuclee); testul nou `kronos --test-fire 6` (200 de lucrări cu 1..6 părți, fiecare element scris o dată) trece; `--profil` arată barierele, `--cauta` „bariera goala pe 2 fire: ~1,4 us”. Suita locală 86 trecute + 1 sărit (`kronos-calcul` = temporizatorul cunoscut la `-smp 4`, pică la pornire, înainte de program). Viteza reală = workstation (Adrian).

- [x] **P6-44** Fixarea cyber-gpu ≥ `af02bea` (C-61, capătul WPR2): fixare pe `3286a3b`; după `gpu i6 --scrie` / `gpu lant` se citesc LO/HI brute (`falcon::check_frts_detail`, 3 citiri) și se afișează forma capătului; test în QEMU/simulare; Release nou cu stick-ul de test. Dep: P6-42
  Rezultat (2026-11-03): fixat pe `3286a3b`; în simulare `gpu i6 --scrie` / `gpu lant` scriu „WPR2 citit dupa I6: LO 0x01ffe000 HI 0x01ffeff0 (capat: ultima pagina)”; `gpu-i6`, `gpu-tur`, `gpu-scrie`, `gpu-sim` trec; `after-gpu.sh` ok. CI run 41645206747 verde → **Release `os-219`** (cu `cyber-universe-gpu-scrie-usb.img`). În același Release: reparația lacătului nucleului (`try_lock`, CI roșu la `kronos-calcul`).

- [x] **P6-45** Kronos: ideile lui Adrian din C-59 — profilul despărțit (adunare/trezire vs. cel mai lent), bucățele, echipe 9×4/6×6, un singur bloc pe simbol, un fir liber; toate măsurate în `--cauta`, câștigă cea mai rapidă; simbolurile identice. Dep: P6-43 — gata 2026-11-03: `--profil` despărțit, `--bucati`, `--echipe`, `--bloc` (+ un procesor liber în `--cauta`), Release `os-222`.
  Pas 1 gata (2026-11-03): `kronos --profil` arată „bariera in medie: trezire … us, asteptat cel mai lent … us, adunare … us” (din ora TSC a fiecărui fir).
  Pas 2 gata (2026-11-03): „bucățele” — `kronos --bucati N` / `bucati = N` în reglaj: N bucăți pe fir, contor comun, cine termină ia următoarea; QEMU: `--test-fire 6 --bucati 8`, `--verifica` Q8_0 pe 4 fire cu 8 bucăți = referința, `--control --fire 3 --bucati 4` trece. `kronos-calcul --cauta` măsoară 1/2/4/8 bucăți pe fir pe trecerea prin model (fiecare verificată cu controlul) și scrie `bucati = N` în reglaj dacă bate împărțirea fixă cu peste 1 % (QEMU: scris și citit de `kronos`). Rămân: echipe (3), un bloc pe simbol (4), un fir liber (5).

- [ ] **P6-10** Închiderea Fazei 6 (testul pe laptop cu Adrian). Dep: P6-01 … P6-09.

## Mai târziu (mutate din Faza 5 — după ce suita office își atinge ținta)

- [ ] BLOCAT **P5-07** Suita office ca modul (după ce citim proiectul din Cyber Hub). Dep: P4-11.
- [ ] **P5-08** Kronos ca modul. Dep: P5-07.

## Faza 7 și următoarele

Sarcinile se generează de Claude Code din `ROADMAP.md` la închiderea fazei anterioare (P6-10),
în același format, cu ID-uri `P7-xx` etc.
