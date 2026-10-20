#!/usr/bin/env bash
# Checks the disk the `fatwrite` scenario wrote to (argument: the disk image): the FAT32 partition
# (at 1 MiB, see build.sh) must pass fsck.fat without errors, and the files the kernel left behind
# must be there with the right contents, as Linux tools see them.
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-fatwrite] EȘUAT: $*"; exit 1; }

dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
if ! out="$(fsck.fat -n "$TMP/part.img" 2>&1)"; then
    echo "$out"; fail "fsck.fat a găsit erori"
fi
echo "$out" | grep -qi "differ\|lost\|wrong\|free cluster summary" && { echo "$out"; fail "fsck.fat: avertismente"; }

[[ "$(mtype -i "$IMG@@1M" ::/SCURT.TXT)" == "salut din Cyber Universe" ]] || fail "SCURT.TXT are alt conținut"
mcopy -i "$IMG@@1M" "::/scris/Fisier Nou Lung.txt" "$TMP/lung.bin" || fail "fișierul cu nume lung lipsește"
python3 - "$TMP/lung.bin" <<'PY' || fail "conținutul fișierului cu nume lung"
import sys
data = open(sys.argv[1], "rb").read()
want = bytearray(((i * 13) ^ (i >> 7)) & 0xFF for i in range(10000))
want[5000:5005] = b"CYBER"
sys.exit(0 if data == bytes(want) else 1)
PY
mdir -b -i "$IMG@@1M" ::/scris | grep -q "sterge.txt" && fail "sterge.txt nu a fost șters"
n="$(mdir -b -i "$IMG@@1M" ::/scris/multe | wc -l)"
[[ "$n" == 20 ]] || fail "în /scris/multe sunt $n fișiere, așteptat 20"
echo "[after-fatwrite] fsck.fat curat, fișierele scrise de nucleu se citesc corect în Linux"
