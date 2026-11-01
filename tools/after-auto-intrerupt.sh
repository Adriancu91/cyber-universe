#!/usr/bin/env bash
# After scenario `auto-intrerupt` (P6-41): a list cut off last time is not run again; its report gets INTRERUPT.
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
files="$(m mdir -b -i "$IMG@@1M" ::/auto 2>/dev/null)"
echo "$files" | grep -q '/teste.txt$' && { echo "[after-auto-intrerupt] EȘUAT: teste.txt a rămas"; exit 1; }
echo "$files" | grep -q 'in-lucru.txt$' && { echo "[after-auto-intrerupt] EȘUAT: marcajul a rămas"; exit 1; }
echo "$files" | grep -qE '/teste-facut-[0-9]{8}-[0-9]{4}\.txt$' || { echo "[after-auto-intrerupt] EȘUAT: lista nu e redenumită"; exit 1; }
text="$(m mtype -i "$IMG@@1M" ::/auto/raport-vechi.txt 2>/dev/null)"
grep -qF "== INTRERUPT ==" <<<"$text" || { echo "[after-auto-intrerupt] EȘUAT: raportul vechi nu are INTRERUPT"; echo "$text"; exit 1; }
[[ "$(echo "$files" | grep -c '/raport-')" == 1 ]] || { echo "[after-auto-intrerupt] EȘUAT: s-a scris un raport nou (lista s-a rulat din nou?)"; exit 1; }
echo "[after-auto-intrerupt] ok: lista întreruptă nu s-a reluat, raportul ei are INTRERUPT"
