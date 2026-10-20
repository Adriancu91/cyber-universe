# Starea proiectului

> Claude Code actualizează acest fișier la finalul FIECĂREI sesiuni. Sesiunea următoare pornește de aici.

## Ultima sesiune
- **Data:** 2026-10-20, 12:00–12:20 (ora României), sesiune în cloud cu Adrian prezent.
- **Terminate:** P1-01 … P1-10 (jurnal, GDT/TSS, IDT + excepții, hartă memorie, alocator cadre, paginare, heap,
  ACPI/MADT, APIC + temporizator 100 Hz, tastatură PS/2). Adăugată sarcina nouă **P1-13** (cu plan).
- **Ce merge acum:** nucleul pornește, gestionează memoria (cadre, pagini, heap cu `Vec`/`Box`/`String`),
  prinde orice excepție a procesorului cu mesaj clar, are ceas de 100 Hz (jurnalul are timp), primește tastatura
  și afișează pe ecran ce se tastează.
- **Build:** OK, fără avertismente (normal și toate variantele de test).
- **Teste:** `tools/test.sh` → **9 scenarii trecute**: boot, log, panic, divzero, pagefault, doublefault, unmapped,
  smp (-smp 4), keyboard (taste trimise prin QEMU). CI verde pe toate commit-urile.

## Următoarea sarcină
**P1-11** — testul de stabilitate (5 minute). Plan: `docs/plans/P1-11.md`.
Apoi P1-12 (măsurători + generarea sarcinilor Fazei 2) și P1-13 (tabele de pagini proprii).

## Mediu (verificat la P0-01, sesiune cloud)
| Unealtă | Versiune |
|---|---|
| rustc (nightly) | 1.101.0-nightly (2026-10-19) |
| cargo (stable) | 1.97.0 |
| qemu-system-x86_64 | 8.2.2 |
| git | 2.43.0 |
| xorriso | 1.5.6 |
| OVMF | /usr/share/OVMF/OVMF_CODE_4M.fd |

## Cum funcționează testele (important pentru sesiunile următoare)
- Scenariile sunt în `tests/scenarios.txt`: nume | feature-uri cargo | `success`/`failed` | rânduri așteptate (`;;`) | captură | argumente QEMU sau `keys=...`.
- Un scenariu care trebuie să „pice” (excepție, panică) se termină cu codul QEMU 35 și are `failed`.
- În build-ul de test nucleul rulează `run_tests()` din `main.rs` și se termină cu `qemu::finish_test()`.
- Funcții noi folosite doar de teste: `#[cfg(feature = "qemu-test")]`; API pentru mai târziu: `#[allow(dead_code)]` cu motiv.

## Avertismente pentru sesiunea următoare
- Fiecare sesiune cloud nouă: rulează întâi `tools/env-check.sh --install`.
- Testul de stabilitate (P1-11) durează 5 minute: nu-l pune în CI la fiecare push (plan: workflow separat, programat).
- Stiva de pornire și tabelele de pagini sunt încă ale lui Limine (vezi P1-13, cu riscurile din plan).
- Pașii de pe laptop (`PREGATIRE.md` 2–5) nu blochează Fazele 1–2.
