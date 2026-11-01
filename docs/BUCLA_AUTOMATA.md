# Bucla automată pe workstation (P6-41, comanda C-53)

Scopul: workstation-ul testează singur versiunile noi ale sistemului, **fără Adrian de față**, doar cu teste care **citesc**
(viteza Kronos, citiri din placa video și din calculator, raportul). Scrierile în placa video rămân doar cu Adrian de față.
Doar ziua (08:00–22:00), nu în nopțile de antrenare Kronos.

## Cum merge o tură
1. În Windows, scriptul `bucla-automata.ps1 pregateste` (în fiecare Release, din `os-213`; sursa: `tools/windows/`) (rulat ca administrator) pune versiunea nouă pe stick
   (păstrează `kronos\`, `firmware\`, `cyber.rw`), scrie lista de teste `auto\teste.txt`, cere **o singură pornire** de pe stick
   și repornește calculatorul.
2. Cyber Universe pornește de pe stick, vede `auto\teste.txt` și rulează comenzile pe rând — doar pe cele din **lista albă**
   (orice altceva, inclusiv `gpu i6 --scrie`, e refuzat și notat). Raportul `auto\raport-<data>.txt` se scrie pe loc, după fiecare comandă.
3. La final lista devine `auto\teste-facut-<data>.txt` (nu se mai rulează) și calculatorul repornește singur → Windows.
4. Scriptul `bucla-automata.ps1 citeste` copiază rapoartele noi într-un dosar, cu nume gata de pus în hub (`rezultate\`).

**Ceasul de siguranță:** dacă lista nu termină în 10 minute (sau cât scrie la `ceas N` în listă, 1–120), sistemul repornește singur.
Raportul are tot până la comanda blocată (rândul „in lucru: …”). Dacă stick-ul ar porni din nou cu aceeași listă, sistemul nu o reia:
scrie „INTRERUPT” în raport, redenumește lista și repornește.

**Lista albă (modul automat):** `hw`, `lspci`, `lsusb`, `net`, `mem`, `uptime`, `log`, `ls`, `cat`, `raport`, `cpu …`,
`gpu`, `gpu identifica|gfw|vbios|fwsec|firmware|pregatire-i6|plan`, `run kronos …`, `run kronos-calcul …`.
Refuzate mereu: tot ce conține `--scrie`, `gpu i6`, `reboot`, rețeaua, celelalte programe.

## Pregătirea cu Adrian (o singură dată, ~20 min, la întoarcere)
1. **BIOS-ul workstation-ului** (la pornire, tasta Del sau F2):
   - **Secure Boot: oprit** (Cyber Universe nu e semnat de Microsoft);
   - **ordinea de pornire: Windows primul** (stick-ul NU primul — pornirea de pe stick o cere scriptul, o singură dată);
   - salvezi și ieși (F10).
2. **Intrarea automată în Windows** (ca bucla să continue după repornire fără parolă tastată): o setăm împreună, cu varianta
   aleasă de Adrian (de ex. Windows „Sign-in options”). Parola nu se scrie niciodată în script sau în repo.
3. **Dreptul de administrator doar pentru script:** scriptul cere administrator doar pentru `bcdedit` („pornește o dată de pe stick”)
   și repornire. Îl pornim din „PowerShell (Administrator)”; dacă Windows refuză scripturile, o singură dată:
   `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.
4. **Prima tură, cu Adrian de față:** stick-ul în workstation, apoi
   `.\bucla-automata.ps1 pregateste -DosarRelease <dosarul cu Release-ul descărcat>`.
   Dacă scriptul spune „nu e clar care intrare e stick-ul”, alegem împreună ID-ul din listă și îl dăm cu `-IdStick {…}`.
   Urmărim: pornirea de pe stick, rândurile `auto: [1/n] …`, repornirea, revenirea în Windows.
5. Înapoi în Windows: `.\bucla-automata.ps1 citeste` → rapoartele în `cyber-bucla\rapoarte` (în dosarul utilizatorului),
   cu rândul „REZULTAT: … rulate, … refuzate”.

## Dacă ceva nu merge
- Calculatorul rămâne în Cyber Universe (ecran blocat): butonul de pornire ținut 10 s; la pornire intră în Windows (Windows e primul).
- Lista nu se rulează: verifică `auto\teste.txt` (text simplu, câte o comandă pe rând) și `cyber.rw` în rădăcina stick-ului.
- Marcajul `auto\in-lucru.txt` rămas (tură întreruptă): scriptul îl mută singur deoparte la următoarea `pregateste`.

## Limite cunoscute
- Ceasul de siguranță e un fir al sistemului: o blocare totală a nucleului (cu întreruperile oprite) nu o vede. Watchdog-ul
  chipsetului = pas următor, după ce vedem ce are placa de bază a workstation-ului.
- Scriptul PowerShell e verificat doar citindu-l (în cloud nu există PowerShell); prima rulare se face cu Adrian de față.
