# Compatibilitate hardware

> Se completează după fiecare test pe un calculator real (sarcina P3-09). Ghidul pentru pornirea de pe stick: `docs/USB.md`.

## Ce verificăm pe fiecare calculator
| Verificare | Cum se vede |
|---|---|
| Pornește de pe stick | apare textul jurnalului, apoi `Cyber Universe OS` și `cyber>` |
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
| ThinkPad L15 Gen 2 | — | — | — | — | — | — | — | de testat de Adrian |
| al doilea calculator | — | — | — | — | — | — | — | de testat de Adrian |
