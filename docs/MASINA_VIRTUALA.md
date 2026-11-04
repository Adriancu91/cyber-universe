# Mașina virtuală Cyber Universe pe Windows (F1, comanda C-68)

Cyber Universe pornește într-o **mașină virtuală QEMU** pe workstation, cât Kronos se antrenează în Windows.
Dacă sistemul se blochează, se repornește doar mașina — Windows și Kronos nu sunt atinși.

## Ce e în fiecare Release (începând cu P6-48)
- `cyber-universe-vm.qcow2` — discul mașinii (1 GiB, crește doar cât scrie sistemul; în Release are câțiva MB).
  Are aceleași fișiere ca stick-ul, plus `cyber.rw` (sistemul poate scrie: `actualizeaza`, rapoarte în `raport/`) și `VERSIUNE`.
  Mașina îl vede **ca pe un stick USB**, deci `/usb` merge exact ca pe calculatorul real.
- `vm-pornire.ps1` — scriptul care pornește / oprește mașina.

## Limitele (C-68, C-70)
- **Procesor:** implicit 8 nuclee (`-Nuclee`, cel mult 16). Restul rămân lui Kronos.
- **Memorie:** implicit 16 GB (`-MemorieGB`, cel mult 32).
- **Placa video NU e dată mașinii.** Scriptul nu are nicio opțiune pentru asta; testele cu placa se fac doar pe hardware real, cu Adrian.
- Rețea: placa virtuală e1000e, prin rețeaua lui Windows (mașina iese spre internet, nu e vizibilă din rețea).
- Remote (F2, P6-49): portul 2323 al OS-ului se vede pe Windows la `127.0.0.1:2323` (`-PortRemote` îl schimbă), doar local.
  Cu `remote.cheie` (și opțional `remote.porneste`) pe discul mașinii: `py remote-client.py 127.0.0.1 -k remote.cheie` (ghid: `docs/REMOTE.md`).

## Pregătire (o singură dată, centrul de comandă — C-70)
1. QEMU pentru Windows, descărcat de pe pagina oficială (qemu.org → Download → Windows), instalat în `C:\Program Files\qemu`
   (calea implicită). Firmware-ul UEFI (`share\edk2-x86_64-code.fd`) vine odată cu QEMU.
2. Din Release-ul ales (GitHub → Releases): `cyber-universe-vm.qcow2` și `vm-pornire.ps1` într-un dosar, de exemplu `C:\cyber\vm`.

## Comenzi (PowerShell, în dosarul cu scriptul)
Dacă Windows refuză scripturile, se pune în față: `powershell -ExecutionPolicy Bypass -File`.

| Comandă | Ce face |
|---|---|
| `.\vm-pornire.ps1 porneste` | Pornește mașina (prima dată copiază discul în `%USERPROFILE%\CyberUniverseVM`). |
| `.\vm-pornire.ps1 stare` | Spune dacă e pornită și arată ultimele 8 rânduri din jurnalul serial. |
| `.\vm-pornire.ps1 opreste` | Oprește mașina curat (prin monitorul QEMU); forțat după 15 s dacă nu răspunde. |
| `.\vm-pornire.ps1 reporneste` | Oprește + pornește (jurnal serial nou). |
| `.\vm-pornire.ps1 supraveghe` | **Ceasul de siguranță:** pornește mașina și o verifică la 30 s; o repornește dacă s-a oprit singură, dacă scrie `PANIC:` sau dacă QEMU nu mai răspunde. Cel mult 3 reporniri pe oră; a patra oară se oprește și scrie de ce în `jurnal\supraveghere.txt`. |
| `.\vm-pornire.ps1 disc-nou` | Pune discul din Release-ul nou (de lângă script sau `-DiscNou cale`); cel vechi rămâne ca `.vechi`. Doar cu mașina oprită. |

Opțiuni utile: `-Nuclee 4`, `-MemorieGB 8`, `-FaraEcran` (fără fereastră), `-Accel whpx`, `-MinuteMaxim 120` (la `supraveghe`: oprire după 2 ore).

## Unde sunt jurnalele
În `%USERPROFILE%\CyberUniverseVM\jurnal\`:
- `serial-AAAALLZZ-HHMMSS.txt` — tot ce scrie sistemul la pornire (un fișier pe pornire);
- `vm.txt` — ce a făcut scriptul; `supraveghere.txt` — repornirile ceasului de siguranță;
- `ultima-comanda.txt` — comanda QEMU exactă; `qemu-erori.txt` — mesajele QEMU.

## Viteza (accelerarea)
- Implicit `tcg` = emulare: merge oriunde, dar e mai lentă decât calculatorul real (bună pentru dezvoltare, nu pentru măsurători de viteză).
- `-Accel whpx` folosește accelerarea Windows (cere funcția Windows „Windows Hypervisor Platform”). Dacă nu merge, QEMU trece singur la `tcg`.
  **De încercat o dată cu Adrian**; vitezele reale ale lui Kronos se măsoară tot pe hardware real.

## Cum e testat
`tools/vm-image.sh` face discul; `tools/vm-test.sh` (în CI, cu PowerShell 7) rulează scriptul: `porneste` → sistemul montează `/usb` cu scriere și ia adresă prin DHCP → `stare` = pornită → a doua pornire refuzată → `opreste` închide QEMU → `supraveghe`: QEMU omorât din afară e repornit singur.
