#!/usr/bin/env bash
# After scenario `auto-agent` (P6-206): the background `agent veghe` wrote its tour on the disk, the final tour too
# (each process counts from 1: two "tura 1" in the day's log), and none of its background lines reached the list's report
# (only the final tour's line is there, once); no secret in what it wrote.
set -uo pipefail
IMG="${1:?disk image}"
m() { MTOOLS_SKIP_CHECK=1 "$@"; }
live="$(m mdir -b -i "$IMG@@1M" ::/agent/live 2>/dev/null)"
day="$(echo "$live" | grep -E '/agent-[0-9]{8}\.txt$' | head -1)"
[[ -n "$day" ]] || { echo "[after-auto-agent] EȘUAT: lipsește jurnalul zilei din /agent/live"; echo "$live"; exit 1; }
text="$(m mtype -i "$IMG@@1M" "::${day#::}" 2>/dev/null)"
n=$(grep -c ' tura 1' <<<"$text")
(( n >= 2 )) || { echo "[after-auto-agent] EȘUAT: trebuiau >= 2 ture (fundal + finala), sunt $n"; echo "$text"; exit 1; }
files="$(m mdir -b -i "$IMG@@1M" ::/auto 2>/dev/null)"
rep="$(echo "$files" | grep -E '/raport-[0-9]{8}-[0-9]{4}\.txt$' | head -1)"
[[ -n "$rep" ]] || { echo "[after-auto-agent] EȘUAT: lipsește raportul listei"; echo "$files"; exit 1; }
rtext="$(m mtype -i "$IMG@@1M" "::${rep#::}" 2>/dev/null)"
grep -qF "auto: agent veghe pornit in fundal" <<<"$rtext" || { echo "[after-auto-agent] EȘUAT: raportul nu spune ca agentul a pornit"; exit 1; }
k=$(grep -c 'agent: veghe tura ' <<<"$rtext")
(( k == 1 )) || { echo "[after-auto-agent] EȘUAT: in raport trebuia doar tura finala (1 rand), sunt $k"; echo "$rtext" | grep 'agent:'; exit 1; }
if grep -qE 'tok-gh|sk-test' <<<"$text$rtext"; then echo "[after-auto-agent] EȘUAT: un secret in jurnal/raport"; exit 1; fi
echo "[after-auto-agent] ok: $n ture in jurnalul zilei, raportul doar cu tura finala"
