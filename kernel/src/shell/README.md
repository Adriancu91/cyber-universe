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
