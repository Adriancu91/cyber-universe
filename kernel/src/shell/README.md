# shell — linia de comandă

- **Ce face:** după pornire apare promptul `cyber> `. Se scrie o comandă de la tastatură (Backspace șterge), Enter o rulează.
  Tot ce afișează merge pe ecran și pe portul serial.
- **Comenzi:** `help`, `ls [director]`, `cat <fișier>...`, `run <program> [argumente]` (din `/bin` dacă nu dai calea;
  shell-ul așteaptă programul și arată codul de ieșire dacă nu e 0), `mem`, `uptime`, `reboot` (prin controlerul tastaturii).
- **Stadiu:** în Faza 2 shell-ul rulează în nucleu (firul 0). Mutarea lui ca program separat se face doar dacă aduce un plus.
- **Depinde de:** `drivers::keyboard::wait_key`, `fs`, `proc`, `mem`, `time`.
- **Cum se testează:** scenariul `shell` — QEMU tastează `help`, `ls /bin`, `mem`, `run cat /etc/motd`, o comandă greșită
  (cu Backspace), `run lipsa`, `uptime`; se verifică răspunsurile. Comanda `test-gata` există doar în build-ul de test
  (închide shell-ul ca testul să se termine). `reboot` nu se testează automat.
- **Surse:** design propriu.
- **Terminal (P4-13):** același shell rulează și pentru fereastra `terminal` (`term.rs`): un fir de shell pe fereastră,
  care citește tastele trimise de fereastră și scrie acolo (inclusiv ce scriu programele pornite cu `run`); `exit` îl închide.
- **La pornire (P4-13):** nucleul normal pornește întâi desktopul (`/bin/desktop`); când desktopul se închide (Esc),
  rulează shell-ul în consola text, ca înainte.

- **`hw` (P3-11):** raport hardware pe o pagină, pentru o singură poză pe un calculator real: procesorul, memoria, de cât
  timp merge, dispozitivele PCI cu driverul care le folosește (`[-]` = fără driver), USB, `/disk` și `/usb`, rețeaua.
  Testat în scenariul `shell`.
- **`log [cuvânt] [n]` (P6-01d):** ultimele n rânduri (implicit 30) din jurnalul nucleului, ținut în memorie (64 KiB), doar cele
  care conțin cuvântul (ex. `log rtl`, `log usb 50`). Pentru calculatoare fără port serial. Testat în scenariul `shell`.
- **`raport` — „cutia neagră” (P6-22, comanda C-22):** scrie pe stick `/usb/raport/AAAALLZZ-<nr>.txt` (primul număr liber):
  data, motivul, versiunea (nucleu, Cyber-GPU, `VERSIUNE` de pe stick), ieșirea lui `hw`, `lspci`, `net`, jurnalele `net`
  (cu rândurile de registre ale plăcilor de rețea), `usb`, `pci`, `gpu`, erorile și avertismentele, apoi tot jurnalul păstrat.
  **Automat:** înainte de prompt, dacă în jurnal a apărut un rând ERR nou și stick-ul e montat cu scriere, se scrie singur un raport
  (cel mult 3 pe pornire). La panică **nu** se scrie (nucleul oprit poate ține încuietorile discului; scrierea ar putea strica stick-ul).
  **Fără chei:** conținutul fișierelor `claude.key`/`github.token` (din `/usb`, `/disk`, `/etc`) și orice cuvânt care arată a cheie
  (`sk-ant-`, `ghp_`, `github_pat_`, `Bearer`, `Authorization:`, `x-api-key:`) devine `[ascuns]`. Captura comenzilor: `emit` scrie
  în raport în loc de ecran cât timp `raport` rulează o comandă pe același fir.
- **`trimite` (P6-61, comanda C-84):** rapoartele noi din `/usb/raport` pleacă în hub (`rezultate/os-automat/<data>/`) cu
  `/bin/gh-trimite --rapoarte`; se face singur și după `raport` și după `gpu lant`. Doar dacă există `/usb/github.token` și rețea;
  altfel mesaj clar și rapoartele rămân pe stick. Lista celor trimise: `/usb/raport/trimise.txt`. Test: `ghpush`.
