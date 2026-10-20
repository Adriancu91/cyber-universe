# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 20:41–22:00 UTC (23:41–01:00 ora României), sesiune programată în cloud, fără Adrian.
- **Terminate (4 sarcini):** P5-12 (ChaCha20-Poly1305), P5-13 (numere mari + semnături RSA PKCS#1 v1.5 și PSS),
  P5-14 (semnături ECDSA P-256), P5-15 (certificate X.509 + verificarea lanțului). P5-13 inițial a fost împărțit în
  P5-13 … P5-15 (cu planuri), cum permitea planul lui.
- **Blocate (au nevoie de Adrian):** P3-09 (B-001, test pe un calculator real; P3-10 după el), P5-07 (B-003, mai târziu).
- **Ce merge acum (rezumat pentru Adrian):** toată criptografia de care are nevoie HTTPS e scrisă de noi și verificată:
  amprente (SHA-2), chei de sesiune (HKDF, X25519), criptarea datelor (ChaCha20-Poly1305), verificarea semnăturilor
  (RSA, ECDSA) și a certificatelor site-urilor (lanțul până la o rădăcină de încredere, numele site-ului, data).
  Lipsește doar „conversația” TLS 1.3 propriu-zisă (P5-06).
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **40 de scenarii trecute**; scenariul `crypto` are acum **108 verificări** (erau 39).
  `cd crypto && cargo test --release` trece (și în CI). Verificări încrucișate cu OpenSSL, în afara repo-ului:
  300 de mesaje ChaCha20-Poly1305, 116 semnături RSA (40 de chei), 200 de semnături ECDSA, lanțurile cu `openssl verify` —
  zero diferențe. CI verde pe commit-urile verificate (cele mai noi încă rulau la final).

## Următoarea sarcină
**P5-06** — HTTPS de bază (client TLS 1.3). Plan: `docs/plans/P5-06.md`. E mare: propunerea mea e să o împarți la început
(ex. 1) programarea cheilor + înregistrările criptate, verificate cu RFC 8448; 2) strângerea de mână cu un server Python
`ssl` pe gazdă, suita TLS_CHACHA20_POLY1305_SHA256 + X25519; 3) `fetch https://…` + certificat greșit refuzat).
Atenție: programele n-au heap și au stiva de 64 KiB — bufferele TLS (înregistrări de 16 KiB, certificate) trebuie să fie
`static mut`; verificarea RSA folosește ~3 KiB de stivă. Funcțiile utile: `x509::verify_chain`, `x509::verify_signature`
(și pentru CertificateVerify), `aead::seal/open`, `hkdf::expand_label`, `x25519`.
Când Adrian trimite rezultatele de pe stick (B-001): P3-09, apoi P3-10. Suita office gata (B-003): P5-07, P5-08.

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
- Criptografia e în crate-ul `crypto/` (fără dependențe); nucleul îl ia doar cu feature-ul `test-crypto`. Vectorii noi
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
