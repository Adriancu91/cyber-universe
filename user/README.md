# user — programele utilizator

- **Ce face:** programele care rulează în modul utilizator (ring 3), separate de nucleu.
  `src/lib.rs` e biblioteca lor mică: apeluri de sistem (`exit`, `write`, `yield_now`, `sleep_ms`), `print!`/`println!`,
  punctul de intrare (`entry!(main)`) și tratarea panicii. Fiecare fișier din `src/bin/` e un program.
- **Cum se construiește:** `tools/build.sh` le compilează (executabile ELF statice la `0x400000`, vezi `user.ld`)
  și le pune în ISO la `/boot/bin/`, încărcate de Limine ca module.
  **Ținta (P6-29, D-014):** `x86_64-cyber-user.json` = `x86_64-unknown-none` cu SSE/SSE2 în procesor (ABI-ul x86-64 obișnuit),
  construită cu `-Zbuild-std=core,alloc` (`.cargo/config.toml`). Înainte, virgula mobilă a programelor era emulată în software
  (soft-float, zeci de cicluri pe operație); rezultatele sunt aceleași (IEEE 754), doar mult mai repede. `_start` (din `entry!`)
  aliniază stiva la 16 octeți, cum cere SSE. Nucleul rămâne soft-float.
- **Programe:** `hello` — scrie „hello din ELF” și iese cu 0; `cat <fișier>...` — afișează fișiere (cod 1 dacă unul lipsește).
- `demo` — deschide o fereastră 320×200 cu un dreptunghi roșu, marchează clicurile, iese la tasta `q` (scenariul `appwindow`).
- `desktop` — desktopul „Cyber Universe”: fundal cu numele, bară jos cu butonul Start (meniu cu programele din `/bin`,
  clic = pornește programul) și ceasul (din ceasul calculatorului, actualizat la fiecare minut). Esc îl închide.
  Din shell: comanda `desktop`. Test: scenariul `desktop`.
- `mod_salut` — programul modulului de test `salut` (`modules/salut`); programele `mod_*` nu intră în `/bin`, ci în pachetele de module.
- `monitor` — monitorul sistemului: memorie, procesor, programele care merg (actualizat la o secundă); clic pe un program + `o` îl oprește.
- `files` — managerul de fișiere: navigare, deschidere (directoare, programe), copiere/lipire, ștergere; butoane sus
  și taste (Enter, Backspace, j/k, c, v, s, q). Primul argument = directorul de pornire.
  Test pentru amândouă: scenariul `sysapps` (clicuri și taste simulate, captura `tests/screens/sysapps.png`).
- `editor [fișier]` — editor de text: scriere, săgeți, Home, End, Backspace, Delete; Ctrl+S salvează, Ctrl+Q iese
  (de două ori dacă sunt modificări nesalvate); fișiere UTF-8 până la 64 KiB. Managerul de fișiere deschide aici
  fișierele care nu sunt programe.
- `settings` — setări: fusul orar al ceasului (desktopul îl folosește) și aranjamentul tastaturii (se aplică după P4-12),
  salvate în `/disk/config/settings.toml` (`src/settings.rs` le citește). Test: scenariul `editor`.
- `terminal` — shell-ul nucleului într-o fereastră (aceleași comenzi ca în consolă); programele pornite cu `run`
  scriu tot acolo; `exit` sau închiderea ferestrei îl termină. Test: scenariul `terminal`.
- `network` (P5-05) — aplicația „Rețea”: placa (MAC), adresa, masca, ruta, DNS-ul, cât mai ține adresa DHCP, pachetele;
  o căsuță pentru ping (adresă sau nume, Enter = 4 ping-uri). Test: scenariul `appnet` (captura `tests/screens/appnet.png`).
- `kronos-calcul [n]` (P6-19, comanda C-16) — calculul lui Kronos pe procesor, cu operațiile bibliotecii `gpu-compute` din
  proiectul Cyber-GPU (backend CPU, fixat pe `41e44ba`): un bloc de tip Kronos (embedding, layernorm, QKV, atenție cauzală,
  MLP cu GELU, logit-uri, cross-entropy; greutăți bf16) calculat o dată cu referința bibliotecii pe un fir și o dată pe `n` fire
  (implicit: toate nucleele pornite) — **toleranță 0** (biblioteca promite rezultate identice bit cu bit); apoi un matmul 128³:
  referința, `kernels::matmul_blocked` pe 1 și n fire (toleranță 0) și nucleul nostru vectorial (AVX2+FMA / AVX2 / SSE2, în
  assembler: ținta `x86_64-unknown-none` nu permite `target_feature` cu SSE) pe 1 și n fire — cu FMA toleranța e
  `|dif| <= 1e-5·(1+|ref|)`, fără FMA 0. Afișează timpii pe 1 și pe n fire; cod 0 dacă totul e în toleranță.
  Timpii din QEMU nu spun nimic despre hardware (AVX e emulat încet). Test: scenariile `kronos-calcul` (`-smp 4 -cpu max`)
  și `kronos-calcul-1` (un nucleu, SSE2).
