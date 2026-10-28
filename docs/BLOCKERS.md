# Blocaje

> Probleme pe care Claude Code nu le-a rezolvat după 3 încercări diferite, sau care au nevoie de Adrian.
> Când o problemă se rezolvă, mută intrarea la secțiunea „Rezolvate” cu data și soluția.

## Active

### B-005 · CI și Release (construirea în sesiune: rezolvată) · 2026-10-28
- **Problema:** din 28 oct 14:33 repo-ul `Adriancu91/cyber-gpu` e privat. OS-ul îl folosește ca dependență (`kernel/Cargo.toml`,
  `user/Cargo.toml`, fixat pe `dac2e32`), iar sesiunea programată a OS-ului are acces doar la `cyber-universe`, `cyber-hub`, `cyberspace`.
  Fără el **nu se poate construi nimic** (nici nucleul, nici programele), deci nici testele QEMU, nici C-31.
  Și CI-ul va pica la următorul commit cu cod: `github.token` din Actions vede doar `cyber-universe`.
- **Am încercat:** 1) `git ls-remote https://github.com/Adriancu91/cyber-gpu` → „could not read Username”; 2) API-ul GitHub →
  „GitHub access to this repository is not enabled for this session” (403); 3) `tools/build.sh` → cargo: „revision dac2e32… not found”.
- **Eroarea exactă:** ```failed to clone into: /root/.cargo/git/db/cyber-gpu-…  revision dac2e32c33c7a5f88a3f5ef18dde45375bae5fb9 not found```
- **Ce ar debloca (una din):** (a) Adrian adaugă repo-ul `cyber-gpu` la sarcina programată a OS-ului (în aplicația Claude, la depozitele
  sesiunii) — rezolvă sesiunile; pentru CI mai trebuie un token doar-citire pentru `cyber-gpu` pus ca secret în `cyber-universe`;
  (b) copiem bibliotecile Cyber-GPU (același autor, aceeași licență proprietară) direct în repo-ul OS-ului, actualizate de sesiunea OS
  din ce publică Cyber-GPU în hub — cere „da”-ul lui Adrian (schimbă regula „nu modifici codul altui proiect”); (c) `cyber-gpu` din nou public.
- **Rezolvat pentru sesiuni (28 oct 19:00, centrul de comandă):** copia Git a `cyber-gpu` în hub (`integrare/cyber-gpu/snapshot/`,
  actualizată de Cyber-GPU, C-34). `tools/cyber-gpu-local.sh` (chemat și de `tools/env-check.sh --install`) o face oglindă locală;
  verificat 28 oct 17:20 UTC: build + teste QEMU merg. P6-31 făcut așa.
- **Rămâne (CI / Release):** CI-ul nu pornește (fără minute) și nici n-ar putea descărca `cyber-gpu`; fără CI nu apar Release-uri pentru stick.
- **Nevoie de Adrian:** da — cum vrea să ajungă stick-ul nou la el: (1) repo public din nou → CI + Release gratuite (dar CI-ul ar mai avea
  nevoie de acces la `cyber-gpu`: și acela public, sau un token doar-citire ca secret); (2) minute plătite (doar cu „da”-ul lui) + același token;
  (3) imaginea construită în sesiune pusă undeva privat de unde o descarcă (de stabilit unde).
- **CI (28 oct 12:51 UTC, run 198):** GitHub Actions nu mai pornește („recent account payments have failed or your spending limit
  needs to be increased”) — repo privat, minute terminate. Nu se plătește nimic fără acordul lui Adrian (DECIZII 14:33); fără CI nu apar
  Release-uri noi pentru stick (ultimul `os-197`). Ce ar debloca: repo public din nou, minute plătite (doar cu „da”-ul lui), sau
  Release-uri făcute din sesiune (de discutat, după B-005).

- **Actualizare 28 oct 21:45 UTC (după decizia hub 23:35 „Adrian a plătit GitHub, Actions repornite”):** CI pornește din nou, dar rularea 201
  a picat în 25 s: `revision 83139736… not found` / `git fetch failed for https://github.com/Adriancu91/cyber-gpu` (0/81 teste, niciun Release).
  P6-33: CI-ul folosește acum secretul `CYBER_GPU_TOKEN` (`tools/ci-cyber-gpu-acces.sh`); fără el se oprește cu un singur mesaj clar.
- **Pașii pentru Adrian (5 minute, merge și de pe telefon, în browser):**
  1. github.com → poza ta → **Settings** → **Developer settings** → **Personal access tokens** → **Fine-grained tokens** → **Generate new token**.
  2. Nume: `cyber-universe-ci`; expirare: 1 an; **Repository access: Only select repositories → `cyber-gpu`**;
     **Permissions → Repository → Contents: Read-only** (nimic altceva). **Generate token** și copiază-l.
  3. github.com/Adriancu91/cyber-universe → **Settings** → **Secrets and variables** → **Actions** → **New repository secret**:
     nume `CYBER_GPU_TOKEN`, valoarea = token-ul copiat → **Add secret**.
  4. Gata: sesiunea următoare repornește CI-ul; dacă e verde, apare Release-ul nou (cu `gpu vbios --detalii`).

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

### B-004 · CI / Release · 2026-10-23
- **Problema:** GitHub Actions nu mai pornește joburile din 2026-10-23 11:03 UTC (runs 178–184, ultima verificare 15:17 UTC): „failure” în 2–4 s, fără log
  (descărcarea logului dă 404). Aceeași problemă e raportată în hub de Cyber-GPU (de la 13:46 ora României).
- **Am încercat:** 1) verificat joburile: `build-and-test` se termină în 2 s, fără pași rulați; 2) log-ul jobului nu există;
  3) testele locale trec (68). Cauza probabilă: minutele gratuite GitHub Actions ale contului s-au terminat (repo-uri private).
- **Eroarea exactă:** `failed to download logs: HTTP 404` (jobul nu a primit calculator).
- **Ce ar debloca:** Adrian verifică pe GitHub → Settings → Billing and plans → Actions (minute / limită de cheltuieli), sau face
  repo-ul public, sau așteaptă resetarea lunară a minutelor.
- **Nevoie de Adrian:** da — 2 minute de pe telefon, când poate (punctul 1 din PANOU-ul hub-ului e aceeași cauză).
- **Rezolvat 2026-10-23 (comanda C-15):** cauza exactă = facturarea GitHub: „The job was not started because recent account
  payments have failed or your spending limit needs to be increased” — minutele gratuite pentru repo-uri private s-au terminat.
  Adrian a făcut repo-ul **public** (19:05 ora României) → Actions e gratuit; prima rulare după (run 186, `9f67d7b`) a pornit normal.
  CI-ul e și mai econom acum (P6-18): fără rulări la commit-uri doar cu documente, rularea veche anulată de una nouă, stres săptămânal.


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

