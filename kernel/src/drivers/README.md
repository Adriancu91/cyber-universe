# drivers — drivere din nucleu

Aici stau doar driverele fără de care nucleul nu pornește sau nu poate fi depanat. Restul vor fi module (Faza 4).

- `serial.rs` — port serial COM1 (16550): canalul principal de jurnal și de test. Detectează dacă portul există.
- `keyboard.rs` — tastatură PS/2 prin controlerul 8042, IRQ 1 rutat prin I/O APIC; aranjament US
  (cel românesc vine ca sarcină separată). Pe laptopuri moderne tastatura internă poate să nu fie PS/2:
  pe hardware real folosește la început o tastatură USB (driver XHCI în Faza 3).

- `ahci.rs` — discuri SATA prin controlerul AHCI (P3-02), doar citire, o comandă odată (cu așteptare activă):
  preia controlerul de la firmware, pornește primul port cu disc, `IDENTIFY` (model, mărime), `READ DMA EXT` în bucăți de 4 KiB.
  Implementează `block::BlockDevice`; discul dă `/disk` (prima partiție FAT32).
- `nvme.rs` — SSD-uri NVMe (P3-03), doar citire: resetare, coada de administrare, `Identify` (model, mărime),
  o pereche de cozi I/O, comanda `Read` în bucăți de 4 KiB, cu așteptare activă. La pornire NVMe are prioritate față de SATA pentru `/disk`.

**Test:** scenariul `nvme` (discul de test ca NVMe → `/disk`, `test: disc OK`); scenariul `ahci` (discul de test atașat de QEMU ca SATA → `/disk` citit prin AHCI, `test: disc OK`); scenariul `keyboard` trimite tastele h, i, Enter prin monitorul QEMU și verifică ce a primit nucleul.

**Surse NVMe:** NVM Express Base Specification — doar documentație.

**Surse AHCI:** Serial ATA AHCI 1.3.1 Specification (Intel), ATA Command Set — doar documentație.

**Inspirație:** OSDev Wiki „Serial Ports”, „8042 PS/2 Controller”, „PS/2 Keyboard” (doar descrierea).
