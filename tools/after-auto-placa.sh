#!/usr/bin/env bash
# After scenario `auto-placa` (P6-139, C-158 point 5): four board tests in one power-on (pretend chain), each with its own
# report; the first "recovers", the next ones "reset"; the fourth fails and the fifth is refused.
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
files="$(m mdir -b -i "$IMG@@1M" ::/auto 2>/dev/null)"
echo "$files" | grep -q '/teste.txt$' && { echo "[after-auto-placa] EȘUAT: teste.txt nu a fost redenumit"; exit 1; }
mapfile -t reps < <(echo "$files" | grep -E '/ciclu-auto-[0-9]{8}-[0-9]{4}(-[0-9]+)?\.txt$' | sort)
(( ${#reps[@]} == 4 )) || { echo "[after-auto-placa] EȘUAT: trebuiau 4 rapoarte ciclu-auto, sunt ${#reps[@]}"; echo "$files"; exit 1; }
all=""
for r in "${reps[@]}"; do all+="$(m mtype -i "$IMG@@1M" "::${r#::}" 2>/dev/null)"$'\n'; done
for n in 1 2 3 4; do
    grep -qF "testul pe placa nr. $n in aceasta pornire, cel mult 4" <<<"$all" || { echo "[after-auto-placa] EȘUAT: lipsește raportul testului nr. $n"; exit 1; }
done
grep -qF -- "-- gpu recupereaza (SIMULARE)" <<<"$all" || { echo "[after-auto-placa] EȘUAT: primul test fără gpu recupereaza"; exit 1; }
(( $(grep -c "gpu recupereaza SARIT" <<<"$all") == 3 )) || { echo "[after-auto-placa] EȘUAT: testele 2-4 trebuiau cu resetul din interior"; exit 1; }
rep="$(echo "$files" | grep -E '/raport-[0-9]{8}-[0-9]{4}\.txt$' | head -1)"
text="$(m mtype -i "$IMG@@1M" "::${rep#::}" 2>/dev/null)"
for want in "== [2/6] test-placa-sim --gresit ==" "REFUZAT: forma corecta" "== [6/6] test-placa-sim ==" "REFUZAT: placa nu a revenit" \
            "REZULTAT TEST PLACA: testul pe placa nr. 4 in aceasta pornire (cu resetul din interior): rezultat lant: oprit la resetul din interior" \
            "== REZULTAT: 4 rulate, 2 refuzate" "teste pe placa pornite: 4"; do
    grep -qF -- "$want" <<<"$text" || { echo "[after-auto-placa] EȘUAT: raportul listei nu are: $want"; echo "$text" | head -80; exit 1; }
done
echo "[after-auto-placa] ok: 4 teste pe placa intr-o pornire, ${#reps[@]} rapoarte, al cincilea refuzat"
