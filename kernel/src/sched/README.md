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
- **Registrele vectoriale (K6-01):** fiecare fir are o zonă `fpu::Area` (aliniată la 64 de octeți, mărimea din CPUID 0Dh);
  la fiecare comutare, `yield_now` face XSAVE în zona firului care pleacă și XRSTOR din zona celui care vine (FXSAVE/FXRSTOR
  pe procesoarele fără XSAVE). Nucleul e compilat fără SSE, deci registrele vectoriale sunt doar ale firelor și programelor.
  `arch/x86_64/fpu.rs` pornește unitatea (CR0, CR4, XCR0 = x87 + SSE + AVX, doar ce raportează CPUID). Test: scenariile `fpu`
  (`-cpu max`: XSAVE, AVX, un program ring 3 cu AVX2) și `fpu-fx` (procesor fără XSAVE: FXSAVE, SSE2); fără salvare testul pică.
- **Mai multe nuclee (K6-02b):** o singură coadă de fire gata, comună tuturor nucleelor (varianta simplă); fiecare nucleu are
  firul lui curent și firul lui idle (`Cpu` în `mod.rs`). Un nucleu pornit cu `cpu porneste` intră cu `ap_join`: codul lui de
  pornire devine firul idle, i se pornește temporizatorul, apoi ia fire din coadă. Încuietoarea planificatorului e **predată peste
  comutare**: firul care pleacă o ia, se pune în coadă, comută, iar firul care continuă (sau `thread_entry`, la un fir nou) o
  eliberează — așa niciun alt nucleu nu poate lua un fir ale cărui registre nu sunt încă salvate.
  **Trezire fără pierderi între nuclee:** o așteptare face `prepare_wait(cheie)` înainte să verifice condiția; un `wake` venit
  de pe alt nucleu între verificare și adormire găsește firul „în pregătire” (sau deja marcat blocat, dar încă pe procesor) și
  îl face gata, iar `block_current` nu mai adoarme. Fără asta, `Mutex` putea rămâne cu un fir adormit pentru totdeauna.
  **Încuietori cu întreruperile oprite** (`spin.rs`): planificatorul, heap-ul, alocatorul de cadre și paza DMA, tabelele de pagini,
  jurnalul, portul serial, cozile tastaturii și mouse-ului. Consola de pe ecran rămâne cu întreruperile pornite (desenul durează).
  În timp ce așteaptă o încuietoare, nucleul răspunde cererilor de golire a TLB (altfel două nuclee s-ar putea aștepta reciproc).
  Ceasul sistemului (`apic::ticks`) îl numără doar nucleul de pornire; fiecare nucleu are temporizatorul lui pentru preempțiune.
  Întreruperile dispozitivelor merg tot la nucleul de pornire. Pornirea obișnuită rămâne pe un singur nucleu: celelalte pornesc
  doar la cerere (`cpu porneste`) — aceasta e și „opțiunea un singur nucleu” din plan.
  **Test:** `smp-start` (`-smp 4`) și `smp-2` — fire pe toate nucleele, `mutex: 4 fire x 10000 = 40000` pe mai multe nuclee,
  2000 de predări ale unui jeton între două fire (o trezire pierdută ar bloca testul), programe `vectori` în paralel,
  50 de golirii de TLB; `log-smp` — tot testul `log` cu toate nucleele pornite chiar după ceas; `fpu-smp` — registrele vectoriale
  păstrate când firele trec de pe un nucleu pe altul; `stress-smp` — testul de stres pe 4 nuclee.
  `cpu_ticks(IDLE)` = firul idle al nucleului de pornire (baza monitorului de sistem).
- **Surse:** Intel SDM vol. 1 cap. 13 (XSAVE), vol. 3 §2.5–2.6 (CR0, CR4, XCR0), vol. 3 cap. 8–10 (procesoare multiple, APIC, IPI), §4.10.5 (golirea TLB pe mai multe procesoare); System V AMD64 ABI (registrele păstrate de funcția chemată); OSDev Wiki „Kernel Multitasking” (doar ideea). Cod propriu.
