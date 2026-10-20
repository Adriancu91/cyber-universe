# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 17:41–19:40 UTC (20:41–22:40 ora României), sesiune programată în cloud, fără Adrian.
  (S-a suprapus până la 18:05 cu sesiunea anterioară: amândouă au făcut P5-02; am păstrat varianta ei și am pus DNS peste.)
- **Terminate (3 sarcini):** P5-03 (DNS), P5-04 (TCP), P5-05 (rețea pentru programe + aplicația „Rețea”).
- **Blocate (au nevoie de Adrian):** P3-09 (B-001, test pe calculatoare reale; P3-10 după el), **P5-06** (B-002: „da” pentru
  criptografie scrisă de noi), **P5-07** (B-003: codul suitei office e doar pe laptop; mai târziu). P5-08 și P5-09 depind de ele.
  **Nu mai există sarcini independente** — sesiunea s-a oprit aici (CLAUDE.md §3).
- **Ce merge acum (rezumat pentru Adrian):**
  - Rețea completă pentru pagini web simple: adresă automată (DHCP), nume → adresă (DNS, `host <nume>`),
    conexiuni TCP sigure (retransmisie, pachete pierdute recuperate), `http <nume>` în shell.
  - Programele pot folosi rețeaua: `fetch` (descarcă o pagină), `udp`, aplicația grafică **Rețea** (adresă, rută, DNS,
    pachete, ping la o adresă sau un nume) — apare în meniul Start.
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh` → **39 de scenarii trecute** (noi: `dns`, `tcp`, `appnet`). Testul de stres: vezi mai jos.
  `tools/test.sh --stress` (5 min) trecut după schimbarea din planificator. CI verde pe toate commit-urile împinse.
- **Reparat pe drum:** testul de preempțiune pica 7:1 în `sysapps`/`fatwrite`. Nici „felia minimă” a sesiunii anterioare,
  nici varianta mea pe tick-uri nu erau cauza (o simulare arată că rotația simplă e corectă); testul judecă acum după
  tick-urile planificatorului (iterațiile depind și de emulator), iar felia minimă a fost scoasă.
  Detalii: `kernel/src/sched/README.md`. Mai multe rulări complete: 34:34 tick-uri peste tot.

## Următoarea sarcină
Niciuna liberă. Când Adrian răspunde:
- „da” la B-002 → **P5-06** (HTTPS): întâi SHA-256, HMAC/HKDF, X25519, ChaCha20-Poly1305 cu vectorii oficiali (RFC 8448,
  RFC 7748, RFC 8439), apoi TLS 1.3 în `user/src/tls/`. Planul: `docs/plans/P5-06.md`. Merită împărțit în 3–4 sarcini.
- rezultatele de pe stick (B-001) → P3-09, apoi P3-10.
- suita office gata (B-003) → P5-07, P5-08.

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
- Cârlige de test pentru pierderi TCP: `tcp::DROP_NEXT_SEND`, `tcp::DROP_RECV_AFTER` (doar în build-ul de test).
- **`tools/test.sh --no-build` nu construiește imaginile de test** — folosește-l doar după o rulare completă.
- **Atenție: la 17:41 UTC a pornit o ALTĂ sesiune programată în paralel** (a marcat P5-02 „în lucru” în timp ce
  această sesiune îl termina; conflictul din TASKS.md a fost rezolvat prin merge). Dacă două sesiuni se suprapun,
  fă `git pull` des și verifică TASKS.md înainte să începi o sarcină.
- Firele de sistem (`sched::spawn_system`, ex. citirea tastaturilor USB) nu sunt numărate de `sched::alive()`.
- Stiva de pornire e proprie (128 KiB); tabelele Limine nu se mai folosesc după `mem::init`.
- Testul complet durează ~8 minute în cloud; testul de stres e doar la cerere și zilnic în CI (`stress.yml`).
