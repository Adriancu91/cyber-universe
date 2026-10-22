# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
### Sesiunea programată 2026-10-22 14:40–15:30 UTC (17:40–18:30 ora României), în cloud, fără Adrian — reparat CI roșu
- **Pornire:** clonele locale aveau un `main` vechi, cu alt istoric → păstrat ca ramura locală `local-vechi-main` (nepublicată), `main` nou din GitHub.
- Hub: la pornire (14:44 UTC) nicio comandă nouă. Între timp a sosit **C-9** (testul 6), preluată de **altă sesiune la 15:05 UTC** (P6-10e/P6-10f, `[~]`)
  → această sesiune NU a lucrat la C-9, a reparat doar CI-ul.
- **CI roșu** pe `4fe5e76` (run 140, doar STATE.md schimbat): scenariul `net-high` — `PANIC: scurgere in heap: 1346688 inainte, 1346624 dupa`.
  Reprodus local: 2 eșecuri din 3. Cauza: testul de heap compară memoria folosită înainte/după, dar firul de rețea (DHCP terminat chiar atunci)
  eliberează 64 de octeți în același timp. **Nu e o problemă a driverului.** Reparat doar în testul nucleului (`test_heap`): întreruperile sunt
  oprite cât se măsoară (`sched::irq_save/irq_restore`), deci nimic altceva nu rulează. După reparație: `net-high` 6 din 6 trecute.
  Comportamentul driverelor nu s-a schimbat (regula D-013 — o schimbare pe Release — nu e atinsă: e doar cod de test).
- **Următorul pas:** neschimbat — testul 6 pe laptop (lista în hub, `stare/cyber-universe.md`).

### Pornire manuală 2026-10-22 (cerută de centrul de comandă la 16:40 pentru C-8) — fără lucru nou
- C-8 era deja făcută în această sesiune (P6-10d, Release os-138, CI verde). Nicio comandă nouă, niciun rezultat de la testul 6,
  nicio sarcină liberă (P6-10 așteaptă testul 6; C-7 rămâne `nou` până merge rețeaua) → oprire (CLAUDE.md §3).

### Sesiunea programată 2026-10-22 13:28–14:30 UTC (16:28–17:30 ora României), în cloud, fără Adrian — P6-10d (C-8)
- **Pornire:** clonele vechi din `/home/claude` aveau alt istoric (fără bază comună cu GitHub) → mutate deoparte (`*.vechi`), clonat curat.
- **Hub:** comanda nouă **C-8** (testul 5: regresie 0 trimise / 0 primite; No Snoop era deja oprit; completări 16:45 studiu, 16:50 RCR,
  16:55 regula „o schimbare pe Release”) → preluată; **P6-10d** în TASKS cu plan. Regula scrisă ca **D-013** în DECISIONS.md.
- **Terminată: P6-10d.**
  - **Revenire:** pornirea implicită e iar exact cea din os-125: nu se mai scrie Device Control (P6-10c ștersese Relaxed Ordering:
    2010 → 2000), nicio golire de cache. Configurarea PCI: Command doar bit cu bit (Memory, Bus Master), PMCSR doar câmpul de stare,
    fiecare scriere în jurnal; `log rtl` arată Command, Bus Master, Device Control (doar citit).
  - **RCR 0002c70a (os-125) vs 0000c70a (os-136):** driverul scrie aceeași valoare (`0000e70a`) în ambele versiuni și nu atinge bitul 17
    → diferența vine din starea plăcii la pornire, nu din cod. Bănuială: testul 5 a pornit „la rece” direct de pe stick (BIOS setat),
    testele anterioare poate după Windows → poarta RXDV (MISC bit 19, 8168G+) lăsată închisă de firmware oprește orice transfer.
  - **Variante, alese din shell, câte una:** `rtl porneste [bloc] [rxdv] [cache]` repornește placa: `bloc` = buffere RX într-un bloc
    contiguu sub 4 GB; `rxdv` = deschide poarta RXDV dacă e închisă (citire → bit 19 → scriere, în jurnal); `cache` = golirea de la P6-10c.
  - **Diagnostic:** `log rtl` (registre + MISC/MCU, RDSAR/TNPDS pe 64 de biți, primele 3 buffere, RAM, PCI); `log rx` (adresa din descriptor
    pe 64 de biți, adresa virtuală citită și fizica ei din tabelele de pagini, primii 16 octeți); `net` (Bus Master, MISC, varianta).
  - **Studiu clean-room:** FreeBSD `re(4)` (r261531, BSD): la 8168G șterge bitul 19 din MISC (0xF0) la pornire și folosește EarlyOff v2
    (RCR bit 11). Bitul 11 = candidat pentru o versiune următoare. SerenityOS/Haiku nu au fost citite în această sesiune.
  - **QEMU:** scenariul nou `net-high` (`-m 6G`, alocările forțate peste 4 GB): e1000e cu inele la 0x1_0010_9000 face DHCP + ping;
    blocul „sub 4 GB” chiar e sub 4 GB. `shell` verifică `rtl porneste`. `alloc_contiguous_below` nou în alocatorul de cadre.
