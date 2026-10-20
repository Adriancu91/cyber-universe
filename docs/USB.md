# Cyber Universe pe un stick USB

Ghid pentru a porni Cyber Universe pe un calculator adevărat (de exemplu ThinkPad L15 Gen 2).
Nu se șterge nimic de pe calculator: sistemul rulează doar de pe stick și din memorie.

## 1. Ce îți trebuie
- un stick USB de cel puțin 128 MB (tot ce e pe el se șterge!);
- programul **balenaEtcher** (https://etcher.balena.io) sau **Rufus** (https://rufus.ie) pe Windows.

## 2. Descarcă imaginea
1. Pe GitHub, în depozitul `cyber-universe`, apasă **Actions**.
2. Alege ultima rulare **CI** cu bifă verde.
3. Jos, la **Artifacts**, descarcă **stick-usb** și dezarhivează-l: vei avea `cyber-universe-usb.img`.

## 3. Scrie imaginea pe stick
- **balenaEtcher:** „Flash from file” → alegi `cyber-universe-usb.img` → „Select target” → stick-ul → „Flash”.
- **Rufus:** alegi stick-ul, apoi fișierul `.img`; dacă te întreabă, alege **modul DD** (scriere ca imagine).

## 4. Pornește calculatorul de pe stick (ThinkPad)
1. **Secure Boot trebuie oprit** (Cyber Universe nu e semnat de Microsoft): la pornire apasă **F1** → *Security* →
   *Secure Boot* → *Off* → **F10** (salvează și ieși). Îl poți porni la loc oricând.
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

## Pentru Claude (tehnic)
- Imaginea: `tools/build.sh --usb` → `build/cyber-universe-usb.img` (GPT, o partiție de sistem EFI FAT32 de 62 MiB cu
  `EFI/BOOT/BOOTX64.EFI` (Limine), `boot/limine/limine.conf`, `boot/kernel`, `boot/initrd.tar`).
- Test automat: scenariul `usb` — QEMU pornește de pe imagine atașată ca stick USB (XHCI + usb-storage) și nucleul
  verifică faptul că a fost încărcat de pe disc, nu de pe CD.
