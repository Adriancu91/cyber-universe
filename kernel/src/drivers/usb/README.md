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
  **Citire de 64 KiB (P6-29, C-29), variantă din shell:** `usb citire mare` = o comandă `READ(10)` aduce până la 64 KiB
  (un singur TRB Normal, buffer de 64 KiB aliniat la 64 KiB, xHCI 1.2 §4.11.7.1), `usb citire mica` = 4 KiB, ca înainte
  (implicit, D-013: devine implicită doar după ce merge pe hardware). Test: scenariul `usb-disk` verifică discul cu ambele.
- Evenimentele de transfer sunt păstrate de `Xhci` până le cere driverul lor (`take_transfer`/`wait_transfer`),
  așa că tastatura și stick-ul pot lucra în același timp.
- **Depinde de:** `pci`, `mem` (cadre pentru DMA, `map_mmio`), `sched::sleep_ms`.
- **Cum se testează:** scenariul `usb-enum` — QEMU cu `-device qemu-xhci -device usb-kbd -device usb-storage`:
  jurnalul arată tastatura (0627:0001, HID) și stick-ul (46f4:0001, Mass Storage); `usb-kbd` — QEMU tastează pe tastatura
  USB și nucleul primește tastele; `usb-disk` — discul de test ca stick USB → `/usb` citit corect; `shell` (`lsusb` fără dispozitive).
- **Surse:** USB Mass Storage Bulk-Only Transport 1.0, SCSI Primary/Block Commands (descrieri publice); USB HID 1.11 (anexa B) și HID Usage Tables; eXtensible Host Controller Interface for USB (xHCI) 1.2 (Intel, publică); USB 2.0 Specification, cap. 9. Cod propriu.

## Hubs and the boot stick (P6-14, P6-15)
- `xhci::explore_hub`: hub descriptor, Configure Endpoint with the Hub bit, SET_HUB_DEPTH (SuperSpeed), port power, reset,
  then Address Device with Route String / TT (USB 2.0 ch. 11, USB 3.2 ch. 10, xHCI 1.2 §4.6.6, §8.9). Test: `usb-hub`.
- `storage`: "Medium Not Present" (sense 2/3A) five times in a row = card reader without a card, skipped.
  `fs::mount_best` mounts at `/usb` the stick with `cyber.rw` and `VERSIUNE`. Test: `usb-pick`.
- **Mai multe controlere (P6-25):** toate controlerele xHCI de pe PCI pornesc (Tiger Lake are două: Thunderbolt și USB-A);
  dispozitivele de pe al doilea apar ca „c2/<port>”. Test: `usb-2xhci`.

## Plugged in after start-up; every HID boot interface (P6-80)
- `Xhci::port_changes` (called by the USB thread every ~250 ms) reads the "connect status change" bit of each root port:
  a new device gets an address (a hub: its devices too) and a keyboard/mouse starts; the devices of an emptied port lose
  their slot (`Disable Slot`, rings and context freed). The USB thread now runs whenever there is a controller.
  Not yet: devices plugged in later **behind a hub** (needs the hub's status endpoint).
- `UsbDevice::hid`: up to two HID boot interfaces from the whole configuration (a wireless keyboard + mouse receiver,
  or a mouse whose first interface is something else); `hid::Keyboard::start_all` starts all of them with one
  `SET_CONFIGURATION` and one `Configure Endpoint`.
- Test: scenario `usb-hotplug` — QEMU `device_add usb-mouse` after start-up, movement + click (counted as USB reports),
  `device_del` → slot freed. Sources: xHCI 1.2 §4.19.2, §5.4.8; USB 2.0 §7.1.7.3; HID 1.11 §7.2.6.
