# sched — fire de execuție ale nucleului și planificatorul

- **Ce face:** `mod.rs` ține firele (threads) nucleului: firul curent, coada celor gata (round-robin) și firele terminate,
  ale căror stive se eliberează de firul următor. `spawn(f)` creează un fir cu stivă proprie de 16 KiB,
  `yield_now()` cedează procesorul, `exit()` termină firul. Codul de pornire devine firul 0 („main”) în `init()`.
  `switch.rs` conține comutarea de context (salvează registrele rbx, rbp, r12–r15 și `rsp`) și pregătirea stivei unui fir nou.
- **Stadiu:** P2-01 — fire cooperative (comutare doar la `yield_now`). Comutarea automată pe temporizator vine la P2-02.
- **Depinde de:** heap (`mem::heap`) pentru stive și cozi.
- **Cum se testează:** scenariul `log` — `test: fire A1 B1 C1 A2 B2 C2 A3 B3 C3, stive eliberate`
  (trei fire care își cedează rândul; ordinea dovedește round-robin; heap-ul revine exact la valoarea de dinainte).
- **Surse:** System V AMD64 ABI (registrele păstrate de funcția chemată); OSDev Wiki „Kernel Multitasking” (doar ideea). Cod propriu.
