# Blocaje

> Probleme pe care Claude Code nu le-a rezolvat după 3 încercări diferite, sau care au nevoie de Adrian.
> Când o problemă se rezolvă, mută intrarea la secțiunea „Rezolvate” cu data și soluția.

## Active

### B-001 · P3-09 · 2026-10-20
- **Problema:** testul pe calculatoare reale cere acces fizic la ThinkPad L15 Gen 2 și la încă un calculator.
- **Am încercat:** nu e o problemă tehnică — tot ce se poate face fără Adrian e gata: imaginea de stick (P3-04, artefactul
  CI `stick-usb`, testată în QEMU ca stick USB), ghidul `docs/USB.md`, lista de verificare din `docs/HARDWARE.md`.
- **Eroarea exactă:** —
- **Ce ar debloca:** Adrian scrie imaginea pe un stick, pornește calculatoarele de pe el și trimite poze cu ecranul.
- **Nevoie de Adrian:** da — urmează pașii din `docs/USB.md` pe ThinkPad (și pe un al doilea calculator) și trimite poze.

### B-002 · P5-06 · 2026-10-20
- **Problema:** HTTPS (TLS 1.3) are nevoie de criptografie (SHA-256, HMAC/HKDF, X25519, ChaCha20-Poly1305 sau AES-GCM,
  verificarea semnăturilor din certificate). Planul cere decizia lui Adrian înainte de cod.
- **Am încercat:** nu e o problemă tehnică. Decizia din hub (2026-10-20 19:40, „OS de la zero, fără cod copiat și fără
  alte licențe”) arată spre varianta „scrisă de noi”, dar nu spune explicit nimic despre criptografie.
- **Eroarea exactă:** —
- **Ce ar debloca:** un „da” / „nu” de la Adrian.
- **Nevoie de Adrian:** da — „Criptografia pentru HTTPS o scriem noi, după standardele publice, verificată cu exemplele
  oficiale (fără audit de securitate făcut de specialiști)? Recomandarea mea: da, e în spiritul deciziei tale din 19:40.”

### B-003 · P5-07 · 2026-10-20
- **Problema:** suita office ca modul în Cyber Universe cere codul suitei, care e doar pe laptop (decizia din hub
  2026-10-20: suita se termină și se testează de oameni întâi; codul rămâne pe laptop până decide Adrian).
- **Am încercat:** am citit în hub `stare/suita-office.md` (Electron + React + TypeScript, ținta în 1–2 săptămâni).
  Portarea ar însemna rescrierea în Rust pentru biblioteca grafică proprie — de planificat după ce suita e gata.
- **Eroarea exactă:** —
- **Ce ar debloca:** suita își atinge ținta, iar Adrian decide cum ajunge codul (sau doar formatul fișierelor) la OS.
- **Nevoie de Adrian:** mai târziu, nu acum.

<!-- Șablon:
### B-001 · P0-01 · 2026-10-27
- **Problema:** ...
- **Am încercat:** 1) ... 2) ... 3) ...
- **Eroarea exactă:** ```...```
- **Ce ar debloca:** ...
- **Nevoie de Adrian:** da/nu — ce anume, în cuvinte simple
-->

## Rezolvate

_(niciunul)_
