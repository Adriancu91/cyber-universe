# acpi — tabelele ACPI

**Ce face:** găsește tabelele ACPI pornind de la RSDP (primit de la Limine), le verifică sumele de control și citește
MADT: procesoarele, adresa APIC-ului local, controlerele I/O APIC și redirectările de IRQ.
Paginile firmware-ului care nu sunt în HHDM se mapează doar-citire, la nevoie.
`fadt.rs` citește la pornire din FADT doar registrul de repornire (`RESET_REG`, `RESET_VALUE`, bitul `RESET_REG_SUP`),
pe care îl folosește `power.rs` (comanda `reboot`: ACPI → portul 0xCF9 → 8042 → triple fault, câte un rând în jurnal).

**Depinde de:** cererea `RSDP` din `boot.rs`, `mem::paging` (pentru maparea tabelelor), heap (pentru lista de nume).

**Test:** scenariul `log` verifică lista de tabele și MADT (1 procesor); scenariul `smp` pornește QEMU cu `-smp 4`
și verifică „4 procesoare”. `reboot`, `reboot-cf9`, `reboot-8042`, `reboot-triplu`: QEMU rulează cu `-no-reboot`, deci
resetarea îl oprește (cod 0); decodarea FADT are și un test pe octeți cunoscuți.

**Procesoarele (K6-02):** lista din MADT e folosită de `arch/x86_64/smp.rs` (`cpu porneste`): INIT-SIPI-SIPI către fiecare
nucleu, trambulina sub 1 MiB, apoi nucleele stau parcate. Test: `smp-start` (`-smp 4`), `cpu` (`-smp 2`).

**Nu face încă:** interpretorul AML (alimentare, oprire, dispozitive) — sarcină mare, separată, în Faza 3.

**Inspirație:** ACPI Specification 6.5, cap. 5 (doar specificația); Intel PCH datasheet (registrul 0xCF9); Intel SDM vol. 3 (triple fault).
