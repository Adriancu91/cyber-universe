# modmgr — managerul de module

**Ce face (P4-08):** instalează, pornește, oprește și dezinstalează module. Un modul e un pachet `.cum` (arhivă tar
ustar) cu `module.toml` și fișierele lui; formatul e descris în `docs/specs/MODULE.md`.
- **Instalare:** pachetul se verifică (`module.toml` valid, nume `a-z 0-9 -`, nicio cale cu `..` sau absolută,
  dependențele deja instalate), apoi fișierele se copiază în `/disk/modules/<nume>/` și numele se adaugă în
  `/disk/modules/index`. Dacă ceva nu merge la jumătate, directorul modulului se șterge.
- **Pornire / oprire:** `start` rulează programul din cheia `start`; `stop` îl oprește (`proc::kill`, cod de ieșire -9)
  și așteaptă să se termine.
- **Ștergere:** doar dacă modulul e oprit și niciun alt modul instalat nu are nevoie de el.
- **Shell:** `mod list`, `mod install <pachet.cum>`, `mod start <nume>`, `mod stop <nume>`, `mod remove <nume>`.

**Depinde de:** `fs` (scriere FAT32 pe un disc marcat cu `cyber.rw`, P4-07), `fs::tar`, `proc` (pornire, oprire).

**Test:** scenariul `modules` — pe o copie a discului de test: dependență lipsă refuzată, instalare, instalare dublă
refuzată, ștergere refuzată cât timp alt modul are nevoie de el sau cât timp merge, pornire (programul scrie
„modul salut: pornit”), oprire, ștergere (directorul dispare); apoi `mod list` în shell; `tools/after-modules.sh`
verifică discul cu `fsck.fat`. Pachetele de test sunt construite din `modules/` de `tools/build.sh` (`tools/mkmodule.sh`)
și puse pe discul de test în `/pachete/`.

**Limită cunoscută:** un program oprit în timp ce așteaptă alt program (`wait`) se oprește abia când acela se termină.

**Inspirație:** design propriu.
