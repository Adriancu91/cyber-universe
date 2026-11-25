# Propunerile de cod ale agentului de bord (C-166 pasul 5, P6-220)

Agentul de bord (`agent "<întrebare>"` în OS) poate scrie o modificare de cod **doar ca propunere**, cu unealta `propune`:

- o ramură nouă `agent/AAAALLZZ-<subiect>` din vârful lui `main` (niciodată `main`, niciodată peste o ramură care există);
- pe ramură: `docs/agent/propuneri/AAAALLZZ-<subiect>.md` (explicația, dovada, lista fișierelor) + fișierele schimbate, întregi;
- cel mult 8 fișiere de cel mult 128 KiB; refuzate de OS: `.github/`, `LICENSE`, `CLAUDE.md`, `docs/DECISIONS.md`;
- CI-ul nu pornește pe `agent/**` (doar `main` și `wip/**`);
- pe stick, `agent/propuneri.txt` ține fiecare propunere, făcută sau refuzată.

## Ce face sesiunea OS cu o propunere

1. `git fetch origin 'refs/heads/agent/*:refs/remotes/origin/agent/*'` și citește nota din `docs/agent/propuneri/`.
2. `git diff origin/main...origin/agent/<ramura>` — verifică: regulile din `CLAUDE.md` (clean-room, cod, siguranță),
   că nu atinge placa în afara regulilor, că dovada se potrivește cu codul.
3. Dacă e bună: o ia într-o sarcină obișnuită (`docs/TASKS.md` + plan), pe `wip/<id>`, testată în QEMU, apoi `main`.
   Dacă nu: notează în `docs/STATE.md` de ce a refuzat-o. Ramura rămâne (nu ștergem nimic).
