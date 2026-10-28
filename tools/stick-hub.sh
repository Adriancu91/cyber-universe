#!/usr/bin/env bash
# P6-32 (C-35): while GitHub Actions is off, the stick archive is built in the session and delivered through the private hub.
# Packs the files of build/release/ (made by `tools/build.sh --usb` + `tools/release-files.sh`, i.e. what a CI Release held)
# into stick-cyber-universe-os-<nr>.zip, replaces the previous archive in <hub>/integrare/cyber-universe/stick/ and writes
# VERSIUNE.txt there. <nr> starts at 1001 so it never collides with the CI Release numbers (os-197, ...).
# Refuses to pack a Kronos model (KGR1 header) or NVIDIA firmware (D-015: never distributed).
#   tools/stick-hub.sh <hub-clone>          -> next number
#   tools/stick-hub.sh <hub-clone> <nr>     -> given number
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HUB="${1:?folosire: tools/stick-hub.sh <clona hub-ului> [numar]}"
REL="$ROOT/build/release"
DEST="$HUB/integrare/cyber-universe/stick"
[ -f "$REL/SHA256SUMS" ] || { echo "lipseste build/release/ (ruleaza tools/build.sh --usb && tools/release-files.sh)" >&2; exit 1; }
( cd "$REL" && sha256sum -c --quiet SHA256SUMS ) || { echo "SHA256SUMS nu se potriveste" >&2; exit 1; }

# The commit in the files must be the commit checked out now (no stale build).
grep -qx "commit $(git -C "$ROOT" rev-parse HEAD)" "$REL/VERSIUNE" || { echo "build/release/ e din alt commit; refa-l" >&2; exit 1; }

# No Kronos model, no NVIDIA firmware: neither as separate files nor inside the initrd or the stick image.
check_blob() {
    local f="$1"
    # The programs contain the string "KGR1" as the format magic in code; only a file that STARTS with it is a model.
    if [ "$(head -c 4 "$f" | od -An -c | tr -d ' ')" = "KGR1" ]; then
        echo "model Kronos in $f" >&2; exit 1
    fi
}
for f in "$REL"/*; do check_blob "$f"; done
if tar -tf "$REL/initrd.tar" | grep -Eiq '(\.kgr|gsp.*\.bin|nvidia/|firmware/)'; then
    echo "initrd.tar contine model sau firmware:" >&2; tar -tf "$REL/initrd.tar" | grep -Ei '(\.kgr|gsp.*\.bin|nvidia/|firmware/)' >&2; exit 1
fi
if command -v mdir >/dev/null; then
    # The FAT partition of the stick image starts at 1 MiB (tools/build.sh --usb).
    if MTOOLS_SKIP_CHECK=1 mdir -/ -b -i "$REL/cyber-universe-usb.img@@1M" :: 2>/dev/null | grep -Eiq '(\.kgr|gsp.*\.bin|/nvidia|/firmware)'; then
        echo "imaginea stick-ului contine model sau firmware" >&2; exit 1
    fi
fi

mkdir -p "$DEST"
if [ -n "${2:-}" ]; then
    NR="$2"
else
    LAST=$(sed -n 's/^numar: os-\([0-9]*\)$/\1/p' "$DEST/VERSIUNE.txt" 2>/dev/null || true)
    NR=$(( ${LAST:-1000} + 1 ))
fi
ZIP="stick-cyber-universe-os-$NR.zip"
TMP="$(mktemp -d)"
( cd "$REL" && zip -q -X "$TMP/$ZIP" BOOTX64.EFI kernel initrd.tar limine.conf limine-uefi-cd.bin cyber.rw cyber-universe-usb.img VERSIUNE SHA256SUMS )
# Test: unpack and verify the sums.
mkdir "$TMP/proba" && ( cd "$TMP/proba" && unzip -q "../$ZIP" && sha256sum -c --quiet SHA256SUMS )

find "$DEST" -maxdepth 1 -name 'stick-cyber-universe-os-*.zip' -delete
mv "$TMP/$ZIP" "$DEST/"
SUM=$(cd "$DEST" && sha256sum "$ZIP" | cut -d' ' -f1)
{
    echo "numar: os-$NR"
    echo "arhiva: $ZIP"
    echo "commit: $(git -C "$ROOT" rev-parse HEAD)"
    echo "ora (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "sha256: $SUM"
    echo "marime: $(stat -c %s "$DEST/$ZIP") octeti"
} > "$DEST/VERSIUNE.txt"
rm -rf "$TMP"
echo "[stick-hub] $DEST/$ZIP ($SUM)"
