# acpi — tabelele ACPI

**Ce face:** găsește tabelele ACPI pornind de la RSDP (primit de la Limine), le verifică sumele de control și citește
MADT: procesoarele, adresa APIC-ului local, controlerele I/O APIC și redirectările de IRQ.
Paginile firmware-ului care nu sunt în HHDM se mapează doar-citire, la nevoie.

**Depinde de:** cererea `RSDP` din `boot.rs`, `mem::paging` (pentru maparea tabelelor), heap (pentru lista de nume).

**Test:** scenariul `log` verifică lista de tabele și MADT (1 procesor); scenariul `smp` pornește QEMU cu `-smp 4`
și verifică „4 procesoare”.

**Nu face încă:** interpretorul AML (alimentare, oprire, dispozitive) — sarcină mare, separată, în Faza 3.

**Inspirație:** ACPI Specification 6.5, cap. 5 (doar specificația).
