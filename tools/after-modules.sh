#!/usr/bin/env bash
# Checks the disk of the `modules` scenario: fsck.fat finds no errors, and the index lists `salut`
# (installed again at the end of the test), with its program in place.
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-modules] EȘUAT: $*"; exit 1; }
dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
out="$(fsck.fat -n "$TMP/part.img" 2>&1)" || { echo "$out"; fail "fsck.fat a găsit erori"; }
[[ "$(mtype -i "$IMG@@1M" ::/modules/index)" == "salut" ]] || fail "index: $(mtype -i "$IMG@@1M" ::/modules/index)"
mdir -b -i "$IMG@@1M" ::/modules/salut/bin | grep -q "salut" || fail "programul modulului lipsește"
mdir -b -i "$IMG@@1M" ::/modules | grep -q "cere-salut" && fail "cere-salut nu a fost șters"
echo "[after-modules] fsck.fat curat; index = salut"
