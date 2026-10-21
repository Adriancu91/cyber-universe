#!/usr/bin/env bash
# Builds the kernel and a UEFI-bootable ISO.
#   tools/build.sh           -> build/cyber-universe.iso
#   tools/build.sh --test    -> build/cyber-universe-test.iso (kernel built with the `qemu-test` feature)
#   tools/build.sh --usb     -> also build/cyber-universe-usb.img, to write on a USB stick (docs/USB.md)
#   tools/build.sh --test --features "test-panic"
#                            -> build/cyber-universe-test-test-panic.iso (extra scenario features)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$ROOT/build"

LIMINE_VERSION="v12.9.4"   # docs/DECISIONS.md, D-004a
LIMINE_SHA256="847334bc4bc142de996fbd132806b89ed73ac9ab34dbc1e90cfebf1fcc68c2b1"
LIMINE_URL="https://github.com/limine-bootloader/limine/releases/download/${LIMINE_VERSION}/limine-binary.tar.gz"

MODE="normal"
USB=0
FEATURES=()
ISO_NAME="cyber-universe.iso"
EXTRA_FEATURES=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --test) MODE="test"; shift ;;
        --usb) USB=1; shift ;;
        --features) EXTRA_FEATURES="$2"; shift 2 ;;
        *) echo "Opțiune necunoscută: $1" >&2; exit 2 ;;
    esac
done
if [[ "$MODE" == "test" ]]; then
    FEATURES=(--features "qemu-test${EXTRA_FEATURES:+,$EXTRA_FEATURES}")
    ISO_NAME="cyber-universe-test.iso"
    if [[ -n "$EXTRA_FEATURES" ]]; then
        SUFFIX="$(echo "$EXTRA_FEATURES" | tr ',' '-')"
        MODE="test-$SUFFIX"
        ISO_NAME="cyber-universe-test-$SUFFIX.iso"
    fi
fi

mkdir -p "$BUILD"

# 1. Limine (downloaded once, verified by checksum)
LIMINE_DIR="$BUILD/limine-$LIMINE_VERSION"
if [[ ! -f "$LIMINE_DIR/BOOTX64.EFI" ]]; then
    echo "[build] Descarc Limine $LIMINE_VERSION..."
    TMP="$BUILD/limine-download.tar.gz"
    curl -fsSL -o "$TMP" "$LIMINE_URL"
    echo "$LIMINE_SHA256  $TMP" | sha256sum -c --quiet - || { echo "[build] EROARE: suma SHA-256 Limine nu corespunde"; exit 1; }
    rm -rf "$LIMINE_DIR" && mkdir -p "$LIMINE_DIR"
    tar -xzf "$TMP" -C "$LIMINE_DIR" --strip-components=1
    rm -f "$TMP"
fi

# 2. Kernel
echo "[build] Construiesc nucleul ($MODE)..."
( cd "$ROOT/kernel" && cargo build --release "${FEATURES[@]}" --target-dir "$BUILD/target-$MODE" )
KERNEL="$BUILD/target-$MODE/x86_64-unknown-none/release/cyber_kernel"

# 2b. User programs (user/src/bin/*.rs), put in the initrd
echo "[build] Construiesc programele utilizator..."
( cd "$ROOT/user" && cargo build --release --target-dir "$BUILD/target-user" )
USER_BIN="$BUILD/target-user/x86_64-unknown-none/release"
PROGRAMS=()
# Programs named mod_* belong to modules (modules/*/module.toml), not to /bin.
for src in "$ROOT"/user/src/bin/*.rs; do
    name="$(basename "$src" .rs)"
    [[ "$name" == mod_* ]] || PROGRAMS+=("$name")
done

# 3. ISO tree
ISO_ROOT="$BUILD/iso-$MODE"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine" "$ISO_ROOT/EFI/BOOT"
cp "$KERNEL" "$ISO_ROOT/boot/kernel"
cp "$ROOT/boot/limine/limine.conf" "$ISO_ROOT/boot/limine/"
# initrd: the root file system (ramfs) = boot/initrd/ + the programs in /bin, as a ustar archive
INITRD_ROOT="$BUILD/initrd-$MODE"
rm -rf "$INITRD_ROOT" && mkdir -p "$INITRD_ROOT/bin"
cp -r "$ROOT/boot/initrd/." "$INITRD_ROOT/"
for p in "${PROGRAMS[@]}"; do cp "$USER_BIN/$p" "$INITRD_ROOT/bin/$p"; done
# Trusted web roots (tools/roots.py, Mozilla list) -> /etc/certs, looked up by name by /bin/fetch.
mkdir -p "$INITRD_ROOT/etc/certs"
cp "$ROOT"/boot/certs/*.[0-9] "$INITRD_ROOT/etc/certs/"
( cd "$INITRD_ROOT" && tar --format=ustar --owner=0 --group=0 --sort=name -cf "$ISO_ROOT/boot/initrd.tar" * )
echo "    module_path: boot():/boot/initrd.tar" >> "$ISO_ROOT/boot/limine/limine.conf"

# Test disk build/disk.img: GPT, one FAT32 partition = boot/disk/ + a 100 KiB file and its checksum.
# QEMU attaches it as a SATA disk (AHCI driver, /disk). Builds with the `test-disk` feature also carry it
# inside the ISO (read from memory); the normal build does not, because Limine needs ~13 s in QEMU
# without acceleration to load 40 MiB.
DISK_ROOT="$BUILD/disk-$MODE"
rm -rf "$DISK_ROOT" && mkdir -p "$DISK_ROOT"
cp -r "$ROOT/boot/disk/." "$DISK_ROOT/"
# Module packages (modules/<name>/ -> pachete/<name>.cum) for the module manager tests.
mkdir -p "$DISK_ROOT/pachete"
for m in "$ROOT"/modules/*/; do
    "$ROOT/tools/mkmodule.sh" "$m" "$USER_BIN" "$DISK_ROOT/pachete/$(basename "$m").cum"
