# proc — procese

- **Ce face:** un proces = un spațiu de adrese propriu (`mem::address_space`) + un fir care rulează programul în ring 3.
  `spawn_elf(fișier)` încarcă un executabil ELF64 static (`elf.rs`: verifică antetul și segmentele înainte de orice alocare,
  mapează fiecare segment cu drepturile lui — cod executabil, date fără execuție, `.bss` pus pe zero);
  `spawn_flat(cod)` încarcă cod mașină simplu (pentru teste). Stiva programului: 16 KiB sub `0x7fff_fff0_0000`.
  `wait(id)` așteaptă (dormind) terminarea și dă codul de ieșire; la ieșire toată memoria procesului se eliberează.
- **Urmează:** argumente și fișiere deschise (P2-08).
- **Depinde de:** `mem::address_space`, `sched`, `arch::x86_64::syscall::enter_user`, `boot::file` (programele de la Limine).
- **Cum se testează:** scenariul `log` (`test: mod utilizator OK`, `test: ELF OK`: `hello` iese cu 0, fișiere ELF stricate
  sunt refuzate cu mesaj clar, cadrele libere revin exact).
- **Surse:** System V gABI (formatul ELF), AMD64 psABI. Cod propriu.
