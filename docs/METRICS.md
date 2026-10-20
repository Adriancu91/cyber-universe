# Măsurători

> Se completează la sfârșitul fiecărei faze cu `tools/metrics.sh` (5 porniri în QEMU, media).
> QEMU: mașină q35, 256 MiB RAM, firmware OVMF (UEFI). Nucleul normal (nu cel de test).

**Ce înseamnă coloanele**
- **Pornire totală** — de la lansarea QEMU până când nucleul e gata (firmware UEFI + Limine + nucleu). În cloud
  QEMU rulează fără accelerare (TCG), deci pe un calculator real cifra va fi mult mai mică.
- **Pornire nucleu** — doar partea noastră: de la intrarea în nucleu până la „gata” (măsurat cu ceasul procesorului).
  Include ~10 ms de calibrare a temporizatorului.
- **Memorie nucleu** — imaginea nucleului în memorie + toate cadrele de memorie luate (heap, tabele de pagini, harta cadrelor).
- **Fișier nucleu** — mărimea fișierului `kernel` din ISO (include simboluri de depanare).

| Data | Versiune | Emulare | Pornire totală | Pornire nucleu | Memorie nucleu | Fișier nucleu |
|---|---|---|---|---|---|---|
| 2026-10-20 | 0.1.0 (e8faab3), sfârșitul Fazei 1 | TCG (fără accelerare) | 3563 ms | 39.6 ms | 1124 KiB | 1126 KiB |
| 2026-10-20 | 0.1.0 (59a4e7a), sfârșitul Fazei 2 | TCG (fără accelerare) | 3978 ms | 57.2 ms | 2412 KiB | 2072 KiB |

Faza 2 față de Faza 1: nucleul face mai mult la pornire (planificator, sistem de fișiere din initrd), iar memoria crește
mai ales din cele 256 de tabele de pagini prealocate pentru spațiile de adrese (1 MiB, vezi P2-04). Din pornirea totală,
~3,9 s sunt firmware-ul UEFI și Limine în emulare; discul de test (40 MiB) NU e în build-ul normal, pentru că doar
încărcarea lui ar adăuga ~13 s în emulare (va fi citit de pe disc de driverul AHCI, P3-02).

## Stabilitate
| Data | Test | Rezultat |
|---|---|---|
| 2026-10-20 | `tools/test.sh --stress` — 5 minute, alocări + paginare continue, ceas 100 Hz | trecut: 2.231.660 runde, 30.000 întreruperi de ceas, 0 cadre și 0 octeți de heap pierduți |
| 2026-10-20 | `tools/test.sh --stress` (Faza 2) — în plus, un proces nou pornit și oprit în fiecare secundă, preempțiune activă | trecut: 1.894.934 runde, 300 de procese, 0 cadre și 0 octeți de heap pierduți |