- **Teste:** `cd rtl8168 && cargo test --release` → 27 (poarta RXDV reproduce exact 0/0 de la testul 5); `tools/test.sh` → 49 trecute +
  `https-real` sărit local; `tcp`, `mouse`, `appwindow` au picat o dată în rularea completă (calculator încărcat) și trec rulate singure.
- **Următorul pas:** testul 6 pe laptop (lista în hub, `stare/cyber-universe.md`). Apoi varianta confirmată devine implicită (o schimbare).
- **Avertisment:** dacă testul 6 arată „Bus Master NU” sau MISC fără bitul 19 și tot 0/0, cauza e alta (inițializarea 8168G/H: OOB/MCU) → sarcină nouă.

### Sesiunea programată 2026-10-22 11:40–13:00 UTC (14:40–16:00 ora României), în cloud, fără Adrian — paralelă, fără lucru nou pe `main`
- A pornit în timp ce sesiunea de la 11:25 lucra deja la P6-10c (marcat `[~]` la 11:30) și a făcut aceeași sarcină în paralel
  (ramura `wip/P6-10c-b`). Sesiunea de la 11:25 a pus-o pe `main` prima; din varianta de aici a preluat rândul `log arp` și
  golirea cache-ului pentru cadrele trimise (`c1a03d5`). Testele locale pe varianta de aici: 48 trecute + 1 sărit, `rtl8168` 23.
- Greșeală de proces: un `push -f` pe propria ramură `wip/P6-10c-b` (la 2 minute după creare); commit-ul inițial `629019f` a fost
  pus înapoi în istoric printr-un merge (`d0e648b`), nimic pierdut. Pe `wip/P6-10c-b`, `0aa9c75` are autorul „Claude” în loc de Adriancu91.
- Coada: nicio sarcină liberă (P6-10 așteaptă testul 5 pe laptop; C-7 rămâne `nou` până merge rețeaua pe laptop). `wip/P6-10c-b` poate fi ștearsă.
- **Avertisment:** două sesiuni pornesc uneori la câteva minute distanță. La pornire, dacă ultimul commit e un „în lucru” de mai puțin
  de ~2 ore și nu există STATE pentru el, sesiunea nouă nu reia aceeași sarcină: verifică hub-ul/CI și se oprește sau ia altă sarcină independentă.

### Sesiunea programată 2026-10-22 11:25–12:30 UTC (14:25–15:30 ora României), în cloud, fără Adrian
- **Hub:** comanda nouă **C-6** (testul 4: stick-ul merge, primirea nu se mai blochează, dar cadrele 2+ au tipul 0000 → DHCP 0)
  → preluată; **P6-10c** adăugată în TASKS cu plan.
