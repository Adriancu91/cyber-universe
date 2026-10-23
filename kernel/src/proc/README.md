# proc — procese

- **Ce face:** un proces = un spațiu de adrese propriu (`mem::address_space`) + un fir care rulează programul în ring 3.
  `spawn_elf(fișier)` încarcă un executabil ELF64 static (`elf.rs`: verifică antetul și segmentele înainte de orice alocare,
  mapează fiecare segment cu drepturile lui — cod executabil, date fără execuție, `.bss` pus pe zero);
  `spawn_flat(cod)` încarcă cod mașină simplu (pentru teste). Stiva programului: 64 KiB sub `0x7fff_fff0_0000` (programele n-au heap, tabelele lor stau pe stivă).
  `wait(id)` așteaptă (dormind) terminarea și dă codul de ieșire; la ieșire toată memoria procesului se eliberează.
- **Argumente (P2-08):** `spawn_path(cale, argumente)`; la pornire programul primește `rdi = argc`, `rsi` = tabel de perechi
  (adresă, lungime) pe stiva lui. Fiecare proces are tabelul lui de fișiere deschise (închise automat la ieșire).
- **Depinde de:** `mem::address_space`, `sched`, `arch::x86_64::syscall::enter_user`, `boot::file` (programele de la Limine).
- **Cum se testează:** scenariul `log` (`test: mod utilizator OK`, `test: ELF OK`: `hello` iese cu 0, fișiere ELF stricate
  sunt refuzate cu mesaj clar, cadrele libere revin exact).
- **Surse:** System V gABI (formatul ELF), AMD64 psABI. Cod propriu.
- **Oprire (P4-08):** `kill(id)` cere oprirea unui program: iese cu codul -9 la următoarea întoarcere în ring 3 (după un
  apel de sistem sau la întreruperea de ceas, deci și un program într-o buclă fără apeluri); un program care doarme
  sau așteaptă un eveniment e trezit. `list()` — programele care merg (id, nume); `running(id)`.
  Test: scenariul `log` (o buclă infinită `jmp $` e oprită), `modules`, `sysapps` (monitorul oprește `demo`).
- **Fire multiple (K6-04):** `thread_spawn` adaugă un fir în același spațiu de adrese (aceeași memorie, aceleași fișiere și
  ferestre); firul pornește în ring 3 la adresa dată, cu stiva dată, `rdi`/`rsi` = două argumente. Id-ul programului rămâne
  id-ul firului principal (`wait`, `kill`, lista de programe). `thread_join(id)` dă codul firului. Reguli:
  `exit` sau o excepție într-un fir oarecare **opresc tot programul** (cu acel cod); `thread_exit` oprește doar firul (pe firul
  principal = tot programul); la ieșirea programului toate firele sunt oprite și **așteptate** înainte să se elibereze memoria;
  `kill` pe program le oprește pe toate. `mem_unmap` merge doar când programul mai are un singur fir (`-23` altfel), ca niciun
  alt nucleu să nu mai folosească paginile. Cel mult 64 de fire în plus. Biblioteca `user/`: `cyber_user::thread` și heap-ul
  comun (încuietoare). Test: scenariile `fire` (4 nuclee) și `fire-1` (un nucleu), `/bin/fire`.

