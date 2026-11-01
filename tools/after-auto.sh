#!/usr/bin/env bash
# After scenario `auto` (P6-41, C-53): the list ran once, was renamed, and the report was written in place.
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
files="$(m mdir -b -i "$IMG@@1M" ::/auto 2>/dev/null)"
echo "$files" | grep -q '/teste.txt$' && { echo "[after-auto] EȘUAT: teste.txt nu a fost redenumit"; exit 1; }
echo "$files" | grep -q 'in-lucru.txt$' && { echo "[after-auto] EȘUAT: marcajul in-lucru.txt a rămas"; exit 1; }
done_list="$(echo "$files" | grep -E '/teste-facut-[0-9]{8}-[0-9]{4}\.txt$' | head -1)"
[[ -n "$done_list" ]] || { echo "[after-auto] EȘUAT: lipsește teste-facut-<data>.txt"; echo "$files"; exit 1; }
rep="$(echo "$files" | grep -E '/raport-[0-9]{8}-[0-9]{4}\.txt$' | head -1)"
[[ -n "$rep" ]] || { echo "[after-auto] EȘUAT: lipsește raport-<data>.txt"; exit 1; }
text="$(m mtype -i "$IMG@@1M" "::${rep#::}" 2>/dev/null)"
for want in "== bucla automata: raport Cyber Universe OS ==" "ceasul de siguranta: 5 min" "== [2/7] gpu i6 --scrie ==" "REFUZAT: scrierile in placa video" \
            "in lucru: uptime" "pornit de" "== [6/7] run kronos --control ==" "salut de pe disc" "kronos: niciun model" "== REZULTAT: 5 rulate, 2 refuzate" "== raport Cyber Universe OS ==" "== sfarsit =="; do
    grep -qF "$want" <<<"$text" || { echo "[after-auto] EȘUAT: raportul nu are: $want"; echo "$text" | head -60; exit 1; }
done
# What `run kronos` printed (program output, not shell output) must be in the report too.
sed -n '/== \[6\/7\]/,/-- gata in/p' <<<"$text" | grep -vE '^(== |in lucru|-- gata)' | grep -q . || { echo "[after-auto] EȘUAT: ieșirea programului kronos lipsește din raport"; exit 1; }
echo "[after-auto] ok: lista rulată o dată și redenumită ($done_list), raportul $(wc -l <<<"$text") rânduri"
