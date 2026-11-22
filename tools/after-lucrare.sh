#!/usr/bin/env bash
# After scenario `lucrare` (P6-140, C-158 point 8): the job states, the reports for the hub and the stop file on the disk.
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
t() { m mtype -i "$IMG@@1M" "::$1" 2>/dev/null; }
fail() { echo "[after-lucrare] EȘUAT: $*"; exit 1; }
jobs="$(m mdir -b -i "$IMG@@1M" ::/lucrari 2>/dev/null)"
st="$(t /lucrari/proba.stare)"
grep -qx "stare=terminata" <<<"$st" || fail "proba.stare nu e terminata: $st"
grep -qx "facuti=6" <<<"$st" || fail "proba.stare nu are 6 pasi: $st"
grep -qx "tur=2" <<<"$st" || fail "proba.stare nu are 2 tururi: $st"
grep -q "stare=oprita" <<<"$(t /lucrari/oprita.stare)" || fail "oprita.stare nu e oprita"
grep -q "motiv=durata s-a implinit" <<<"$(t /lucrari/scurta.stare)" || fail "scurta nu s-a oprit la durata"
echo "$jobs" | grep -q 'oprita.opreste$' && fail "fisierul oprita.opreste trebuia sters dupa oprire"
echo "$jobs" | grep -q 'stricata.stare$' && fail "lucrarea refuzata nu trebuia sa aiba stare"
echo "$jobs" | grep -q '\.stare\.nou$' && fail "a ramas o stare .nou"
grep -qF "== turul 2, pasul 3/3: ls /disk/lucrari" <<<"$(t /lucrari/proba-jurnal.txt)" || fail "jurnalul lucrarii nu are ultimul pas"
mapfile -t reps < <(m mdir -b -i "$IMG@@1M" ::/raport 2>/dev/null | grep -E '/lucrare-proba-[0-9]{8}-[0-9]{4}(-[0-9]+)?\.txt$' | sort -V)
(( ${#reps[@]} >= 3 )) || fail "trebuiau cel putin 3 rapoarte pentru proba (pornire, pe drum, final), sunt ${#reps[@]}"
all=""; for r in "${reps[@]}"; do all+="$(t "${r#::}")"$'\n'; done
for want in "moment: pornita" "moment: in lucru" "moment: terminata" "progres: 2 tururi terminate din 2" "ultimul pas: ls /disk/lucrari (turul 2, pasul 3" \
            "oprire: la comanda: lucrare opreste proba"; do
    grep -qF -- "$want" <<<"$all" || fail "rapoartele nu au: $want"
done
echo "[after-lucrare] ok: proba terminata (6 pasi, ${#reps[@]} rapoarte), oprita la comanda, scurta la durata, stricata refuzata"
