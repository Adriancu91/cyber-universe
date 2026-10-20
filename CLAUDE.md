# Instrucțiuni pentru Claude Code — Cyber Universe OS

Acest fișier se citește la ÎNCEPUTUL fiecărei sesiuni. Proprietarul proiectului (Adrian) nu programează:
tu scrii, testezi și documentezi tot codul. Lucrezi autonom, în cascadă, chiar dacă Adrian nu e prezent.

## 1. La pornirea sesiunii (obligatoriu, în ordine)

1. Citește `docs/STATE.md` — unde s-a oprit ultima sesiune.
2. Citește `docs/BLOCKERS.md` — ce e blocat și nu trebuie reîncercat orbește.
3. Citește `docs/TASKS.md` și alege **prima sarcină nebifată `[ ]` care nu e marcată `BLOCAT`
   și ale cărei dependențe sunt bifate `[x]`**.
4. Citește planul sarcinii: `docs/plans/<ID>.md` (ex. `docs/plans/P0-04.md`).
5. Dacă sarcina e într-o fază nouă, citește secțiunea fazei din `docs/ROADMAP.md`.
6. `git pull` înainte de orice modificare.
7. Dacă uneltele lipsesc (sesiune nouă în cloud), rulează `tools/env-check.sh` sau, înainte să existe, pașii din `docs/plans/P0-01.md`.

Referințe de studiu: `docs/research/STUDIU-P0-P1.md` și prototipul verificat din `docs/research/prototip-p0/`.

## 2. Bucla de lucru (cascadă)

Pentru fiecare sarcină:

1. Marchează sarcina `[~]` (în lucru) în `docs/TASKS.md`, commit scurt.
2. Implementează în pași mici.
3. Construiește: `tools/build.sh` (după ce există, din sarcina P0-04).
4. Testează: `tools/test.sh` rulează nucleul în QEMU fără ecran (`-display none -serial stdio`),
   cu limită de timp, și verifică textul de pe portul serial. Un test trece doar dacă ieșirea e cea așteptată.
5. Dacă trece: bifează `[x]`, notează în STATE.md ce s-a făcut, commit + push.
6. Treci la următoarea sarcină. Continuă cât timp sesiunea permite.

Când adaugi o sarcină nouă în coadă, scrii obligatoriu și planul ei în `docs/plans/<ID>.md`
(format: Scop, Pași, Fișiere, Test, Riscuri, Surse — vezi `docs/plans/README.md`).

Commit-uri: mici, cu mesaj de forma `P1-03: alocator de cadre de memorie`.
Nu lăsa niciodată `main` într-o stare care nu se construiește. Munca neterminată merge pe ramura `wip/<id-sarcină>`.

## 3. Când te blochezi

- Maximum **3 încercări diferite** pe aceeași problemă.
- După 3 eșecuri: scrie în `docs/BLOCKERS.md` (sarcină, ce ai încercat, eroarea exactă, ce ar putea debloca),
  marchează sarcina `BLOCAT` în TASKS.md și treci la **următoarea sarcină independentă**.
- Dacă nu mai există nicio sarcină independentă: scrie un rezumat clar în STATE.md și oprește sesiunea.
- Nu șterge, nu rescrie istoria git (`push --force` interzis), nu modifica deciziile din DECISIONS.md
  fără să adaugi o decizie nouă care o înlocuiește explicit.

## 4. La sfârșitul sesiunii (obligatoriu)

Actualizează `docs/STATE.md`:
- data și ora, ce sarcini s-au terminat;
- starea build-ului și a testelor;
- următoarea sarcină exactă;
- orice avertisment pentru sesiunea următoare.
Apoi commit + push. Sesiunea următoare pornește DOAR din ce e scris în repo.

## 5. Reguli de cod

- Nucleul: Rust (`no_std`), x86_64, pornire UEFI prin Limine. Vezi `docs/DECISIONS.md`.
- Doar funcții care aduc un plus clar. Fără cod „pentru orice eventualitate”.
- Fiecare modul nou: un fișier README scurt (ce face, de ce depinde, cum se testează).
- Fiecare funcționalitate nouă: cel puțin un test automat în QEMU.
- Cod `unsafe` doar unde e inevitabil, cu un comentariu `// SAFETY:` care explică de ce e corect.
- Comentariile din cod: în engleză. Documentația din `docs/`: în română.

## 6. Reguli legale (clean-room) — vezi `docs/CLEAN_ROOM.md`

- **Windows:** doar documentație publică Microsoft și comportament observat. Niciodată cod scurs,
  dezasamblat sau decompilat.
- **Proiecte open source** (Linux, ReactOS, Haiku, SerenityOS etc.): poți citi codul ca să înțelegi ideile,
  dar NU copia cod în proiect. Scrie implementarea ta. Notează sursa de inspirație în README-ul modulului.
- Codul nostru e MIT; cod GPL copiat ar obliga tot proiectul la GPL.

## 7. Cyber Hub (centrul comun al proiectelor)

Acest proiect e conectat la `Adriancu91/cyber-hub`, alături de Kronos și suita office. ID-ul nostru: `cyber-universe`.
- **La început** (după pasul 1.6): clonează/actualizează hub-ul, citește `REGULI.md`, `PROIECTE.md`, stările celorlalte
  proiecte din `stare/` și propunerile din `idei/` care ne privesc. Doar context — nu schimbi planul fără aprobarea lui Adrian.
- **La sfârșit** (după pasul 4): rescrie `stare/cyber-universe.md`, adaugă 1–3 rânduri în `jurnal/cyber-universe.md`,
  opțional o propunere în `idei/`; commit + push în hub, cum scrie în `REGULI.md` din hub.
- Nu scrii niciodată în fișierele altui proiect și nu modifici codul altui proiect.

## 8. Raport pentru Adrian

La finalul fiecărei sesiuni, ultimul mesaj e un raport scurt, în română, fără termeni tehnici inutili:
ce s-a făcut, ce merge acum, ce urmează, dacă e nevoie de ceva de la Adrian.
