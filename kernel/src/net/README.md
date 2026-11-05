# net — rețeaua (Faza 5)

**Ce face (P5-01):**
- `mod.rs` — pornește stiva pe placa de rețea găsită la boot (`drivers::e1000`); un fir de sistem ia cadrele primite
  la fiecare ~2 ms și le dă protocolului lor; `send_frame`, suma de control Internet, `parse_ip`.
  Adresa e fixă (10.0.2.15/24, ruta prin 10.0.2.2 — rețeaua „user” a QEMU) până la DHCP (P5-02).
- `arp.rs` — află adresa Ethernet a unui vecin (întreabă din nou la 0,5 s, renunță după limită), răspunde pentru adresa
  noastră; păstrează răspunsurile 5 minute.
- `ipv4.rs` — trimite pachete (direct în rețeaua noastră sau prin ruta implicită), verifică pachetele primite
  (versiune, sumă de control, destinație); pachetele fragmentate se ignoră deocamdată (cu mesaj în jurnal).
- `icmp.rs` — răspunde la ping și trimite ping-uri (`ping`, comanda `ping <ip>` din shell).
- `udp.rs` (P5-02) — datagrame pe porturi: `bind`, `send`, `receive` (cu limită de timp), suma de control cu pseudo-antet.
- `dhcp.rs` (P5-02) — clientul DHCP: DISCOVER → OFFER → REQUEST → ACK la pornire (fir de sistem), reînnoire la jumătatea
  duratei; adresa, masca, ruta și DNS-ul se văd cu `net` în shell. Fără răspuns: 10.0.2.15/24 prin 10.0.2.2.
- `dns.rs` (P5-03) — nume → adresă IPv4: cerere tip A la serverul DNS de la DHCP (3 încercări × 2 s, port temporar),
  sare peste CNAME, „numele nu există” (NXDOMAIN), memorie după TTL (max. o zi, 64 de nume). Comenzile `host <nume>`
  și `ping <nume>`.
- `tcp.rs` (P5-04) — client TCP: `connect`, `send`, `recv`, `close`. Mașina de stări (SYN-SENT → ESTABLISHED → FIN-WAIT /
  CLOSE-WAIT → TIME-WAIT), MSS din opțiunea serverului, fereastră de primire de 16 KiB (încape în cele 32 de buffere ale
  plăcii), segmentele venite înainte de vreme se păstrează și se pun la loc, retransmisie cu temporizator RFC 6298
  (1 s la început, estimare din timpul dus-întors, dublare la fiecare pierdere, renunță după 8). Toate segmentele pleacă
  prin firul TCP: firul de primire nu are voie să aștepte ARP (răspunsul ar veni tot pe el). RST pentru conexiuni
  necunoscute. Comanda `http <nume> [port] [cale]` din shell.
  **Server (P6-49):** `listen(port)`, `accept(port, timp)`, `unlisten(port)`: SYN la un port ascultat → SYN-RECEIVED (SYN-ACK
  retrimis cu același temporizator) → ESTABLISHED, apoi conexiunea e la fel ca una din `connect`; cel mult 4 conexiuni
  așteaptă `accept` pe un port (restul SYN-urilor se ignoră, clientul reîncearcă).
  Încă lipsesc: „persist timer” pentru fereastra zero, SACK și scalarea ferestrei.
