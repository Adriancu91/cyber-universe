# modules — pachetele de module

Fiecare director de aici e un modul: `module.toml` + fișierele lui (format: `docs/specs/MODULE.md`).
`tools/build.sh` face din fiecare un pachet `.cum` (cu `tools/mkmodule.sh`) și îl pune pe discul de test în `/pachete/`.

- `salut` — modul de probă: un program (`user/src/bin/mod_salut.rs`) care pornește și rămâne pornit până e oprit.
- `cere-salut` — modul de probă fără program, care are nevoie de `salut` (verificarea dependențelor).
