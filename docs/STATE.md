# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## OPRIT: necesită analiza lui Adrian
Trei sesiuni programate la rând (2026-10-20 23:41, 2026-10-21 02:41 și 05:41 UTC) n-au terminat nicio sarcină, pentru că
**nu mai există sarcini libere** (nu e un eșec tehnic). Sesiunile următoare NU încep sarcini noi până când Adrian:
- răspunde la întrebarea din hub (închiderea Fazei 5 fără P5-07/P5-08, ca să pornească Faza 6), sau
- trimite o comandă nouă pentru `cyber-universe` în `COMENZI.md` din hub, sau
- trimite rezultatele testului de pe stick (B-001 → P3-09, P3-10).
O sesiune care găsește una dintre acestea șterge secțiunea „OPRIT” și reia lucrul normal.

## Ultima sesiune
- **2026-10-21, 05:41–05:50 UTC (08:41 ora României), sesiune programată:** a treia la rând fără sarcină liberă și fără
  comandă nouă în `COMENZI.md`; nicio decizie nouă în `DECIZII.md`; CI verde (rularea pentru `e628f2a`). Nu s-a schimbat
  cod; uneltele nu s-au instalat (nu era nimic de construit). S-a aplicat regula de oprire: „OPRIT” mai sus.

### Sesiunea precedentă fără lucru (a doua)
- **2026-10-21, 02:41–02:50 UTC (05:41 ora României), sesiune programată:** nicio sarcină liberă și nicio comandă nouă;
  CI verde (rularea 90, `9973b59`). Nu s-a schimbat cod.

### Prima sesiune fără lucru
- **2026-10-20, 23:41–23:50 UTC (02:41 ora României), sesiune programată:** nicio sarcină liberă și nicio comandă nouă
  pentru `cyber-universe` în `COMENZI.md` (hub); CI verde (rularea 89, `cd76b1e`). Nu s-a schimbat cod; uneltele nu s-au
  instalat (nu era nimic de construit). Întrebarea pentru Adrian (închiderea Fazei 5) rămâne deschisă.
  Prima sesiune la rând fără sarcină terminată (motiv: lipsa sarcinilor, nu un eșec).