- `remote.rs` (P6-49, F2) — **consola text prin rețea**: `remote porneste [port]` (implicit 2323) citește cheia din
  `remote.cheie` (stick `/usb`, apoi disc `/disk`; cel puțin 16 caractere; **niciodată în repo**) și ascultă pe port.
  Protocol **criptat** (P6-54, `remote_crypto.rs`): OS-ul trimite `CYBER-REMOTE 2` + o valoare X25519 nouă; clientul o trimite
  pe a lui și un prim record `AUTH`; cheile recordurilor (ChaCha20-Poly1305) ies din X25519 + cheie (HKDF-SHA256), cheia nu
  circulă niciodată; `OK` (criptat) → un shell propriu (ca o
  fereastră Terminal): ce scrie shell-ul pleacă la client, ce trimite clientul sunt taste. Doar comenzile din **lista albă**
  (`shell::remote_allowed` = lista modului automat + `help`, `exit`, `remote stare|opreste`): nimic care scrie pe placa video,
  fără `reboot`, fără alte programe. Fiecare comandă și fiecare refuz intră în jurnal (`log remote`).
  Siguranță: o cheie greșită = primul record nu se deschide (eticheta comparată în timp constant); 3 chei greșite la rând → pauză 60 s pentru orice conexiune; 10 greșite → serviciul
  se oprește singur; `remote.cheie` e fișier secret (`cat` nu-l arată, raportul îl ascunde); o singură sesiune odată; 30 min fără
  taste → sesiunea se închide. Un record schimbat sau repetat închide sesiunea. Valori X25519 noi la fiecare conexiune
  (ce s-a înregistrat rămâne secret chiar dacă cheia se află mai târziu). Pornire automată la boot doar dacă există `remote.porneste` lângă cheie (conținutul: portul).
  Clientul: `tools/remote-client.py` (doar biblioteca standard Python, criptografia scrisă după RFC-uri; `--test` o verifică
  pe vectorii oficiali; merge pe Windows). Ghidul: `docs/REMOTE.md`.

**Abatere de la plan:** primirea se face printr-un fir care verifică placa des, nu prin întreruperi: rutarea întreruperilor
PCI cere tabelele ACPI de rutare (AML) sau MSI; se va reveni la întreruperi când va conta consumul procesorului.

**Depinde de:** `drivers::e1000`, `sched` (fir de sistem, așteptări cu limită), ceasul APIC.

**Test:** scenariile `net` (e1000e) și `net-e1000` — DHCP de la QEMU (10.0.2.15, ruta 10.0.2.2, DNS 10.0.2.3), ARP către 10.0.2.2, verificarea sumei de control ICMP, ping la
10.0.2.2 cu răspuns (`net: ping 10.0.2.2: raspuns in N ms`).
Scenariul `dns`: `tools/test.sh` pornește `tools/before-dns.sh` (serverul DNS de test `tools/dns-test-server.py`,
fără internet), la care nucleul ajunge ca 10.0.2.2:15353; verifică răspuns direct, CNAME, nume lipsă, memorie.
Scenariul `remote` (P6-49): `tools/before-remote.sh` pune pe discul de test o cheie nouă, aleatoare, și `remote.porneste`, apoi
`tools/remote-test.py` se conectează prin QEMU (`hostfwd` 127.0.0.1:12323 → 2323): fără cheie serviciul nu pornește; pornirea
automată; criptarea verificată la pornire (`test: remote: criptarea OK`, vectorul comun cu clientul Python); 3 chei greșite
refuzate, a 4-a conexiune refuzată (pauză), un record stricat închide sesiunea, salutul e `CYBER-REMOTE 2`, cheia bună → `uptime`,
`log --sumar`, `timp uptime`, `help` răspund, `gpu i6 --scrie`,
`reboot` refuzate, `cat` pe cheie = „fisier secret”, `remote opreste` din sesiune oprește serviciul; `tools/after-remote.sh`
verifică jurnalul clientului și că cheia nu apare pe serial.
Scenariul `tcp`: `tools/before-tcp.sh` pornește `python3 -m http.server` (10.0.2.2:18080): pagină mică, fișier de
200 000 de octeți cu un segment primit „pierdut” (cârlig de test), o cerere „pierdută” la trimitere (retransmisie),
conexiune refuzată pe un port închis.

**Surse:** RFC 854 (Telnet, doar ideea de protocol pe rânduri, pentru `remote`), RFC 768 (UDP), RFC 2131 și 2132 (DHCP), RFC 1034/1035 (DNS), RFC 9293 (TCP), RFC 6298 (retransmisie), RFC 826 (ARP), RFC 791 (IPv4), RFC 792 (ICMP), RFC 1071 (suma de control). Cod propriu.
