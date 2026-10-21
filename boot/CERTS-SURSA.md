# Rădăcinile de încredere din `/etc/certs` (P6-02)

- **Sursa:** lista Mozilla a rădăcinilor de încredere pentru site-uri (Mozilla CA Certificate Program, publicată prin CCADB:
  `https://ccadb.my.salesforce-sites.com/mozilla/IncludedRootsPEMTxt?TrustBitsInclude=Websites`). Folosită doar ca date (licența MPL 2.0 permite); certificatele sunt publice.
- **Descărcată:** 2026-10-21 · **121 rădăcini** · generată de `tools/roots.py` (se rulează de mână când vrem lista nouă).
- Rădăcinile cu chei pe care nu le suportăm (de ex. P-521) sunt păstrate, dar sărite la verificare.

| Fișier | Nume | Cheie | Expiră |
|---|---|---|---|
| `001-microsec-e-szigno-ro.der` | Microsec e-Szigno Root CA 2009 | RSA 2048 | 2029-12-30 |
| `002-atos-trustedroot-roo.der` | Atos TrustedRoot Root CA ECC TLS 2021 | P-384 | 2041-04-17 |
| `003-atos-trustedroot-roo.der` | Atos TrustedRoot Root CA RSA TLS 2021 | RSA 4096 | 2041-04-17 |
| `004-accvraiz1.der` | ACCVRAIZ1 | RSA 4096 | 2030-12-31 |
| `005-ac-raiz-fnmt-rcm-ser.der` | AC RAIZ FNMT-RCM SERVIDORES SEGUROS | P-384 | 2043-12-20 |
| `006-anf-secure-server-ro.der` | ANF Secure Server Root CA | RSA 4096 | 2039-08-30 |
| `007-actalis-authenticati.der` | Actalis Authentication Root CA | RSA 4096 | 2030-09-22 |
| `008-amazon-root-ca-1.der` | Amazon Root CA 1 | RSA 2048 | 2038-01-17 |
| `009-amazon-root-ca-2.der` | Amazon Root CA 2 | RSA 4096 | 2040-05-26 |
| `010-amazon-root-ca-3.der` | Amazon Root CA 3 | P-256 | 2040-05-26 |
| `011-amazon-root-ca-4.der` | Amazon Root CA 4 | P-384 | 2040-05-26 |
| `012-autoridad-de-certifi.der` | Autoridad de Certificacion Firmaprofesional CIF A62634068 | RSA 4096 | 2036-05-05 |
| `013-bjca-global-root-ca1.der` | BJCA Global Root CA1 | RSA 4096 | 2044-12-12 |
| `014-bjca-global-root-ca2.der` | BJCA Global Root CA2 | P-384 | 2044-12-12 |
| `015-buypass-class-2-root.der` | Buypass Class 2 Root CA | RSA 4096 | 2040-10-26 |
| `016-buypass-class-3-root.der` | Buypass Class 3 Root CA | RSA 4096 | 2040-10-26 |
| `017-ca-disig-root-r2.der` | CA Disig Root R2 | RSA 4096 | 2042-07-19 |
| `018-cfca-ev-root.der` | CFCA EV ROOT | RSA 4096 | 2029-12-31 |
| `019-comodo-ecc-certifica.der` | COMODO ECC Certification Authority | P-384 | 2038-01-18 |
| `020-comodo-rsa-certifica.der` | COMODO RSA Certification Authority | RSA 4096 | 2038-01-18 |
| `021-certainly-root-e1.der` | Certainly Root E1 | P-384 | 2046-04-01 |
| `022-certainly-root-r1.der` | Certainly Root R1 | RSA 4096 | 2046-04-01 |
| `023-certigna-root-ca.der` | Certigna Root CA | RSA 4096 | 2033-10-01 |
| `024-certum-ec-384-ca.der` | Certum EC-384 CA | P-384 | 2043-03-26 |
| `025-certum-trusted-netwo.der` | Certum Trusted Network CA 2 | RSA 4096 | 2046-10-06 |
| `026-certum-trusted-netwo.der` | Certum Trusted Network CA | RSA 2048 | 2029-12-31 |
| `027-certum-trusted-root.der` | Certum Trusted Root CA | RSA 4096 | 2043-03-16 |
| `028-d-trust-br-root-ca-1.der` | D-TRUST BR Root CA 1 2020 | P-384 | 2035-02-11 |
| `029-d-trust-br-root-ca-2.der` | D-TRUST BR Root CA 2 2023 | RSA 4096 | 2038-05-09 |
| `030-d-trust-ev-root-ca-1.der` | D-TRUST EV Root CA 1 2020 | P-384 | 2035-02-11 |
| `031-d-trust-ev-root-ca-2.der` | D-TRUST EV Root CA 2 2023 | RSA 4096 | 2038-05-09 |
| `032-d-trust-root-class-3.der` | D-TRUST Root Class 3 CA 2 2009 | RSA 2048 | 2029-11-05 |
| `033-d-trust-root-class-3.der` | D-TRUST Root Class 3 CA 2 EV 2009 | RSA 2048 | 2029-11-05 |
| `034-digicert-assured-id.der` | DigiCert Assured ID Root G2 | RSA 2048 | 2038-01-15 |
| `035-digicert-assured-id.der` | DigiCert Assured ID Root G3 | P-384 | 2038-01-15 |
| `036-digicert-global-root.der` | DigiCert Global Root G2 | RSA 2048 | 2038-01-15 |
| `037-digicert-global-root.der` | DigiCert Global Root G3 | P-384 | 2038-01-15 |
| `038-digicert-tls-ecc-p38.der` | DigiCert TLS ECC P384 Root G5 | P-384 | 2046-01-14 |
| `039-digicert-tls-rsa4096.der` | DigiCert TLS RSA4096 Root G5 | RSA 4096 | 2046-01-14 |
| `040-digicert-trusted-roo.der` | DigiCert Trusted Root G4 | RSA 4096 | 2038-01-15 |
| `041-gdca-trustauth-r5-ro.der` | GDCA TrustAUTH R5 ROOT | RSA 4096 | 2040-12-31 |
| `042-gts-root-r1.der` | GTS Root R1 | RSA 4096 | 2036-06-22 |
| `043-gts-root-r3.der` | GTS Root R3 | P-384 | 2036-06-22 |
| `044-gts-root-r4.der` | GTS Root R4 | P-384 | 2036-06-22 |
| `045-globalsign-root-e46.der` | GlobalSign Root E46 | P-384 | 2046-03-20 |
| `046-globalsign-root-r46.der` | GlobalSign Root R46 | RSA 4096 | 2046-03-20 |
| `047-globalsign.der` | GlobalSign | P-256 | 2038-01-19 |
| `048-globalsign.der` | GlobalSign | P-384 | 2038-01-19 |
| `049-globalsign.der` | GlobalSign | RSA 2048 | 2029-03-18 |
| `050-globalsign.der` | GlobalSign | RSA 4096 | 2034-12-10 |
| `051-go-daddy-root-certif.der` | Go Daddy Root Certificate Authority - G2 | RSA 2048 | 2037-12-31 |
| `052-harica-tls-ecc-root.der` | HARICA TLS ECC Root CA 2021 | P-384 | 2045-02-13 |
| `053-harica-tls-rsa-root.der` | HARICA TLS RSA Root CA 2021 | RSA 4096 | 2045-02-13 |
| `054-hellenic-academic-an.der` | Hellenic Academic and Research Institutions ECC RootCA 2015 | P-384 | 2040-06-30 |
| `055-hellenic-academic-an.der` | Hellenic Academic and Research Institutions RootCA 2015 | RSA 4096 | 2040-06-30 |
| `056-hipki-root-ca-g1.der` | HiPKI Root CA - G1 | RSA 4096 | 2037-12-31 |
| `057-hongkong-post-root-c.der` | Hongkong Post Root CA 3 | RSA 4096 | 2042-06-03 |
| `058-isrg-root-x1.der` | ISRG Root X1 | RSA 4096 | 2035-06-04 |
| `059-isrg-root-x2.der` | ISRG Root X2 | P-384 | 2040-09-17 |
| `060-identrust-commercial.der` | IdenTrust Commercial Root CA 1 | RSA 4096 | 2034-01-16 |
| `061-identrust-public-sec.der` | IdenTrust Public Sector Root CA 1 | RSA 4096 | 2034-01-16 |
| `062-izenpe-com.der` | Izenpe.com | RSA 4096 | 2037-12-13 |
| `063-microsoft-ecc-root-c.der` | Microsoft ECC Root Certificate Authority 2017 | P-384 | 2042-07-18 |
| `064-microsoft-rsa-root-c.der` | Microsoft RSA Root Certificate Authority 2017 | RSA 4096 | 2042-07-18 |
| `065-naver-global-root-ce.der` | NAVER Global Root Certification Authority | RSA 4096 | 2037-08-18 |
| `066-netlock-arany-class.der` | NetLock Arany (Class Gold) Főtanúsítvány | RSA 2048 | 2028-12-06 |
| `067-oiste-server-root-ec.der` | OISTE Server Root ECC G1 | P-384 | 2048-05-24 |
| `068-oiste-server-root-rs.der` | OISTE Server Root RSA G1 | RSA 4096 | 2048-05-24 |
| `069-oiste-wisekey-global.der` | OISTE WISeKey Global Root GB CA | RSA 2048 | 2039-12-01 |
| `070-oiste-wisekey-global.der` | OISTE WISeKey Global Root GC CA | P-384 | 2042-05-09 |
| `071-quovadis-root-ca-1-g.der` | QuoVadis Root CA 1 G3 | RSA 4096 | 2042-01-12 |
| `072-quovadis-root-ca-2-g.der` | QuoVadis Root CA 2 G3 | RSA 4096 | 2042-01-12 |
| `073-quovadis-root-ca-3-g.der` | QuoVadis Root CA 3 G3 | RSA 4096 | 2042-01-12 |
| `074-secom-tls-ecc-root-c.der` | SECOM TLS ECC Root CA 2024 | P-384 | 2049-01-14 |
| `075-secom-tls-rsa-root-c.der` | SECOM TLS RSA Root CA 2024 | RSA 4096 | 2049-01-14 |
| `076-ssl-com-ev-root-cert.der` | SSL.com EV Root Certification Authority ECC | P-384 | 2041-02-12 |
| `077-ssl-com-ev-root-cert.der` | SSL.com EV Root Certification Authority RSA R2 | RSA 4096 | 2042-05-30 |
| `078-ssl-com-root-certifi.der` | SSL.com Root Certification Authority ECC | P-384 | 2041-02-12 |
| `079-ssl-com-root-certifi.der` | SSL.com Root Certification Authority RSA | RSA 4096 | 2041-02-12 |
| `080-ssl-com-tls-ecc-root.der` | SSL.com TLS ECC Root CA 2022 | P-384 | 2046-08-19 |
| `081-ssl-com-tls-rsa-root.der` | SSL.com TLS RSA Root CA 2022 | RSA 4096 | 2046-08-19 |
| `082-szafir-root-ca2.der` | SZAFIR ROOT CA2 | RSA 2048 | 2035-10-19 |
| `083-sectigo-public-serve.der` | Sectigo Public Server Authentication Root E46 | P-384 | 2046-03-21 |
| `084-sectigo-public-serve.der` | Sectigo Public Server Authentication Root R46 | RSA 4096 | 2046-03-21 |
| `085-securesign-root-ca14.der` | SecureSign Root CA14 | RSA 4096 | 2045-04-08 |
| `086-securesign-root-ca15.der` | SecureSign Root CA15 | P-384 | 2045-04-08 |
| `087-security-communicati.der` | Security Communication ECC RootCA1 | P-384 | 2038-01-18 |
| `088-starfield-root-certi.der` | Starfield Root Certificate Authority - G2 | RSA 2048 | 2037-12-31 |
| `089-starfield-services-r.der` | Starfield Services Root Certificate Authority - G2 | RSA 2048 | 2037-12-31 |
| `090-swisssign-rsa-tls-ro.der` | SwissSign RSA TLS Root CA 2022 - 1 | RSA 4096 | 2047-06-08 |
| `091-t-telesec-globalroot.der` | T-TeleSec GlobalRoot Class 2 | RSA 2048 | 2033-10-01 |
| `092-t-telesec-globalroot.der` | T-TeleSec GlobalRoot Class 3 | RSA 2048 | 2033-10-01 |
| `093-tubitak-kamu-sm-ssl.der` | TUBITAK Kamu SM SSL Kok Sertifikasi - Surum 1 | RSA 2048 | 2043-10-25 |
| `094-twca-cyber-root-ca.der` | TWCA CYBER Root CA | RSA 4096 | 2047-11-22 |
| `095-twca-global-root-ca.der` | TWCA Global Root CA | RSA 4096 | 2030-12-31 |
| `096-twca-root-certificat.der` | TWCA Root Certification Authority | RSA 2048 | 2030-12-31 |
| `097-telekom-security-tls.der` | Telekom Security TLS ECC Root 2020 | P-384 | 2045-08-25 |
| `098-telekom-security-tls.der` | Telekom Security TLS RSA Root 2023 | RSA 4096 | 2048-03-27 |
| `099-telia-ec-tls-root-ca.der` | Telia EC TLS Root CA v3 | P-384 | 2048-05-23 |
| `100-telia-rsa-tls-root-c.der` | Telia RSA TLS Root CA v3 | RSA 4096 | 2048-05-23 |
| `101-telia-root-ca-v2.der` | Telia Root CA v2 | RSA 4096 | 2043-11-29 |
| `102-trustasia-global-roo.der` | TrustAsia Global Root CA G3 | RSA 4096 | 2046-05-19 |
| `103-trustasia-global-roo.der` | TrustAsia Global Root CA G4 | P-384 | 2046-05-19 |
| `104-trustasia-tls-ecc-ro.der` | TrustAsia TLS ECC Root CA | P-384 | 2044-05-15 |
| `105-trustasia-tls-rsa-ro.der` | TrustAsia TLS RSA Root CA | RSA 4096 | 2044-05-15 |
| `106-tuntrust-root-ca.der` | TunTrust Root CA | RSA 4096 | 2044-04-26 |
| `107-uca-extended-validat.der` | UCA Extended Validation Root | RSA 4096 | 2038-12-31 |
| `108-uca-global-g2-root.der` | UCA Global G2 Root | RSA 4096 | 2040-12-31 |
| `109-usertrust-ecc-certif.der` | USERTrust ECC Certification Authority | P-384 | 2038-01-18 |
| `110-usertrust-rsa-certif.der` | USERTrust RSA Certification Authority | RSA 4096 | 2038-01-18 |
| `111-e-szigno-root-ca-201.der` | e-Szigno Root CA 2017 | P-256 | 2042-08-22 |
| `112-e-szigno-tls-root-ca.der` | e-Szigno TLS Root CA 2023 | secp521r1 (nu e suportată) | 2038-07-17 |
| `113-emsign-ecc-root-ca-c.der` | emSign ECC Root CA - C3 | P-384 | 2043-02-18 |
| `114-emsign-ecc-root-ca-g.der` | emSign ECC Root CA - G3 | P-384 | 2043-02-18 |
| `115-emsign-root-ca-c1.der` | emSign Root CA - C1 | RSA 2048 | 2043-02-18 |
| `116-emsign-root-ca-g1.der` | emSign Root CA - G1 | RSA 2048 | 2043-02-18 |
| `117-vtrus-ecc-root-ca.der` | vTrus ECC Root CA | P-384 | 2043-07-31 |
| `118-vtrus-root-ca.der` | vTrus Root CA | RSA 4096 | 2043-07-31 |
| `119-ou-ac-raiz-fnmt-rcm.der` | OU=AC RAIZ FNMT-RCM,O=FNMT-RCM,C=ES | RSA 4096 | 2030-01-01 |
| `120-ou-security-communic.der` | OU=Security Communication RootCA2,O=SECOM Trust Systems CO.\,LTD.,C=JP | RSA 2048 | 2029-05-29 |
| `121-ou-certsign-root-ca.der` | OU=certSIGN ROOT CA G2,O=CERTSIGN SA,C=RO | RSA 4096 | 2042-02-06 |