- **Terminată (1 sarcină): P6-10c** — verificate în cod ipotezele (a)–(c) din C-6: adresa din descriptor = adresa citită
  (aceeași listă, pas 2048 = RMS, HHDM), copierea e înainte de re-armare, indexul e corect. Cauza cea mai probabilă: **cache-ul**
  (PCIe „Enable No Snoop” e 1 implicit → placa scrie în memorie fără să anunțe cache-ul; procesorul vede zerourile din pornire).
  Reparații: `Device::capability` + `pcie_strict_dma` (No Snoop și Relaxed Ordering oprite, în `log rtl`); `Hw::dma_sync`
  (`CLFLUSH` + `MFENCE`) după zeroizarea fiecărei pagini DMA și înainte de citirea fiecărui cadru; barieră Acquire după OWN.
  Diagnostic: `log rx` — primele 8 cadre (index, lungime, adresa din descriptor / adresa citită, 16 octeți înainte și după
  golire); `net` — „cadre cu tip 0000”, „reparate de golirea cache-ului”, „PCIe No Snoop: oprit (era pornit)”.
  Model: mod `no_snoop` (datele devin vizibile doar după `dma_sync`), test cu 12 cadre consecutive (unul câte unul și în rafală).
  Plus, la cererea lui Adrian (completarea C-6 de la 14:45, „log arp nu are niciun rând”): primele 16 pachete ARP primite apar în
  jurnal (`log arp`: cerere/răspuns, de la cine, pentru cine), înainte de verificarea adresei; scenariul `net` verifică rândul.
  Completarea 14:45 bănuia buffere alocate separat cu adresa `baza + i × 2048`; în codul nostru fiecare adresă vine din pagina ei
  (`p.phys`, `p.phys + 2048`), deci nu e cazul — `log rx` o arată oricum la testul 5 („(DIFERITE)” dacă n-ar fi așa).
- **Sesiune paralelă:** o altă sesiune a făcut tot P6-10c pornind de la commit-ul meu „în lucru” și a pus varianta ei pe
  **`wip/P6-10c-b`** (nefolosită; din ea s-au luat ideile: rândul ARP și golirea cache-ului și pentru cadrele TRIMISE — dacă placa
  citea cu No Snoop, cererile DHCP puteau pleca cu octeți vechi). Ramura poate fi ștearsă de Adrian oricând.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 48 trecute + `https-real` sărit local (`https` a picat o dată la
  verificarea temporizatorului — „79 intreruperi”, calculator încărcat, problemă cunoscută; rulat singur trece);
  `cd rtl8168 && cargo test --release` → 23. Verificat că testul nou pică dacă se scoate golirea cache-ului.
- **CI verde** pe `3a64f7d` (os-128), `bb383c5` (os-131), `c1a03d5` (**Release `os-134`** = versiunea pentru testul 5).
- **C-7** (bucla de test automată) rămâne `nou` în hub: comanda spune explicit că se preia abia când rețeaua merge pe laptop.
- **Pe laptop:** urmează **testul 5** (lista în hub, `stare/cyber-universe.md`). Cum se citește: „reparate de golirea cache-ului”
  > 0 → cauza era cache-ul; `(DIFERITE)` în `log rx` → adrese; zerouri și după golire → placa scrie altundeva (revizia H).

### (anterioară)
### Sesiunea programată 2026-10-22 08:40 UTC (11:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe` (C-5 e `gata`); rezultatele testului 4 nu există încă în hub (`rezultate/`).
- `docs/TASKS.md`: nicio sarcină liberă (P6-10 și P3-10 așteaptă testul 4 pe laptop; P5-07/P5-08 după suită) → oprire (CLAUDE.md §3).
- CI verde pe `ffc342b` (Release `os-125`). Nu s-a schimbat cod. Clona locală veche a hub-ului avea alt istoric → re-clonat curat ca `Cyber-hub`.
- Regula de oprire: e prima sesiune goală după una cu lucru (06:25 a terminat P6-10a/b) → nu se aplică.

