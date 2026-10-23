# drivers — drivere din nucleu

Aici stau doar driverele fără de care nucleul nu pornește sau nu poate fi depanat. Restul vor fi module (Faza 4).

- `serial.rs` — port serial COM1 (16550): canalul principal de jurnal și de test. Detectează dacă portul există.
- `keyboard.rs` — tastatură PS/2 prin controlerul 8042, IRQ 1 rutat prin I/O APIC; aranjament US
  (cel românesc vine ca sarcină separată). Recunoaște și săgețile, Home, End, Delete (coduri E0) și Ctrl+literă
  (`Key::Ctrl`), la fel și tastatura USB (P4-10, testate în scenariul `editor` cu taste trimise de QEMU). Pe laptopuri moderne tastatura internă poate să nu fie PS/2:
  pe hardware real folosește la început o tastatură USB (driver XHCI în Faza 3).

- **Aranjamentul românesc (P4-12):** „Română (standard)” (SR 13392:2004, varianta principală), la fel ca în Windows:
  ă î â ș ț pe tastele US `[ ] \ ; '`, „ ” pe tasta din stânga lui 1, AltGr (Alt dreapta) dă `[ ] \ ; ' €` etc.;
  accentele „moarte” (AltGr + cifre) nu sunt suportate. Pentru PS/2 și USB (`keyboard::romanian`). Se alege cu
  `tastatura ro|us` în shell sau din aplicația Setări (aplicat la pornire din `/disk/config/settings.toml`).
  Sursa: datele aranjamentului KBDROST publicate de Microsoft (fișierul KLC, prin kbdlayout.info) — doar date publice.
  Test: scenariile `keyboard-ro`, `keyboard-ro-usb` (taste trimise de QEMU), `layout-boot` (setarea salvată).
- `mouse.rs` — mouse PS/2 pe portul 2 al controlerului 8042 (IRQ 12), pachete de 3 octeți → `input::MouseEvent` (P4-01).
  Pornirea se face cu întreruperile oprite (altfel răspunsurile controlerului ar fi luate de handler-ul tastaturii).
- `rtc.rs` — ceasul calculatorului (CMOS, compatibil MC146818, P4-06): data și ora, citite de două ori până coincid
  (o actualizare la mijloc nu le poate amesteca), BCD sau binar, 12 sau 24 de ore. La pornire: `rtc: AAAA-LL-ZZ hh:mm:ss`
  în jurnal (scenariul `log`); programele o citesc cu apelul `time` (ceasul desktopului). Ora e cea ținută de calculator
  (de obicei ora locală sub Windows, UTC sub Linux; în QEMU: UTC).
- **Scriere pe disc (P4-07):** AHCI (`WRITE DMA EXT`), NVMe (`Write`) și stick-uri USB (`WRITE(10)`) pot scrie; sistemul de
  fișiere scrie doar pe partițiile marcate (vezi `fs/README.md`). Încă nu se trimite comanda de golire a memoriei cache a
  discului (FLUSH) — de adăugat înainte de scrieri pe discuri reale importante.
- `ahci.rs` — discuri SATA prin controlerul AHCI (P3-02), citire și scriere, o comandă odată (cu așteptare activă):
  preia controlerul de la firmware, pornește primul port cu disc, `IDENTIFY` (model, mărime), `READ DMA EXT` în bucăți de 4 KiB.
  Implementează `block::BlockDevice`; discul dă `/disk` (prima partiție FAT32).
- `nvme.rs` — SSD-uri NVMe (P3-03), doar citire: resetare, coada de administrare, `Identify` (model, mărime),
  o pereche de cozi I/O, comanda `Read` în bucăți de 4 KiB, cu așteptare activă. La pornire NVMe are prioritate față de SATA pentru `/disk`.

**Test:** scenariul `nvme` (discul de test ca NVMe → `/disk`, `test: disc OK`); scenariul `ahci` (discul de test atașat de QEMU ca SATA → `/disk` citit prin AHCI, `test: disc OK`); scenariul `keyboard` trimite tastele h, i, Enter prin monitorul QEMU și verifică ce a primit nucleul.

**Surse NVMe:** NVM Express Base Specification — doar documentație.

**Surse AHCI:** Serial ATA AHCI 1.3.1 Specification (Intel), ATA Command Set — doar documentație.

**Inspirație:** OSDev Wiki „Serial Ports”, „8042 PS/2 Controller”, „PS/2 Keyboard” (doar descrierea).

## Rețea

- `nic.rs` — interfața comună a plăcilor de rețea (`Card`: trimite/primește cadre, MAC, nume, contoare) și placa găsită la
  pornire (`NIC`): întâi Intel e1000/e1000e, apoi Realtek. Stiva `net/`, apelurile de sistem și shell-ul folosesc doar interfața.
- `rtl8168.rs` — Realtek RTL8168/8111 (P6-01): registrele mapate, pagini DMA, întârzieri; logica e în crate-ul `rtl8168/`
  (testat pe gazdă față de un model al cipului). Pornește placa din starea de economie D3 dacă firmware-ul a lăsat-o așa.
- `e1000.rs` — plăci de rețea Intel e1000 (82540EM, 82545EM) și e1000e (82574L) (P3-08): reset, adresa MAC, inelele de
  descriptori pentru primire și trimitere (32 × 2 KiB), cu așteptare activă. `send(cadru)`, `receive()`; comanda `net` din shell.
  Placa Intel I219 din ThinkPad (tot familia e1000e) are diferențe (PHY, registre) — se tratează după testul pe calculatorul real.
- **Test:** scenariile `net` (e1000e, implicită la q35) și `net-e1000`: o cerere ARP către poarta virtuală QEMU (10.0.2.2)
  primește răspuns. **Surse:** Intel 8254x Software Developer's Manual, Intel 82574 datasheet, RFC 826 (ARP).

## Plăci video (nivel 0, doar citire)

- `gpu.rs` — comanda `gpu` (P6-11, comanda C-12): plăcile de clasă 03 din PCI (furnizor, model, subsistem, Command, starea
  de alimentare, BAR-uri citite din configurație — fără dimensionare, Resizable BAR doar citit) și, **doar la NVIDIA**, două
  registre publice de identitate: `NV_PMC_BOOT_0` (BAR0+0x0) și `NV_PMC_BOOT_42` (BAR0+0xA00) → cipul (de ex. `0x174` = GA104).
  **Zero scrieri:** nicio scriere în configurația PCI (nu pornim „Memory Space”, nu schimbăm starea D), iar pagina de registre se
  mapează la `0xFFFF_C100_0000_0000` **fără drept de scriere** și fără cache, apoi se demapează. `gpu plan` = pașii ferestrei de pe workstation.
- **Test:** scenariul `gpu` (VGA-ul QEMU → „nicio placa NVIDIA”, `gpu plan`, `log gpu`) + `tools/after-gpu.sh` (sursa nu conține
  nicio funcție de scriere); scenariul `gpu-ro` (o scriere prin fereastra de registre dă page fault); `test_gpu_decode` (decodarea
  BOOT_0/BOOT_42 pe valori cunoscute). Calea NVIDIA nu se poate rula în QEMU: se verifică doar pe workstation.
- **Surse:** specificațiile PCI / PCI Express (antet, BAR-uri, capabilitatea extinsă 0x15); documentația publică NVIDIA
  (open-gpu-doc) și antetele MIT `dev_boot.h` din open-gpu-kernel-modules, citite doar pentru adresele și câmpurile celor două
  registre. Niciun cod copiat. Patch-ul proiectului Cyber-GPU nu a putut fi citit (repo privat) — comanda e scrisă de la zero.
