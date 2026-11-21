#!/usr/bin/env bash
# P6-98 (C-148 point 1): checks the disk of the `jurnal-panica` scenario (argument: the disk image). The kernel held
# 20 routine rows of the gpu journal in memory (15 written at the 16th token flush, 5 still held when it panicked):
# every one of them, in order, plus the first row, must be in /raport/gpu-jurnal-001.txt.
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1
fail() { echo "[after-jurnal-panica] EȘUAT: $*"; exit 1; }

mcopy -i "$IMG@@1M" ::/raport/gpu-jurnal-001.txt "$TMP/j.txt" || fail "jurnalul lipsește de pe disc"
grep -q "test jurnal-panica: inceput" "$TMP/j.txt" || fail "lipsește primul rând (Flush::Now)"
got="$(grep -o 'valoare test jurnal-panica rand = 0x[0-9a-f]*' "$TMP/j.txt" | sed 's/.*= //' | tr '\n' ' ')"
want="$(for i in $(seq 0 19); do printf '0x%08x ' "$i"; done)"
[[ "$got" == "$want" ]] || fail "rândurile ținute: găsit '$got', așteptat '$want'"
# P6-112: the generation's ring: rows 512..1099 kept (the oldest 512 dropped), written by the panic, in order.
got="$(grep -o 'valoare test jurnal-panica inel = 0x[0-9a-f]*' "$TMP/j.txt" | sed 's/.*= //' | tr '\n' ' ')"
want="$(for i in $(seq 512 1099); do printf '0x%08x ' "$i"; done)"
[[ "$got" == "$want" ]] || fail "inelul generarii: rândurile 512..1099 lipsesc sau nu sunt în ordine"
echo "[after-jurnal-panica] cele 20 de rânduri ținute sunt pe disc, în ordine (5 scrise de panică)"
