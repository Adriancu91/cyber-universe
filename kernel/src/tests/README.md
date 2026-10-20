# tests — teste lungi ale nucleului

- **Ce face:** `stress.rs` (P1-11) — testul de stabilitate: alocă/eliberează memorie din heap și
  mapează/demapează pagini în buclă, cu temporizatorul pornit. La fiecare 10 s scrie un rezumat;
  la final verifică faptul că nu s-a pierdut niciun cadru de memorie și niciun octet din heap, apoi scrie `STRESS OK`.
- **Depinde de:** `mem` (cadre, paginare, heap), `arch::x86_64::apic` (ceasul de 100 Hz).
- **Cum se testează:**
  - `tools/test.sh --stress` — varianta completă, 5 minute (rulată și de `.github/workflows/stress.yml`, zilnic).
  - scenariul `stress-quick` din `tools/test.sh` — aceeași buclă, 15 secunde, la fiecare push.
- **Surse:** design propriu.
