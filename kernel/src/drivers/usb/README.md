# usb — controlerul USB XHCI și dispozitivele USB

- **Ce face:** `xhci.rs` pornește controlerul XHCI (USB 2 și 3): îl preia de la firmware, îl resetează, pregătește
  inelul de comenzi, inelul de evenimente și tabelul de contexte (inclusiv paginile „scratchpad” cerute de controler).
  Pentru fiecare port cu ceva conectat: reset (la USB 2), `Enable Slot`, `Address Device`, apoi descriptorul de dispozitiv
  (corectând mărimea pachetelor la USB 1/2) și descriptorul de configurație (clasa primei interfețe).
  Totul cu așteptare activă (fără întreruperi încă). În shell: `lsusb`.
- `hid.rs` — tastaturi USB (P3-06) prin protocolul HID „boot”: `SET_CONFIGURATION`, `SET_PROTOCOL(boot)`, endpoint-ul
  de întreruperi (`Configure Endpoint`), rapoarte de 8 octeți → aceleași taste ca la PS/2 (aceeași coadă, `wait_key`).
  Un fir de sistem verifică la fiecare 8 ms transferurile terminate (fără întreruperi încă).
- **Urmează:** stick-ul USB ca disc (P3-07).
- **Depinde de:** `pci`, `mem` (cadre pentru DMA, `map_mmio`), `sched::sleep_ms`.
- **Cum se testează:** scenariul `usb-enum` — QEMU cu `-device qemu-xhci -device usb-kbd -device usb-storage`:
  jurnalul arată tastatura (0627:0001, HID) și stick-ul (46f4:0001, Mass Storage); `usb-kbd` — QEMU tastează pe tastatura
  USB și nucleul primește tastele; `shell` (`lsusb` fără dispozitive).
- **Surse:** USB HID 1.11 (anexa B) și HID Usage Tables; eXtensible Host Controller Interface for USB (xHCI) 1.2 (Intel, publică); USB 2.0 Specification, cap. 9. Cod propriu.
