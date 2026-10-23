# Blocaje

> Probleme pe care Claude Code nu le-a rezolvat după 3 încercări diferite, sau care au nevoie de Adrian.
> Când o problemă se rezolvă, mută intrarea la secțiunea „Rezolvate” cu data și soluția.

## Active

### B-003 · P5-07 · 2026-10-20
- **Problema:** suita office ca modul în Cyber Universe cere codul suitei, care e doar pe laptop (decizia din hub
  2026-10-20: suita se termină și se testează de oameni întâi; codul rămâne pe laptop până decide Adrian).
- **Am încercat:** am citit în hub `stare/suita-office.md` (Electron + React + TypeScript, ținta în 1–2 săptămâni).
  Portarea ar însemna rescrierea în Rust pentru biblioteca grafică proprie — de planificat după ce suita e gata.
- **Eroarea exactă:** —
- **Ce ar debloca:** suita își atinge ținta, iar Adrian decide cum ajunge codul (sau doar formatul fișierelor) la OS.
- **Nevoie de Adrian:** mai târziu, nu acum.

### B-004 · CI / Release · 2026-10-23
- **Problema:** GitHub Actions nu mai pornește joburile din 2026-10-23 11:03 UTC (runs 178–184, ultima verificare 15:17 UTC): „failure” în 2–4 s, fără log
  (descărcarea logului dă 404). Aceeași problemă e raportată în hub de Cyber-GPU (de la 13:46 ora României).
- **Am încercat:** 1) verificat joburile: `build-and-test` se termină în 2 s, fără pași rulați; 2) log-ul jobului nu există;
  3) testele locale trec (68). Cauza probabilă: minutele gratuite GitHub Actions ale contului s-au terminat (repo-uri private).
- **Eroarea exactă:** `failed to download logs: HTTP 404` (jobul nu a primit calculator).
- **Ce ar debloca:** Adrian verifică pe GitHub → Settings → Billing and plans → Actions (minute / limită de cheltuieli), sau face
  repo-ul public, sau așteaptă resetarea lunară a minutelor.
- **Nevoie de Adrian:** da — 2 minute de pe telefon, când poate (punctul 1 din PANOU-ul hub-ului e aceeași cauză).

<!-- Șablon:
### B-001 · P0-01 · 2026-10-27
- **Problema:** ...
- **Am încercat:** 1) ... 2) ... 3) ...
- **Eroarea exactă:** ```...```
- **Ce ar debloca:** ...
- **Nevoie de Adrian:** da/nu — ce anume, în cuvinte simple
-->

## Rezolvate

### B-001 · P3-09 · 2026-10-20
- **Problema:** testul pe calculatoare reale cere acces fizic la ThinkPad L15 Gen 2 și la încă un calculator.
- **Am încercat:** nu e o problemă tehnică — tot ce se poate face fără Adrian e gata: imaginea de stick (P3-04, artefactul
  CI `stick-usb`, testată în QEMU ca stick USB), ghidul `docs/USB.md`, lista de verificare din `docs/HARDWARE.md`.
- **Eroarea exactă:** —
- **Ce ar debloca:** Adrian scrie imaginea pe un stick, pornește calculatoarele de pe el și trimite poze cu ecranul.
- **Nevoie de Adrian:** da — urmează pașii din `docs/USB.md` pe ThinkPad (și pe un al doilea calculator) și trimite poze.
- **Rezolvat 2026-10-21 22:05 (comanda C-2):** Adrian a pornit stick-ul pe un Lenovo ThinkPad cu AMD Ryzen (altul decât cel al
  suitei): cu Secure Boot oprit ajunge la desktop; ecran, touchpad, clic, tastatura internă, Demo și Editor merg; `lspci`, `lsusb`, `mem`
  merg. Nu merge: placa de rețea (Realtek `10ec:8168`, fără driver → P6-01). Rămân de verificat la testul 2 (P3-11): `ls /disk`,
  `ls /usb`, consola Esc, tastatură USB externă, timpul de pornire. Al doilea calculator real: mai târziu (ThinkPad-ul L15 e al suitei).

### B-002 · P5-06 · 2026-10-20
- **Problema:** HTTPS (TLS 1.3) are nevoie de criptografie (SHA-256, HMAC/HKDF, X25519, ChaCha20-Poly1305 sau AES-GCM,
  verificarea semnăturilor din certificate). Planul cere decizia lui Adrian înainte de cod.
- **Am încercat:** nu e o problemă tehnică. Decizia din hub (2026-10-20 19:40, „OS de la zero, fără cod copiat și fără
  alte licențe”) arată spre varianta „scrisă de noi”, dar nu spune explicit nimic despre criptografie.
- **Eroarea exactă:** —
- **Ce ar debloca:** un „da” / „nu” de la Adrian.
- **Nevoie de Adrian:** da — „Criptografia pentru HTTPS o scriem noi, după standardele publice, verificată cu exemplele
  oficiale (fără audit de securitate făcut de specialiști)? Recomandarea mea: da, e în spiritul deciziei tale din 19:40.”
- **Rezolvat 2026-10-20 22:35:** Adrian a răspuns „da” → D-011; P5-06 împărțit în P5-10 … P5-13.

