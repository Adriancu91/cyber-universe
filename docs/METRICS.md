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
| 2026-10-20 | 0.1.0 (a7ecb26), sfârșitul Fazei 4 | TCG (fără accelerare) | 4862 ms | 114.3 ms | 2680 KiB | 2973 KiB |
| 2026-10-21 | 0.1.0 (c8fac5a), sfârșitul Fazei 5 | TCG (fără accelerare) | 4846 ms | 95.0 ms | 2768 KiB | 3597 KiB |

Faza 2 față de Faza 1: nucleul face mai mult la pornire (planificator, sistem de fișiere din initrd), iar memoria crește
mai ales din cele 256 de tabele de pagini prealocate pentru spațiile de adrese (1 MiB, vezi P2-04). Din pornirea totală,
~3,9 s sunt firmware-ul UEFI și Limine în emulare; discul de test (40 MiB) NU e în build-ul normal, pentru că doar
încărcarea lui ar adăuga ~13 s în emulare (va fi citit de pe disc de driverul AHCI, P3-02).

Faza 4 față de Faza 2: pornirea nucleului include acum drivere (PCI, AHCI, NVMe, USB/XHCI cu așteptarea legăturilor USB 3,
e1000) și citirea ceasului; memoria nucleului crește puțin (2,6 MiB). Interfața grafică (server + desktop) nu pornește la boot;
cu desktopul pornit memoria totală e ~10 MiB (secțiunea „Desktop” de mai jos).

Faza 5 față de Faza 4: pornirea și memoria aproape la fel (rețeaua, DHCP și HTTPS nu adaugă mult la pornire); fișierul nucleului
crește cu ~0,6 MiB (stiva TCP/IP, criptografia, clientul TLS). Pe hardware real (Lenovo AMD, 21 oct): OS-ul ocupă ~21 MB cu
desktopul pornit; timpul de pornire pe laptop îl măsoară Adrian la testul 2.

## Stabilitate
| Data | Test | Rezultat |
|---|---|---|
| 2026-10-20 | `tools/test.sh --stress` — 5 minute, alocări + paginare continue, ceas 100 Hz | trecut: 2.231.660 runde, 30.000 întreruperi de ceas, 0 cadre și 0 octeți de heap pierduți |
| 2026-10-20 | `tools/test.sh --stress` (Faza 2) — în plus, un proces nou pornit și oprit în fiecare secundă, preempțiune activă | trecut: 1.894.934 runde, 300 de procese, 0 cadre și 0 octeți de heap pierduți |

## Desktop (P4-06)
| Data | Ce | Rezultat |
|---|---|---|
| 2026-10-20 | memoria totală folosită cu desktopul pornit (scenariul `desktop`, ecran 1280×800, QEMU) | **10 MiB** (țintă: sub 32 MiB). Heap ~8 MiB, din care ~4 MiB imaginea ecranului din serverul grafic și ~4 MiB imaginea ferestrei desktopului; programul `desktop` are ~26 KiB pe disc și ~700 KiB în memorie (imaginile barei și ale meniului). |

Unde se poate câștiga mai târziu, dacă e nevoie: fereastra desktopului nu are nevoie de o imagine proprie cât tot ecranul
(fundalul e o singură culoare) — ar economisi ~4 MiB la 1280×800.
