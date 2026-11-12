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
# The write marker (P6-08): copied to the stick's root (variant B), it lets /bin/actualizeaza update the stick.
printf 'Cyber Universe: sistemul poate scrie pe acest stick (comanda actualizeaza).\n' > "$OUT/cyber.rw"
{
    echo "commit $(git -C "$ROOT" rev-parse HEAD)"
    echo "data $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT/VERSIUNE"
# P6-40 (C-52): the normal kernel must not be able to write to a graphics card (no `gpu-scrie` adapters inside).
if grep -q "gardul Bus Master" "$OUT/kernel"; then
    echo "[release] EROARE: nucleul normal contine adaptoarele gpu-scrie"; exit 1
fi
FILES=(BOOTX64.EFI kernel initrd.tar limine.conf limine-uefi-cd.bin cyber.rw cyber-universe-usb.img VERSIUNE)
# P6-41 (C-53): the Windows script of the automatic loop, next to the files it copies.
cp "$ROOT/tools/windows/bucla-automata.ps1" "$OUT/"
FILES+=(bucla-automata.ps1)
# The TEST STICK of the I6 window (`tools/build.sh --gpu-scrie --usb`), when built: its kernel and its whole image.
GS="$ROOT/build/iso-gpu-scrie/boot/kernel"
if [[ -f "$GS" && -f "$ROOT/build/cyber-universe-gpu-scrie-usb.img" ]]; then
    grep -q "gardul Bus Master" "$GS" || { echo "[release] EROARE: nucleul gpu-scrie nu are adaptoarele"; exit 1; }
    cp "$GS" "$OUT/kernel-gpu-scrie"
    cp "$ROOT/build/cyber-universe-gpu-scrie-usb.img" "$OUT/"
    FILES+=(kernel-gpu-scrie cyber-universe-gpu-scrie-usb.img)
    # P6-84 (C-114): the single test stick without Rufus: every file of the test image's FAT partition + VERSIUNE, to be
    # copied over the root of a large FAT32 stick (variant B); Adrian adds firmware/, kronos/ and github.token once.
    ONE="$(mktemp -d)"
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$ROOT/build/cyber-universe-gpu-scrie-usb.img@@1M" ::/ "$ONE/"
    cp "$OUT/VERSIUNE" "$ONE/VERSIUNE"
    cp "$ROOT/tools/windows/STICK-TEST-CITESTE.txt" "$ONE/CITESTE.txt"
    [[ -f "$ONE/EFI/BOOT/BOOTX64.EFI" && -f "$ONE/boot/kernel" && -f "$ONE/cyber.rw" ]] || { echo "[release] EROARE: stick-test-complet incomplet"; exit 1; }
    cmp -s "$ONE/boot/kernel" "$GS" || { echo "[release] EROARE: stick-test-complet nu are nucleul gpu-scrie"; exit 1; }
    ( cd "$ONE" && zip -q -X -r "$OUT/stick-test-complet.zip" . )
    rm -rf "$ONE"
    FILES+=(stick-test-complet.zip)
fi
( cd "$OUT" && sha256sum "${FILES[@]}" > SHA256SUMS )
echo "[release] $(ls "$OUT" | wc -l) fisiere in build/release/"
