# usb — controlerul USB XHCI și dispozitivele USB

- **Ce face:** `xhci.rs` pornește controlerul XHCI (USB 2 și 3): îl preia de la firmware, îl resetează, pregătește
  inelul de comenzi, inelul de evenimente și tabelul de contexte (inclusiv paginile „scratchpad” cerute de controler).
  Pentru fiecare port cu ceva conectat: reset (la USB 2), `Enable Slot`, `Address Device`, apoi descriptorul de dispozitiv
  (corectând mărimea pachetelor la USB 1/2) și descriptorul de configurație (clasa primei interfețe).
  Totul cu așteptare activă (fără întreruperi încă). În shell: `lsusb`.
- `hid.rs` — tastaturi USB (P3-06) și mouse-uri USB (P4-01, rapoarte de 3 octeți → `input::MouseEvent`) prin protocolul HID „boot”: `SET_CONFIGURATION`, `SET_PROTOCOL(boot)`, endpoint-ul
  de întreruperi (`Configure Endpoint`), rapoarte de 8 octeți → aceleași taste ca la PS/2 (aceeași coadă, `wait_key`).
  Un fir de sistem verifică la fiecare 8 ms transferurile terminate (fără întreruperi încă).
- `storage.rs` — stick-uri USB (P3-07): clasa Mass Storage, Bulk-Only Transport, comenzi SCSI `INQUIRY`,
  `READ CAPACITY(10)`, `READ(10)`; implementează `block::BlockDevice`, montat la `/usb` (prima partiție FAT32).
- Evenimentele de transfer sunt păstrate de `Xhci` până le cere driverul lor (`take_transfer`/`wait_transfer`),
  așa că tastatura și stick-ul pot lucra în același timp.
- **Depinde de:** `pci`, `mem` (cadre pentru DMA, `map_mmio`), `sched::sleep_ms`.
- **Cum se testează:** scenariul `usb-enum` — QEMU cu `-device qemu-xhci -device usb-kbd -device usb-storage`:
  jurnalul arată tastatura (0627:0001, HID) și stick-ul (46f4:0001, Mass Storage); `usb-kbd` — QEMU tastează pe tastatura
  USB și nucleul primește tastele; `usb-disk` — discul de test ca stick USB → `/usb` citit corect; `shell` (`lsusb` fără dispozitive).
- **Surse:** USB Mass Storage Bulk-Only Transport 1.0, SCSI Primary/Block Commands (descrieri publice); USB HID 1.11 (anexa B) și HID Usage Tables; eXtensible Host Controller Interface for USB (xHCI) 1.2 (Intel, publică); USB 2.0 Specification, cap. 9. Cod propriu.
