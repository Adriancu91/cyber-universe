# pci — dispozitivele PCI / PCI Express

- **Ce face:** la pornire găsește toate dispozitivele (discuri, USB, rețea, video) prin spațiul de configurare mapat în
  memorie (ECAM), a cărui adresă vine din tabela ACPI `MCFG`; urmează punțile PCI-PCI de la magistrala 0.
  Pentru drivere: `find(clasă, subclasă, interfață)`, `Device::bar(i)` (adresa registrelor, inclusiv BAR-uri pe 64 de biți),
  `Device::enable()` (acces la memorie + DMA). În shell: comanda `lspci`.
- **Depinde de:** `acpi` (MCFG), `mem::paging::map_mmio`.
- **Cum se testează:** scenariul `log` (pe mașina q35 apar puntea gazdă 8086:29c0 și controlerul AHCI 8086:2922),
  scenariul `pci-usb` (cu `-device qemu-xhci` apare controlerul USB, clasa 0c:03:30), `shell` (`lspci`).
- **Limitări:** doar segmentul PCI 0 (toate PC-urile vizate); fără metoda veche prin porturile 0xCF8/0xCFC.
- **Surse:** PCI Local Bus Specification și PCI Express Base Specification (antetul de configurare, ECAM),
  specificația ACPI (MCFG); OSDev Wiki „PCI” (doar ideea). Cod propriu.
