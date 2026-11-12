#!/usr/bin/env bash
# P6-83 (C-113): checks the 64 GiB stick the `fat-mare` scenario wrote to (build/fat-mare.img, no partition table):
# fsck.fat without errors, and the journal the kernel wrote is the one expected, as Linux tools see it.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMG="$ROOT/build/fat-mare.img"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-fat-mare] EȘUAT: $*"; exit 1; }

if ! out="$(fsck.fat -n "$IMG" 2>&1)"; then
    echo "$out"; fail "fsck.fat a găsit erori"
fi
echo "$out" | grep -qi "differ\|lost\|wrong\|free cluster summary" && { echo "$out"; fail "fsck.fat: avertismente"; }
mcopy -i "$IMG" ::/raport/gpu-jurnal-001.txt "$TMP/j.txt" || fail "jurnalul lipsește"
python3 - "$TMP/j.txt" <<'PY' || fail "conținutul jurnalului"
import sys
want = "# jurnal de proba P6-83 (C-113)\n"
want += "".join(f"[{i:5}.000000] gpu: rand de proba {i:04} pe stick-ul FAT32 de 64 GB\n" for i in range(800))
want += "# dupa sincronizarea refuzata\n"
sys.exit(0 if open(sys.argv[1], encoding="ascii").read() == want else 1)
PY
echo "[after-fat-mare] fsck.fat curat pe 64 GB, jurnalul de 800 de rânduri se citește corect în Linux"
