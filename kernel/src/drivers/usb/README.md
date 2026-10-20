# usb — controlerul USB XHCI și dispozitivele USB

- **Ce face:** `xhci.rs` pornește controlerul XHCI (USB 2 și 3): îl preia de la firmware, îl resetează, pregătește
  inelul de comenzi, inelul de evenimente și tabelul de contexte (inclusiv paginile „scratchpad” cerute de controler).
  Pentru fiecare port cu ceva conectat: reset (la USB 2), `Enable Slot`, `Address Device`, apoi descriptorul de dispozitiv
  (corectând mărimea pachetelor la USB 1/2) și descriptorul de configurație (clasa primei interfețe).
  Totul cu așteptare activă (fără întreruperi încă). În shell: `lsusb`.
- **Urmează:** tastatura USB (P3-06) și stick-ul USB ca disc (P3-07), care folosesc inelele de transfer de aici.
- **Depinde de:** `pci`, `mem` (cadre pentru DMA, `map_mmio`), `sched::sleep_ms`.
- **Cum se testează:** scenariul `usb-enum` — QEMU cu `-device qemu-xhci -device usb-kbd -device usb-storage`:
  jurnalul arată tastatura (0627:0001, HID) și stick-ul (46f4:0001, Mass Storage); `shell` (`lsusb` fără dispozitive).
- **Surse:** eXtensible Host Controller Interface for USB (xHCI) 1.2 (Intel, publică); USB 2.0 Specification, cap. 9. Cod propriu.