- **`hub [comenzi|stare|decizii] [proiect]` (P6-61):** pornește `/bin/gh-ia --hub` — citește Cyber Hub, nu scrie nimic.
- **`hub ruleaza [--arata]` (P6-64, comanda C-87):** `hubcmd.rs` — ia din hub `integrare/cyber-universe/os-comenzi.md`
  (`/bin/gh-ia --hub-ia`), alege rândurile `- [ ] K-<nr>: <comandă>` încă nerulate (`/usb/hub/facute.txt`, notat înainte de
  rulare), cel mult 20, rulează doar lista albă (a modului automat + `test tot` + `trimite`; `DA`, `hub`, `gpu lant`, `--gpu`,
  `--scrie`, `reboot` refuzate), scrie `/usb/raport/hub-<data>.txt` și face `trimite`. `/usb/hub/la-pornire` = la fiecare
  pornire, pe un fir al sistemului. Test: `ghpush` (+ verificarea listei în `auto`).
- **`hub asculta [--la N]` / `hub opreste` (P6-73, comanda C-104):** `hubcmd.rs` — aceeași rulare în buclă, la fiecare N minute
  (1–60, implicit 2), până la Esc; `/usb/hub/asculta` (gol sau cu numărul de minute) = pornită la pornire pe un fir al sistemului,
  oprită cu `hub opreste`. Fiecare comandă (și cele refuzate) primește răspunsul ei în hub, `integrare/cyber-universe/raspunsuri/K-<nr>.txt`
  (`gh-trimite --raspuns`, doar acel dosar), copie pe stick în `/usb/hub/raspunsuri/`. Test: `ghpush` (3 runde la 1 s, K-5 apoi K-6).
- **`live <minute>` — canalul viu (P6-108, P6-109, comanda C-159 pasul 1):** `live.rs` — poarta (semnătura HMAC-SHA-256 cu
  `/usb/hub/live.cheie`, numărul exact următor, o singură dată, placa doar cu `permite-live-placa`, ecoul exact) și bucla: deschide
  `live/<sesiune>/` pe ramura `os/live` a hub-ului, întreabă dosarul la 2,5 s cu ETag (`gh-ia --live-dosar`), ia `c-NNNN.json` în
  ordine (`gh-ia --live-ia`), notează în `/usb/hub/live/<sesiune>/jurnal.txt` ÎNAINTE de rulare, rulează cu ieșirea prinsă, trimite
  `r-NNNN.json` (`gh-trimite --live`, fără rescriere). Sfârșit: `gata`, <minute> fără comenzi, Esc. Și ca rând `live <minute>` în
  `auto/teste.txt` (ceasul de siguranță e împins cât lucrează). Forma fișierelor: `docs/LIVE.md`. Test: `ghpush` (9 comenzi: 5 refuzuri, 4 rulate).