### (anterioară)
### Sesiunea programată 2026-10-22 06:25–07:30 UTC (09:25–10:30 ora României), în cloud, fără Adrian
- **Hub:** comanda nouă **C-5** (testul 3: Realtek primește 16 cadre apoi nimic; stick-ul nu trece de configurarea USB) → preluată;
  P6-10a, P6-10b adăugate în TASKS cu planuri. Regula de oprire nu s-a aplicat (era comandă nouă; C-5 cere explicit să nu ne oprim).
- **Terminate (2 sarcini):**
  - **P6-10a** — Realtek: inelul de primire are acum 256 de descriptori (era 16 = exact cât a primit laptopul); lista bufferelor
    într-o pagină separată (driverul mic pe stivă); după un RDU urmat de `STALL_POLLS` (50) citiri fără cadre → primirea e repornită
    (CR fără RE, descriptori redați, RDSAR rescris, RE din nou); re-sincronizare dacă placa scrie în alt descriptor. `net` are două rânduri
    noi: `inel RX 256: index, ai placii, RDU vazut, reporniri, resincronizari, raspunsuri DHCP (udp 68)` și `primele cadre: 0806 0800/udp68 …`.
    Modelul cipului imită blocarea după RDU (`stall_after_rdu`) și indexul păstrat (`keep_index_on_restart`): 20 de teste.
  - **P6-10b** — xHCI/stick: așteptările se măsoară cu ceasul (comenzi 2 s, transferuri 5 s; înainte erau bucle de ~1–40 ms pe un
    procesor rapid — cel mai probabil motivul eșecului pe laptop: QEMU răspunde instant). Fiecare pas al enumerării și al pornirii
    stick-ului apare în `log usb` cu codul TRB în cuvinte (Enable Slot, Address Device, descriptori, SET_CONFIGURATION, Configure Endpoint,
    INQUIRY pe etape CBW/date/CSW). STALL: Reset Endpoint + Set TR Dequeue + CLEAR_FEATURE(HALT). Transferuri întârziate ignorate.
    Burst SuperSpeed din descriptorul companion; Average TRB Length 3072 la bulk. INQUIRY reîncercat de 3 ori.
- **Completare (a doua pornire a sesiunii, după C-5 completat la 09:40 cu poza `log usb`):** stick-ul e High-Speed; INQUIRY se
    reîncearcă de 10 ori cu REQUEST SENSE (Unit Attention), iar jurnalul scrie starea CSW și Sense Key/ASC/ASCQ. Scenariile USB + `shell` trec.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 49 de scenarii trecute (nou: `usb-disk-hs`, stick USB 2 pe port USB 2;
  `usb-disk` verifică acum pașii din jurnal și burst 15) + `https-real` sărit local; `cd rtl8168 && cargo test --release` → 20.
- **Pe laptop:** urmează **testul 4** (lista în hub, `stare/cyber-universe.md`). Dacă primirea tot se oprește: din `net` se vede dacă
  s-au făcut reporniri; următorul pas ar fi resetarea completă a plăcii la blocare. Dacă stick-ul tot nu pornește: `log usb` arată pasul și codul.

### (anterioară)
### Sesiunea programată 2026-10-22 05:40 UTC (08:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe`, niciun rezultat nou de la Adrian (testul 3 / P6-10 nefăcut încă).
- `docs/TASKS.md`: nicio sarcină liberă → sesiunea s-a oprit (CLAUDE.md §3). CI verde pe `06c69c9`. Mediul instalat (rustc nightly 2026-10-21).
- **Atenție (regula de oprire):** aceasta e a 2-a sesiune la rând fără nicio sarcină terminată (02:40, 05:40). Dacă și următoarea
  nu termină nimic, ea scrie „OPRIT: necesită analiza lui Adrian” aici și nu mai începe sarcini până la testul 3 sau o comandă nouă.

