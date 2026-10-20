# crypto — criptografia proprie (decizia D-011)

**Ce face (P5-10):** funcții scrise de noi după standardele publice, fără biblioteci externe, `no_std`, fără heap:
- `sha2.rs` — SHA-256, SHA-384, SHA-512 (FIPS 180-4), dintr-o bucată (`digest`) sau pe bucăți (`update`, `finish`);
- `hmac.rs` — HMAC (RFC 2104) peste orice `Digest`;
- `hkdf.rs` — HKDF extract/expand (RFC 5869) și `expand_label` din TLS 1.3 (RFC 8446 §7.1);
- `lib.rs` — trăsătura `Digest`, `ct_eq` (comparare în timp constant, pentru etichete);
- `x25519.rs` (P5-11) — schimbul de chei X25519 (RFC 7748): câmp modulo 2^255 − 19 în 5 bucăți de 51 de biți,
  scara Montgomery în timp constant (schimbări fără ramificații), `x25519`, `x25519_base`;
- `chacha20.rs` (P5-12) — cifrul ChaCha20 (RFC 8439): blocul de 64 de octeți, contor de 32 de biți, nonce de 96 de biți;
- `poly1305.rs` (P5-12) — autentificatorul Poly1305: aritmetică modulo 2^130 − 5 în 5 bucăți de 26 de biți, reducerea
  finală aleasă prin mască (fără ramificații), pe bucăți (`update`, `finish`) sau dintr-o bucată (`mac`);
- `aead.rs` (P5-12) — ChaCha20-Poly1305 cu date adiționale (`seal`, `open`), suita TLS_CHACHA20_POLY1305_SHA256;
  `open` compară eticheta în timp constant și decriptează doar dacă e corectă (altfel textul rămâne neatins);
- `bignum.rs` (P5-13) — numere mari fără heap (până la 4096 de biți, cuvinte de 64 de biți), înmulțire Montgomery, exponențiere;
- `rsa.rs` (P5-13) — verificarea semnăturilor RSA (RFC 8017): PKCS#1 v1.5 și PSS (MGF1), cu SHA-256/384/512; refuză
  lungimi greșite, semnături ≥ n, umpluturi greșite; chei de 1024–4096 de biți;
- `p256.rs` (P5-14) — verificarea semnăturilor ECDSA pe curba P-256 (FIPS 186-5): câmpul și ordinul în formă Montgomery
  (4 cuvinte de 64 de biți), puncte în coordonate Jacobi, u1·G + u2·Q dintr-o singură trecere; cheia publică e verificată
  că stă pe curbă; semnătura se citește din DER strict (`parse_der_sig`);
- `der.rs` (P5-15) — cititor DER strict (etichete de un octet, lungimi minime; restul e refuzat);
- `x509.rs` (P5-15) — certificate X.509 v3 (RFC 5280): emitent, subiect, valabilitate, cheia (RSA sau P-256), extensiile
  de bază (CA, lungimea lanțului, folosirea cheii, numele DNS); extensie critică necunoscută = refuz. `verify_chain`
  verifică lanțul server → intermediari → rădăcină de încredere (semnături, date, CA, lungime, numele serverului,
  inclusiv `*.`); `verify_signature` va servi și la TLS (CertificateVerify);
- `testcerts/` — certificatele de test (vezi README-ul de acolo);
- `vectors_rsa.rs` — chei și semnături RSA făcute cu OpenSSL (2047, 2048, 2049, 3072, 4096 de biți; e = 3 și 65537);
- `vectors.rs` — vectorii oficiali (FIPS 180-4, RFC 4231, RFC 5869, RFC 6979 (doar vectorii), FIPS 186-5, SEC 1, RFC 7748, RFC 8439, RFC 8448), rulați de `self_test()`.

Urmează: TLS 1.3 (P5-06), care le folosește pe toate.

**Atenție:** codul nu e auditat de specialiști în securitate. Trece vectorii oficiali, dar nu se folosește pentru date
sensibile până la o verificare separată (D-011).

**Depinde de:** nimic. Nucleul îl folosește doar în testul `crypto` deocamdată (dependență opțională).

**Cum se testează:** `cd crypto && cargo test --release` pe gazdă (și în CI); scenariul QEMU `crypto`
(`crypto: 108 vectori oficiali trecuti`, plus viteza SHA-256 și ChaCha20-Poly1305 în emulare).
Valorile așteptate au fost verificate și cu OpenSSL (pachetul Python `cryptography`), iar la P5-12 încă 300 de
mesaje aleatoare (lungimi 0–299, date adiționale 0–69 octeți) au dat exact aceleași rezultate ca OpenSSL; la P5-13, 116 semnături RSA pe 40 de chei aleatoare (1024–4096 de biți,
v1.5 și PSS, o parte stricate intenționat) au primit același verdict ca la OpenSSL; la P5-14, 200 de semnături ECDSA P-256 (SHA-256/384, o parte stricate, chei
scoase de pe curbă) la fel; la P5-15, lanțurile de test au fost verificate și cu `openssl verify` (aceleași verdicte).

**Surse:** RFC 5280, ITU-T X.690, RFC 4055, RFC 6125, RFC 8017 (PKCS#1), FIPS 180-4, RFC 2104, RFC 4231, RFC 5869, RFC 6979 (doar vectorii), FIPS 186-5, SEC 1, RFC 7748, RFC 8439, RFC 8446, RFC 8448. Cod propriu.
