# block — dispozitive de blocuri

- **Ce face:** interfața `BlockDevice` (citește și, unde se poate, scrie sectoare de 512 octeți) pe care o folosesc
  sistemele de fișiere. `write` are implicit răspunsul „doar citire” (`BlockError::ReadOnly`); AHCI, NVMe și USB îl implementează.
  `MemDisk` = o imagine de disc din memorie, doar citire (acum: `disk.img`, încărcat de Limine); `Partition` = o bucată dintr-un alt dispozitiv.
  P6-100: `Shared` = același disc folosit de mai multe partiții (OS-ul se uită la toate partițiile FAT ca s-o găsească pe cea
  cu `cyber.rw`); inventarul discurilor (`note`, `inventory`): ce a văzut OS-ul la pornire, doar din citiri — comanda `discuri`
  și raportul ciclului de test pe placă.
- **De ce:** driverul de disc real (AHCI/NVMe, Faza 3) va implementa aceeași interfață, fără schimbări în FAT32.
- **Depinde de:** nimic (doar `alloc`).
- **Cum se testează:** scenariul `disc-propriu` (inventarul, partiția cu `cyber.rw` aleasă); scenariul `fatwrite` (scriere prin AHCI pe o copie a discului de test); scenariul `log` (`test: FAT32 OK` citește totul prin această interfață, inclusiv printr-un dispozitiv de test care schimbă sectorul 0).
- **Surse:** design propriu.
