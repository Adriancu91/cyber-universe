# Compatibilitate hardware

> Se completează după fiecare test pe un calculator real (sarcina P3-09). Ghidul pentru pornirea de pe stick: `docs/USB.md`.

## Ce verificăm pe fiecare calculator
| Verificare | Cum se vede |
|---|---|
| Pornește de pe stick | apare textul jurnalului, apoi desktopul (bara cu Start și ceasul); Esc → `cyber>` |
| Mouse | săgeata se mișcă; clic pe Start deschide meniul |
| Ecran | textul e clar, nu e tăiat |
| Tastatură internă | se poate scrie la `cyber>` |
| Tastatură USB externă | idem, cu tastatura USB |
| Disc intern (NVMe/SATA) | `ls /disk` (doar dacă discul are o partiție FAT32; altfel jurnalul arată `nvme: disc ...` / `ahci: disc ...`) |
| Stick USB | `ls /usb` arată `EFI` și `boot` |
| Placa de rețea | `net` arată placa și adresa MAC |
| Lista de dispozitive | `lspci`, `lsusb` (poză) |

## Calculatoare testate
| Calculator | Firmware | Data | Pornește | Tastatură | Disc | USB | Rețea | Observații |
|---|---|---|---|---|---|---|---|---|
| QEMU 8.2 (q35, OVMF) | OVMF | 2026-10-20 | da | da (PS/2 și USB) | da (SATA, NVMe) | da (stick, tastatură) | da (e1000, e1000e) | referința testelor automate |
| Lenovo ThinkPad (AMD Ryzen 4000 „Renoir”, ~8 GB RAM; nu e cel al suitei) | UEFI Lenovo, **Secure Boot oprit** | 2026-10-21 | **da** (până la desktop) | da (internă; USB externă —) | NVMe Samsung `144d:a809` văzut în `lspci`; `ls /disk` — | da (xHCI AMD `1022:1639`; stick ADATA `125f:db8a` văzut în `lsusb`; `ls /usb` —) | **nu**: Realtek `10ec:8168` fără driver (P6-01); Wi-Fi Intel AX200 `8086:2723` fără driver | ecran nativ clar, touchpad + clic merg, Demo și Editor merg; OS-ul ocupă ~21 MB; cu Secure Boot pornit revine la meniul de boot fără mesaj. Detalii: hub `rezultate/2026-10-21-test-stick/REZULTAT.md` |
| ThinkPad L15 Gen 2 (al suitei) | — | — | — | — | — | — | — | rezervat suitei office (decizia lui Adrian, 20 oct) |

## Detalii: Lenovo ThinkPad AMD (2026-10-21)
- Imaginea: artefactul CI `stick-usb` din `cf80c3e`, copiată ca **fișiere** pe un stick FAT32 (vezi `docs/USB.md`, varianta B).
- `lspci`: punte gazdă `1022:1630` (Renoir), punți `1022:1632/1634/1635`, SMBus `1022:790b`, ISA `1022:790e`, rețea Realtek
  `10ec:8168` (01:00.0), Wi-Fi Intel `8086:2723` (02:00.0), NVMe Samsung `144d:a809` (03:00.0), video AMD `1002:1636`, audio
  `1002:1637`, `1022:15e2`, `1022:15e3`, două controlere USB xHCI `1022:1639`.
- `lsusb`: cameră Chicony `04f2:b6be`, stick ADATA `125f:db8a` (Mass Storage).
- `mem`: liberă 7377 MiB, folosită 21672 KiB (heap 17738 KiB).
- **Testul 2 (2026-10-22 ~00:00, imaginea `4bd215d`, hub `rezultate/2026-10-22-test-2/`):** pornire ~15 s până la desktop;
  `hw`: AMD Ryzen 5 4500U; drivere legate: `[rtl8168]`, `[nvme]`, `[xhci]` ×2. **NVMe merge** (`/disk` = partiția EFI a discului
  Samsung: `EFI/`, `System Volume Information/`). Realtek: MAC real `38:f3:ab:6b:11:34`, **4 trimise, 0 primite** → DHCP fără răspuns
  (reparat în `fix/rtl-rx`: PHY pornit + așteptarea legăturii, starea ISR ștearsă, DHCP reîncearcă). `/usb: nemontat` pe un stick
  formatat în Windows → stick-uri fără tabelă și comenzi de „trezire” SCSI adăugate; `hw` arată acum și motivul.
- **Rețeaua de test (de la testul 7, 2026-10-22, decizia din hub 22:50):** cablu între laptopul de test și laptopul cu Windows, cu
  **partajarea conexiunii (ICS)** pornită în Windows (Wi-Fi → Ethernet). Windows dă adrese **192.168.137.x**, ruta și DNS
  **192.168.137.1**, și internet. Echivalent cu routerul (verificat: DHCP, DNS, HTTPS cu TLS 1.3). Testul 7: la cald, după
  `rtl porneste`, adresa 192.168.137.88 și `fetch https://example.com/` corect; la rece 0 / 0.
- Neverificat încă (testul 3): `ls /disk`, `ls /usb`, `net` după driverul Realtek, consola text (Esc), tastatură USB externă,
  timpul de pornire, comanda nouă `hw`.
