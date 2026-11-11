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

**Cât timp CI-ul e oprit (de la 28 oct, P6-32 / C-35):** sesiunea construiește și testează stick-ul ea însăși și pune arhiva
`stick-cyber-universe-os-<număr>.zip` (numere de la 1001, ca să nu se confunde cu Release-urile CI) în hub-ul privat, la
`integrare/cyber-universe/stick/`, cu `VERSIUNE.txt` (commit, ora, suma SHA-256). În arhivă sunt aceleași fișiere ca în Release.
Centrul de comandă ți-o trimite. (`actualizeaza` de pe stick caută tot Release-urile GitHub, deci nu vede aceste arhive.)

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

## 6b. Rețeaua pentru teste (partajarea din Windows)
Fără router: un cablu de rețea între laptopul de test și un calculator cu Windows care are internet (de exemplu prin Wi-Fi).
În Windows: *Panou de control → Rețea → Modificare setări adaptor* → clic dreapta pe conexiunea Wi-Fi → *Proprietăți* → fila
*Partajare* → bifezi „Se permite altor utilizatori de rețea să se conecteze prin conexiunea la Internet a acestui computer” și alegi
conexiunea **Ethernet**. Cyber Universe primește o adresă **192.168.137.x** (`net` o arată) și iese pe internet prin Windows.

## 7. Actualizarea din Cyber Universe (P6-08, fără Windows)
Când stick-ul merge o dată, versiunile noi le poate lua singur, prin internet (cablu de rețea):
1. **Nu mai e nevoie de token** (din 2026-10-23 depozitul `cyber-universe` e public): `actualizeaza` întreabă GitHub fără token.
   Dacă pe stick există deja `github.token`, e folosit în continuare (nu strică). Tokenul rămâne necesar **doar pentru
   `gh-trimite`** (trimiterea de fișiere spre GitHub, care cere drept de scriere). Pentru `gh-trimite`: pe GitHub, poza ta →
   *Settings* → *Developer settings* → *Personal access tokens* → *Fine-grained tokens* → *Generate new token*, doar depozitul
   respectiv, *Contents*: **Read and write**; fișierul `github.token` (un rând) în rădăcina stick-ului. Tokenul nu apare pe ecran.
   Verifică și că pe stick e `cyber.rw` (secțiunea 3).
2. În Cyber Universe: **Start → settings → Actualizeaza** (sau, în terminal, `run actualizeaza`).
3. Aștepți mesajul **„gata - stick-ul are acum os-…”**, apoi repornești calculatorul. Dacă scrie „nimic schimbat pe
   stick”, stick-ul a rămas exact cum era (pornește ca înainte) — trimite-mi o poză cu mesajul.
4. **Prima pornire după actualizare e o încercare** (P6-50): dacă versiunea nouă nu merge un minut, la pornirea următoare
   sistemul pune singur la loc versiunea veche (`docs/ACTUALIZARE.md`).
5. **Git simplu (P6-52):** `run gh-ia <proprietar/depozit> <cale> <fișier>` ia un fișier de pe GitHub pe stick (ex.
   `run gh-ia Adriancu91/cyber-universe docs/STATE.md /usb/STATE.md`); `run gh-trimite <fișier> <proprietar/depozit> <cale>`
   îl trimite înapoi. Pentru depozitele publice `gh-ia` nu are nevoie de token.
6. **Rapoartele pleacă singure în hub (P6-61, comanda C-84):** dacă pe stick e `github.token` cu drept de scriere pe
   **`Cyber-hub`** (la pasul 1: *Only select repositories* → `Cyber-hub`, *Contents*: **Read and write**, nimic altceva), după
   `raport` și după `gpu lant` sistemul trimite singur rapoartele noi din `/usb/raport` în hub, la
   `rezultate/os-automat/<data>/`. Comanda `trimite` le trimite oricând. Ce s-a trimis e scris în `/usb/raport/trimise.txt`
   (fiecare raport pleacă o singură dată). Fără token sau fără rețea apare un mesaj, iar rapoartele rămân pe stick, ca până acum.
