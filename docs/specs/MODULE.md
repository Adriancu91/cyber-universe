# Formatul modulelor Cyber Universe (versiunea 1)

Un **modul** se distribuie ca un fișier `.cum` (Cyber Universe Module) = o arhivă **tar în format ustar**, cu:
- `module.toml` — descrierea (obligatoriu, în rădăcina arhivei);
- orice alte fișiere și directoare ale modulului (programe, date, documentație), cu căi relative, fără `..`.

## module.toml

Un subset mic de TOML: rânduri `cheie = "text"` sau `cheie = ["a", "b"]`, comentarii cu `#`, rânduri goale.

| Cheie | Obligatoriu | Ce înseamnă |
|---|---|---|
| `name` | da | numele modulului: litere mici `a-z`, cifre, `-`; maximum 32 de caractere |
| `version` | da | versiunea, ca text (ex. `"1.0"`) |
| `description` | nu | o propoziție despre modul (apare în `mod list`) |
| `start` | nu | programul pornit de `mod start`, cale relativă în modul (ex. `"bin/salut"`) |
| `depends` | nu | lista modulelor care trebuie să fie instalate înainte (ex. `["salut"]`) |

Cheile necunoscute sunt ignorate (pachetele mai noi pot adăuga chei). `program_source` e folosită doar la construire
de `tools/mkmodule.sh`: programul din `user/src/bin/<program_source>.rs` e pus în pachet la calea din `start`.

Exemplu (`modules/salut/module.toml`):
```toml
name = "salut"
version = "1.0"
description = "Modul de proba: spune salut si ramane pornit"
start = "bin/salut"
program_source = "mod_salut"
```

## Pe disc

- Modulele instalate stau în `/disk/modules/<nume>/` (discul trebuie să fie marcat pentru scriere cu fișierul `cyber.rw`).
- `/disk/modules/index` — numele modulelor instalate, câte unul pe rând, în ordinea instalării.

## Construire

`tools/mkmodule.sh <director modul> <director cu programele construite> <ieșire.cum>`; `tools/build.sh` face pachetele
pentru tot ce e în `modules/` și le pune pe discul de test în `/pachete/`. Programele `user/src/bin/mod_*.rs` sunt ale
modulelor și nu intră în `/bin`.
