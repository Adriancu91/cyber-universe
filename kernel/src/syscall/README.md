# syscall — apelurile de sistem

- **Ce face:** serviciile pe care programele din modul utilizator (ring 3) le cer nucleului cu instrucțiunea `syscall`.
  Intrarea (salvarea registrelor, stiva nucleului) e în `arch/x86_64/syscall.rs`; aici se verifică argumentele și se face treaba.
- **Convenție:** numărul apelului în `rax`; argumentele în `rdi, rsi, rdx, r10, r8`; rezultatul în `rax`
  (negativ = eroare: `-1` argument greșit, `-2` pointer în afara memoriei programului). `rcx` și `r11` se pierd.
  Al patrulea argument (unde există) e în `r10`.
- **Apeluri:** `0 exit(cod)`, `1 write(fd, buf, len)` (fd 1 și 2 = consola: ecran + serial), `2 yield()`, `3 sleep_ms(ms)`,
  `4 open(cale, lungime)` → descriptor (de la 3), `5 read(fd, buf, len)`, `6 close(fd)`, `7 stat(cale, lungime)` → mărimea.
- **Ferestre (P4-05, `ui.rs`):** `8 window_create(lățime | înălțime << 32, titlu, lungime)` → id fereastră;
  (cu bitul 63 din mărime: fereastra desktopului — tot ecranul, fără ramă, mereu sub celelalte; una singură);
  `9 window_present(id, pixeli, x | y << 16 | l << 32 | î << 48, pas)` — copiază o imagine l × î din memoria programului
  (0x00RRGGBB; rândul i începe la `pixeli + i × pas`; pas 0 = același rând repetat, adică o culoare) la (x, y) în fereastră
  și o arată; `10 event_wait(id, ms)` → tip << 48 | date (1 = tastă: caracterul; 2 = clic: butoane << 32 | x << 16 | y;
  0 = nimic în `ms` milisecunde, 0 = fără limită); `-3` când fereastra a fost închisă; `11 window_close(id)`.
- **Taste speciale (P4-10):** în `event_wait`, săgețile sunt caracterele U+F700 (sus), U+F701 (jos), U+F702 (stânga),
  U+F703 (dreapta), Delete U+F728, Home U+F729, End U+F72B; o literă apăsată cu Ctrl are tipul 3.
- **Programe și sistem (P4-06):** `12 spawn(cale, lungime, arg, lungime_arg)` → id-ul programului nou (argumente: calea
  și, dacă `lungime_arg` > 0, textul `arg`); `13 wait(id)` → codul lui de ieșire
  (`-3` dacă nu există); `14 list(cale, lungime, buf, mărime)` → numele din director, câte unul pe rând (`/` după directoare);
  `15 time()` → an << 40 | lună << 32 | zi << 24 | oră << 16 | minut << 8 | secundă (`-3` fără ceas);
  `16 screen_size()` → lățime | înălțime << 32.
- **Informații, oprire, fișiere (P4-09):** `17 sysinfo(tip, buf, mărime)` — tip 0: 7 numere u64 (memorie totală, folosită,
  heap folosit, heap rezervat, ms de la pornire, tick-uri libere ale procesorului, tick-uri pe secundă); tip 1: programele
  care merg, câte un rând `id tick-uri nume`; `18 kill(id)`; `19 create(cale, lungime, tip)` (0 fișier, 1 director);
  `20 remove(cale, lungime)`; `write(fd ≥ 3, ...)` scrie într-un fișier deschis, la poziția lui.
  `21 module(op, text, lungime, buf, mărime)` — managerul de module (0 listă, 1 instalare, 2 pornire, 3 oprire, 4 ștergere).
- **Terminal (P4-13):** `22 term_open()` → id (pornește un shell pentru program); `23 term_read(id, buf, mărime)` → ce a
  scris shell-ul (0 = nimic nou, `-3` după `exit`); `24 term_write(id, text, lungime)` — tastele din fereastră.
  Un program pornit dintr-un terminal scrie (fd 1, 2) în acel terminal.
  Erori noi: `-7` există deja, `-8` disc nemarcat pentru scriere, `-9` director plin, `-10` disc plin.
  Ferestrele unui program se închid singure când programul iese. Doar programul care a deschis fereastra o poate folosi.
  **Alegere:** imaginea rămâne în memoria programului și se copiază la `window_present` (nu cadre partajate): nimic de
  eliberat de două ori, iar copia unei zone e mică față de desen.
