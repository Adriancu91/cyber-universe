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
  `{"forma":"cyber-live-1","sesiune":"s-…","pornit":"AAAA-LL-ZZ HH:MM:SS","minute":10,"intrebare_ms":2500,"puls_ms":60000,"placa":false,"sistem":"…"}`.
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
- Nimeni nu rescrie un fișier existent; fiecare parte doar adaugă. **Singura excepție: `puls.json`**, pe care OS-ul îl
  rescrie la fiecare minut (P6-129).

## Comanda `c-NNNN.json`
```json
{"nr": 1, "sesiune": "s-20261121-0900", "fel": "comanda", "text": "hw", "semnatura": "<64 de cifre hex>"}
```
- `nr`: întreg pozitiv, exact următorul (prima comandă = 1).
- `sesiune`: numele dosarului (`s-AAAALLZZ-HHMM`, ales de OS la pornire și scris în `live/<sesiune>/sesiune.json` — P6-109).
- `fel` (pasul 1): `comanda` (un rând de shell), `stare`, `gata` (încheie sesiunea). Pasul 2, partea 1 (P6-111): `scrie`,
  `tasta`, `ecran` (terminalul viu, mai jos); P6-127: `captura`, `muta`, `clic` (desktopul, mai jos). Pasul 2 e complet.
  Pasul 3 (P6-130): `actualizeaza` (versiunea nouă + repornire cu reluarea sesiunii, mai jos).
  P6-132: `citeste` și `salveaza` (fișiere de pe stick, mai jos). Pasul 3 e complet.
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
  `--simplu` / `--ln-nou`, `--p8`, `--colsum`). Merge la același test ca lista automată, fără „DA” de la tastatură; cere **trei chei**:
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

## Pulsul: `puls.json` (P6-129, C-159 pasul 3, prima parte)
- Cât merge sesiunea, OS-ul rescrie **`live/<sesiune>/puls.json`** la fiecare **60 s** (`puls_ms` din `sesiune.json`) și încă o
  dată la sfârșit:
  ```json
  {"forma":"cyber-live-1","sesiune":"s-…","puls":3,"ora":"AAAA-LL-ZZ HH:MM:SS","stare":"merge","astept_nr":12,
   "rulate":9,"refuzate":2,"paza":"<P6-133, mai jos>","sistem":"<ca în răspunsuri>"}
  ```
  `puls` crește cu 1 la fiecare bătaie; `astept_nr` = numărul comenzii pe care o așteaptă OS-ul; `stare` = `merge`, iar la
  sfârșit `incheiata: <motiv>` (`gata (cerut de centrul de comanda)`, `<minute> minute fara comenzi`, `oprita cu Esc`).
- **Cum îl folosește centrul de comandă:** dacă `ora` din `puls.json` e mai veche de ~3 minute și `stare` = `merge`, OS-ul s-a
  oprit sau a pierdut rețeaua (pasul 4: ceasul de pază îl repornește). O bătaie care nu pleacă nu se repetă (vine alta peste
  un minut); copia de pe stick e mereu ultima.
- Proba în QEMU (`ghpush`, prima sesiune, puls la 5 s): `puls.json` creat, apoi rescris; ultimul din hub = cel de pe stick,
  `stare: incheiata: gata`, `astept_nr` 10, 4 rulate, 5 refuzate.

## `actualizeaza` și reluarea după repornire (P6-130, C-159 pasul 3, partea 2)
- Comanda: `{"fel": "actualizeaza", "text": "os-346", …}` (sau `"text": "--versiune os-346"`). Altă formă = `REFUZAT`
  (`actualizeaza: forma e os-<numar> …`). Nu atinge placa video, deci nu cere `permite-live-placa`.
- OS-ul rulează `/bin/actualizeaza --versiune os-N` (exact acel Release, fiecare fișier verificat cu SHA256SUMS, versiunea de azi
  păstrată în `boot/vechi/`). `iesire` = tot ce a scris programul + ce urmează:
  - a picat (`cod` ≠ 0): stick-ul rămâne pe versiunea veche, sesiunea merge mai departe;
  - stick-ul avea deja os-N: nicio repornire, sesiunea merge mai departe;
  - versiunea nouă e pe stick (în încercare: dacă nu se confirmă în 5 minute, revine singură cea veche): OS-ul scrie pe stick
    `hub/live/reluare.txt` **înainte** să plece răspunsul, apoi sesiunea se încheie cu motivul
    `repornire in os-N (actualizeaza); sesiunea se reia dupa pornire` (același text în `puls.json`, câmpul `stare`).
