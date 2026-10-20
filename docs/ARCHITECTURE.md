# Arhitectura Cyber Universe OS

## Straturi
```
┌───────────────────────────────────────────────┐
│ Aplicații proprii (module): office, Kronos... │  apps/
├───────────────────────────────────────────────┤
│ Interfață: desktop, ferestre, shell           │  modules/ui, modules/shell
├───────────────────────────────────────────────┤
│ Servicii de sistem: fișiere, rețea, module    │  modules/
├───────────────────────────────────────────────┤
│ Drivere (module): disc, USB, rețea, ecran     │  modules/drivers
├───────────────────────────────────────────────┤
│ Nucleu: memorie, procese, întreruperi, apeluri│  kernel/
├───────────────────────────────────────────────┤
│ Pornire: UEFI → Limine → nucleu               │  boot/
└───────────────────────────────────────────────┘
```

## Principii
- **Nucleu mic:** în nucleu intră doar memoria, procesele, întreruperile și apelurile de sistem.
- **Totul în rest e modul:** driverele, serviciile și aplicațiile se pot adăuga sau scoate.
- **Fiecare modul** are: `README.md`, `module.toml` (nume, versiune, dependențe), cod, teste.

## Structura folderelor
```
boot/        limine.conf și resurse de pornire
kernel/      src/ (arch/x86_64, mem, int, sched, syscall, log)
modules/     drivers/, fs/, net/, shell/, ui/, modmgr/
apps/        aplicații proprii, ca module
tools/       build.sh, run.sh, test.sh
tests/       teste QEMU, capturi de ecran
docs/        plan, stare, decizii, specificații (docs/specs/)
build/       rezultate build (ignorat de git)
```
