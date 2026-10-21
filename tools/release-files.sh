#!/usr/bin/env bash
# Gathers the files of a release (P6-07) in build/release/: the files of variant B of docs/USB.md (flat names), the
# whole stick image, VERSIUNE (the commit) and SHA256SUMS. Run after `tools/build.sh --usb`.
#   tools/release-files.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/build/iso-normal"
OUT="$ROOT/build/release"
rm -rf "$OUT" && mkdir -p "$OUT"
cp "$SRC/EFI/BOOT/BOOTX64.EFI" "$SRC/boot/kernel" "$SRC/boot/initrd.tar" "$SRC/boot/limine/limine.conf" \
   "$SRC/boot/limine/limine-uefi-cd.bin" "$OUT/"
cp "$ROOT/build/cyber-universe-usb.img" "$OUT/"
{
    echo "commit $(git -C "$ROOT" rev-parse HEAD)"
    echo "data $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT/VERSIUNE"
( cd "$OUT" && sha256sum BOOTX64.EFI kernel initrd.tar limine.conf limine-uefi-cd.bin cyber-universe-usb.img VERSIUNE > SHA256SUMS )
echo "[release] $(ls "$OUT" | wc -l) fisiere in build/release/"