### (anterioară)
### Sesiunea programată 2026-10-22 02:40 UTC (05:40 ora României), în cloud, fără Adrian — fără lucru nou
- Hub: nicio comandă `nou` pentru `cyber-universe`; nu există rezultate noi de la Adrian (testul 3 / P6-10 nefăcut încă).
- `docs/TASKS.md`: nicio sarcină liberă (P6-10, P3-10 așteaptă testul pe laptop; P5-07/P5-08 după suită) → sesiunea s-a oprit (CLAUDE.md §3).
- CI verde pe `e34d8f1` (ultimul commit de cod: `3b24e00`). Mediul instalat cu `tools/env-check.sh --install` (rustc nightly 2026-10-21).
- Notă tehnică: clona locală a hub-ului era „shallow” și `git pull` refuza; rezolvat cu `git fetch --unshallow` + fast-forward.

### (anterioară)
### Sesiunea programată 2026-10-21 23:40 – 2026-10-22 00:45 UTC (02:40–03:45 ora României), în cloud, fără Adrian
- **Hub:** nicio comandă nouă (`nou`) pentru `cyber-universe`; C-2, C-4 gata, C-3 în lucru (acum: pașii 4 și 5 gata).
- **Terminate (2 sarcini):**
  - **P6-08** — `/bin/actualizeaza` (și butonul „Actualizeaza” din Setări, care deschide un terminal cu comanda): ultimul
    Release prin API-ul GitHub (token din `/usb/github.token`, trimis doar API-ului, nu și stocării la care redirecționează),
    fiecare fișier verificat cu `SHA256SUMS`, scris ca `.nou` și citit înapoi, apoi `vechi → .vechi`, `.nou → nume`, `sync`.
    O actualizare oprită la mijloc se repară la următoarea rulare. Nucleu: apelurile `rename` (33) și `sync` (34),
    `rename`/`flush` în FAT32, cache de un sector pentru FAT (lanțurile lungi nu mai cer o citire pe cluster),
    SYNCHRONIZE CACHE la stick. `cyber.rw` intră acum în imaginea de stick și în Release (stick-ul trebuie marcat ca să se
    poată scrie pe el). Reparat în `cyber_user`: un `{}` gol oprea restul rândului la `println!`.
  - **P6-09** — `/bin/gh-trimite <fișier> <proprietar/depozit> <cale> [mesaj]` (API „contents”: GET pentru `sha`, PUT cu
    base64), base64 propriu în `json/` (`cyber_json::base64`), unealta `github_push` pentru Claude (cu confirmare),
    client HTTPS comun `user/src/http_client.rs`.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → **47 de scenarii trecute** (noi: `update`, `ghpush`;
  `claude-tools` acoperă și `github_push`) + `https-real` sărit local; `cd json && cargo test --release` (base64 nou).
  CI verde pe `7bd2f6c` (P6-08) și `3b24e00` (P6-09).
- **Nu mai e nicio sarcină liberă:** P6-10 (testul pe laptop) și P3-10 așteaptă testul lui Adrian; P5-07/P5-08 după suită.

### (anterioară)
### Sesiunea programată 2026-10-21 20:40–22:35 UTC (23:40–01:35 ora României), în cloud, fără Adrian
- **Terminate (6 sarcini):** P6-02 (117 rădăcini Mozilla în `/etc/certs`, căutate după nume; ECDSA P-384; `fetch https://`
  cu site-uri reale), P6-03 (JSON propriu, crate `json/`), P6-04 (`/bin/claude` — clientul API Claude), P6-05 (`claude-app`,
  chat în fereastră; alocator cu eliberare reală pentru programe), P6-06 (uneltele lui Claude cu confirmare Da/Nu),
  P6-07 (CI publică Release-ul `os-<număr>` cu fișierele stick-ului și `SHA256SUMS`).