- Repornirea: pornită de mână (`live <minute>` tastat) → imediat; din lista automată → la sfârșitul listei, ca întotdeauna.
  **Atenție:** dacă stick-ul a fost pornit o singură dată (din Windows), repornirea duce în Windows; sesiunea se reia la
  **următoarea** pornire a OS-ului de pe stick (centrul de comandă îl pornește din nou, ca la un test). `reluare.txt` rămâne
  până atunci.
- La pornire, înaintea listei automate, OS-ul găsește `reluare.txt`, îl **șterge întâi** (o cădere în sesiunea reluată nu
  produce o buclă) și reia **aceeași sesiune**: același dosar, numărul următor din jurnalul de pe stick (aceleași reguli: o
  comandă deja rulată nu mai rulează), cel mult o comandă de placă pe sesiune ținută peste repornire. Trimite un fișier nou:
  ```json
  {"forma":"cyber-live-1","sesiune":"s-…","reluare":1,"ora":"…","ceruta":"os-346","versiune":"<rândul 1 din /usb/VERSIUNE>",
   "dupa_nr":2,"astept_nr":3,"placa":true,"sistem":"…"}
  ```
  `live/<sesiune>/reluare-<k>.json` (k = a câta reluare). **Cum verifici:** `versiune` = ce ai cerut în `ceruta`; altfel noua
  versiune nu s-a confirmat și a revenit cea veche (detalii în `rezultate/os-actualizari/`). Apoi trimiți `c-<astept_nr>.json`
  ca de obicei; `puls` continuă numărătoarea de dinainte; `<minute>` fără comenzi se numără din nou de la reluare.
- Sesiunea reluată dintr-o listă automată: la sfârșitul ei calculatorul repornește (înapoi în Windows), ca la sfârșitul listei.
- Proba în QEMU (`ghpush`, sesiunea a cincea; `actualizeaza` jucat fără server de Release-uri, programul adevărat are proba lui
  în `update`): forma greșită refuzată, `reluare.txt` cu nr 2, `puls.json` = `repornire in os-999…`; reluarea: același nume,
  `reluare-1.json` (astept_nr 3), `stare` și `gata` rulate, `reluare.txt` șters, a doua reluare refuzată.

## Fișiere: `citeste` și `salveaza` (P6-132, C-159 pasul 3, partea 3)
- **`citeste`**, `text` = calea întreagă de pe stick, de ex. `/usb/auto/teste.txt` sau `/usb/hub/live/s-…/jurnal.txt`.
  - Doar sub `/usb` (stick-ul), fără `..`; **niciodată un fișier secret** (token, `live.cheie`, `permite-live-placa`, aceeași listă
    ca `cat`; literele mari nu contează) → `REFUZAT` la poartă (`citeste: fisier secret …`, `citeste: doar de pe stick …`).
  - **Un dosar** → `iesire` = lista lui: `citeste: dosarul <cale> (N intrari)`, apoi câte un rând `  nume  M octeti` (sau `  nume/`).
  - **Un fișier** (cel mult 900 KiB) → urcat lângă răspuns ca **`live/<sesiune>/fisier-NNNN.bin`** (NNNN = numărul comenzii;
    octeții exacți, orice fel de fișier); `iesire` = `citeste: <cale> -> fisier-NNNN.bin (M octeti, sha256 <64 hex>)` și, pentru
    text UTF-8 de cel mult 32 KiB, `== continut ==` urmat de tot textul.
  - Un fișier care poartă octeții unei chei sau ai token-ului (oriunde ar fi copiat) **nu pleacă**: cod 1, `iesire` spune de ce.
  - Lipsă / prea mare / nu pleacă în hub → cod 1 și motivul în `iesire`.
