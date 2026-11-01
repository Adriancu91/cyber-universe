#!/usr/bin/env bash
# After scenario `kronos-reglaj` (P6-28): the tuning file kronos-calcul --cauta wrote on the test disk (argument: the
# disk image) must be readable by Linux tools (fsck.fat clean) and name a correct variant (never the broken one).
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-kronos-reglaj] EȘUAT: $*"; exit 1; }

dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
out="$(fsck.fat -n "$TMP/part.img" 2>&1)" || { echo "$out"; fail "fsck.fat a găsit erori"; }
mapfile -t files < <(mdir -b -i "$IMG@@1M" ::/kronos | grep -i 'reglaj-.*\.txt$')
(( ${#files[@]} == 1 )) || fail "în /kronos sunt ${#files[@]} fișiere reglaj, așteptat 1: ${files[*]}"
mcopy -n -i "$IMG@@1M" "${files[0]}" "$TMP/r.txt" || fail "nu pot citi ${files[0]}"
grep -Eq '^vectori = (avx2|scalar)$' "$TMP/r.txt" || fail "vectori lipsă"
grep -Eq '^fire = [0-9]+$' "$TMP/r.txt" || fail "fire lipsă"
grep -Eq '^ordine = (t-j|j-t)$' "$TMP/r.txt" || fail "ordine lipsă sau varianta stricată"
if grep -q '^fire_proiectie' "$TMP/r.txt"; then
    grep -Eq '^fire_proiectie = [0-9]+$' "$TMP/r.txt" || fail "fire_proiectie nu e un număr"
fi
grep -Eq '^# proiectia: [0-9]+ fir' "$TMP/r.txt" || fail "nota despre proiecție lipsește (P6-39)"
echo "[after-kronos-reglaj] OK: ${files[0]##*/}: $(grep -E '^(vectori|fire|ordine) =' "$TMP/r.txt" | tr '\n' ' ')"
