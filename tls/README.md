# tls — clientul TLS 1.3 propriu (P5-06, decizia D-011)

**Ce face:** strângerea de mână TLS 1.3 ca un client (RFC 8446) și apoi date criptate, peste orice „țeavă” de octeți
(trăsătura `Io`: un socket TCP pe gazdă, `net::TcpStream` în OS). `no_std`, fără heap: apelantul dă bufferul
(`MIN_BUFFER` ≈ 33 KiB).
- `record.rs` — înregistrările: antetul, criptarea ChaCha20-Poly1305 (nonce = iv XOR număr), tipul real la final.
- `client.rs` — `Tls::connect` (ClientHello cu SNI, X25519, o singură suită TLS_CHACHA20_POLY1305_SHA256; ServerHello;
  programarea cheilor; EncryptedExtensions; Certificate — lanțul verificat cu `crypto::x509::verify_chain`;
  CertificateVerify — ECDSA P-256 sau RSA-PSS; Finished), apoi `send`, `recv` (biletele NewSessionTicket sunt ignorate),
  `close` (close_notify). Orice abatere oprește conexiunea cu o eroare (`Error`).

**Nu face (încă):** reluarea sesiunii, date 0-RTT, HelloRetryRequest, KeyUpdate, alte suite, certificate de client.

**Atenție:** cod neauditat de specialiști; nu pentru date sensibile până la o verificare separată (D-011).

**Depinde de:** `crypto/`.

**Cum se testează:** `cd tls && cargo test --release` (și în CI): pornește `tests/server.py` (OpenSSL prin modulul `ssl`
din Python, doar TLS 1.3) pe 127.0.0.1 și descarcă o pagină de ~26 KB de la un server cu cheie P-256 și de la unul cu RSA;
refuzul pentru rădăcină necunoscută, nume greșit și certificat expirat. Certificatele de test: `tests/data/` (`make.py`;
cheile private sunt publice intenționat, doar pentru test).

**Surse:** RFC 8446, RFC 8439, RFC 7748, RFC 5280. Cod propriu.
