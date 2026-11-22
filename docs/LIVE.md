# Canalul viu (C-159): forma fișierelor

> Forma 1, scrisă de sesiunea OS (21 nov 2026, P6-108; bucla P6-109). Codul: `kernel/src/shell/live.rs`.
> Proba completă în QEMU: scenariul `ghpush` (`tools/ghpush-test-server.py` joacă rolul centrului de comandă).

## Cum pornește
- Pe stick: `/usb/github.token` (cel de azi, cu drept de scriere în hub) și **`/usb/hub/live.cheie`** (cel puțin 32 de octeți;
  aceiași octeți la centrul de comandă). Fără cheie: `live: cheia /usb/hub/live.cheie lipseste …` și nimic nu pornește.
- În `auto\teste.txt` un rând **`live <minute>`** (1–120), de ex. `live 10` (sau de mână, în shell, `live 10`).
  `<minute>` = cât așteaptă OS-ul fără nicio comandă nouă; fiecare comandă repornește numărătoarea. Ceasul de siguranță al
  listei e împins cât timp sesiunea lucrează. După sesiune lista continuă; la sfârșitul listei repornire în Windows, ca azi.
- OS-ul alege sesiunea `s-AAAALLZZ-HHMM` (ora OS-ului; `-2`, `-3` dacă există deja pe stick), face ramura `os/live` din `main`
  dacă lipsește și scrie **`live/<sesiune>/sesiune.json`**:
  `{"forma":"cyber-live-1","sesiune":"s-…","pornit":"AAAA-LL-ZZ HH:MM:SS","minute":10,"intrebare_ms":2500,"placa":false,"sistem":"…"}`.
  Centrul de comandă găsește sesiunea nouă listând `live/` pe ramura `os/live` (dosarul cel mai nou cu `sesiune.json`).
- Apoi OS-ul întreabă dosarul `live/<sesiune>/` la fiecare **2,5 s**, cu cerere condiționată (ETag; un 304 nu se numără la
  limita GitHub). Când apare `c-<următorul>.json`, îl ia, îl trece prin poartă, scrie rândul în jurnalul de pe stick
  (`/usb/hub/live/<sesiune>/jurnal.txt`) ÎNAINTE de rulare, rulează și scrie `r-<același număr>.json`.
- **Fiecare număr de fișier se citește o singură dată, și cel refuzat:** după un `c-0002.json` refuzat, următoarea comandă e
  `c-0003.json`. Comenzile se pot pune mai multe deodată; OS-ul le ia în ordine.
- Sfârșit: `fel: gata` (răspunsul vine, apoi sesiunea se închide), `<minute>` fără comenzi, sau Esc (doar de mână).
  Copii pe stick: `/usb/hub/live/<sesiune>/` (c-, r-, jurnal). Un răspuns care nu pleacă după 3 încercări rămâne pe stick
  (rând `# r-NNNN.json netrimis` în jurnal).

## Unde
- Hub-ul `Adriancu91/Cyber-hub`, ramura **`os/live`**, dosarul `live/<sesiune>/`.
- Centrul de comandă scrie `c-0001.json`, `c-0002.json`, … (numere de 4 cifre, fără goluri).
- OS-ul scrie `r-0001.json`, … (răspunsul la comanda cu același număr) și `puls.json` (pasul 3).
- Nimeni nu rescrie un fișier existent; fiecare parte doar adaugă.

## Comanda `c-NNNN.json`
```json
{"nr": 1, "sesiune": "s-20261121-0900", "fel": "comanda", "text": "hw", "semnatura": "<64 de cifre hex>"}
```
- `nr`: întreg pozitiv, exact următorul (prima comandă = 1).
- `sesiune`: numele dosarului (`s-AAAALLZZ-HHMM`, ales de OS la pornire și scris în `live/<sesiune>/sesiune.json` — P6-109).
- `fel` (pasul 1): `comanda` (un rând de shell), `stare`, `gata` (încheie sesiunea). Pasul 2, partea 1 (P6-111): `scrie`,
  `tasta`, `ecran` (terminalul viu, mai jos); P6-127: `captura`, `muta`, `clic` (desktopul, mai jos). Pasul 2 e complet.
