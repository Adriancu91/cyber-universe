# mem — gestionarea memoriei

**Ce face:**
- `map.rs` — copiază harta memoriei fizice și adresa HHDM (toată memoria fizică mapată de Limine) și le raportează în jurnal.
- `frame.rs` — alocatorul de cadre fizice (4 KiB): o hartă de biți, câte un bit pe cadru.
- `paging.rs` — paginare pe 4 niveluri: mapare, demapare, traducere adresă virtuală → fizică (inclusiv pagini mari); activează bitul NX.
- `heap.rs` — heap-ul nucleului (`Box`, `Vec`, `String`): listă de blocuri libere sortată, cu unire; crește la nevoie de la `0xffff_9000_0000_0000`, maxim 256 MiB.

**Depinde de:** cererile `MEMORY_MAP` și `HHDM` din `boot.rs`.

**Test:** scenariul `log` din `tests/scenarios.txt` verifică raportul de memorie și testul alocatorului
(10.000 de cadre alocate, verificate că sunt unice prin scriere/citire, apoi eliberate; numărul de cadre libere revine exact).

**Teste suplimentare:** `test: paginare OK`, `test: heap OK` (Vec de 1.000.000 de elemente, 10.000 de Box eliberate amestecat, fără scurgeri) și scenariul `unmapped` (acces la o pagină demapată → page fault).

**Urmează:** tabele de pagini complet proprii (în locul celor de la Limine) și recuperarea memoriei „bootloader recuperabile” — sarcină de adăugat înainte de Faza 2.

**Inspirație:** specificația protocolului Limine; OSDev Wiki „Page Frame Allocation” (doar descrierea).
