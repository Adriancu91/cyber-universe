# mem — gestionarea memoriei

**Ce face:**
- `map.rs` — copiază harta memoriei fizice și adresa HHDM (toată memoria fizică mapată de Limine) și le raportează în jurnal.
- `frame.rs` — alocatorul de cadre fizice (4 KiB): o hartă de biți, câte un bit pe cadru.
- `paging.rs` — paginare pe 4 niveluri: mapare, demapare, traducere adresă virtuală → fizică (inclusiv pagini mari); activează bitul NX și CR0.WP.
  La pornire copiază tabelele Limine în cadre proprii (`adopt_tables`), ca memoria bootloader-ului să poată fi recuperată.
- `address_space.rs` — spații de adrese pentru procese (P2-04): fiecare are PML4 propriu; jumătatea de sus (nucleul)
  e comună, jumătatea de jos e a procesului; la distrugere se eliberează toate cadrele și tabelele lui.
  Ca jumătatea de sus să rămână identică peste tot, `adopt_tables` creează din start toate cele 256 de tabele de nivel 3 ale nucleului (1 MiB).
- `heap.rs` — heap-ul nucleului (`Box`, `Vec`, `String`): listă de blocuri libere sortată, cu unire; crește la nevoie de la `0xffff_9000_0000_0000`, maxim 256 MiB.

**Depinde de:** cererile `MEMORY_MAP` și `HHDM` din `boot.rs`.

**Test:** scenariul `log` din `tests/scenarios.txt` verifică raportul de memorie și testul alocatorului
(10.000 de cadre alocate, verificate că sunt unice prin scriere/citire, apoi eliberate; numărul de cadre libere revine exact).

**Teste suplimentare:** `test: paginare OK`, `test: heap OK` (Vec de 1.000.000 de elemente, 10.000 de Box eliberate amestecat, fără scurgeri) și scenariul `unmapped` (acces la o pagină demapată → page fault).

**Recuperare (P1-13):** `frame::reclaim_bootloader` dă alocatorului memoria „bootloader recuperabilă” (~43 MiB în QEMU),
după ce nucleul are tabele și stivă proprii. Teste: `test: toate cele N cadre libere scrise si eliberate` (scenariul `log`)
și scenariul `rodata` (scriere în `.rodata` → page fault).

**Spații de adrese:** `test: spatii de adrese separate OK` — aceeași adresă în două spații arată date diferite,
firele pornite cu `sched::spawn_in` văd datele spațiului lor, iar după distrugere toate cadrele revin.

**Inspirație:** specificația protocolului Limine; OSDev Wiki „Page Frame Allocation” (doar descrierea).
- **Paza DMA (P6-10g):** `frame::pin(adresă, pagini, proprietar)` marchează paginile în care scrie o placă (rtl8168, e1000, ahci, nvme).
  `free_frame` refuză o astfel de pagină și scrie în jurnal cine a cerut-o („paza DMA ... REFUZATA”, cu fișier:linie), `reclaim_bootloader`
  o sare, `check_pins()` verifică că toate sunt încă ocupate. `alloc_contiguous_between(n, jos, sus)` dă un bloc într-un interval
  (driverul Realtek: între 16 MiB și 4 GB). Testat în QEMU de `test_dma_guard` și de `test_all_frames`.
