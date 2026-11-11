# arch/x86_64 — ce ține de procesorul x86-64

- **Ce face:** `gdt.rs` (segmente, TSS pe fiecare nucleu), `idt.rs` (excepții), `apic.rs` (temporizatorul, I/O APIC, întreruperi
  între procesoare), `syscall.rs` (intrarea din programe), `fpu.rs` (SSE/AVX, K6-01), `smp.rs` (pornirea celorlalte nuclee, K6-02),
  `percpu.rs` (datele fiecărui nucleu, K6-02b).
- **`percpu.rs` (K6-02b):** fiecare nucleu are un bloc: indexul lui, stiva de nucleu pentru `syscall`, TSS-ul, id-ul APIC.
  Blocul se găsește prin MSR-ul `IA32_KERNEL_GS_BASE`. `syscall_entry` face `swapgs`, își ia stiva din bloc și face imediat la loc
  `swapgs`: nucleul nu folosește niciodată GS, deci un program care își schimbă GS nu poate păcăli nucleul.
- **Golirea TLB între nuclee (`apic::shootdown`):** când nucleul demapează o pagină din jumătatea lui (comună tuturor nucleelor),
  trimite o întrerupere (vectorul 0xF0) fiecărui nucleu care rulează fire și așteaptă să răspundă toate (cel mult 1 s, apoi un
  avertisment în jurnal). Paginile programelor nu au nevoie: de la K6-04 un program poate avea mai multe fire, dar își
  eliberează memoria (`mem_unmap`) doar când îi mai rulează un singur fir (altfel `-23`), iar la ieșire întâi se termină toate
  firele lui; un fir terminat trece pe tabelele nucleului (CR3 reîncărcat = TLB golit pentru paginile programului).
- **Calibrarea ceasului și x2APIC (P6-79, comanda C-109):** pe laptopul ASUS TUF F15 pornirea stătea după `acpi: MADT`, adică în
  `apic::init`, singura buclă fără limită fiind așteptarea PIT (ceasul 8254 poate fi oprit de firmware). Acum orice așteptare are limită
  (după TSC, socotit pentru 10 GHz); calibrarea încearcă pe rând PIT → temporizatorul ACPI PM (`acpi::fadt::pm_timer`, 3,579545 MHz)
  → frecvența TSC din CPUID 0x15/0x16 și respinge un rezultat fără sens (TSC în afara 200 MHz–10 GHz); dacă niciunul nu merge, mesaj și
  pornire fără ceas, nu blocaj. Dacă firmware-ul a lăsat APIC-ul local în modul x2APIC (IA32_APIC_BASE bitul 10), registrele merg prin
  MSR-uri (0x800 + offset/16, ICR = MSR 0x830), iar procesoarele suplimentare trec și ele în x2APIC. Rândurile `apic: mod …`,
  `apic: calibrez ceasul …`, `apic: ceasul de referinta: …` arată pe ecran unde a ajuns pornirea.
  Modul x2APIC nu se poate încerca în QEMU 8.2 fără KVM (TCG nu are x2APIC): pe placă, neîncercat.
- **Depinde de:** `mem` (pagini, heap), `acpi::madt` (lista de procesoare), `sched` (firele).
- **Intel și AMD:** `sysret` pe AMD ia SS = STAR[63:48]+8 fără să-i pună RPL 3, deci baza din STAR are deja RPL 3 (0x13);
  altfel programele rulează cu SS=0x18 și prima întoarcere după o întrerupere cade cu #GP(0x18) (C-23, P6-24).
- **Cum se testează:** scenariile `log` (rândul „sysret”), `kronos-calcul-amd` (`-cpu EPYC-Rome`), `fpu`, `fpu-fx`, `smp-start`, `smp-2`, `cpu`, `log-smp`, `fpu-smp`, `stress-smp`,
  `fara-pit` (QEMU `-machine pit=off`: calibrare prin ACPI PM, P6-79).
- **Surse:** Intel SDM vol. 2 (SYSCALL, SWAPGS, RDMSR), vol. 3 cap. 2–10 (segmente, întreruperi, procesoare multiple, APIC);
  ACPI 6.5 (MADT; FADT `PM_TMR_BLK`); Intel SDM vol. 3A cap. 11.12 (x2APIC), vol. 2 CPUID 0x15/0x16; AMD APM vol. 3 (SYSRET). Cod propriu.
