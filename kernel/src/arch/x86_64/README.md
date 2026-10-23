# arch/x86_64 — ce ține de procesorul x86-64

- **Ce face:** `gdt.rs` (segmente, TSS pe fiecare nucleu), `idt.rs` (excepții), `apic.rs` (temporizatorul, I/O APIC, întreruperi
  între procesoare), `syscall.rs` (intrarea din programe), `fpu.rs` (SSE/AVX, K6-01), `smp.rs` (pornirea celorlalte nuclee, K6-02),
  `percpu.rs` (datele fiecărui nucleu, K6-02b).
- **`percpu.rs` (K6-02b):** fiecare nucleu are un bloc: indexul lui, stiva de nucleu pentru `syscall`, TSS-ul, id-ul APIC.
  Blocul se găsește prin MSR-ul `IA32_KERNEL_GS_BASE`. `syscall_entry` face `swapgs`, își ia stiva din bloc și face imediat la loc
  `swapgs`: nucleul nu folosește niciodată GS, deci un program care își schimbă GS nu poate păcăli nucleul.
- **Golirea TLB între nuclee (`apic::shootdown`):** când nucleul demapează o pagină din jumătatea lui (comună tuturor nucleelor),
  trimite o întrerupere (vectorul 0xF0) fiecărui nucleu care rulează fire și așteaptă să răspundă toate (cel mult 1 s, apoi un
  avertisment în jurnal). Paginile programelor nu au nevoie: un program are un singur fir, iar la comutare CR3 se reîncarcă.
  Când vor exista fire multiple într-un proces (K6-04), demapările din programe vor trebui și ele golite pe celelalte nuclee.
- **Depinde de:** `mem` (pagini, heap), `acpi::madt` (lista de procesoare), `sched` (firele).
- **Cum se testează:** scenariile `log`, `fpu`, `fpu-fx`, `smp-start`, `smp-2`, `cpu`, `log-smp`, `fpu-smp`, `stress-smp`.
- **Surse:** Intel SDM vol. 2 (SYSCALL, SWAPGS, RDMSR), vol. 3 cap. 2–10 (segmente, întreruperi, procesoare multiple, APIC);
  ACPI 6.5 (MADT). Cod propriu.
