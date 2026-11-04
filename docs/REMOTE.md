# Remote text — consola Cyber Universe prin rețea (F2, P6-49)

Ecranul text al shell-ului și tastatura, prin rețea, direct din OS (fără Windows la mijloc).
Merge în mașina virtuală (F1) și pe un calculator real din rețeaua de acasă.

## Pregătire (o singură dată)
1. Fă o cheie lungă (cel puțin 16 caractere, fără spații). Pe Windows, în PowerShell:
   `py -c "import secrets; print(secrets.token_urlsafe(32))" > remote.cheie`
2. Pune fișierul `remote.cheie` **pe stick** (sau pe discul mașinii virtuale — pentru ea e tot „stick-ul”, `/usb`).
   Păstrează o copie lângă client (de ex. `C:\cyber\remote.cheie`). **Cheia nu intră niciodată în repo.**
3. Opțional: un fișier `remote.porneste` lângă cheie (conținutul: `2323`) → serviciul pornește singur la boot.
   Fără el, scrii în OS `remote porneste`.

## Folosire
- În OS: `remote porneste` (portul 2323), `remote stare`, `remote opreste`.
- Pe Windows: `py tools\remote-client.py 127.0.0.1 -k C:\cyber\remote.cheie` (mașina virtuală: `vm-pornire.ps1`
  redirecționează portul 2323 doar pe 127.0.0.1). Pe alt calculator din casă: adresa OS-ului (o vezi cu `net`).
- O singură comandă și ieșire: `py tools\remote-client.py 127.0.0.1 -k remote.cheie -c uptime -c "log remote"`.
- `exit` închide sesiunea; `remote opreste` oprește tot serviciul.

## Ce e permis prin remote
Doar lista albă (aceeași ca la bucla automată, doar citire): `help`, `uptime`, `mem`, `hw`, `net`, `lspci`, `lsusb`, `log`,
`ls`, `cat` (fără fișierele secrete), `cpu`, `raport`, `gpu` (identifica, tur, plan... — **niciodată `--scrie`**, `i6`, `lant`),
`run kronos`, `run kronos-calcul`, `remote stare`, `remote opreste`, `exit`. Restul primește `REFUZAT`.

## Siguranță
- Fără cheie (sau cheie prea scurtă) serviciul nu pornește. Implicit e oprit.
- 3 chei greșite la rând → orice conexiune e refuzată 60 de secunde; 10 chei greșite → serviciul se oprește singur.
- Cheia se compară în timp constant; `cat` nu o arată; rapoartele o ascund.
- O singură sesiune odată; 30 de minute fără taste → sesiunea se închide.
- Fiecare comandă și fiecare refuz apar în jurnalul OS-ului: `log remote`.
- **Încă fără criptare:** folosește-l doar în mașina virtuală (127.0.0.1) sau în rețeaua de acasă. TLS vine într-un pas următor.

## Test automat
Scenariul `remote` din `tests/scenarios.txt` (`tools/test.sh remote`), descris în `kernel/src/net/README.md`.
