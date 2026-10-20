# syscall — apelurile de sistem

- **Ce face:** serviciile pe care programele din modul utilizator (ring 3) le cer nucleului cu instrucțiunea `syscall`.
  Intrarea (salvarea registrelor, stiva nucleului) e în `arch/x86_64/syscall.rs`; aici se verifică argumentele și se face treaba.
- **Convenție:** numărul apelului în `rax`; argumentele în `rdi, rsi, rdx, r10, r8`; rezultatul în `rax`
  (negativ = eroare: `-1` argument greșit, `-2` pointer în afara memoriei programului). `rcx` și `r11` se pierd.
- **Apeluri:** `0 exit(cod)`, `1 write(fd, buf, len)` (fd 1 și 2 = consola: ecran + serial), `2 yield()`, `3 sleep_ms(ms)`,
  `4 open(cale, lungime)` → descriptor (de la 3), `5 read(fd, buf, len)`, `6 close(fd)`, `7 stat(cale, lungime)` → mărimea.
- **Ferestre (P4-05, `ui.rs`):** `8 window_create(lățime | înălțime << 32, titlu, lungime)` → id fereastră;
  `9 window_present(id, pixeli, x | y << 16 | l << 32 | î << 48)` — copiază zona din imaginea programului (lățime × înălțime,
  0x00RRGGBB, în memoria lui) în fereastră și o arată; `10 event_wait(id)` → tip << 48 | date (1 = tastă: caracterul;
  2 = clic: butoane << 32 | x << 16 | y); `-3` când fereastra a fost închisă; `11 window_close(id)`.
  Ferestrele unui program se închid singure când programul iese. Doar programul care a deschis fereastra o poate folosi.
  **Alegere:** imaginea rămâne în memoria programului și se copiază la `window_present` (nu cadre partajate): nimic de
  eliberat de două ori, iar copia unei zone e mică față de desen.
- **Erori:** `-1` argument invalid, `-2` adresă invalidă, `-3` nu există, `-4` e director, `-5` descriptor închis, `-6` prea multe fișiere.
- **Siguranță:** orice pointer primit e verificat pagină cu pagină (`paging::user_accessible`): în jumătatea de jos,
  mapat cu drept USER la toate nivelurile. O excepție în ring 3 oprește doar programul (cod de ieșire `-1`).
- **Depinde de:** `sched`, `mem::paging`, `proc`, `ui` (ferestre).
- **Cum se testează:** scenariul `log` — `salut din ring 3`, `a iesit cu codul 7`, programul cu pointer greșit și
  citire din memoria nucleului e oprit, nucleul continuă (`test: mod utilizator OK`) și `test: fisiere din programe OK` (`/bin/cat` citește `/etc/motd`).
  Ferestre: scenariul `appwindow` (`/bin/demo`).
- **Surse:** Intel SDM Vol. 2 (SYSCALL/SYSRET/IRETQ), Vol. 3A cap. 5 și 7. Cod propriu.
