# Bancul de antrenare: comparația cinstită Windows ↔ OS (P6-187)

De ce: decizia lui Adrian din 24 nov 2026, 14:15 — cifra de referință a proiectului e comparația pe același calculator
(Precision 5820, RTX 3070 Ti) între Windows/PyTorch și OS-ul nostru, **în aceleași condiții**: același model (#513),
același lot, aceeași precizie, aceleași date, același număr de pași. Tabelul comun e în hub:
`integrare/kronos/BANC_ANTRENARE.md` (îl ține centrul de comandă / Kronos).

## Ce scrie OS-ul

La sfârșitul pasului #513 pe placă (`test-placa --antrenare-mini --vram --antrenare-513 [--placi3]`), după rândul
ceasurilor și profilul pasului, două rânduri (doar dacă toți pașii au rulat):

```
… antrenare #513 pe placa (arena in VRAM): banc antrenare: sistem OS os-408 | model #513 | lot 2 x 512 | precizie F32 | loturi sintetice MT19937 0x164 | atentia de baza | pasi 10 | ms pe pas 1123.236 | simb/s 911 | placa ms pe pas 1093.282 | pierderea 9.8512 9.8433 … | norma 0.4321 …
… antrenare #513 pe placa (arena in VRAM): banc rand tabel: | OS os-408 | #513 | 2 x 512 | F32 | sintetice MT19937 0x164 | 10 | 1123.236 | 911 | 9.8512 -> 10.1774 |
```

Primul rând: câmpuri despărțite de ` | `, fiecare `nume valoare` (numele câmpurilor nu se schimbă; un program le poate
citi). Al doilea: rândul gata de lipit în tabel.

| câmp | ce înseamnă |
|---|---|
| `sistem` | `OS` + versiunea din `/usb/VERSIUNE` (`necunoscuta` dacă lipsește) |
| `model` | `#513` pe placă; `modelul mic (SIMULARE)` în QEMU |
| `lot` | numărul de rânduri × contextul (simboluri pe rând) |
| `precizie` | `F32` azi (`bf16`/`f16 mixt` când va exista pasul mixt pentru #513) |
| `loturi` | `sintetice MT19937 0x164` (fără dosar de date: aceleași loturi ca oracolul `ORACLE_513_SYNTH`) sau `datele lui Kronos din <dosar> (… simboluri, verificate ca la Kronos, amprenta …; loturile ca Kronos, samanta 0)` |
| `atentia` | `de baza` sau `pe placi gen 3` (`--placi3`) |
| `pasi` | câți pași au rulat |
| `ms pe pas` | **pasul întreg pe ceasul procesorului**, median: de la lotul urcat până la pierderea + norma citite înapoi (fără punctele de control, fără urcarea greutăților de la început) |
| `simb/s` | `lot × context × 1 000 000 / (ms pe pas în µs)` — simbolurile de antrenare pe secundă, ca la PyTorch |
| `placa ms pe pas` | doar timpul plăcii (înregistrările ei), median — pentru noi, nu pentru comparație |
| `pierderea` | pierderea (entropia încrucișată medie pe simbol, logaritm natural) la fiecare pas, **înainte** de actualizare, 4 zecimale |
| `norma` | norma gradientului la fiecare pas, înainte de tăiere, 4 zecimale |

## Ce trebuie să măsoare partea Windows la fel

- `ms pe pas`: tot pasul (înainte, înapoi, tăierea, AdamW, `optimizer.zero_grad`), cu `torch.cuda.synchronize()` înainte
  de a citi ceasul, median pe pași, fără primul pas (încălzirea) dacă așa scrie în tabel — la noi primul pas intră în
  mediană, dar mediana pe 10 pași nu e mișcată de un singur pas lent.
- Aceleași loturi: fie generatorul MT19937 cu sămânța `0x164` (rânduri de `context + 1` simboluri `% 16 384`, intrarea =
  primele `context`, ținta = ultimele `context`), fie aceleași date ale lui Kronos în aceeași ordine.
- Aceleași greutăți de pornire și aceeași rețetă AdamW (OS-ul rulează cu lr 1e-3, tăierea normei la 1,0) — rețeta
  exactă, cu betas, eps și weight decay, e în oracolul Cyber-GPU (`gpu_compute::train`, `ORACLE_513_SYNTH`,
  `examples/oracol_513.rs`); partea Windows o ia de acolo, nu din această notă.

## Ce urmează (nu e încă în OS)

- Lotul 12 × 512 (cu recalcularea activărilor) și precizia bf16/f16 pentru #513 — când vin notele Cyber-GPU; rândul
  `banc` le va scrie în aceleași câmpuri.
- Numărul de pași ales din `teste.txt` (10–50) — azi 10.
