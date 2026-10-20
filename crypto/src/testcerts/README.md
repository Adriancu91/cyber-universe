# Certificate de test (P5-15)

Certificate X.509 făcute cu OpenSSL (pachetul Python `cryptography`) de scriptul `make.py`, folosite doar de testele din
`crypto/src/vectors.rs`. Cheile private nu se păstrează. Toate sunt valabile 2026-01-01 … 2036-01-01 (testul folosește o
dată fixă, 2026-10-20), cu excepția lui `expired_a.der` (2020–2021).

| Fișier | Ce e | Rezultat așteptat |
|---|---|---|
| `root_a.der` | rădăcină RSA-2048, semnată de ea însăși (SHA-256) | de încredere |
| `inter_a.der` | intermediar P-256, semnat RSA-PSS (SHA-256) de `root_a`, lungime de lanț 0 | bun |
| `leaf_a.der` | server P-256 (`test.cyber.local`, `*.exemplu.ro`), semnat ECDSA de `inter_a` | bun |
| `expired_a.der` | ca `leaf_a`, dar expirat | refuzat: expirat |
| `critext_a.der` | ca `leaf_a`, cu o extensie critică necunoscută | refuzat |
| `under_leaf.der` | semnat de `leaf_a` (care nu e autoritate) | refuzat: nu e CA |
| `inter2_a.der`, `deep_leaf.der` | al doilea intermediar sub `inter_a` (lungime de lanț depășită) | refuzat: lanț prea lung |
| `root_b.der` | rădăcină P-256 (ECDSA SHA-384) | de încredere |
| `leaf_b.der` | server RSA-2048 (`server.b`), semnat ECDSA SHA-384 de `root_b` | bun |

Verificare cu OpenSSL (la data testului): `leaf_a` și `leaf_b` → OK; celelalte → refuzate.
Regenerare: `python3 crypto/src/testcerts/make.py` (apoi actualizează testele dacă se schimbă ceva).