- **`ciclu-test os-<N>` din hub (P6-75, comanda C-105) — DEZACTIVAT implicit:** `ciclu.rs` — actualizare la exact acel Release
  (`actualizeaza --versiune`), repornire (încercare + confirmare, `revenire.rs`), `gpu recupereaza` + `gpu lant` fără DA, raport +
  răspuns în hub; fazele în `/usb/hub/ciclu.txt` (`actualizare` → `test` → `test-pornit`); eșecul ciclului = versiunea veche înapoi
  (`revenire::return_to_old`). Rulează doar cu ambele chei: `/usb/hub/permite-test-placa` + un rând de decizie cu `ciclu-test activat`
  în `DECIZII.md` din hub; altfel REFUZAT. Test: `ghpush` (refuzuri, oprire curată la actualizare, toate deciziile de după repornire) +
  `update` (`--versiune`).
  **P6-96 (C-143, cheile puse de Adrian pe 18 nov):** `ciclu-test os-<N> [--i9 c125|c133|c135|os302]` (implicit `c135`); testul =
  `gpu recupereaza` → dacă pregătirea I6 nu trece după ea: „CICLU OPRIT: trebuie pornire rece (Adrian)”, fără altă încercare → altfel
  lanțul cu I9-ul ales și, dacă `/usb/kronos/kronos_513_q8_0.kgr` există, `kronos --gpu --n 16 Buna ziua` în același lanț (cheile țin
  locul lui DA pentru acel program). Același test din lista automată: `test-placa [--i9 …]` în `auto/teste.txt` (`auto.rs`), doar cu
  ambele chei; raportul `raport/ciclu-auto-<data>.txt` + gpu-jurnalul urcă în hub (`trimite`), apoi lista se termină → repornire →
  Windows. Test: `ghpush` (forma, rândul real din DECIZII.md, recuperarea), `auto` (`test-placa` refuzat fără chei).
  **P6-139 (C-158 p.5):** până la 4 teste pe placă într-o pornire (`ciclu::board_slot`): primul cu `gpu recupereaza`, următoarele
  cu resetul din interior (`FLR_ARMED`) la începutul lanțului; placa blocată = restul refuzat. Test: `auto-placa` (`test-placa-sim`,
  doar în build-ul `test-auto`: aceeași numărătoare, lanț prefăcut).
- **`lucrare <nume>` (P6-140, C-158 p.8, unealta 8):** `lucrare.rs` — lucrări lungi din `/usb/lucrari/<nume>.txt` (pornire la o
  oră sau după N minute, raport în hub la N minute, oprire la comandă / la oră / după o durată, pași doar din lista albă a
  modului automat); starea `<nume>.stare` scrisă după fiecare pas, reluată la pornire de la pasul următor (un pas tăiat de 2 ori
  la rând oprește lucrarea). `lucrare stare [nume]`, `lucrare opreste <nume>`; rând în `auto/teste.txt`. Detalii: `docs/LUCRARI.md`.
  Test: `lucrare` (lista automată), `lucrare-reluare` (după o pană de curent prefăcută), `lucrare::self_test`.
- **`test tot` (P6-61, F9):** `testtot.rs` — autotestul pe calculatorul real (procesor, ceas, temporizator, stick, rețea, placa
  video doar prin ID-urile PCI); `PASS`/`FAIL`/`LIPSA`, raport `raport/test-tot-*.txt`, apoi `trimite`. Test: scenariul `testtot`.
- **`ora [server [port]]` (P6-61):** SNTP (`net/sntp.rs`, RFC 4330): ora din rețea în UTC și ora României (regula UE pentru
  ora de vară), diferența față de ceasul CMOS (doar citit). Test: scenariul `ora` (`tools/ora-test-server.py`).
  Test: scenariul `raport` (build `test-raport`: în QEMU raportul merge pe `/disk`): raportul automat (eroarea de test de la pornire)
  și cel manual, `tools/after-raport.sh` le citește cu `mtools` (fsck curat, secțiunile, niciun token); filtrul are și un test la pornire.
- **`cat` și fișierele secrete (P6-23):** `cat /usb/claude.key` (sau `github.token`) nu afișează nimic: „fisier secret”.
- **Modul automat (P6-41, comanda C-53):** `auto.rs`. La pornire, înainte de desktop, dacă există `/usb/auto/teste.txt`, rulează
  comenzile din lista albă (doar citire, inclusiv `gpu tur` din P6-42; tot ce conține `--scrie`, `gpu i6`, `gpu lant`, `reboot` = refuzat), scrie `auto/raport-<AAAALLZZ-HHMM>.txt`
  pe loc (inclusiv ce scriu programele pornite cu `run`, prin `program_output` din apelul de sistem `write`), redenumește lista în
  `teste-facut-<data>.txt` și repornește. Ceasul de siguranță (`ceas N` minute, implicit 10) repornește dacă lista nu termină;
  marcajul `auto/in-lucru.txt` împiedică reluarea unei liste întrerupte. Scenarii: `auto`, `auto-ceas`, `auto-intrerupt`
  (disc de test `/disk/auto`, feature `test-auto`). Pregătirea pe workstation: `docs/BUCLA_AUTOMATA.md`.
