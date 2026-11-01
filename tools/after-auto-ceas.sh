#!/usr/bin/env bash
# After scenario `auto-ceas` (P6-41): the safety clock restarted the machine during `test-blocheaza`. The report written
# in place shows the stuck command; the list and the marker stay (the next boot from the stick ends it as INTRERUPT).
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
files="$(m mdir -b -i "$IMG@@1M" ::/auto 2>/dev/null)"
echo "$files" | grep -q 'in-lucru.txt$' || { echo "[after-auto-ceas] EȘUAT: lipsește marcajul in-lucru.txt"; exit 1; }
rep="$(echo "$files" | grep -E '/raport-[0-9]{8}-[0-9]{4}\.txt$' | head -1)"
[[ -n "$rep" ]] || { echo "[after-auto-ceas] EȘUAT: lipsește raportul"; exit 1; }
text="$(m mtype -i "$IMG@@1M" "::${rep#::}" 2>/dev/null)"
for want in "ceasul de siguranta: 4 s" "== [1/3] uptime ==" "-- gata in" "in lucru: test-blocheaza"; do
    grep -qF -- "$want" <<<"$text" || { echo "[after-auto-ceas] EȘUAT: raportul nu are: $want"; echo "$text"; exit 1; }
done
grep -qF "REZULTAT" <<<"$text" && { echo "[after-auto-ceas] EȘUAT: lista blocată nu trebuia să ajungă la REZULTAT"; exit 1; }
echo "[after-auto-ceas] ok: raportul se oprește la „in lucru: test-blocheaza”, marcajul a rămas pentru pornirea următoare"
