# Rădăcinile de încredere pentru site-uri (HTTPS)

Generate de `tools/roots.py` — **nu edita de mână**. Pentru o listă nouă: descarcă `certdata.txt` și rulează scriptul.

- **Sursa:** lista publică Mozilla (Mozilla CA Certificate Program), fișierul `lib/ckfw/builtins/certdata.txt` din NSS
  (oglinda oficială `github.com/nss-dev/nss`, revizia `5192d65b7ad0403fba067a10327a33f70ac107e7`), descărcat pe 2026-10-21.
- **SHA-256 al fișierului sursă:** `3ec2b4d240dc12c747eaf450061db84af757c483bbda302f9e29ceb5d70be510`
- **Licența sursei:** Mozilla Public License 2.0 (https://mozilla.org/MPL/2.0/). Folosim lista doar ca date
  (certificatele publice ale autorităților), fără cod din NSS.
- **Ce s-a păstrat:** rădăcinile cu `CKA_TRUST_SERVER_AUTH = CKT_NSS_TRUSTED_DELEGATOR` (de încredere pentru site-uri):
  117 certificate, 125578 octeți. Lăsate deoparte (au o dată „neîncredere după” pentru servere): 1
  (Izenpe.com).
- **Numele fișierelor:** primii 8 octeți din SHA-256 al numelui (subject, DER), în hexazecimal, plus `.0`, `.1` …
  dacă două rădăcini au același nume. `fetch` deschide doar fișierul emitentului căutat.

| Fișier | Rădăcina |
|---|---|
| `7a5aac0a4318822b.0` | AC RAIZ FNMT-RCM |
| `d575c00550d93a3c.0` | AC RAIZ FNMT-RCM SERVIDORES SEGUROS |
| `3ed7cda30a8fd9ef.0` | ACCVRAIZ1 |
| `78c7c30a81e8966e.0` | ANF Secure Server Root CA |
| `3ca568793cb9b2c5.0` | Actalis Authentication Root CA |
| `7afc265b7b428e05.0` | Amazon Root CA 1 |
| `dd7802a00e09ce4d.0` | Amazon Root CA 2 |
| `e244a8ce86d7c8a5.0` | Amazon Root CA 3 |
| `ed63c1cdb7f3b9c5.0` | Amazon Root CA 4 |
| `561cbecede829ca0.0` | Atos TrustedRoot Root CA ECC TLS 2021 |
| `76cfb14372e64f28.0` | Atos TrustedRoot Root CA RSA TLS 2021 |
| `e5b3f8018024641e.0` | Autoridad de Certificacion Firmaprofesional CIF A62634068 |
| `f5d314602bda6a66.0` | BJCA Global Root CA1 |
| `d2a9af31ab91ef7f.0` | BJCA Global Root CA2 |
| `3af7f6d742951a67.0` | Buypass Class 2 Root CA |
| `1fae3f05848a6906.0` | Buypass Class 3 Root CA |
| `509e503ebc5d284a.0` | CA Disig Root R2 |
| `b2c56b255eb9b79d.0` | CFCA EV ROOT |
| `1acbd59614611d5c.0` | COMODO ECC Certification Authority |
| `7d5ee0367555e5cd.0` | COMODO RSA Certification Authority |
| `25d56a2050f60d8a.0` | Certainly Root E1 |
| `de5e4f274f777662.0` | Certainly Root R1 |
| `211e9383e0e26362.0` | Certigna Root CA |
| `8846c8134334dbd7.0` | Certum EC-384 CA |
| `f68b480e36604405.0` | Certum Trusted Network CA |
| `c8f52e90608a9b66.0` | Certum Trusted Network CA 2 |
| `06d99e226e1ff7dc.0` | Certum Trusted Root CA |
| `4395a1e9af934592.0` | D-TRUST BR Root CA 1 2020 |
| `8b627708669bc889.0` | D-TRUST BR Root CA 2 2023 |
| `8a9d375d772f1718.0` | D-TRUST EV Root CA 1 2020 |
| `872845d0e0c045bb.0` | D-TRUST EV Root CA 2 2023 |
| `e3e2caec65aa560c.0` | D-TRUST Root Class 3 CA 2 2009 |
| `c901075949ebc3fe.0` | D-TRUST Root Class 3 CA 2 EV 2009 |
| `dd8eb0f47656c6fa.0` | DigiCert Assured ID Root G2 |
| `a6d14b8dbd31b4d6.0` | DigiCert Assured ID Root G3 |
| `ce6a172b6d7dfe6d.0` | DigiCert Global Root G2 |
| `54431f479fe2a4e5.0` | DigiCert Global Root G3 |
| `ae6377bcdaf87fe1.0` | DigiCert TLS ECC P384 Root G5 |
| `a630cfc3a7be67d2.0` | DigiCert TLS RSA4096 Root G5 |
| `88a9c037ef37d041.0` | DigiCert Trusted Root G4 |
| `2c32f117e7117964.0` | GDCA TrustAUTH R5 ROOT |
| `b4229779897b2bc6.0` | GTS Root R1 |
| `87b9c7236fc72fa7.0` | GTS Root R3 |
| `4082814c38a68e0a.0` | GTS Root R4 |
| `ab2ab798a87d5359.0` | GlobalSign ECC Root CA - R4 |
| `0bb1585315ca1b46.0` | GlobalSign ECC Root CA - R5 |
| `6e1bb15c53cb2632.0` | GlobalSign Root CA - R3 |
| `21836f12ce7969ff.0` | GlobalSign Root CA - R6 |
| `d31764d55bcd0ebd.0` | GlobalSign Root E46 |
| `5eb519fdc42b79df.0` | GlobalSign Root R46 |
| `1a40353973a40245.0` | Go Daddy Root Certificate Authority - G2 |
| `e5b4891bcbe264f0.0` | HARICA TLS ECC Root CA 2021 |
| `396cc5a1c5ad9adb.0` | HARICA TLS RSA Root CA 2021 |
| `a8b2b5fd08100c10.0` | Hellenic Academic and Research Institutions ECC RootCA 2015 |
| `4c3a28919da96f5b.0` | Hellenic Academic and Research Institutions RootCA 2015 |
| `1278c47a15ccadf0.0` | HiPKI Root CA - G1 |
| `eb91244c664288c6.0` | Hongkong Post Root CA 3 |
| `f6db2fbd9dd85d92.0` | ISRG Root X1 |
| `74d0322c9c0b1779.0` | ISRG Root X2 |
| `ddfcffd39ac00536.0` | IdenTrust Commercial Root CA 1 |
| `66c45d9bf088aedc.0` | IdenTrust Public Sector Root CA 1 |
| `0a958c56db1e6430.0` | Microsec e-Szigno Root CA 2009 |
| `2fbd83fe1738277a.0` | Microsoft ECC Root Certificate Authority 2017 |
| `dc537d80c2f1db57.0` | Microsoft RSA Root Certificate Authority 2017 |
| `9544baa1dc711170.0` | NAVER Global Root Certification Authority |
| `4ffb4aa48f6d8c1a.0` | NetLock Arany (Class Gold) Főtanúsítvány |
| `03e7f8944773e7ff.0` | OISTE Server Root ECC G1 |
| `c97a9c61ec4d8e88.0` | OISTE Server Root RSA G1 |
| `1f1112494a274794.0` | OISTE WISeKey Global Root GB CA |
| `4f02f6a2a7846ce9.0` | OISTE WISeKey Global Root GC CA |
| `156f072c19452a9f.0` | QuoVadis Root CA 1 G3 |
| `db7af96bfe120467.0` | QuoVadis Root CA 2 G3 |
| `814ac939e3b96a71.0` | QuoVadis Root CA 3 G3 |
| `5760f32087e78c18.0` | SECOM TLS ECC Root CA 2024 |
| `673208c977d37cdb.0` | SECOM TLS RSA Root CA 2024 |
| `881d6962ec6c6adb.0` | SSL.com EV Root Certification Authority ECC |
| `1bdc4679854362ad.0` | SSL.com EV Root Certification Authority RSA R2 |
| `10f240f1e34e47ff.0` | SSL.com Root Certification Authority ECC |
| `fe10d740f2f93bb4.0` | SSL.com Root Certification Authority RSA |
| `dfeb549aa29240c9.0` | SSL.com TLS ECC Root CA 2022 |
| `8ac831b8ced6d872.0` | SSL.com TLS RSA Root CA 2022 |
| `8627dbe3aa036168.0` | SZAFIR ROOT CA2 |
| `bfa8d36391568aa3.0` | Sectigo Public Server Authentication Root E46 |
| `d7c14e9b9fc7c997.0` | Sectigo Public Server Authentication Root R46 |
| `bf84f85f494a13ce.0` | SecureSign Root CA14 |
| `3ec3b0beabb98cc6.0` | SecureSign Root CA15 |
| `3f769fe0d0db91ee.0` | Security Communication ECC RootCA1 |
| `b46c4c6a33058db6.0` | Security Communication RootCA2 |
| `428f14a76961b8c6.0` | Starfield Root Certificate Authority - G2 |
| `e0e7c9561648177c.0` | Starfield Services Root Certificate Authority - G2 |
| `5365e7b742f50fc2.0` | SwissSign RSA TLS Root CA 2022 - 1 |
| `2cb33e9fb6e14249.0` | T-TeleSec GlobalRoot Class 2 |
| `2005ea7978533561.0` | T-TeleSec GlobalRoot Class 3 |
| `372374ab7a7ed1c6.0` | TUBITAK Kamu SM SSL Kok Sertifikasi - Surum 1 |
| `7a87d65038d9c5c3.0` | TWCA CYBER Root CA |
| `09ef22902622d9f6.0` | TWCA Global Root CA |
| `ffc9a3394968872d.0` | TWCA Root Certification Authority |
| `0a033f1d89554991.0` | Telekom Security TLS ECC Root 2020 |
| `e87d86bd7fef7609.0` | Telekom Security TLS RSA Root 2023 |
| `6d0e94f9f1d57199.0` | Telia EC TLS Root CA v3 |
| `b1fc5fa14a7f984b.0` | Telia RSA TLS Root CA v3 |
| `366aa83b132571cf.0` | Telia Root CA v2 |
| `3aaeab24e58048fb.0` | TrustAsia Global Root CA G3 |
| `1a969bc648e6f114.0` | TrustAsia Global Root CA G4 |
| `13866160ce9d2033.0` | TrustAsia TLS ECC Root CA |
| `aa732757c450829a.0` | TrustAsia TLS RSA Root CA |
| `6b2dd81dc3f955b3.0` | TunTrust Root CA |
| `9d7363a41c7f6d2c.0` | UCA Extended Validation Root |
| `47d291c6f39ac2a3.0` | UCA Global G2 Root |
| `56bdb66f422bf347.0` | USERTrust ECC Certification Authority |
| `e65901f5fcfc65ad.0` | USERTrust RSA Certification Authority |
| `801b0fe2f540f259.0` | certSIGN Root CA G2 |
| `53c16b020f9da0bf.0` | e-Szigno Root CA 2017 |
| `9bfc9f3820035335.0` | e-Szigno TLS Root CA 2023 |
| `8283f3a4c07c2b55.0` | emSign Root CA - G1 |
| `12617e97a74d0cd4.0` | vTrus ECC Root CA |
| `bd8ea54de3ec9ebe.0` | vTrus Root CA |