7. **Hub-ul citit din OS (P6-61):** `hub` arată comenzile tale pentru OS și stările lor; `hub stare kronos` (sau alt proiect)
   arată starea proiectului; `hub decizii` ultimele decizii. Doar citire; merge cu același `github.token` (sau fără, dacă hub-ul e public).
8. **Întrebări pentru Kronos (P6-61):** `run kronos --intreaba Ce este un sistem de operare?` — Kronos răspunde pe un rând și ține
   minte conversația în `/usb/kronos/conversatie.txt` (întrebarea următoare vede ultimele schimburi). Modelul e încă unul care
   continuă text, nu un asistent antrenat: răspunsurile pot fi ciudate.
   **Conversație (P6-65):** `run kronos --conversatie` — încarcă modelul (#513) o singură dată, apoi scrii întrebare după întrebare
   (`Tu: `); o întrebare lungă continuă pe rândul următor dacă pui `\` la capăt; rând gol sau `gata` = sfârșit.
9. **Git simplu pe ramuri (P6-61):** `run gh-ia --lista Adriancu91/cyber-universe docs` arată fișierele unui dosar;
   `run gh-trimite --ramura os/nota /usb/nota.txt Adriancu91/cyber-universe note/nota.txt` pune fișierul pe ramura `os/nota`
   (niciodată pe `main`; tokenul trebuie să aibă drept de scriere și pe acel depozit).
10. **Autotestul (P6-61):** `test tot` verifică procesorul, ceasul, stick-ul, rețeaua și placa video (doar citiri) și scrie
    `PASS` sau `FAIL` pe fiecare rând, plus un raport `raport/test-tot-….txt` pe stick (trimis singur în hub dacă e tokenul).
11. **Ora din rețea (P6-61):** `ora` arată ora exactă luată din internet (UTC și ora României) și cât de departe e ceasul
    calculatorului. Nu schimbă ceasul (îl folosește și Windows).
12. **Fișierele de pe stick:** `run files /usb` (sau Start → files → `usb/`) — copiere, ștergere, deschidere în editor.
    Setările (fus orar, tastatură) se salvează acum pe stick, în `config/settings.toml`, și pornesc cu el pe orice calculator.
13. **Modelul Kronos #513 pe stick (P6-60, comanda C-82):** pe stick, în dosarul `kronos`, pui fișierul `kronos_513_q8_0.kgr`
    (105,7 MiB) și, lângă el, `kronos_513_q8_0.kgr.sha256`. Le iei din Release-ul `kronos-kgr1-513` al depozitului `cyberspace`
    (când există; până atunci sunt pe workstation în `C:\Kronos\cyberspace\lucru\export\`). `run kronos --control` trebuie să
    scrie „CONTROL: TRECE (#513 …)”. Dacă #513 lipsește, `kronos` folosește singur #477, apoi #459, dacă le ai pe stick.
14. **Comenzi din hub (P6-64, comanda C-87):** în hub, în fișierul `integrare/cyber-universe/os-comenzi.md`, scrii (sau îi
    ceri centrului de comandă să scrie) rânduri ca `- [ ] K-1: test tot`. În OS, `hub ruleaza` le ia, rulează **doar** comenzile
    de pe lista albă (`test tot`, `raport`, `trimite`, `hw`, `log …`, `gpu tur`, `run kronos --profil …`, `run kronos --control`),
    fiecare o singură dată (ID-urile rulate stau pe stick în `hub/facute.txt`), și trimite raportul în hub
    (`rezultate/os-automat/<data>/hub-….txt`). `gpu lant`, `--gpu`, `--scrie`, `reboot`, ștergerile și actualizările sunt
    refuzate: acelea le pornești doar tu, la tastatură, cu „DA”. `hub ruleaza --arata` doar arată ce ar rula.
    Ca să ruleze singur la fiecare pornire: pe stick, în dosarul `hub`, un fișier gol `la-pornire`. Are nevoie de `github.token`.
15. **„Agentul de comenzi” — `hub asculta` (P6-73, comanda C-104):** la fel ca `hub ruleaza`, dar în buclă: la fiecare
    2 minute (sau `hub asculta --la 5` = la 5 minute) se uită în hub, rulează comenzile **noi** de pe lista albă (fiecare o
    singură dată) și pune răspunsul fiecăreia într-un fișier separat în hub: `integrare/cyber-universe/raspunsuri/K-<nr>.txt`
    (o copie rămâne pe stick în `hub/raspunsuri/`). **Esc** oprește bucla. Lista albă e aceeași ca la punctul 14 —
    `gpu lant`, `--scrie`, „DA”, `reboot`, ștergerile rămân **refuzate** (doar tu, la tastatură).
    Ca să pornească singur: pe stick, în dosarul `hub`, un fișier `asculta` (gol = la 2 minute, sau cu un număr, de ex. `5`);
    atunci bucla merge în fundal și o oprești cu `hub opreste`.
    **Tokenul (o singură dată, 5 minute, pe Windows):** github.com → poza ta → *Settings* → *Developer settings* →
    *Personal access tokens* → *Fine-grained tokens* → *Generate new token*. Nume: `cyber-os-hub`; *Expiration*: 90 de zile;
    *Repository access*: **Only select repositories** → **doar `Cyber-hub`**; *Permissions* → *Repository permissions* →
    *Contents*: **Read and write** (nimic altceva). *Generate*, copiezi tokenul și îl pui ca singur rând în fișierul
    `github.token` din rădăcina stick-ului. **Tokenul stă DOAR pe stick** — nu-l pui niciodată în depozit, în hub sau în chat
    (OS-ul nu-l afișează și îl ascunde din rapoarte).

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

## 7. Hub-uri și alegerea stick-ului (P6-14, P6-15 — 2026-10-23)
- Dispozitivele din spatele unui hub (inclusiv hub-urile interne ale calculatorului) sunt găsite acum: în `lsusb` apar ca
  `port 26.3` = portul 3 al hub-ului de pe portul 26. `log usb` arată fiecare hub („hub 26: …, N porturi”) și fiecare port conectat.
- `/usb` = stick-ul de pe care a pornit OS-ul: dintre toate stick-urile, cel care are `cyber.rw` și `VERSIUNE` (orice stick
  scris din Release le are). Cititoarele de carduri fără card („fara mediu”) sunt sărite.

## 8. Stick-ul de test GPU + firmware-ul NVIDIA (C-74, 5 nov 2026)
Stick-ul de test (`cyber-universe-gpu-scrie-usb.img`, scris cu Rufus) nu are loc pentru firmware-ul NVIDIA (fișierul mare are ~64 MB,
partiția are ~62 MB) și Windows nu-i dă literă. Nu mai e nevoie să copiezi nimic pe el:
1. Bagă **amândouă** stick-urile: stick-ul de test **și** stick-ul normal pe care ai deja `firmware\nvidia\ga102\gsp\` (cele 4 fișiere).
2. Pornește de pe stick-ul de test (F8 / meniul de pornire). Sistemul alege singur stick-ul de pe care a pornit (`/usb`, are `cyber.rw` și
   `VERSIUNE`) și îl montează pe celălalt la `/usb2`, **doar citire**. Pe ecran (și în `log`) apare:
   `fs: /usb2: al doilea stick, cu dosarul firmware/nvidia/ga102/gsp (doar citire)`.
3. `gpu firmware` verifică fișierele; dacă vin de pe al doilea stick, scrie `gpu: firmware: /usb2/... citit de pe al doilea stick`.
   `gpu lant` (I7) le citește la fel. Firmware-ul nu intră niciodată în repo sau în imagini.
Dacă `ls /usb2` nu arată nimic: stick-ul normal nu are dosarul `firmware` în rădăcină → copiază-l din Windows pe stick-ul normal.