- `kronos` (P6-27, comanda C-25) — modelul Kronos adevărat în OS: motorul nostru KGR1 (`src/kgr.rs`, scris după specificația
  din hub `integrare/kronos/format_greutati.md`, fără codul Kronos) citește un fișier `.kgr` (F32 sau Q8_0) întreg în memorie,
  verifică SHA-256 (față de `--suma <hex>` sau `<model>.kgr.sha256`; suma greșită = refuz, cod 3), apoi face pasul înainte
  strat cu strat (LayerNorm, QKV, atenție cauzală cu memoria cheilor/valorilor, MLP cu GELU exact, logit-uri pe `emb_token`)
  pe toate nucleele, cu vectori AVX2+FMA (în assembler) sau scalar. `exp`/`sqrt`/`erf`/GELU vin din `gpu-compute`.
  `kronos --simboluri 1,2,3 [--n 32] [--model <cale>] [--fire n] [--fara-vector]` continuă textul (greedy) și arată
  simbolurile pe secundă; modelul implicit: vezi „Modelele cunoscute” mai jos. `kronos --verifica <model> <ref>` compară
  cu un fișier de referință (logit-uri în toleranța `1e-4·(1+|ref|)`, continuarea greedy identică).
  Memoria cheilor/valorilor stă într-o zonă `mem_map` separată de heap (modelul #459 cere 36 MiB).
  **Textul direct (P6-27b):** `kronos [--n 32] Ana are mere.` codează textul cu tokenizerul din fișier (`src/bpe.rs`, cheia
  `tokenizer.bpe`: octeți + îmbinări BPE + simboluri speciale, pre-tokenizarea în 6 variante cu clasele de litere din Python,
  grupele fixe de cifre ale regulilor `ws`), continuă greedy, scrie textul pe măsură ce vine (ține octeții unei litere
  neterminate) și se oprește la `<|endoftext|>`. Shell-ul taie la spații, deci cuvintele se lipesc la loc cu un spațiu.
  `kronos --codeaza <text>` arată doar simbolurile și decodarea înapoi. Clasele de caractere (`src/unicode.rs`) sunt generate
  de `tools/gen-unicode.py` (Python 3.13, Unicode 15.1); JSON-ul mare se citește fără arbore (`src/jscan.rs`), fiindcă heap-ul
  programelor e first-fit și 100.000 de alocări mici ar dura zeci de secunde în QEMU.
  `kronos --vectori <vectori.json> f32|q8_0` = vectorii oficiali Kronos (hub `integrare/kronos/vectori/`, copiați în
  `tests/kronos/`): suma SHA-256, codarea și decodarea celor 3 texte, argmax pe fiecare poziție, logit-urile ultimei poziții
  (abatere absolută ≤ 1e-4 la F32, ≤ 1e-3 la Q8_0, toleranța declarată de Kronos), 5 simboluri greedy cu și fără memoria
  cheilor/valorilor și textul lor. Inspirație: descrierea tokenizerului din specificația Kronos; niciun cod străin.
  Test: scenariile `kronos-kgr` (2 nuclee, AVX2) și `kronos-kgr-1` (1 nucleu, scalar), pe modelul minuscul cu greutăți
  aleatoare din `tests/kronos/` (`tools/kgr-mini.py`) și pe vectorii oficiali (`mic_f32.kgr`, `mic_q8_0.kgr`, `vectori.json`);
  `tools/build.sh` le pune pe discul de test la `/kronos` doar în build-urile `test-kronos-*`.
  **Modelele cunoscute (P6-38, C-46):** `kronos` știe de **#477** (`kronos_477_q8_0.kgr`, modelul Kronos pentru OS) și de **#459**
  (`kronos_459_q8_0.kgr`, rezerva): SHA-256 și cele 3 texte de control cu primele 5 simboluri greedy, din hub
  `integrare/kronos/MODEL.md`. Modelul implicit din `/usb/kronos/` (sau `--dosar <cale>`): întâi #477, apoi #459, apoi primul `*.kgr`.
  Fără `--suma` și fără `<model>.kgr.sha256`, un fișier cu numele unui model cunoscut e verificat cu suma lui din tabel.
  `kronos --control` rulează cele 3 texte pe modelul recunoscut după sumă → „CONTROL: TRECE (#477 …)” sau „NU TRECE” (cod 1);
  pe un model necunoscut: cod 1. Test: tot în `kronos-kgr*` (`--control` pe `mic_q8_0.kgr`, care e și el în tabel; copii ale modelului
  minuscul numite ca modelele mari în `/kronos-477`, `/kronos-459` → alese în ordinea cerută și refuzate de suma din tabel, cod 3).
  **Viteza (P6-29, comanda C-29):** firele lucrează într-un **bazin pornit o dată** (înainte: fire noi la fiecare produs,
  49 de produse pe simbol); lucrul se dă printr-un contor de generație, firele așteaptă cu `pause` (și `yield` din când în
  când, niciodată somn de 10 ms); fiecare rând al rezultatului e calculat de un singur fir, deci rezultatul e același bit cu bit
  pe orice număr de fire. Atenția se împarte pe (poziție, cap) în același bazin. `kronos --profil <text>` arată, după
  generare, unde s-a dus timpul simbolurilor noi (cicluri TSC): produsele qkv / proiecție / fc / ieșire pe straturi, capul de
  ieșire, atenția, restul, așteptarea firelor, afișarea pe ecran, argmax — în ms, ms/simbol, procente și GB/s de greutăți.
  **P6-43 (C-59):** `par` așteaptă **doar firele care au o parte de lucru** (generația și numărul de părți într-un singur cuvânt
  atomic), fiecare fir semnalează „gata” pe linia lui de cache (fără contor comun), iar când firele programului nu sunt mai multe
  decât procesoarele, cele care așteaptă nu mai cedează procesorul la ~4000 de pauze (abia la ~65 000, câteva ms). `--profil` are rândul
  „bariere: N pe simbol, … us în medie”, `kronos-calcul --cauta` rândul „bariera goala pe N fire”, `kronos --test-fire N` verifică
  bazinul (200 de lucrări cu 1..N părți, fiecare element scris o dată).
  `kronos --repetitie <p>` (opțional, implicit oprit): logit-ul fiecărui simbol deja apărut se împarte la `p` (dacă e pozitiv)
  sau se înmulțește (dacă e negativ) înainte de alegere; fără opțiune, alegerea e greedy simplă, ca înainte.
  **P6-45 (C-59, ideile lui Adrian):** `--profil` desparte așteptarea pe barieră în „trezire” (lucrarea publicată → ultimul fir pornit),
  „așteptat cel mai lent” (prima parte gata → ultima) și „adunare” (ultima parte gata → firul principal o vede), din ora TSC a fiecărui fir.
  `--bucati N` (sau `bucati = N` în reglaj): fiecare împărțire e tăiată în N bucăți pe fir, iar firul care termină ia următoarea bucată
  dintr-un contor comun (nimeni nu stă după cel lent); fiecare element tot de un singur fir, cu același cod → aceleași simboluri.
  Test: `kronos --test-fire 6 --bucati 8`, `--verifica … --fire 4 --bucati 8`, `--control --fire 3 --bucati 4` (scenariul `kronos-kgr`).
  **P6-45 pașii 4–5 (ideile 3 și 5):** `--bloc` (sau `planificare = bloc` în reglaj): un simbol generat = **un singur bloc**
  (`kgr::run_block`): toate calculele lui (pe strat: qkv → atenție → proiecție → fc → ieșire MLP, apoi capul) sunt date firelor deodată;
  fiecare fir ia bucăți din contorul comun al etapei, iar firul care termină ultima bucată face partea scurtă de după (adunare + LayerNorm)
  și deschide etapa următoare — fără firul principal care adună și republică (o singură împărțire pe simbol în loc de ~49).
  Fiecare valoare tot cu aceeași funcție → aceleași simboluri. `kronos-calcul --cauta` măsoară blocul cu 1/2/4/8 bucăți, pe firele alese
  și cu **un procesor liber** (toate minus unul), și îl scrie în reglaj doar dacă e cu peste 1 % mai rapid.
  Test: `kronos --test-fire 6 --bloc`, `--verifica … --fire 4 --bloc`, F32 `--fire 3 --bloc --bucati 8`, `--control --fire 3 --bloc`,
  `--profil --bloc` (scenariul `kronos-kgr`); `kronos-reglaj` cere rândul „un bloc pe simbol”.
  **P6-47 (C-67, qkv și proiecția mai lente decât fc/MLP):** în bloc, qkv, proiecția, fc și ieșirea MLP au deja aceeași formă de cod
  (aceeași funcție `output`, bucăți din contorul etapei); diferența apare doar la rularea reală, nu în `--cauta` (acolo tot modelul =
  2088 µs/simbol, în rularea reală produsele = 4,85 ms). Ca să vedem de ce, `--profil` arată acum pentru fiecare etapă a blocului,
  văzută de TOATE firele: timpul pe etapă al firului 0, lucrul împărțit la fire (cât ar dura cu toate firele ocupate), câte fire
  au lucrat, cât de târziu a început ultimul fir după deschiderea etapei și cedările procesorului în așteptări; plus câte procesoare
  sunt și după câte pauze cedează un fir care așteaptă. Pe 4 nov: 32 729 cedări în 315 ms (~520 pe simbol) — bănuiala: firele care
  cedează procesorul ajung târziu la etapele următoare. De încercat: `--fara-cedari` (firele nu cedează niciodata; refuzat dacă sunt
  mai multe fire decât procesoare). Fără blocul de simbol, „atentia” e acum despărțită de „produs qkv” (timpul firului 0 pe capetele lui).
  Test: `--profil --bloc` (rândurile „etapa …”), `--fara-cedari` refuzat cu 3 fire pe 2 procesoare, `--control --fire 2 --bloc --fara-cedari`.
- `kronos-calcul --cauta [--emb E] [--iesire <dir>]` (P6-28, comanda C-27, `src/cauta.rs`) — primul pas spre „OS-ul care se
  reglează singur”: încearcă variante ale produsului din motorul KGR1 (vectori AVX2+FMA sau scalar; 1, 2, 4, … fire și toate
  nucleele; ordinea buclelor t-j sau j-t) pe o matrice Q8_0 [4E, E], pe 1 rând (generare) și pe 16 rânduri (citirea textului).
  **Fiecare variantă e verificată față de un calcul de control în float64** (toleranța `1e-5·(1+Σ|w·x|)`); una greșită e
  aruncată oricât de rapidă ar fi. Timpul se măsoară cu numărătorul de cicluri al procesorului (TSC), calibrat pe ceasul de
  10 ms. Cea mai bună variantă (scor = 16 × un rând + 16 rânduri) se scrie în `<dir>/reglaj-<procesor>.txt` (implicit
  `/usb/kronos`, stick-ul trebuie să aibă `cyber.rw`); `kronos` îl citește la pornire (`--reglaj <cale>` pentru alt fișier;
  `--fire`/`--fara-vector` au întâietate). Singura scriere e acel fișier. Idee: „autotuning” (tinygrad BEAM, ATLAS), cod scris
  de noi. AVX-512 nu e încă printre variante (QEMU nu-l are, deci n-ar avea test). Test: scenariul `kronos-reglaj`
  (`--test-stricat` adaugă o variantă stricată intenționat, care trebuie respinsă; `after-kronos-reglaj.sh` citește fișierul
  de pe imagine).
  **Pe dimensiunile modelului (P6-29, C-29):** dacă e un `.kgr` pe stick (`--model <cale>`, implicit primul din `<dir>`), se
  citesc doar primii 4 MiB (cheile: straturi, E, vocabular, F32/Q8_0) și fiecare variantă corectă se mai măsoară pe toate
  produsele unui simbol generat de acel model (qkv, proiecție, fc, ieșire pe fiecare strat + capul), cu greutăți aleatoare de
  aceleași mărimi (în memorie cât modelul); capul se verifică și el față de control. Alegerea se face atunci pe model.
  Numărul de fire încercat: 1, 2, 4, …, jumătate din procesoare (un fir pe nucleu fizic, cu Hyper-Threading) și toate.
- `fetch <nume> [port] [cale]` (P5-05) — descarcă o pagină web prin HTTP (TCP) și o scrie (fără antet); cod 0 dacă serverul a răspuns 200.
  `fetch https://<nume>[:port][/cale] [adresă]` (P5-16) — la fel prin HTTPS (TLS 1.3, biblioteca `tls/`): certificatul
  serverului se verifică față de certificatele din `/etc/certs/` (DER), ora din ceasul RTC (luată ca UTC), cheile din
  apelul de sistem `random`. `adresă` = unde se conectează în loc să caute numele (numele tot trebuie să fie în certificat).
  `/etc/certs/` are rădăcinile reale (lista Mozilla, P6-02); se citește doar fișierul emitentului căutat (`src/https.rs`).
  Testul `https` adaugă rădăcina de test; `https-real` (doar cu internet, în CI) descarcă example.com și api.github.com.
- `claude <întrebare>` (P6-04) — întreabă Claude prin API (`POST /v1/messages`, JSON prin biblioteca `json/`, HTTPS) și
  scrie răspunsul. Cheia: un rând în `/usb/claude.key` — **nu se afișează și nu intră niciodată în repo**. `/etc/claude.conf`
  (opțional, rânduri `nume = valoare`): `model` (implicit `claude-sonnet-5-5`), `max_tokens` (1024), `host`, `port`,
  `address`, `key` (alt fișier cu cheia). Erori pe înțeles: fără cheie, cheie greșită (401), prea multe cereri (429),
  supraîncărcat (529). Test: scenariul `claude-cli`, față de o imitație a API-ului (`tools/claude-test-server.py`,
  verifică antetele și JSON-ul cererii; răspunde „chunked” și cu `\u` în text), fără nicio cheie reală.
- `claude-app` (P6-05) — fereastra „Claude” din meniul Start: conversația sus (Tu / Claude), caseta de scris jos,
  Enter trimite **toată** conversația (același client și aceeași configurare ca `claude`, din `src/claude_api.rs`),
  „Claude se gândește...” cât așteaptă; săgețile sus/jos derulează, Esc închide. Erorile (fără cheie, 401, fără rețea)
  apar în conversație cu roșu și nu se trimit. Test: scenariul `claude-app` (captura `tests/screens/claude-app.png`).
  Nu salvează încă conversația (pasul opțional din plan).
  **Uneltele lui Claude (P6-06, `src/claude_tools.rs`):** Claude poate cere `read_file` (cel mult 32 KiB), `list_dir`,
  `write_file` și `run_command` (în shell-ul aplicației, așteaptă cel mult 10 s, ieșirea tăiată la 8 KiB). Citirea și listarea
  se fac direct; **scrierea și rularea apar într-o casetă cu exact ce se va face, cu „Da (D)” / „Nu (N)”** — fără „Da” nu
  se face nimic. Căile sunt limitate la `/disk`, `/usb`, `/tmp` (fără `..`); o scriere în altă parte e refuzată fără să
  întrebe. Fiecare unealtă apare în conversație („Unealta: …”). Test: scenariul `claude-tools` (imitația API-ului cere pe
  rând fiecare unealtă; „Nu” la prima scriere, „Da” la a doua și la comandă; discul e verificat după, cu `mtype`/`fsck.fat`).
- `udp <adresă> <port> <text>` (P5-05) — trimite textul într-o datagramă UDP și scrie răspunsul (așteaptă 3 s).
- **Rețea (`src/net.rs`):** `resolve(nume)`, `TcpStream::connect/write_all/read`, `UdpSocket::bind/send_to/recv_from`,
  `info()`, `ping()`, `parse_ip()`; fără heap. Socket-urile se închid când obiectul dispare (și când programul iese).
- **Ferestre (`src/gfx.rs`):** `Window::open(titlu, l, î)`, `Canvas` (imaginea în memoria programului, `fill`),
  `present`/`present_all`, `draw` (o imagine mică oriunde în fereastră), `fill`, `Canvas::text` (același font ca
  nucleul), `Window::open_desktop`, `wait_event_for(ms)`, `wait_event()` → `Event::Key(c)` sau `Event::Click { x, y, buttons }`, `close()`.
- **HTTPS (`src/https.rs`):** `open(program, gazdă, port, adresă, buffer)` → (`Tcp`, `Tls`) cu certificatul verificat
  față de `/etc/certs` (`EtcCerts`), `now()` (ora pentru certificate).
- **Heap (`src/heap.rs`, P6-04/P6-05):** `cyber_user::heap!(MĂRIME);` dă programului un alocator peste o zonă statică, ca
  să poată folosi `alloc` (Vec, String, JSON). Blocuri cu antet de 16 octeți, primul care încape; la eliberare, blocurile
  libere vecine se unesc, deci merge și pentru aplicații care rulează mult (chatul). Verificat pe gazdă cu 200 000 de
  alocări/eliberări/realocări la întâmplare (conținutul neatins, totul liber la final).
  De la K6-04 heap-ul are o încuietoare: firele unui program pot aloca în același timp (verificat în `/bin/fire`).
- **Fire (`cyber_user::thread`, K6-04):** `spawn(f, arg, stivă)` / `spawn_with_stack(f, arg)` (stivă de 256 KiB din `mem_map`)
  pornesc `f(arg)` pe un fir nou al programului (aceeași memorie, pe toate nucleele); `join(id)` dă valoarea întoarsă de `f`;
  `exit(cod)` oprește doar firul. `cyber_user::exit` dintr-un fir oprește tot programul. Stivele din `mem_map` se pot elibera
  după `join` (`mem_unmap` dă `-23` cât timp alte fire rulează).
- **Biblioteca** are și `text::Line` (text fără heap), `mem_info`, `programs`, `kill`, `create`, `remove`, `spawn`, `spawn_with` (program + un argument), `wait`, `settings` (citirea setărilor), `list`, `time`, `screen_size`, și `open`, `read`, `close`, `stat`, `args()` (argumentele programului) și `error_text`.
- **Cum se testează:** scenariul `log` (`hello din ELF`, `test: ELF OK`).
- **Depinde de:** apelurile de sistem ale nucleului (`kernel/src/syscall/README.md`).
- `actualizeaza [--forteaza]` (P6-08) — actualizează sistemul de pe stick: întreabă GitHub de ultimul Release (`os-<număr>`,
  P6-07; API `releases/latest`, apoi fiecare fișier cu `Accept: application/octet-stream`, urmând redirecționarea către
  stocarea GitHub — tokenul se trimite doar API-ului), verifică fiecare fișier cu `SHA256SUMS`, îl scrie ca `<nume>.nou`
  și îl citește înapoi; abia când toate sunt bune: vechiul devine `.vechi`, noul îi ia numele, vechiul se șterge, apoi
  golirea memoriei stick-ului (apelul `sync`). Dacă ceva pică înainte, stick-ul rămâne neatins; o actualizare oprită la
  mijloc e reparată la pornirea următoare a comenzii (`.vechi` → numele lui). Dacă `VERSIUNE` de pe stick e aceeași:
  „nimic de făcut” (`--forteaza` rescrie oricum). Tokenul GitHub: un rând în `/usb/github.token` (depozitul e privat;
  drept de citire ajunge) — nu se afișează. Stick-ul trebuie marcat pentru scriere (`cyber.rw` în rădăcină).
  `/etc/update.conf` (opțional): `repo`, `host`, `port`, `address`, `token`, `dir`. Din Setări: butonul „Actualizeaza”
  deschide un terminal care rulează comanda (`terminal <comandă>` scrie comanda în shell la pornire).
  Test: scenariul `update`, față de o imitație a GitHub (`tools/update-test-server.py`: API + stocare, ca la un depozit
  privat), fără token, cu o sumă greșită, cu o actualizare întreruptă de reparat, apoi actualizarea și „la zi”;
  `tools/after-update.sh` compară discul cu fișierele versiunii.
- `gh-trimite <fișier> <proprietar/depozit> <cale> [mesaj]` (P6-09) — pune un fișier într-un depozit GitHub prin API-ul
  „contents”: `GET` întâi (dacă există: `sha`-ul lui; dacă e deja identic, nu trimite nimic), apoi `PUT` cu conținutul în
  base64 (`cyber_json::base64`), mesajul commit-ului (implicit „Din Cyber Universe: <cale>”) și `sha`-ul. Cel mult 1 MiB.
  Tokenul: același `/usb/github.token` (pentru trimitere trebuie „Contents: Read and write” pe depozitul ales); nu se
  afișează. `/etc/github.conf` (opțional): `host`, `port`, `address`, `token`. Claude îl poate folosi din chat prin unealta
  `github_push` (cu confirmarea lui Adrian pe ecran, ca la scriere și rulare). Clientul HTTPS comun cu `actualizeaza`:
  `src/http_client.rs`. Test: scenariul `ghpush` față de `tools/ghpush-test-server.py` (fără token, token greșit, fișier
  nou cu diacritice, „deja la fel”, fișier existent cu `sha`, nume cu spațiu, 300 KB, cale cu `..`); unealta: `claude-tools`.