- **`compara <f1> <f2>` (P6-52 pas 1, C-68 F9):** rândurile care diferă între două rapoarte sau jurnale (`-` doar în primul,
  `+` doar în al doilea, cu numărul rândului), apoi „N la fel, X doar în primul, Y doar în al doilea”. Caută următorul rând comun
  până la 64 de rânduri înainte (memorie puțină, fără tabel complet); cel mult 200 de rânduri afișate; fișierele secrete refuzate.
  E pe lista albă a buclei automate (doar citește). Testat la pornire (`test: compara OK`, scenariul `boot`).
- **Unelte de profil/jurnal (P6-52 pas 3, C-68 F9):** `log --sumar` (rânduri pe nivel și pe modul, cele cu avertismente/erori
  primele), `log --avertismente [n]` (doar WARN/ERR), `timp <comanda>` (durata în ms + memoria liberă/heap înainte și după;
  în jurnal `timp: <comanda> = N ms`). Pe lista albă a buclei automate și a remote-ului doar dacă și comanda măsurată e permisă
  (`timp reboot`, `timp gpu i6 --scrie`, `timp timp ...`, `timp exit` refuzate). Testat în scenariile `remote` și `auto`.
- **`remote porneste [port] | opreste | stare` (P6-49, F2):** consola text prin rețea (`net/remote.rs`, ghid în `docs/REMOTE.md`).
  Shell-ul unei sesiuni remote e un terminal „restricționat” (`term::open_restricted`): înainte de fiecare comandă se verifică
  `remote_allowed` (lista modului automat + `help`, `exit`, `remote stare|opreste`); refuzurile și comenzile merg în jurnal.
  Testat în scenariul `remote`.
- **`antrenare proba [dosar]` (P6-121, C-161 punctul 3, `antrenare.rs`):** pasul de aur al lui Kronos (S132, hub
  `integrare/kronos/pas_de_aur_mic/`) refăcut în OS cu oracolul Cyber-GPU al unui pas de antrenare (`gpu_compute::train::golden_run`,
  doar procesorul, placa neatinsă): SHA-256 al rețetei verificat, 10 pași `sgd` și 10 `adamw_513`, comparați cu PyTorch cu pragurile
  scrise (`train::TOL_*`) → `antrenare proba: ACELEASI NUMERE / DIFERITE / NU A RULAT`. Depinde de `gpu-compute` (cyber-gpu) și de
  datele de pe disc (`/disk/antrenare/pas_de_aur_mic`, apoi `/usb/…`). Inspirația: testul T151 al Cyber-GPU (cod propriu). Testat în
  scenariul `antrenare` (+ `self_test` în `ghpush`).
- **`antrenare mini … --placa-sim [--fara-oracol]` (P6-158, C-161 punctul 5, `antrenare_sim.rs`, doar build-ul `test-gpu-sim`):**
  pasul lui `antrenare mini` (P6-157) făcut de lista plăcii (`nvidia_ga104::train_step::Plan::step`, 13 programe, 75 de
  lansări, o trimitere pe pas) pe placa simulată (`gpu_sim::cmdproc`), cu starea AdamW a rulării urcată în arenă; oracolul
  `gpu_compute::train` merge alături pe aceleași loturi → `ACELEASI NUMERE / DIFERA` (toleranța 1e-3). Depinde de `nvidia-ga104`,
  `gpu-sim`, `gpu-compute` (cyber-gpu). Inspirația: testul T165 al Cyber-GPU (cod propriu). Testat în scenariul `antrenare-mini-placa`.