done
python3 - "$DISK_ROOT/docs" <<'PY'
import os, sys
os.makedirs(sys.argv[1], exist_ok=True)  # boot/disk/docs/ is empty in git, so it may not exist
data = bytes(((i * 7) ^ (i >> 8)) & 0xFF for i in range(100 * 1024))
h = 0xcbf29ce484222325  # FNV-1a 64
for b in data:
    h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
open(sys.argv[1] + "/mare.bin", "wb").write(data)
open(sys.argv[1] + "/mare.fnv", "w").write(f"{h:016x}\n")
PY
DISK="$BUILD/disk.img"
rm -f "$DISK" && truncate -s 40M "$DISK"
printf 'label: gpt\nstart=2048, size=38M, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7\n' | PATH="$PATH:/usr/sbin:/sbin" sfdisk -q "$DISK"
PATH="$PATH:/usr/sbin:/sbin" mkfs.fat -F 32 -s 1 -n CYBERDISK --offset 2048 "$DISK" $((38 * 1024)) >/dev/null 2>&1
MTOOLS_SKIP_CHECK=1 mcopy -s -i "$DISK@@1M" "$DISK_ROOT"/* ::/
# The same files on a disk without a partition table ("superfloppy", as Windows sometimes formats sticks): usb-floppy.
FLOPPY="$BUILD/floppy.img"
rm -f "$FLOPPY" && truncate -s 40M "$FLOPPY"
PATH="$PATH:/usr/sbin:/sbin" mkfs.fat -F 32 -s 1 -n CYBERSTICK "$FLOPPY" >/dev/null 2>&1
MTOOLS_SKIP_CHECK=1 mcopy -s -i "$FLOPPY" "$DISK_ROOT"/* ::/
if [[ ",$EXTRA_FEATURES," == *",test-disk,"* ]]; then
    cp "$DISK" "$ISO_ROOT/boot/disk.img"
    echo "    module_path: boot():/boot/disk.img" >> "$ISO_ROOT/boot/limine/limine.conf"
fi
cp "$LIMINE_DIR/limine-uefi-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/BOOTX64.EFI" "$ISO_ROOT/EFI/BOOT/"

# 4. ISO image (UEFI only, see D-003)
xorriso -as mkisofs -R -r -J \
    --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$BUILD/$ISO_NAME" 2>/dev/null

echo "[build] Gata: build/$ISO_NAME ($(du -h "$BUILD/$ISO_NAME" | cut -f1))"

# 5. USB stick image (UEFI): GPT with one EFI system partition (FAT32) holding the same files as the ISO.
if (( USB )) || [[ ",$EXTRA_FEATURES," == *",test-usb,"* ]]; then
    USB_IMG="$BUILD/${ISO_NAME%.iso}-usb.img"
    rm -f "$USB_IMG" && truncate -s 64M "$USB_IMG"
    printf 'label: gpt\nstart=2048, size=62M, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, name="CYBER UNIVERSE"\n' \
        | PATH="$PATH:/usr/sbin:/sbin" sfdisk -q "$USB_IMG"
    PATH="$PATH:/usr/sbin:/sbin" mkfs.fat -F 32 -s 1 -n CYBERUNIV --offset 2048 "$USB_IMG" $((62 * 1024)) >/dev/null 2>&1
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$USB_IMG@@1M" "$ISO_ROOT/EFI" "$ISO_ROOT/boot" ::/
    echo "[build] Gata: build/$(basename "$USB_IMG") (de scris pe stick, vezi docs/USB.md)"
fi
