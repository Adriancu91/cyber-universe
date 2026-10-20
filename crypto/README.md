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
- `vectors.rs` — vectorii oficiali (FIPS 180-4, RFC 4231, RFC 5869, RFC 7748, RFC 8439, RFC 8448), rulați de `self_test()`.

Urmează: semnături și certificate (P5-13), apoi TLS 1.3 (P5-06).

**Atenție:** codul nu e auditat de specialiști în securitate. Trece vectorii oficiali, dar nu se folosește pentru date
sensibile până la o verificare separată (D-011).

**Depinde de:** nimic. Nucleul îl folosește doar în testul `crypto` deocamdată (dependență opțională).

**Cum se testează:** `cd crypto && cargo test --release` pe gazdă (și în CI); scenariul QEMU `crypto`
(`crypto: 61 vectori oficiali trecuti`, plus viteza SHA-256 și ChaCha20-Poly1305 în emulare).
Valorile așteptate au fost verificate și cu OpenSSL (pachetul Python `cryptography`), iar la P5-12 încă 300 de
mesaje aleatoare (lungimi 0–299, date adiționale 0–69 octeți) au dat exact aceleași rezultate ca OpenSSL.

**Surse:** FIPS 180-4, RFC 2104, RFC 4231, RFC 5869, RFC 7748, RFC 8439, RFC 8446, RFC 8448. Cod propriu.
