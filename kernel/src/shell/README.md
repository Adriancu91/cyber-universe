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
- **`ciclu-test os-<N>` din hub (P6-75, comanda C-105) — DEZACTIVAT implicit:** `ciclu.rs` — actualizare la exact acel Release
  (`actualizeaza --versiune`), repornire (încercare + confirmare, `revenire.rs`), `gpu recupereaza` + `gpu lant` fără DA, raport +
  răspuns în hub; fazele în `/usb/hub/ciclu.txt` (`actualizare` → `test` → `test-pornit`); eșecul ciclului = versiunea veche înapoi
  (`revenire::return_to_old`). Rulează doar cu ambele chei: `/usb/hub/permite-test-placa` + un rând de decizie cu `ciclu-test activat`
  în `DECIZII.md` din hub; altfel REFUZAT. Test: `ghpush` (refuzuri, oprire curată la actualizare, toate deciziile de după repornire) +
  `update` (`--versiune`).
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
