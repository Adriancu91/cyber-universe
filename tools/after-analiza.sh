#!/usr/bin/env bash
# After scenario `analiza`: the two runs (`analiza`, then `hw uhl` 5 s later) left two DIFFERENT folders, each with its
# inventory and report. In the same minute the second is `<minute>-2`; across a minute boundary it is the next minute
# (CI runs 44512180282 and 44638299581 fell across one: the scenario used to require `-2` and failed there).
set -uo pipefail
IMG="$1"
export MTOOLS_SKIP_CHECK=1
fail() { echo "[after-analiza] EȘUAT: $*"; exit 1; }
base="::/raport/uhl/Cyber-Test-AnalizaModel"
mapfile -t dirs < <(mdir -b -i "$IMG@@1M" "$base" 2>/dev/null | sed 's#/$##' | xargs -n1 basename 2>/dev/null | grep -E '^[0-9]{8}-[0-9]{4}(-[0-9]+)?$' | sort)
[[ ${#dirs[@]} -eq 2 ]] || fail "trebuie 2 dosare de analiză, sunt ${#dirs[@]}: ${dirs[*]}"
for d in "${dirs[@]}"; do
    for f in inventory.json inventory.redacted.json report.txt; do
        mdir -b -i "$IMG@@1M" "$base/$d/$f" >/dev/null 2>&1 || fail "$d/$f lipsește"
    done
done
echo "[after-analiza] ok: două dosare diferite (${dirs[*]}), fiecare cu inventarul și raportul"