- `semnatura` = HMAC-SHA-256 cu cheia `/usb/hub/live.cheie` (cel puțin 32 de octeți, aceeași la centrul de comandă) peste textul:
  ```
  cyber-live-1\n<sesiune>\n<nr>\n<fel>\n<text>
  ```
  (fără `\n` la sfârșit; `<text>` exact cum e în JSON, decodat). Cifre hex mici sau mari, 64.
  Exemplu Python: `hmac.new(cheie, f"cyber-live-1\n{s}\n{nr}\n{fel}\n{text}".encode(), "sha256").hexdigest()`; unealta gata: `tools/live-semneaza.py <cheie> <sesiune> <nr> <fel> [text]`.
- Fișierul are cel mult 64 KiB.

## Răspunsul `r-NNNN.json`
```json
{"forma":"cyber-live-1","sesiune":"s-…","nr":1,"ecou":"<fișierul comenzii, exact, ca text>","stare":"ok",
 "cod":0,"durata_ms":12,"iesire":"<tot ce a scris comanda>","ecran":"<textul ecranului după>","sistem":"<versiune, temperaturi, memorie>"}
```
sau, la refuz: `"stare":"REFUZAT","motiv":"<de ce>"` (fără `cod`/`iesire`). `nr` e `null` dacă fișierul nu se putea citi.
- `cod`: 0, sau codul de ieșire al programului pornit cu `run` (126 = programul nu pornește, 127 = comandă necunoscută).
- `iesire`: tot ce a scris comanda (shell + programe). `ecran` (P6-111, cerut la 16:15): **textul ecranului adevărat**, rând
  cu rând (consola de text, cum o vede cine stă la monitor); cât desktopul are ecranul: `(desktopul are ecranul: imaginea lui =
  fel captura)`; fără ecran: ultimele 25 de rânduri din jurnalul sistemului. `sistem`: versiunea, `/usb/VERSIUNE`, de cât timp merge, ora, memoria liberă, temperatura procesorului.
- `fel: stare` → `iesire` = rândul `sistem`. `fel: gata` → `iesire` = „sesiunea se incheie (gata)”.
**`ecou` e mereu fișierul primit, octet cu octet** — centrul de comandă îl compară cu ce a trimis.

## Refuzurile (fiecare are proba ei în testul QEMU)
| Motiv | Când |
|---|---|
| `semnatura gresita` | semnătura nu se potrivește (alt text, altă cheie, alt număr) |
| `numar gresit: astept nr N` | numărul nu e exact următorul |
| `comanda repetata: nr N s-a executat deja (o singura data)` | numărul e deja în jurnalul de pe stick (scris ÎNAINTE de execuție) |
| `comanda scrie in placa video: fara cheia permite-live-placa …` | `gpu lant/i6/recupereaza/…`, `--scrie`, `--gpu`, `test-placa`, `ciclu-test`, `gpu tur --trezire` fără `/usb/hub/permite-live-placa` |
| `o sesiune vie nu porneste alta sesiune vie` | comanda `live …` din modul viu |
| `a doua comanda pe placa in aceeasi sesiune: …` | al doilea `test-placa …` cu formă corectă în aceeași sesiune (P6-118; cel mult unul pe sesiune până la C-158 punctul 5) |
| `alta sesiune`, `fisierul comenzii nu e JSON valid`, `lipseste un camp …`, `fel necunoscut …`, `comanda goala` | forma greșită |
| `cheia live.cheie lipseste sau e prea scurta …` | fără cheie modul viu nu pornește deloc |

