#!/usr/bin/env bash
# After scenario `lucrare-reluare` (P6-140): the job cut off went on from its step; the one cut twice stopped; the
# finished one untouched.
set -uo pipefail
IMG="${1:?disk image}"
t() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::$1" 2>/dev/null; }
fail() { echo "[after-lucrare-reluare] EȘUAT: $*"; exit 1; }
st="$(t /lucrari/reluata.stare)"
for want in "stare=terminata" "facuti=6" "tur=2" "reluari=1"; do grep -qx "$want" <<<"$st" || fail "reluata.stare nu are $want: $st"; done
st="$(t /lucrari/blocata.stare)"
# P6-225: stopped after the second cut, then taken up on request (`lucrare reia blocata`) and finished.
for want in "stare=terminata" "facuti=1" "reluari=3" "cerute=1"; do grep -qx "$want" <<<"$st" || fail "blocata.stare nu are $want: $st"; done
grep -qF "RELUATA la cerere" <<<"$(t /lucrari/blocata-jurnal.txt)" || fail "jurnalul lui blocata nu are reluarea la cerere"
grep -qx "pornit=2026-11-21 10:00:00" <<<"$(t /lucrari/veche.stare)" || fail "veche.stare a fost atinsa"
j="$(t /lucrari/reluata-jurnal.txt)"
grep -qF "== turul 2, pasul 2/3: test-asteapta 1" <<<"$j" || fail "reluarea nu a inceput la turul 2 pasul 2"
grep -qF "turul 1, pasul" <<<"$j" && fail "reluarea a refacut pasi din turul 1"
grep -qF "== turul 2, pasul 1/3" <<<"$j" && fail "reluarea a refacut pasul 1 din turul 2"
echo "[after-lucrare-reluare] ok: reluata de la turul 2 pasul 2 (6 pasi), blocata oprita dupa a doua taiere apoi reluata la cerere (lucrare reia), veche neatinsa"
