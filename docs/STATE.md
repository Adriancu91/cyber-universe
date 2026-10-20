# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 10:45–11:05 (ora României), sesiune în cloud cu Adrian prezent.
- **Terminate:** P0-01, P0-02, P0-03, P0-04, P0-05, P0-06, P0-07, P0-09, P0-08 — **toată Faza 0**.
- **Ce merge acum:**
  - `tools/build.sh` construiește nucleul și ISO-ul UEFI (Limine v12.9.4, sumă SHA-256 verificată);
  - nucleul pornește în QEMU, scrie pe serial și desenează text pe ecran cu fontul propriu;
  - `tools/test.sh` testează automat în ~5 s (cod QEMU 33 = trecut), inclusiv captură de ecran;
  - GitHub Actions rulează testul la fiecare push.
- **Build:** OK, fără avertismente.
- **Teste:** `tools/test.sh --expect "ECRAN OK" --screenshot tests/screens/boot.png` → TRECUT.

## Următoarea sarcină
**P1-01** — jurnalul unificat (logger). Plan: `docs/plans/P1-01.md`.

## Mediu (verificat la P0-01, sesiune cloud)
| Unealtă | Versiune |
|---|---|
| rustc (nightly) | 1.101.0-nightly (2026-10-19) |
| cargo (stable) | 1.97.0 |
| qemu-system-x86_64 | 8.2.2 |
| git | 2.43.0 |
| xorriso | 1.5.6 |
| OVMF | /usr/share/OVMF/OVMF_CODE_4M.fd |

## Avertismente pentru sesiunea următoare
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install`.
- Testele cu ecran folosesc `qemu-test`: nucleul scrie `TEST READY` și așteaptă un octet pe COM1 înainte să iasă.
  Orice test nou trebuie să se termine tot prin `qemu::finish_test()` sau `qemu::exit(...)`.
- Pașii de pe laptop (`PREGATIRE.md` 2–5) nu blochează Fazele 1–2.