## Comanda de placă a modului viu (P6-118, C-161 punctul 1)
- **`test-placa […]`** cu `fel: comanda`, exact formele din `auto\teste.txt` (`--i9 …`, `--flr-proba`, `--tablou`, `--lipit` /
  `--simplu` / `--ln-nou`, `--p8`). Merge la același test ca lista automată, fără „DA” de la tastatură; cere **trei chei**:
  `permite-live-placa` (poarta), `permite-test-placa` pe stick și rândul din `DECIZII.md` (ca lista).
- `iesire` = raportul întreg al testului (ca în raportul listei), care pleacă și în hub (`trimite`), ca de la listă.
- Formă greșită sau o cheie lipsă: `stare: ok`, `cod: 1`, `iesire` = `REFUZAT: <motivul> (nimic nu s-a scris in placa)`.
- **Cel mult o comandă `test-placa` pe sesiune** (orice formă corectă se numără, chiar dacă o cheie lipsește); a doua: `REFUZAT`
  cu motivul de mai sus. Pentru încă un test: o sesiune nouă.
- **Orice comandă care cere „DA” de la tastatură** (`gpu lant`, `gpu recupereaza`, `gpu i6 --scrie`, `gpu tur --trezire`,
  `run kronos --gpu`) primește în modul viu răspuns **pe loc, niciodată „DA”**: rândul `cere DA de la tastatura: doar cu Adrian
  de fata …`, apoi comanda se oprește ca fără DA (nimic scris în placă). Tastele trimise prin canal (pasul 2) nu pot răspunde la
  „DA”: cât merge o sesiune vie, întrebarea nici nu citește tastatura.
- Proba în QEMU (`ghpush`, a doua sesiune): `gpu lant` → DA refuzat pe loc; `test-placa --p8 --tablou` → formă greșită;
  `test-placa --p8` fără `permite-test-placa` → refuzat pentru cheie; al doilea `test-placa` → refuzat de poartă.

## Terminalul viu: `scrie`, `tasta`, `ecran` (P6-111, C-159 pasul 2, partea 1)
- Sesiunea vie are **terminalul ei**: un shell întreg, pe firul lui (ca terminalul din desktop), deschis la primul `scrie` /
  `tasta` / `ecran` și închis odată cu sesiunea (după `exit` se deschide altul la următoarea tastă). Tastele merg doar în el,
  nu în tastatura adevărată (Esc-ul trimis nu oprește sesiunea; ce tastează Adrian la calculator nu se amestecă).
- **`scrie`**: textul tastat literă cu literă (`\n` = Enter, tab = Tab; alte caractere de control → `REFUZAT`), cel mult 1024.
- **`tasta`**: nume de taste despărțite prin spații: `enter`, `esc`, `tab`, `backspace`, `spatiu`, `sus`, `jos`, `stanga`,
  `dreapta`, `home`, `end`, `delete`, `ctrl+<litera>` (cel mult 64). `alt+…` (de ex. `alt+tab`) și numele necunoscute → `REFUZAT`
  la poartă, cu motivul.
- După taste, OS-ul citește ce a scris terminalul până tace 0,5 s (cel mult 10 s): asta e `iesire`. Ce vine mai târziu (o
  comandă lungă) apare la următorul `ecran`.
- **`ecran`**: `iesire` = două părți: `== terminalul viu (scrie / tasta) ==` + textul terminalului acum (ultimele 40 de rânduri,
  cel mult 200 de caractere pe rând), apoi `== ecranul adevarat ==` + textul ecranului (ca în câmpul `ecran`).
- **Placa video nu se atinge prin taste, niciodată:** shell-ul terminalului viu refuză orice rând care ar scrie în placă
  (aceeași listă ca la poartă: `gpu lant`, `test-placa`, `--gpu` …), cu sau fără `permite-live-placa`:
  `REFUZAT: prin taste nu ruleaza nimic ce scrie in placa video (doar ca `comanda`, cu cheile ei)`. La fel `live …`.
  O întrebare „DA” nu citește tastele cât merge sesiunea (răspuns pe loc, niciodată „DA”).