- **Rețea (P5-05, `net.rs`):** `25 resolve(nume, lungime)` → adresa IPv4 ca număr (a<<24|b<<16|c<<8|d);
  `26 tcp_connect(ip, port, ms)` → descriptor; `read`/`write`/`close` pe el = primire (așteaptă max. 30 s; 0 = serverul a
  închis), trimitere, închidere; `27 udp_open(port sau 0)` → descriptor; `28 udp_send(fd, buf, n, ip, port)`;
  `29 udp_recv(fd, buf, n, ms, de_la)` (de_la = 6 octeți: ip + port); `30 net_info(buf, 48)`; `31 ping(ip, nr, ms)` → ms; `32 random(buf, n)` (P5-16, cel mult 4096 de octeți; vezi `kernel/src/random.rs`).
- **Fișiere (P6-08):** `33 rename(de_la, lungime, la, lungime)` — doar fișiere, pe același disc (FAT32), `la` să nu existe;
  `34 sync(cale, lungime)` — discul pe care e `cale` își golește memoria cache (la stick: SYNCHRONIZE CACHE).
  Socket-urile stau în același tabel ca fișierele și se închid când programul iese. Erori noi: `-17` fără răspuns la timp,
  `-18` refuzat/întrerupt, `-19` rețea fără adresă, `-20` conexiune închisă, `-21` numele nu există.
- **Memorie mare (K6-03):** `35 mem_map(lungime)` → adresa unei regiuni noi, plină de zero, citire + scriere, fără execuție,
  aliniată la 2 MiB (de la `0x1000_0000_0000` în sus, cu 2 MiB liberi între regiuni); `36 mem_unmap(adresă, lungime sau 0)`
  eliberează exact o regiune dată de `mem_map`. Bucățile întregi de 2 MiB folosesc **pagini mari de 2 MiB** (o intrare de
  director de pagini, 512 cadre contigue aliniate), restul pagini de 4 KiB; dacă memoria fizică nu mai are blocuri de 2 MiB libere,
  totul merge cu 4 KiB. **Alegere:** memoria se ia imediat, nu la prima folosire: un program care cere memoria unui model o
  folosește toată, iar „nu e destulă memorie” se vede la cerere (`-22`), nu ca o eroare în mijlocul calculului.
  Nucleului îi rămân mereu 16 MiB. O cerere peste 64 GiB = `-1`. Regiunile rămase se eliberează când programul iese.
  Apelurile de sistem primesc adrese din regiuni ca pe orice memorie a programului (și din paginile de 2 MiB).
  Biblioteca: `cyber_user::mem_map(len)`, `cyber_user::mem_unmap(ptr, len)`. Test: scenariile `memorie` (`-m 2G`: `/bin/memorie`
  alocă 1 GiB = 512 pagini de 2 MiB, scrie și verifică, eliberează; cadrele libere revin exact) și `memorie-smp` (pe 4 nuclee).
- **Fire (K6-04):** `37 thread_spawn(intrare, vârful stivei, arg0, arg1)` → id-ul firului nou (pornește cu `rsp` = vârful dat,
  `rdi = arg0`, `rsi = arg1`; cei 64 de octeți de sub vârf trebuie să fie memorie a programului cu drept de scriere, altfel `-2`;
  peste 64 de fire `-6`); `38 thread_join(id)` → codul firului (`-3` dacă nu e al programului sau a fost deja cules);
  `39 thread_exit(cod)` oprește firul (pe firul principal: tot programul). `exit` dintr-un fir oprește tot programul.
  `mem_unmap` cu alte fire în viață → `-23`. Biblioteca: `cyber_user::thread::{spawn, spawn_with_stack, join, exit}`.
  Detalii: `kernel/src/proc/README.md`. Test: scenariile `fire`, `fire-1`.
- **Erori:** `-1` argument invalid, `-2` adresă invalidă, `-3` nu există, `-4` e director, `-5` descriptor închis, `-6` prea multe fișiere, `-22` memorie insuficientă.
- **Siguranță:** orice pointer primit e verificat pagină cu pagină (`paging::user_accessible`): în jumătatea de jos,
  mapat cu drept USER la toate nivelurile. O excepție în ring 3 oprește doar programul (cod de ieșire `-1`).
- **Depinde de:** `sched`, `mem::paging`, `proc`, `ui` (ferestre).
- **Cum se testează:** scenariul `log` — `salut din ring 3`, `a iesit cu codul 7`, programul cu pointer greșit și
  citire din memoria nucleului e oprit, nucleul continuă (`test: mod utilizator OK`) și `test: fisiere din programe OK` (`/bin/cat` citește `/etc/motd`).
  Ferestre: scenariul `appwindow` (`/bin/demo`); desktop, `spawn`, `list`, `time`: scenariul `desktop`; rețea: scenariul `appnet`.
- **Surse:** Intel SDM Vol. 2 (SYSCALL/SYSRET/IRETQ), Vol. 3A cap. 5 și 7. Cod propriu.