- A lucrat în paralel o sesiune pornită manual (P6-01b/c/d, C-4): `main` a fost unit cu ramura acestei sesiuni (merge, fără
  rescriere de istorie). Varianta ei de P6-02 (`wip/P6-02-b`) rămâne nefolosită — poate fi ștearsă de Adrian oricând.
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 45 de scenarii trecute + `https-real` sărit local (rulează
  doar cu internet, în CI: `CYBER_NET=1`); `crypto` 118 vectori + 117 rădăcini, `tls` (OpenSSL local + 10 site-uri reale în CI),
  `json` (RFC 8259 + 200 de documente față de Python), `rtl8168` 15 teste. CI verde pe `wip/P6-02` (cu site-urile reale).
  La o rulare completă locală, `ahci` a picat o dată la verificarea temporizatorului („54 intreruperi intr-o secunda”) —
  calculatorul era încărcat; rulat singur trece. Dacă reapare în CI, e de făcut verificarea mai tolerantă.
- **Descoperit:** depozitul `cyber-universe` e **privat** → Release-urile se descarcă doar cu token (vezi nota din `plans/P6-08.md`).

### Sesiune pornită manual de centrul de comandă (2026-10-21 20:27–21:40 UTC), în paralel cu sesiunea de la 20:40
- **Atenție:** două sesiuni au lucrat deodată. Sesiunea de la 20:40 UTC face **P6-02** pe `wip/P6-02` (o continuă ea).
  Varianta mea de P6-02 (făcută înainte să văd ramura) e păstrată pe **`wip/P6-02-b`**, nefolosită: rădăcinile Mozilla ca
  fișiere DER, ECDSA P-384 generic (`crypto/src/ecp.rs`, 300 de semnături verificate cu OpenSSL), TLS 0x0503, test cu un lanț
  P-384, workflow `real.yml`, plus o corectură la testul de cadre (`test_all_frames` măsura memoria înainte să-și rezerve lista).
  Dacă `wip/P6-02` nu are corectura de cadre și pică testul `boot` cu „cadre pierdute”, ia-o de acolo.
- **Testul 2 de la Adrian** (hub `rezultate/2026-10-22-test-2/`): NVMe merge; Realtek trimite 4, primește 0; `/usb` nemontat.
  **Reparat în `1976997` (pe `main`):** Realtek — PHY pornit prin `PHYAR` (BMCR), legătura așteptată max. 4 s, biții `ISR` șterși
  la fiecare citire, `net` arată legătura/`PHYstatus`/`ISR`/revizia; DHCP reîncearcă la 10 s după adresa de rezervă (și `net`
  spune că e de rezervă); stick-uri fără tabelă de partiții și MBR cu alt tip, comenzi SCSI TEST UNIT READY + REQUEST SENSE la
  pornirea stick-ului (~3 s); `hw` arată motivul când `/usb` nu e montat. Scenariu nou `usb-floppy`. 42/42 scenarii, `rtl8168` 14 teste.
- **Comanda C-4 (00:15) gata în `f7a455f` (CI verde):** pe lângă cele de mai sus, P6-01b/c/d în TASKS (cu planuri): registrele
  Realtek citite înapoi în jurnal (`net: rtl8168 registre: …`, cu „(GRESIT)” dacă adresele inelelor nu s-au prins), `net` cu
  pachetele pierdute de placă (MPC), ISR văzut și starea descriptorului RX; comanda **`log [cuvânt] [n]`** (jurnal de 64 KiB în
  memorie); **fără DHCP nu mai există adresă de rezervă** (`net` → „fara adresa”).
- **De la Adrian (testul 3):** lista e în hub (`stare/cyber-universe.md`). Dacă tot primește 0: din `log rtl` se vede ce registru
  nu a „prins” — de acolo pornește sesiunea următoare.

- **Data:** 2026-10-21, 19:46–20:20 UTC (22:46–23:20 ora României), sesiune programată în cloud, fără Adrian.
- **Deblocat:** comanda C-2 din hub (primul test pe hardware real a reușit, Lenovo ThinkPad AMD) a ridicat oprirea
  din 10:05; secțiunea „OPRIT” a fost ștearsă. Comanda C-3 (Faza 6) a fost preluată după C-2.
