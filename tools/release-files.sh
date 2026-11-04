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
# P6-48 (C-68, F1): the virtual machine for Windows: its disk (tools/vm-image.sh) and the script that runs it.
[[ -f "$ROOT/build/cyber-universe-vm.qcow2" ]] || { echo "[release] EROARE: lipseste build/cyber-universe-vm.qcow2 (tools/vm-image.sh)"; exit 1; }
cp "$ROOT/build/cyber-universe-vm.qcow2" "$ROOT/tools/windows/vm-pornire.ps1" "$OUT/"
FILES+=(cyber-universe-vm.qcow2 vm-pornire.ps1)
# The TEST STICK of the I6 window (`tools/build.sh --gpu-scrie --usb`), when built: its kernel and its whole image.
GS="$ROOT/build/iso-gpu-scrie/boot/kernel"
if [[ -f "$GS" && -f "$ROOT/build/cyber-universe-gpu-scrie-usb.img" ]]; then
    grep -q "gardul Bus Master" "$GS" || { echo "[release] EROARE: nucleul gpu-scrie nu are adaptoarele"; exit 1; }
    cp "$GS" "$OUT/kernel-gpu-scrie"
    cp "$ROOT/build/cyber-universe-gpu-scrie-usb.img" "$OUT/"
    FILES+=(kernel-gpu-scrie cyber-universe-gpu-scrie-usb.img)
fi
( cd "$OUT" && sha256sum "${FILES[@]}" > SHA256SUMS )
echo "[release] $(ls "$OUT" | wc -l) fisiere in build/release/"
