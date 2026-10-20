# block — dispozitive de blocuri

- **Ce face:** interfața `BlockDevice` (citește și, unde se poate, scrie sectoare de 512 octeți) pe care o folosesc
  sistemele de fișiere. `write` are implicit răspunsul „doar citire” (`BlockError::ReadOnly`); AHCI, NVMe și USB îl implementează.
  `MemDisk` = o imagine de disc din memorie, doar citire (acum: `disk.img`, încărcat de Limine); `Partition` = o bucată dintr-un alt dispozitiv.
- **De ce:** driverul de disc real (AHCI/NVMe, Faza 3) va implementa aceeași interfață, fără schimbări în FAT32.
- **Depinde de:** nimic (doar `alloc`).
- **Cum se testează:** scenariul `fatwrite` (scriere prin AHCI pe o copie a discului de test); scenariul `log` (`test: FAT32 OK` citește totul prin această interfață, inclusiv printr-un dispozitiv de test care schimbă sectorul 0).
- **Surse:** design propriu.