- **Terminate (4 sarcini):** P3-09 (rezultatul testului în `HARDWARE.md`, B-001 rezolvat, `USB.md` cu varianta FAT32 și
  nota despre Secure Boot), P3-11 (comanda `hw` — raport hardware pe o pagină), P5-09 (Faza 5 închisă fără P5-07/P5-08,
  mutate după suită; rând nou în `METRICS.md`), P6-01 (driver Realtek RTL8168/8111: crate `rtl8168/` cu model al cipului,
  interfață comună `nic::Card` pentru e1000 și Realtek). Faza 6 planificată: P6-01 … P6-10 cu planuri (D-012).
- **Build:** OK, fără avertismente. **Teste:** `tools/test.sh` → 41 de scenarii trecute (`shell` verifică și `hw`);
  `cd rtl8168 && cargo test --release` → 11 teste; CI verde pe `c8fac5a` și `a58aef3` (rularea pentru `cfd1dc5` pornită la final).
- Sesiunile 2026-10-20 23:41 … 2026-10-21 17:41: oprite (fără sarcini, apoi decizia lui Adrian din 10:05); nu s-a schimbat cod.

### Sesiunea anterioară (cu lucru)
- **Data:** 2026-10-20, 20:41–22:25 UTC (23:41–01:25 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (6 sarcini):** P5-12 (ChaCha20-Poly1305), P5-13 (numere mari + semnături RSA PKCS#1 v1.5 și PSS),
  P5-14 (semnături ECDSA P-256), P5-15 (certificate X.509 + verificarea lanțului), P5-06 (client TLS 1.3, crate `tls/`),
  P5-16 (HTTPS în OS: `fetch https://…`). P5-13 inițial a fost împărțit în P5-13 … P5-15, iar P5-06 în P5-06 + P5-16
  (cu planuri), cum permiteau planurile lor.
- **Blocate (au nevoie de Adrian):** P3-09 (B-001, test pe un calculator real; P3-10 după el), P5-07 (B-003, suita office),
  P5-08 și P5-09 depind de P5-07. **Nu mai există nicio sarcină liberă** → sesiunea s-a oprit (CLAUDE.md §3).
- **Ce merge acum (rezumat pentru Adrian):** Cyber Universe poate descărca pagini prin **HTTPS**, cu criptografie scrisă
  integral de noi: `fetch https://nume/cale`. Certificatul site-ului e verificat (lanț până la o rădăcină de încredere,
  numele, data); unul greșit e refuzat. În emulator: o pagină de 18 KB în ~1,3 s.
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **41 de scenarii trecute** (nou: `https`); `crypto` are 108 verificări; `cd crypto && cargo test
  --release` și `cd tls && cargo test --release` (clientul nostru față de un server OpenSSL real) trec, și în CI.
  Verificări încrucișate cu OpenSSL, în afara repo-ului: 300 de mesaje ChaCha20-Poly1305, 116 semnături RSA, 200 ECDSA,
  lanțurile cu `openssl verify` — zero diferențe. CI verde pe toate commit-urile terminate (ultimele încă rulau la final).

## Următoarea sarcină
**Nicio sarcină liberă.** Așteaptă **testul 4** al lui Adrian (C-5 pasul 3): `log rtl`, `net`, `ping 1.1.1.1`, `log usb`, `hw`, `ls /usb`.
Din pozele lui: reparații (sarcini noi P6-10c…), apoi P6-10 și P3-10. Sesiunile fără rezultate noi: verifică hub-ul și CI, apoi se opresc
(dar NU scriu „OPRIT” cât timp C-5 e `preluat`, cum cere C-5 pasul 4).

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
- **Programele cu HTTPS și heap** folosesc `user/src/http_client.rs` (inclus cu `#[path]`); imitațiile pentru teste:
  `tools/update-test-server.py` (porturile 18445 API + 18446 stocare) și `tools/ghpush-test-server.py` (18447).
- **Scrierea pe stick** cere `cyber.rw` în rădăcina lui; `rename` merge doar pe FAT32 și doar pentru fișiere.
- **Rădăcinile de încredere:** `boot/certs/` e generat de `tools/roots.py` din `certdata.txt` (NSS); pentru o listă nouă
  rulează scriptul (vezi `boot/certs/SURSA.md`). `fetch` și `claude` caută rădăcina după `subject_id` (fișierul `<id>.0`).
- **Internetul în sesiunile cloud ale lui Claude trece printr-un proxy care schimbă certificatele** → testele cu site-uri reale
  merg doar în CI (`CYBER_NET=1`); local, `https-real` e sărit (scenariile cu `internet` în coloana QEMU).
- **Programele cu heap:** `cyber_user::heap!(MĂRIME)`; clientul Claude e în `user/src/claude_api.rs` și uneltele în
  `user/src/claude_tools.rs` (incluse cu `#[path]`, ca biblioteca `cyber_user` să rămână fără heap).
- **Imitația API-ului Claude:** `tools/claude-test-server.py` (port 18444, cheia `sk-test-cyber`); scenariile `claude-cli`,
  `claude-app`, `claude-tools`. Nu rula două `tools/test.sh` deodată: serverele de test folosesc porturi fixe.
- Meniul Start are 16 programe; `test_desktop` caută `demo` după nume (nu mai depinde de poziție).
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install` (acum instalează și dosfstools, mtools, fdisk).
- **Cyber Hub:** se clonează direct `https://github.com/Adriancu91/Cyber-hub` (fără add_repo); merge din sesiunile programate.
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
- Scenariile cu servere de test pe gazdă: `tools/before-<scenariu>.sh` pornește înainte de QEMU și e oprit după
  (`dns`: server DNS 15353; `tcp`: server web 18080; `appnet`: amândouă + ecou UDP 15354). Nucleul le vede la 10.0.2.2.
- Plăcile de rețea: `drivers/nic.rs` (`Card`, `NIC`); logica Realtek în crate-ul `rtl8168/` (teste pe gazdă cu `src/model.rs`).
  Un driver nou de placă implementează `Card` și se adaugă în `nic::init_global` și `nic::driver_for` (`hw` îl arată).
- Criptografia e în crate-ul `crypto/` (fără dependențe); nucleul o folosește pentru `random` (SHA-256) și testul `crypto`.
  Clientul TLS 1.3 e în `tls/` (folosit de `/bin/fetch`). Numerele aleatoare: `kernel/src/random.rs` — fără RDRAND (QEMU-ul
  de test) se bazează doar pe TSC: bune pentru teste, slabe în fața unui atacator serios.
- Scenariul `https` pornește `tools/before-https.sh` (serverul OpenSSL din `tls/tests/server.py`, 10.0.2.2:18443); rădăcina
  de test e scrisă în `/etc/certs/` doar de test (cheia ei privată e publică în repo — nu o pune niciodată în imaginea normală).
- Criptografia: `crypto/` (fără dependențe). Vectorii noi
  se adaugă în `crypto/src/vectors.rs` și se actualizează numărul din scenariul `crypto`.
- Valorile așteptate ale vectorilor se verifică mereu cu Python `cryptography` (OpenSSL) înainte să fie scrise în cod.
  Certificatele de test: `crypto/src/testcerts/` (scriptul `make.py`); OpenSSL face uneori chei RSA cu un bit mai puțin
  decât s-a cerut (2049 → 2048), deci verifică lungimea reală.
- Cârlige de test pentru pierderi TCP: `tcp::DROP_NEXT_SEND`, `tcp::DROP_RECV_AFTER` (doar în build-ul de test).
- **`tools/test.sh --no-build` nu construiește imaginile de test** — folosește-l doar după o rulare completă.
- **Atenție: la 17:41 UTC a pornit o ALTĂ sesiune programată în paralel** (a marcat P5-02 „în lucru” în timp ce
  această sesiune îl termina; conflictul din TASKS.md a fost rezolvat prin merge). Dacă două sesiuni se suprapun,
  fă `git pull` des și verifică TASKS.md înainte să începi o sarcină.
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~8 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
