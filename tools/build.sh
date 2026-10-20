#!/usr/bin/env bash
# Builds the kernel and a UEFI-bootable ISO.
#   tools/build.sh           -> build/cyber-universe.iso
#   tools/build.sh --test    -> build/cyber-universe-test.iso (kernel built with the `qemu-test` feature)
#   tools/build.sh --test --features "test-panic"
#                            -> build/cyber-universe-test-test-panic.iso (extra scenario features)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$ROOT/build"

LIMINE_VERSION="v12.9.4"   # docs/DECISIONS.md, D-004a
LIMINE_SHA256="847334bc4bc142de996fbd132806b89ed73ac9ab34dbc1e90cfebf1fcc68c2b1"
LIMINE_URL="https://github.com/limine-bootloader/limine/releases/download/${LIMINE_VERSION}/limine-binary.tar.gz"

MODE="normal"
FEATURES=()
ISO_NAME="cyber-universe.iso"
EXTRA_FEATURES=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --test) MODE="test"; shift ;;
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

# 3. ISO tree
ISO_ROOT="$BUILD/iso-$MODE"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine" "$ISO_ROOT/EFI/BOOT"
cp "$KERNEL" "$ISO_ROOT/boot/kernel"
cp "$ROOT/boot/limine/limine.conf" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/limine-uefi-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE_DIR/BOOTX64.EFI" "$ISO_ROOT/EFI/BOOT/"

# 4. ISO image (UEFI only, see D-003)
xorriso -as mkisofs -R -r -J \
    --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$BUILD/$ISO_NAME" 2>/dev/null

echo "[build] Gata: build/$ISO_NAME ($(du -h "$BUILD/$ISO_NAME" | cut -f1))"
