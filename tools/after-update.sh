#!/usr/bin/env bash
# Checks the disk of the `update` scenario (P6-08; argument: the disk image), as Linux sees it: every file of the test
# release is in its place on the "stick" with exactly the release's bytes, no .nou/.vechi file is left, the stick image
# was not copied, and the disk passes fsck.fat.
set -uo pipefail
IMG="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-update] EȘUAT: $*"; exit 1; }
python3 "$ROOT/tools/update-test-server.py" --files "$TMP/release" || fail "nu pot genera fișierele de test"
dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
fsck.fat -n "$TMP/part.img" > "$TMP/fsck.txt" 2>&1 || { cat "$TMP/fsck.txt"; fail "fsck.fat a găsit erori"; }
while read -r name dest; do
    mcopy -n -i "$IMG@@1M" "::/$dest" "$TMP/got" 2>/dev/null || fail "$dest lipsește"
    cmp -s "$TMP/got" "$TMP/release/$name" || fail "$dest nu e identic cu $name din versiune"
    rm -f "$TMP/got"
done <<'LIST'
BOOTX64.EFI EFI/BOOT/BOOTX64.EFI
kernel boot/kernel
initrd.tar boot/initrd.tar
limine.conf boot/limine/limine.conf
limine-uefi-cd.bin boot/limine/limine-uefi-cd.bin
VERSIUNE VERSIUNE
LIST
left="$(mdir -/ -b -i "$IMG@@1M" ::/ 2>/dev/null | grep -Ei '\.(nou|vechi)$|usb\.img$' || true)"
[[ -z "$left" ]] || fail "au rămas fișiere: $left"
echo "[after-update] cele 6 fișiere sunt identice cu versiunea, nimic în plus, discul e curat"
