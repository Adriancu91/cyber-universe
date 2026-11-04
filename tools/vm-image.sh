#!/usr/bin/env bash
# Builds the disk of the virtual machine (P6-48, C-68, F1): build/cyber-universe-vm.qcow2.
# A 1 GiB GPT disk with one FAT32 EFI partition holding the stick files (EFI/, boot/), cyber.rw (the OS may write:
# `actualizeaza`, reports), VERSIUNE and an empty raport/ folder. The VM attaches it as a USB stick, so /usb behaves
# exactly as on real hardware. Converted to compressed qcow2: small in the Release, grows as the OS writes.
# Run after `tools/build.sh --usb`.
#   tools/vm-image.sh [--raw]     (--raw also keeps build/cyber-universe-vm.img, for tests)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/build"
ISO_ROOT="$BUILD/iso-normal"
RAW="$BUILD/cyber-universe-vm.img"
OUT="$BUILD/cyber-universe-vm.qcow2"
KEEP_RAW=0
[[ "${1:-}" == "--raw" ]] && KEEP_RAW=1
[[ -f "$ISO_ROOT/boot/kernel" ]] || { echo "[vm] EROARE: lipseste build/iso-normal; ruleaza intai tools/build.sh --usb" >&2; exit 1; }
command -v qemu-img >/dev/null || { echo "[vm] EROARE: lipseste qemu-img (tools/env-check.sh --install)" >&2; exit 1; }

SIZE_MIB=1024
PART_MIB=$((SIZE_MIB - 2))
rm -f "$RAW" "$OUT" && truncate -s ${SIZE_MIB}M "$RAW"
printf 'label: gpt\nstart=2048, size=%dM, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, name="CYBER UNIVERSE VM"\n' "$PART_MIB" \
    | PATH="$PATH:/usr/sbin:/sbin" sfdisk -q "$RAW"
PATH="$PATH:/usr/sbin:/sbin" mkfs.fat -F 32 -n CYBERVM --offset 2048 "$RAW" $((PART_MIB * 1024)) >/dev/null 2>&1
M="$RAW@@1M"
MTOOLS_SKIP_CHECK=1 mcopy -s -i "$M" "$ISO_ROOT/EFI" "$ISO_ROOT/boot" ::/
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
printf 'Cyber Universe: sistemul poate scrie pe acest disc (masina virtuala, comanda actualizeaza).\n' > "$TMP/cyber.rw"
{
    echo "commit $(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo necunoscut)"
    echo "data $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "masina virtuala"
} > "$TMP/VERSIUNE"
printf 'Rapoartele lui Cyber Universe din masina virtuala.\n' > "$TMP/CITESTE.txt"
MTOOLS_SKIP_CHECK=1 mcopy -i "$M" "$TMP/cyber.rw" "$TMP/VERSIUNE" ::/
MTOOLS_SKIP_CHECK=1 mmd -i "$M" ::/raport
MTOOLS_SKIP_CHECK=1 mcopy -i "$M" "$TMP/CITESTE.txt" ::/raport/CITESTE.txt
qemu-img convert -q -c -f raw -O qcow2 "$RAW" "$OUT"
(( KEEP_RAW )) || rm -f "$RAW"
echo "[vm] Gata: build/$(basename "$OUT") ($(du -h "$OUT" | cut -f1), disc de ${SIZE_MIB} MiB)"
