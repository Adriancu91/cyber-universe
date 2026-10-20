# sched — fire de execuție ale nucleului și planificatorul

- **Ce face:** `mod.rs` ține firele (threads) nucleului: firul curent, coada celor gata (round-robin) și firele terminate,
  ale căror stive se eliberează de firul următor. `spawn(f)` creează un fir cu stivă proprie de 16 KiB,
  `yield_now()` cedează procesorul, `exit()` termină firul. Codul de pornire devine firul 0 („main”) în `init()`.
  `switch.rs` conține comutarea de context (salvează registrele rbx, rbp, r12–r15 și `rsp`) și pregătirea stivei unui fir nou.
- **Preempțiune (P2-02):** la fiecare tick al temporizatorului (10 ms) handler-ul întreruperii cheamă `timer_tick()`,
  care trece la următorul fir gata. `preempt_disable()` / `preempt_enable()` opresc temporar comutarea automată.
  Încuietorile nucleului (heap, cadre) nu opresc întreruperile: un fir care așteaptă o încuietoare ținută de alt fir
  e comutat la tick-ul următor, deci nu apare blocaj; doar planificatorul își ține încuietoarea cu întreruperile oprite.
- **Așteptare (P2-03):** `WaitQueue` (`wait_until(condiție)`, `wake_one`, `wake_all` — se poate chema și din întreruperi),
  `sleep_ms(ms)`, `Mutex<T>` (firul care găsește încuietoarea luată doarme). Firul „idle” face `hlt` când toate celelalte dorm.
  Tastatura trezește firele care așteaptă în `keyboard::wait_key()`.
- **Oprire și timp de procesor (P4-08, P4-09):** `kill(id)` marchează un fir („oprește-te”) și îl trezește dacă doarme;
  așteptările cu limită (`wait_until_or`, `sleep_ms`) renunță pentru un fir marcat. `cpu_ticks(id)` — câte tick-uri a
  rulat firul (și firul idle: timpul liber al procesorului, din care monitorul calculează gradul de ocupare).
- **Testul de preempțiune (corectat la P5-03):** judecă împărțirea după timpul numărat de planificator (tick-uri), nu
  după câte iterații a făcut fiecare fir: emulatorul oprește uneori mașina pentru o clipă, iar firul care rula atunci
  face mai puține iterații fără vina planificatorului (testul pica 7:1 în `sysapps`/`fatwrite` după P5-02). O „felie
  minimă de timp” adăugată la P5-02 pentru asta a fost scoasă: nu era cauza (o simulare arată că rotația simplă e corectă).
- **Ieșirea unui proces:** firul trece întâi definitiv pe tabelele nucleului (`use_kernel_tables`), apoi spațiul de adrese
  e distrus — o comutare la mijloc nu mai poate încărca tabele eliberate.
- **Fire de sistem:** `spawn_system(f)` — fire care merg cât nucleul (ex. cel care citește tastaturile USB); nu sunt numărate de `alive()`.
- **Spații de adrese (P2-04):** fiecare fir are tabelele lui de pagini (`spawn_in(root, f)`); la comutare se încarcă `CR3` doar dacă diferă.
- **Depinde de:** heap (`mem::heap`) pentru stive și cozi.
- **Cum se testează:** scenariul `log` — `test: fire A1 B1 C1 A2 B2 C2 A3 B3 C3, stive eliberate`
  (trei fire care își cedează rândul; ordinea dovedește round-robin; heap-ul revine exact la valoarea de dinainte)
  și `test: preemptiune OK` (două fire care nu cedează niciodată procesorul primesc amândouă timp, în proporții apropiate),
  `test: sleep_ms(500) = 50 ticks; mutex: 4 fire x 10000 = 40000` și scenariul `keyboard` (care așteaptă tastele dormind).
- **Surse:** System V AMD64 ABI (registrele păstrate de funcția chemată); OSDev Wiki „Kernel Multitasking” (doar ideea). Cod propriu.
