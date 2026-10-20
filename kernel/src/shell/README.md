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
