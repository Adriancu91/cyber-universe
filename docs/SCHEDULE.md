# Lucru automat (schedule) și reluare după blocaje

## Cum curge munca
Fiecare sesiune Claude Code: citește `STATE.md` → ia prima sarcină liberă din `TASKS.md` → lucrează în
cascadă → la final actualizează `STATE.md` și face push. Repo-ul e singura „memorie” între sesiuni.

## Cele două moduri de rulare

### A. Sesiuni programate (Adrian nu e prezent)
O sarcină programată pornește automat o sesiune nouă cu promptul de mai jos.
- **Ritm recomandat:** de 2 ori pe zi (ex. 09:40 și 21:40), apoi ajustat după cât de repede avansează.
- **Unde rulează:** în cloud (sesiune Claude cu acces la repo) — verificat că merge, vezi D-008 —
  sau pe ThinkPad, în WSL2, dacă e pornit. Fiecare sesiune cloud pornește „goală”: instalează întâi uneltele (P0-01 / `tools/env-check.sh`).
- Rezultatul fiecărei sesiuni: commit-uri pe GitHub + raport în STATE.md.

**Promptul sesiunii programate:**
```
Lucrezi la Cyber Universe OS (repo Adriancu91/cyber-universe).
Citește CLAUDE.md și urmează exact secțiunile 1–4.
Continuă de la docs/STATE.md, în cascadă, până termini sesiunea.
La final: actualizează docs/STATE.md, commit + push, și scrie raportul pentru Adrian (secțiunea 7).
```

### B. Sesiuni cu Adrian prezent
Adrian deschide Claude Code în folderul proiectului și scrie: „continuă Cyber Universe”.

## Când sesiunea se oprește la mijloc (eroare, restart, limită de timp)
- Munca neterminată e pe ramura `wip/<id-sarcină>`, iar sarcina e marcată `[~]`.
- Sesiunea următoare: dacă găsește o sarcină `[~]`, verifică ramura `wip/` și o continuă, nu o ia de la zero.
- Dacă `main` nu se construiește (nu ar trebui să se întâmple): prima prioritate e repararea lui.

## Când e nevoie de restart sau de o acțiune fizică
Exemple: instalare program în Windows, activare virtualizare în BIOS, test pe stick USB.
Claude Code NU poate face aceste lucruri singur. Le notează în `BLOCKERS.md` cu „Nevoie de Adrian: da”
și în raportul final, cu pași simpli, apoi continuă cu alte sarcini independente.

## Oprire de siguranță
Dacă 3 sesiuni la rând nu termină nicio sarcină, sesiunea a treia scrie în STATE.md
„OPRIT: necesită analiza lui Adrian” și nu mai începe sarcini noi până când Adrian răspunde.
