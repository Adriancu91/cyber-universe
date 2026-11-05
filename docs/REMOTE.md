# Remote text — consola Cyber Universe prin rețea (F2, P6-49)

Ecranul text al shell-ului și tastatura, prin rețea, direct din OS (fără Windows la mijloc).
Merge pe un calculator real din rețeaua de acasă (pornit de pe stick). Mașina virtuală (F1) a fost anulată de Adrian pe 4 nov.

## Pregătire (o singură dată)
1. Fă o cheie lungă (cel puțin 16 caractere, fără spații). Pe Windows, în PowerShell:
   `py -c "import secrets; print(secrets.token_urlsafe(32))" > remote.cheie`
2. Pune fișierul `remote.cheie` **pe stick** (`/usb`).
   Păstrează o copie lângă client (de ex. `C:\cyber\remote.cheie`). **Cheia nu intră niciodată în repo.**
3. Opțional: un fișier `remote.porneste` lângă cheie (conținutul: `2323`) → serviciul pornește singur la boot.
   Fără el, scrii în OS `remote porneste`.

## Folosire
- În OS: `remote porneste` (portul 2323), `remote stare`, `remote opreste`.
- Pe Windows, de pe alt calculator din casă: `py tools\remote-client.py <adresa OS-ului> -k C:\cyber\remote.cheie`
  (adresa o vezi în OS cu `net`).
- O singură comandă și ieșire: `py tools\remote-client.py <adresa> -k remote.cheie -c uptime -c "log remote"`.
- `exit` închide sesiunea; `remote opreste` oprește tot serviciul.

## Ce e permis prin remote
Doar lista albă (aceeași ca la bucla automată, doar citire): `help`, `uptime`, `mem`, `hw`, `net`, `lspci`, `lsusb`, `log`,
`ls`, `cat` (fără fișierele secrete), `cpu`, `raport`, `gpu` (identifica, tur, plan... — **niciodată `--scrie`**, `i6`, `lant`),
`run kronos`, `run kronos-calcul`, `compara`, `timp <comanda permisa>`, `remote stare`, `remote opreste`, `exit`. Restul primește `REFUZAT`.

Unelte de profil și jurnal (P6-52, F9), toate doar citire:
- `log --sumar` — câte rânduri are jurnalul pe fiecare nivel (ERR, WARN, INFO) și ce module au scris cele mai multe avertismente/erori.
- `log --avertismente [n]` — doar ultimele n avertismente și erori.
- `timp <comanda>` — rulează o comandă de pe lista albă și spune cât a durat (ms) și cum s-a schimbat memoria (ex.: `timp run kronos --n 16 Buna ziua`).
- `compara <f1> <f2>` — rândurile care diferă între două rapoarte.

## Siguranță
- Fără cheie (sau cheie prea scurtă) serviciul nu pornește. Implicit e oprit.
- 3 chei greșite la rând → orice conexiune e refuzată 60 de secunde; 10 chei greșite → serviciul se oprește singur.
- Cheia se compară în timp constant; `cat` nu o arată; rapoartele o ascund.
- O singură sesiune odată; 30 de minute fără taste → sesiunea se închide.
- Fiecare comandă și fiecare refuz apar în jurnalul OS-ului: `log remote`.
- **Încă fără criptare:** folosește-l doar în rețeaua de acasă. TLS vine într-un pas următor.

## Test automat
Scenariul `remote` din `tests/scenarios.txt` (`tools/test.sh remote`), descris în `kernel/src/net/README.md`.
