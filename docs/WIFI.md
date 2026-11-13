# Wi-Fi în Cyber Universe (comanda C-118)

Ținta: laptopul ASUS TUF F15 are placa Wi-Fi **Intel AX201** (`8086:43F0`, la `00:14.3`). Ea e „CNVi”: jumătate din placă
(partea MAC) e în chipset-ul laptopului, cealaltă jumătate (radioul) e modulul mic de pe M.2. Adrian a spus „da” pe 13 nov 01:05
și a cerut ca **la fiecare test pe laptop să se testeze și Wi-Fi**. Cablul de rețea rămâne soluția de acum; Wi-Fi vine în pași mici.

## Pașii

| Pas | Ce face | Scrie în placă? | Stare |
|---|---|---|---|
| **W0** `wifi identifica` | găsește placa, citește configurația PCI și 5 registre de identitate (dacă placa e trează) | **nu** | gata în QEMU (P6-88), netestat pe laptop |
| W1 pornirea plăcii | reset, încărcarea firmware-ului de pe stick, mesajul „ALIVE” | da, după „DA” | după poza W0 |
| W2 scanare pasivă | lista rețelelor din jur (în hub doar numărul lor și canalele, **niciun nume de rețea**) | da, după „DA” | — |
| W3 conectare | întâi o rețea deschisă de test, apoi WPA2 (parola doar într-un fișier pe stick) | da, după „DA” | — |
| W4 internet | DHCP, `trimite`, `hub asculta` prin Wi-Fi | da | — |

## Ce face Adrian la fiecare test pe laptop
1. `wifi identifica` → poză la ecran (sau `test tot`, care are rândul „wifi”; `analiza` pune placa în inventar).
2. `trimite` dacă există rețea (altfel raportul rămâne pe stick).

Dacă rândul spune „registrele CSR nu s-au citit: Memory Space oprit de firmware”, e normal: W0 nu pornește nimic. Ca la placa
video, pornirea lui va fi un pas separat, cu „DA”.

## Reguli legale (clean-room)
- Firmware-ul Intel (`.ucode` din linux-firmware, `Netwfw*.dat` din Windows) **nu se desface și nu se analizează**; îl descarcă
  Adrian, cu „da” scris la momentul respectiv, și stă doar pe stick (SHA-256 în documentație, fișierul niciodată în repo/hub).
- Driverul Windows nu se analizează.
- Permis: iwlwifi (GPL-2.0 OR BSD-3) citit **ca specificație** (offset-uri, câmpuri, structuri) — codul îl scriem noi; standardele
  IEEE 802.11, RFC-urile WPA2; comportamentul observat pe placa lui Adrian.
