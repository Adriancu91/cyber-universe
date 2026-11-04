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
