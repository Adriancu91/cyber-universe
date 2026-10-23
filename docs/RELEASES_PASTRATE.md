# Release-uri păstrate (nu se șterg niciodată)

Comanda C-17 (Adrian, 2026-10-23): un Release **testat pe hardware** (apare într-un `rezultate/…/REZULTAT.md` din hub) sau
**punct de revenire** nu se șterge. CI-ul (`.github/workflows/ci.yml`, jobul `release`) citește tabelul de mai jos și sare peste
fiecare tag din prima coloană când curăță Release-urile vechi (păstrează în rest ultimele 5).

Cum se adaugă unul: un rând nou în tabel, cu tag-ul exact în prima coloană (`| os-NNN | …`).
Un Release care nu mai există se poate republica din fișierele originale cât timp artefactul rulării CI există (90 de zile):
Actions → „Republica un Release păstrat” → tag + id-ul rulării (`.github/workflows/republica.yml`).

| Tag | Commit | Rularea CI (id) | Motiv | Există la Releases |
|---|---|---|---|---|
| os-120 | 06c69c9 | — | Testul 3 pe laptop (2026-10-22) | nu (șters de curățarea veche; se poate reconstrui din commit) |
| os-125 | ffc342b | 39741529918 | Testul 4 și testul 6 pe laptop (2026-10-22): primul inel RX care merge | nu (artefactul există până în ian. 2027) |
| os-136 | 2a21649 | 39782323694 | Testul 5 pe laptop (2026-10-22) | nu (artefactul există până în ian. 2027) |
| os-138 | 617f817 | 39792543728 | Testul 6 pe laptop (2026-10-22): prima ieșire pe internet (cald, varianta cache) | nu (artefactul există până în ian. 2027) |
| os-150 | af7cd3d | 39830543552 | Testul 7 pe laptop (2026-10-22): internet prin ICS, rece și cald | republicat (C-17) |
| os-161 | d2da9e0 | 39891003818 | Prima pornire pe workstation (2026-10-23 09:45) | republicat (C-17) |
| os-187 | 972fa21 | — | Punct de revenire `stabil-2026-10-23` (C-16): 9f67d7b + o reparație doar în test | da |
