# Cyber Universe pe un stick USB

Ghid pentru a porni Cyber Universe pe un calculator adevărat (de exemplu ThinkPad L15 Gen 2).
Nu se șterge nimic de pe calculator: sistemul rulează doar de pe stick și din memorie.

## 1. Ce îți trebuie
- un stick USB de cel puțin 128 MB;
- pentru varianta A: programul **balenaEtcher** (https://etcher.balena.io) sau **Rufus** (https://rufus.ie) pe Windows;
  pentru varianta B (cea folosită la primul test, 21 oct): nimic în plus, doar Windows Explorer.

## 2. Descarcă imaginea
**Cel mai simplu (de la P6-07):** pe GitHub, în depozitul `cyber-universe`, la dreapta, **Releases** → ultima versiune
(`os-<număr>`, marcată *Latest*). Acolo sunt, fără arhivă: `cyber-universe-usb.img` (varianta A) și fișierele pentru
varianta B (`BOOTX64.EFI`, `kernel`, `initrd.tar`, `limine.conf`, `limine-uefi-cd.bin`), plus `SHA256SUMS` (sumele de
control) și `VERSIUNE` (din ce commit e). CI publică o versiune nouă după fiecare test verde pe `main` și le păstrează pe
ultimele 5. Depozitul e privat, deci trebuie să fii logat pe GitHub.

**Altfel (artefactul CI):**
1. Pe GitHub, în depozitul `cyber-universe`, apasă **Actions**.
2. Alege ultima rulare **CI** cu bifă verde.
3. Jos, la **Artifacts**, descarcă **stick-usb** și dezarhivează-l: vei avea `cyber-universe-usb.img`.

## 3. Pune sistemul pe stick (alege A sau B)
**Varianta A — imaginea întreagă (tot ce e pe stick se șterge):**
- **balenaEtcher:** „Flash from file” → alegi `cyber-universe-usb.img` → „Select target” → stick-ul → „Flash”.
- **Rufus:** alegi stick-ul, apoi fișierul `.img`; dacă te întreabă, alege **modul DD** (scriere ca imagine).

**Varianta B — fișiere copiate, fără Rufus/Etcher (a mers pe Lenovo, 21 oct):**
1. Formatează stick-ul ca **FAT32** (clic dreapta pe stick în Explorer → Formatare → FAT32).
2. Copiază în **rădăcina** stick-ului exact aceste fișiere, cu aceleași dosare (le iei din Release — acolo sunt fără
   dosare, deci le pui tu în dosarele de mai jos — din imagine sau din artefactul CI):
   - `EFI/BOOT/BOOTX64.EFI`
   - `boot/kernel`
   - `boot/initrd.tar`
   - `boot/limine/limine.conf`
   - `boot/limine/limine-uefi-cd.bin`
   - `cyber.rw` (din Release, în rădăcina stick-ului, lângă dosarele `EFI` și `boot`) — îi dă voie sistemului să scrie pe
     stick; fără el, comanda de actualizare (secțiunea 7) refuză. Stick-urile scrise cu varianta A îl au deja.
3. Gata: stick-ul pornește ca la varianta A, iar ce mai ai pe el rămâne neatins.

## 4. Pornește calculatorul de pe stick (ThinkPad)
1. **Secure Boot trebuie oprit** (Cyber Universe nu e semnat de Microsoft): la pornire apasă **F1** → *Security* →
   *Secure Boot* → *Off* → **F10** (salvează și ieși). Îl poți porni la loc oricând.
   **Atenție (Lenovo, 21 oct):** cu Secure Boot pornit **nu apare nicio eroare** — după ce alegi stick-ul, ecranul se înnegrește
   ~1 secundă și revii în meniul de boot. Dacă vezi asta, oprește Secure Boot.
2. Bagă stick-ul, pornește calculatorul și apasă repetat **F12** → alege stick-ul USB din listă.

## 5. Ce ar trebui să vezi
- câteva secunde de text (jurnalul nucleului), apoi **desktopul** (fundal albastru-închis, „Cyber Universe”, bara jos cu
  **Start** și ceasul); mouse-ul (USB sau al laptopului) mișcă săgeata;
- încearcă: **Start → terminal**, apoi scrie în fereastră `help`, `ls /`, `mem`, `lspci`;
- **Esc** pe desktop îl închide și apare consola text cu promptul **`cyber>`** (aceleași comenzi; `desktop` îl repornește).

## 6. Ce îmi trimiți înapoi
- o **poză cu ecranul** (mai ales dacă se oprește: ultimele rânduri contează);
- modelul calculatorului; dacă tastatura merge; ce scrie la `lspci`.

Notez totul în `docs/HARDWARE.md` și repar ce nu merge.

## 7. Actualizarea din Cyber Universe (P6-08, fără Windows)
Când stick-ul merge o dată, versiunile noi le poate lua singur, prin internet (cablu de rețea):
1. O singură dată, în Windows: pune pe stick, în rădăcină, fișierul **`github.token`** — un singur rând, un token GitHub
   cu drept de **citire** pe depozitul `cyber-universe` (depozitul e privat). Pe GitHub: poza ta → *Settings* →
   *Developer settings* → *Personal access tokens* → *Fine-grained tokens* → *Generate new token*; la *Repository access*
   alegi doar `cyber-universe`, la *Permissions* → *Contents*: **Read-only**. Tokenul nu apare niciodată pe ecran.
   Verifică și că pe stick e `cyber.rw` (secțiunea 3).
2. În Cyber Universe: **Start → settings → Actualizeaza** (sau, în terminal, `run actualizeaza`).
3. Aștepți mesajul **„gata - stick-ul are acum os-…”**, apoi repornești calculatorul. Dacă scrie „nimic schimbat pe
   stick”, stick-ul a rămas exact cum era (pornește ca înainte) — trimite-mi o poză cu mesajul.

## Pentru Claude (tehnic)
- Varianta B merge pentru că firmware-ul UEFI caută `EFI/BOOT/BOOTX64.EFI` pe orice partiție FAT32, iar Limine își citește
  `boot/limine/limine.conf` și fișierele de pe aceeași partiție (`limine-uefi-cd.bin` nu e necesar la pornirea de pe stick,
  dar e copiat ca să fie lista identică cu cea testată).
- Imaginea: `tools/build.sh --usb` → `build/cyber-universe-usb.img` (GPT, o partiție de sistem EFI FAT32 de 62 MiB cu
  `EFI/BOOT/BOOTX64.EFI` (Limine), `boot/limine/limine.conf`, `boot/kernel`, `boot/initrd.tar`, `cyber.rw`).
- Actualizarea: `/bin/actualizeaza` (vezi `user/README.md`); fișierele noi se scriu ca `.nou`, se citesc înapoi, apoi
  se pun în locul celor vechi prin redenumire (apelul de sistem `rename`, doar FAT32) și `sync` (SYNCHRONIZE CACHE).
- Test automat: scenariul `usb` — QEMU pornește de pe imagine atașată ca stick USB (XHCI + usb-storage) și nucleul
  verifică faptul că a fost încărcat de pe disc, nu de pe CD.