### Sesiunea anterioară (cu lucru)
- **Data:** 2026-10-20, 20:41–22:25 UTC (23:41–01:25 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (6 sarcini):** P5-12 (ChaCha20-Poly1305), P5-13 (numere mari + semnături RSA PKCS#1 v1.5 și PSS),
  P5-14 (semnături ECDSA P-256), P5-15 (certificate X.509 + verificarea lanțului), P5-06 (client TLS 1.3, crate `tls/`),
  P5-16 (HTTPS în OS: `fetch https://…`). P5-13 inițial a fost împărțit în P5-13 … P5-15, iar P5-06 în P5-06 + P5-16
  (cu planuri), cum permiteau planurile lor.
- **Blocate (au nevoie de Adrian):** P3-09 (B-001, test pe un calculator real; P3-10 după el), P5-07 (B-003, suita office),
  P5-08 și P5-09 depind de P5-07. **Nu mai există nicio sarcină liberă** → sesiunea s-a oprit (CLAUDE.md §3).
- **Ce merge acum (rezumat pentru Adrian):** Cyber Universe poate descărca pagini prin **HTTPS**, cu criptografie scrisă
  integral de noi: `fetch https://nume/cale`. Certificatul site-ului e verificat (lanț până la o rădăcină de încredere,
  numele, data); unul greșit e refuzat. În emulator: o pagină de 18 KB în ~1,3 s.
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **41 de scenarii trecute** (nou: `https`); `crypto` are 108 verificări; `cd crypto && cargo test
  --release` și `cd tls && cargo test --release` (clientul nostru față de un server OpenSSL real) trec, și în CI.
  Verificări încrucișate cu OpenSSL, în afara repo-ului: 300 de mesaje ChaCha20-Poly1305, 116 semnături RSA, 200 ECDSA,
  lanțurile cu `openssl verify` — zero diferențe. CI verde pe toate commit-urile terminate (ultimele încă rulau la final).

## Următoarea sarcină
**Niciuna liberă.** Ce poate debloca (decizia lui Adrian, întrebarea e în hub):
1. Închiderea Fazei 5 fără P5-07/P5-08 (suita office și Kronos ca module se mută într-o fază ulterioară) → P5-09, apoi
   sarcinile Fazei 6 din `docs/ROADMAP.md`.
2. Sau sarcini noi aprobate prin `COMENZI.md` din hub (ex. rădăcinile reale de încredere — ISRG Root X1 etc. — ca
   `fetch https://` să meargă cu site-uri adevărate; azi `/etc/certs/` e gol în imaginea normală).
Când Adrian trimite rezultatele de pe stick (B-001): P3-09, apoi P3-10. Suita office gata (B-003): P5-07, P5-08.
Atenție: dacă 3 sesiuni la rând nu termină nimic, regula de oprire cere „OPRIT: necesită analiza lui Adrian” (aici motivul
e lipsa sarcinilor, nu un eșec).

## Mediu (verificat la P0-01, sesiune cloud)
| Unealtă | Versiune |
|---|---|
| rustc (nightly) | 1.101.0-nightly (2026-10-19) |
| cargo (stable) | 1.97.0 |
| qemu-system-x86_64 | 8.2.2 |
| git | 2.43.0 |
| xorriso | 1.5.6 |
| OVMF | /usr/share/OVMF/OVMF_CODE_4M.fd |

## Cum funcționează testele (important pentru sesiunile următoare)
- Scenariile sunt în `tests/scenarios.txt`: nume | feature-uri cargo | `success`/`failed` | rânduri așteptate (`;;`) | captură | argumente QEMU sau `keys=...`.
- Un scenariu care trebuie să „pice” (excepție, panică) se termină cu codul QEMU 35 și are `failed`.
- În build-ul de test nucleul rulează `run_tests()` din `main.rs` și se termină cu `qemu::finish_test()`.
- Funcții noi folosite doar de teste: `#[cfg(feature = "qemu-test")]`; API pentru mai târziu: `#[allow(dead_code)]` cu motiv.

## Avertismente pentru sesiunea următoare
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install` (acum instalează și dosfstools, mtools, fdisk).
- **Cyber Hub:** se clonează direct `https://github.com/Adriancu91/Cyber-hub` (fără add_repo); merge din sesiunile programate.
- Nu edita `tools/test.sh` cât rulează un test în fundal (bash citește scriptul pe bucăți).
- Discul de test (`build/disk.img`, 40 MiB) e atașat de QEMU ca SATA la orice rulare (`tools/qemu-common.sh`); în ISO intră
  doar cu feature-ul `test-disk` (Limine l-ar încărca ~13 s în emulare). `{BUILD}` în coloana QEMU din `scenarios.txt` = directorul build.
- Scenariile pot trimite taste (`keys=`) și comenzi de monitor QEMU (`mon=cmd:arg,cmd2`, `:` = spațiu). Când le ai pe
  amândouă, scrie `mon=` ÎNAINTEA lui `keys=` (keys ia restul rândului). Tastele pleacă primele, apoi comenzile.
- Rândurile așteptate nu pot conține `|` (e separatorul coloanelor din `scenarios.txt`).
- `disk=NUME.img` în coloana QEMU: discul de test e o copie nouă, marcată pentru scriere (`cyber.rw`), fără snapshot;
  `tests/disk/NUME/` (dacă există) se copiază în ea; după QEMU rulează `tools/after-<scenariu>.sh` (ex. `fsck.fat`).
- Captura la alt moment decât TEST READY: coloana captură `fișier.png@rândul de pe serial`.
- Testele de interfață presupun ecran 1280×800 (QEMU) și ferestre centrate; cursorul pornește în mijlocul ecranului.
  Clicurile simulate din teste: `input::push_mouse`, tastele: `keyboard::push_key`; `ui::cursor()`/`ui::top_window()` ajută.
- Meniul Start listează tot ce e în `/bin` (fără `desktop`): un program nou schimbă numărul de rânduri — actualizează
  `items` în `test_desktop` și rândul `/bin = ...` din scenariul `log`.
- Programele n-au heap: stiva lor e 64 KiB; imaginile ferestrelor sunt `static mut` în program.
- Build-ul normal pornește desktopul; build-ul de test nu (testele pornesc ce le trebuie).
- Scenariile cu servere de test pe gazdă: `tools/before-<scenariu>.sh` pornește înainte de QEMU și e oprit după
  (`dns`: server DNS 15353; `tcp`: server web 18080; `appnet`: amândouă + ecou UDP 15354). Nucleul le vede la 10.0.2.2.
- Criptografia e în crate-ul `crypto/` (fără dependențe); nucleul o folosește pentru `random` (SHA-256) și testul `crypto`.
  Clientul TLS 1.3 e în `tls/` (folosit de `/bin/fetch`). Numerele aleatoare: `kernel/src/random.rs` — fără RDRAND (QEMU-ul
  de test) se bazează doar pe TSC: bune pentru teste, slabe în fața unui atacator serios.
- Scenariul `https` pornește `tools/before-https.sh` (serverul OpenSSL din `tls/tests/server.py`, 10.0.2.2:18443); rădăcina
  de test e scrisă în `/etc/certs/` doar de test (cheia ei privată e publică în repo — nu o pune niciodată în imaginea normală).
- Criptografia: `crypto/` (fără dependențe). Vectorii noi
  se adaugă în `crypto/src/vectors.rs` și se actualizează numărul din scenariul `crypto`.
- Valorile așteptate ale vectorilor se verifică mereu cu Python `cryptography` (OpenSSL) înainte să fie scrise în cod.
  Certificatele de test: `crypto/src/testcerts/` (scriptul `make.py`); OpenSSL face uneori chei RSA cu un bit mai puțin
  decât s-a cerut (2049 → 2048), deci verifică lungimea reală.
- Cârlige de test pentru pierderi TCP: `tcp::DROP_NEXT_SEND`, `tcp::DROP_RECV_AFTER` (doar în build-ul de test).
- **`tools/test.sh --no-build` nu construiește imaginile de test** — folosește-l doar după o rulare completă.
- **Atenție: la 17:41 UTC a pornit o ALTĂ sesiune programată în paralel** (a marcat P5-02 „în lucru” în timp ce
  această sesiune îl termina; conflictul din TASKS.md a fost rezolvat prin merge). Dacă două sesiuni se suprapun,
  fă `git pull` des și verifică TASKS.md înainte să începi o sarcină.
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~8 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