- Proba în QEMU (`ghpush`, a treia sesiune): `ecran` (promptul), `scrie uptime` + `tasta enter` (rulează în terminal),
  `scrie "gpu lant\n"` (refuzat de shell), `tasta alt+tab` (refuzat la poartă), `tasta ctrl+c esc sus`, `ecran` (tot textul).

## `captura` (P6-127, prima parte)
- `fel: captura` (textul nu contează): imaginea ecranului, **micșorată de 4 ori** pe fiecare latură, **în tonuri de gri**
  (din fiecare pătrat 4×4 punctul cel mai luminos din patru, ca textul să rămână vizibil), PNG fără compresie (1920×1080 →
  480×270, ~130 KiB; cel mult 900 KiB). Fișierul: `live/<sesiune>/captura-NNNN.png` (NNNN = numărul comenzii), lângă
  răspunsuri, și copia pe stick. `iesire`: `captura: captura-NNNN.png (L x I pixeli, tonuri de gri, N octeti; …)`.
- Fără ecran sau dacă nu pleacă în hub: `cod: 1` și motivul în `iesire`.
- Proba în QEMU (`ghpush`, a treia sesiune): PNG-ul din hub = cel de pe stick, semnătura PNG, IHDR cu CRC corect (320×200).

## Desktopul: `muta`, `clic` și tastele (P6-127)
- Desktopul se pornește **din terminalul viu**: `scrie` cu textul `desktop\n`. Terminalul îl rulează pe firul lui, deci bucla
  vie citește în continuare comenzi.
- **`muta x y`**: cursorul la (x, y) — pixeli de la colțul din stânga sus (0–8192; marginea ecranului îl oprește).
- **`clic`**, **`clic stanga|dreapta`**, **`clic x y`**, **`clic x y stanga|dreapta`**: întâi mutarea (dacă are x y), apoi
  apăsat + eliberat. `iesire`: `mouse: cursorul la X Y[; clic stanga|dreapta]` (poziția citită din serverul de ferestre).
- Fără desktop (așteaptă 3 s să pornească): `cod: 1`, `mouse: desktopul nu ruleaza …`. Formă greșită → `REFUZAT` la poartă.
- **Cât rulează desktopul, `scrie` / `tasta` merg la fereastra din față** (aceeași coadă ca tastatura adevărată);
  `iesire`: `N taste trimise desktopului …`. `tasta esc` închide desktopul (ecranul revine la text). Într-o sesiune pornită
  **de mână** (`live` tastat), Esc nu se trimite cât rulează desktopul (ar opri sesiunea) — `cod: 1` cu motivul.
- Placa video nu are buton în desktop azi; orice program de placă rămâne doar `comanda`, cu cheile ei.
- Proba în QEMU (`ghpush`, a patra sesiune): desktop pornit, `muta 100 80`, `clic`, `clic 200 150 dreapta`, `tasta esc`
  (desktopul se închide), `muta 5 5` fără desktop (cod 1).

## Ce rămâne păzit de nucleu (neschimbat)
Fișierele secrete nu se arată; scrieri pe disc doar pe partiții `cyber.rw`; placa video doar cu `permite-live-placa`
(o pune doar centrul de comandă, după „da”-ul lui Adrian).

## Proba pe workstation (pentru centrul de comandă)
1. Pune pe stick `hub\live.cheie` (aceeași cheie ca la tine) și în `auto\teste.txt` rândul `live 10`.
2. Când apare `live/<sesiune>/sesiune.json` pe ramura `os/live`, scrie comenzile cu `tools/live-semneaza.py` și urcă-le pe
   `os/live`: întâi doar citiri (`hw`, `mem`, `stare`, `gpu stare`), apoi probele de refuz (semnătură greșită, număr greșit,
   comandă repetată, `gpu lant` fără `permite-live-placa`), la sfârșit `gata`.
3. Compară `ecou` din fiecare `r-NNNN.json` cu ce ai trimis, octet cu octet.
