# crypto — criptografia proprie (decizia D-011)

**Ce face (P5-10):** funcții scrise de noi după standardele publice, fără biblioteci externe, `no_std`, fără heap:
- `sha2.rs` — SHA-256, SHA-384, SHA-512 (FIPS 180-4), dintr-o bucată (`digest`) sau pe bucăți (`update`, `finish`);
- `hmac.rs` — HMAC (RFC 2104) peste orice `Digest`;
- `hkdf.rs` — HKDF extract/expand (RFC 5869) și `expand_label` din TLS 1.3 (RFC 8446 §7.1);
- `lib.rs` — trăsătura `Digest`, `ct_eq` (comparare în timp constant, pentru etichete);
- `vectors.rs` — vectorii oficiali (FIPS 180-4, RFC 4231, RFC 5869, RFC 8448), rulați de `self_test()`.

Urmează: X25519 (P5-11), ChaCha20-Poly1305 (P5-12), semnături și certificate (P5-13), apoi TLS 1.3 (P5-06).

**Atenție:** codul nu e auditat de specialiști în securitate. Trece vectorii oficiali, dar nu se folosește pentru date
sensibile până la o verificare separată (D-011).

**Depinde de:** nimic. Nucleul îl folosește doar în testul `crypto` deocamdată (dependență opțională).

**Cum se testează:** `cd crypto && cargo test --release` pe gazdă (și în CI); scenariul QEMU `crypto`
(`crypto: 32 vectori oficiali trecuti`, plus viteza SHA-256 în emulare).

**Surse:** FIPS 180-4, RFC 2104, RFC 4231, RFC 5869, RFC 8446, RFC 8448. Cod propriu.
