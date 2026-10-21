# user — programele utilizator

- **Ce face:** programele care rulează în modul utilizator (ring 3), separate de nucleu.
  `src/lib.rs` e biblioteca lor mică: apeluri de sistem (`exit`, `write`, `yield_now`, `sleep_ms`), `print!`/`println!`,
  punctul de intrare (`entry!(main)`) și tratarea panicii. Fiecare fișier din `src/bin/` e un program.
- **Cum se construiește:** `tools/build.sh` le compilează (executabile ELF statice la `0x400000`, vezi `user.ld`)
  și le pune în ISO la `/boot/bin/`, încărcate de Limine ca module.
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
- `udp <adresă> <port> <text>` (P5-05) — trimite textul într-o datagramă UDP și scrie răspunsul (așteaptă 3 s).
- **Rețea (`src/net.rs`):** `resolve(nume)`, `TcpStream::connect/write_all/read`, `UdpSocket::bind/send_to/recv_from`,
  `info()`, `ping()`, `parse_ip()`; fără heap. Socket-urile se închid când obiectul dispare (și când programul iese).
- **Ferestre (`src/gfx.rs`):** `Window::open(titlu, l, î)`, `Canvas` (imaginea în memoria programului, `fill`),
  `present`/`present_all`, `draw` (o imagine mică oriunde în fereastră), `fill`, `Canvas::text` (același font ca
  nucleul), `Window::open_desktop`, `wait_event_for(ms)`, `wait_event()` → `Event::Key(c)` sau `Event::Click { x, y, buttons }`, `close()`.
- **HTTPS (`src/https.rs`):** `open(program, gazdă, port, adresă, buffer)` → (`Tcp`, `Tls`) cu certificatul verificat
  față de `/etc/certs` (`EtcCerts`), `now()` (ora pentru certificate).
- **Heap (`src/heap.rs`, P6-04):** `cyber_user::heap!(MĂRIME);` dă programului un alocator peste o zonă statică, ca să
  poată folosi `alloc` (Vec, String, JSON). Simplu: memoria eliberată revine doar când e ultimul bloc sau când totul e
  eliberat — potrivit pentru comenzi care fac un lucru și ies; pentru aplicații de lungă durată (P6-05) trebuie unul mai bun.
- **Biblioteca** are și `text::Line` (text fără heap), `mem_info`, `programs`, `kill`, `create`, `remove`, `spawn`, `spawn_with` (program + un argument), `wait`, `settings` (citirea setărilor), `list`, `time`, `screen_size`, și `open`, `read`, `close`, `stat`, `args()` (argumentele programului) și `error_text`.
- **Cum se testează:** scenariul `log` (`hello din ELF`, `test: ELF OK`).
- **Depinde de:** apelurile de sistem ale nucleului (`kernel/src/syscall/README.md`).
