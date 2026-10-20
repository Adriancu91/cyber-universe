# mem — gestionarea memoriei

**Ce face:**
- `map.rs` — copiază harta memoriei fizice și adresa HHDM (toată memoria fizică mapată de Limine) și le raportează în jurnal.
- `frame.rs` — alocatorul de cadre fizice (4 KiB): o hartă de biți, câte un bit pe cadru.

**Depinde de:** cererile `MEMORY_MAP` și `HHDM` din `boot.rs`.

**Test:** scenariul `log` din `tests/scenarios.txt` verifică raportul de memorie și testul alocatorului
(10.000 de cadre alocate, verificate că sunt unice prin scriere/citire, apoi eliberate; numărul de cadre libere revine exact).

**Urmează:** paginare proprie (P1-06), heap (P1-07). Memoria „bootloader recuperabilă” se poate elibera după P1-06.

**Inspirație:** specificația protocolului Limine; OSDev Wiki „Page Frame Allocation” (doar descrierea).
