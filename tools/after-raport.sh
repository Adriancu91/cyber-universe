#!/usr/bin/env bash
# After scenario `raport` (P6-22): the reports the kernel wrote on the test disk (argument: the disk image) must be
# readable by Linux tools (fsck.fat clean), contain the expected sections, and contain no secret: neither the
# token from /disk/github.token (tests/disk/raport/) nor the key-shaped word typed into the log.
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-raport] EȘUAT: $*"; exit 1; }

dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
out="$(fsck.fat -n "$TMP/part.img" 2>&1)" || { echo "$out"; fail "fsck.fat a găsit erori"; }
mapfile -t files < <(mdir -b -i "$IMG@@1M" ::/raport | grep -i '\.txt$')
(( ${#files[@]} == 2 )) || fail "în /raport sunt ${#files[@]} fișiere, așteptat 2 (automat + manual): ${files[*]}"
auto="" manual=""
for f in "${files[@]}"; do
    mcopy -n -i "$IMG@@1M" "$f" "$TMP/r.txt" || fail "nu pot citi $f"
    if grep -q '^motiv: automat (eroare in jurnal:' "$TMP/r.txt"; then auto="$f"; cp "$TMP/r.txt" "$TMP/auto.txt"; fi
    if grep -q '^motiv: manual' "$TMP/r.txt"; then manual="$f"; cp "$TMP/r.txt" "$TMP/manual.txt"; fi
done
[[ -n "$auto" ]] || fail "lipsește raportul automat"
[[ -n "$manual" ]] || fail "lipsește raportul manual"
[[ "$auto" =~ -1\.txt$ && "$manual" =~ -2\.txt$ ]] || fail "numerotare: automat $auto, manual $manual"
grep -q 'jurnal: test eroare' "$TMP/auto.txt" || fail "raportul automat nu conține eroarea"
for s in '== raport Cyber Universe OS ==' '== hw ==' '== raport hardware ==' 'ecran: framebuffer la adresa fizica 0x' '== lspci ==' '== net ==' \
         '== jurnal net' '== jurnal usb ==' '== jurnal pci ==' '== jurnal gpu ==' '== erori si avertismente ==' \
         '== jurnal complet' '== sfarsit ==' 'test-jurnal: \[ascuns\] \[ascuns\]'; do
    grep -q -- "$s" "$TMP/manual.txt" || fail "raportul manual nu conține: $s"
done
for f in "$TMP/auto.txt" "$TMP/manual.txt"; do
    grep -q 'tokenfals' "$f" && fail "tokenul din github.token a ajuns în raport"
    grep -q 'ghp_abc' "$f" && fail "cuvântul de tip token a ajuns în raport"
done
echo "[after-raport] 2 rapoarte (automat + manual), secțiunile prezente, nicio cheie; fsck.fat curat"