- **`salveaza`**, `text` = `<cale>` + **un rând nou** + conținutul în **base64** (rândurile noi și spațiile din base64 se ignoră).
  - Doar **direct în `/usb/schimb/`** (dosarul se face dacă lipsește): numele din litere, cifre, `.`, `-`, `_`, cel mult 64, nu
    începe cu `.`; cel mult **48 KiB** (fișierul comenzii are cel mult 64 KiB). Altă cale, subdosar, `..`, base64 greșit, prea
    mare → `REFUZAT` la poartă (`salveaza: doar direct in /usb/schimb/ …`, `salveaza: forma …`, `salveaza: continutul nu e base64 valid`).
  - Fișierul e scris, citit înapoi și comparat; `iesire` = `salveaza: <cale> (M octeti, sha256 <hex>; fisier nou)` sau
    `(…; inlocuit, avea K octeti)`. **Nu scrie nicăieri altundeva** (nici `auto/`, nici `boot/`, nici `hub/`).
  - În jurnalul de pe stick și pe ecran apare doar calea și lungimea base64-ului, nu conținutul.
  - Exemplu Python: `text = "/usb/schimb/nota.txt\n" + base64.b64encode(date).decode()`.
- Proba în QEMU (`ghpush`, sesiunea a șasea; stick-ul e `/etc` acolo): `salveaza` un text cu diacritice, `citeste` același fișier
  (textul în `iesire`, `fisier-0002.bin` din hub = octeții salvați), `citeste` dosarul, cheia `live.cheie`, o salvare în `hub/` și
  una cu `..` — refuzate; nimic scris în afara `schimb/`.

## Ceasul de pază în modul viu (P6-133, C-159 pasul 4, leagă C-158 punctul 6)
- Cât merge o sesiune vie (nouă sau reluată), OS-ul armează ceasul de pază al plăcii de bază (TCO, `kernel/src/hw/tco.rs`):
  **~5 minute fără hrănire = repornire**. Un fir al sistemului îl hrănește la 10 s, dar **doar cât bucla sesiunii s-a mișcat în
  ultimele 45 de minute** — deci un OS înghețat (planificatorul oprit) SAU o comandă blocată pentru totdeauna se termină cu o
  repornire singură. La sfârșitul sesiunii (gata, minutele, Esc, `actualizeaza`) ceasul se oprește.
- Dacă ciclul automat îl hrănește deja, sesiunea îl lasă așa (`armat si hranit deja de ciclu`).
- **Pe calculatoare reale armarea e încă OPRITĂ** (ca la ciclu, P6-103: întâi citirea pe workstation): sesiunea scrie
  `ceasul de paza: neactivat (…)` și merge ca înainte. Pornirea pe workstation = o comandă nouă (C-158 punctul 6: „pe workstation
  doar după o comandă nouă”); atunci se schimbă un singur rând (`ARM_ON_HARDWARE`) + lămurirea NO_REBOOT pe PCH-ul nou.
- Unde se vede: un rând `# ceasul de paza: …` în jurnalul de pe stick, pe ecran, și câmpul **`paza`** din `puls.json`
  (`armat de sesiune (~300 s fara hranire = repornire)`, `armat de ciclu`, `neactivat (…)`).
- Proba în QEMU: scenariul `tco-viu` (q35, reset așteptat): armat + bucla merge 12 s = nicio repornire; sfârșitul sesiunii =
  ceasul oprit, 10 s fără repornire; armat din nou și bucla oprită (comandă blocată) = repornire.

## Ce rămâne păzit de nucleu (neschimbat)
Fișierele secrete nu se arată; scrieri pe disc doar pe partiții `cyber.rw`; placa video doar cu `permite-live-placa`
(o pune doar centrul de comandă, după „da”-ul lui Adrian).

## Proba pe workstation (pentru centrul de comandă)
1. Pune pe stick `hub\live.cheie` (aceeași cheie ca la tine) și în `auto\teste.txt` rândul `live 10`.
2. Când apare `live/<sesiune>/sesiune.json` pe ramura `os/live`, scrie comenzile cu `tools/live-semneaza.py` și urcă-le pe
   `os/live`: întâi doar citiri (`hw`, `mem`, `stare`, `gpu stare`), apoi probele de refuz (semnătură greșită, număr greșit,
   comandă repetată, `gpu lant` fără `permite-live-placa`), la sfârșit `gata`.
3. Compară `ecou` din fiecare `r-NNNN.json` cu ce ai trimis, octet cu octet.
