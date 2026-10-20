# syscall — apelurile de sistem

- **Ce face:** serviciile pe care programele din modul utilizator (ring 3) le cer nucleului cu instrucțiunea `syscall`.
  Intrarea (salvarea registrelor, stiva nucleului) e în `arch/x86_64/syscall.rs`; aici se verifică argumentele și se face treaba.
- **Convenție:** numărul apelului în `rax`; argumentele în `rdi, rsi, rdx, r10, r8`; rezultatul în `rax`
  (negativ = eroare: `-1` argument greșit, `-2` pointer în afara memoriei programului). `rcx` și `r11` se pierd.
- **Apeluri:** `0 exit(cod)`, `1 write(fd, buf, len)` (fd 1 = consola: ecran + serial), `2 yield()`, `3 sleep_ms(ms)`.
- **Siguranță:** orice pointer primit e verificat pagină cu pagină (`paging::user_accessible`): în jumătatea de jos,
  mapat cu drept USER la toate nivelurile. O excepție în ring 3 oprește doar programul (cod de ieșire `-1`).
- **Depinde de:** `sched`, `mem::paging`, `proc`.
- **Cum se testează:** scenariul `log` — `salut din ring 3`, `a iesit cu codul 7`, programul cu pointer greșit și
  citire din memoria nucleului e oprit, nucleul continuă (`test: mod utilizator OK`).
- **Surse:** Intel SDM Vol. 2 (SYSCALL/SYSRET/IRETQ), Vol. 3A cap. 5 și 7. Cod propriu.
