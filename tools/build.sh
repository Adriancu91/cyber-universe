#!/usr/bin/env bash
# Builds the kernel and a UEFI-bootable ISO.
#   tools/build.sh           -> build/cyber-universe.iso
#   tools/build.sh --test    -> build/cyber-universe-test.iso (kernel built with the `qemu-test` feature)
#   tools/build.sh --usb     -> also build/cyber-universe-usb.img, to write on a USB stick (docs/USB.md)
#   tools/build.sh --gpu-scrie --usb
#                            -> build/cyber-universe-gpu-scrie{.iso,-usb.img}: the TEST STICK of the I6 window (P6-40, C-52),
#                               whose `gpu i6 --scrie` may write to the graphics card (only with Adrian, after `DA`)
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
        --gpu-scrie) MODE="gpu-scrie"; shift ;;
        *) echo "Opțiune necunoscută: $1" >&2; exit 2 ;;
    esac
done
if [[ "$MODE" == "gpu-scrie" ]]; then
    FEATURES=(--features "gpu-scrie")
    ISO_NAME="cyber-universe-gpu-scrie.iso"
elif [[ "$MODE" == "test" ]]; then
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
USER_BIN="$BUILD/target-user/x86_64-cyber-user/release"
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
# P6-27/P6-28: the tiny KGR1 model (tools/kgr-mini.py) only on the disks of the Kronos engine scenarios, so the other
# scenarios keep listing the same files.
if [[ ",$EXTRA_FEATURES," == *",test-kronos-"* ]]; then
    mkdir -p "$DISK_ROOT/kronos" && cp "$ROOT"/tests/kronos/* "$DISK_ROOT/kronos/"
    # P6-38 (C-46): which model `kronos` picks: copies of the tiny model under the names of the real ones (their
    # built-in SHA-256 must refuse them), with an earlier name (a.kgr) that must not win.
    # P6-60 (C-82): #513 before #477 before #459.
    mkdir -p "$DISK_ROOT/kronos-513" "$DISK_ROOT/kronos-477" "$DISK_ROOT/kronos-459"
    for f in a.kgr kronos_459_q8_0.kgr kronos_477_q8_0.kgr kronos_513_q8_0.kgr; do cp "$ROOT/tests/kronos/mini-q8.kgr" "$DISK_ROOT/kronos-513/$f"; done
    for f in a.kgr kronos_459_q8_0.kgr kronos_477_q8_0.kgr; do cp "$ROOT/tests/kronos/mini-q8.kgr" "$DISK_ROOT/kronos-477/$f"; done
    for f in a.kgr kronos_459_q8_0.kgr; do cp "$ROOT/tests/kronos/mini-q8.kgr" "$DISK_ROOT/kronos-459/$f"; done
fi
# P6-62 (C-83): I14 in `gpu lant --sim` reads Kronos's tiny control model (random weights, hub vectori/) from the disk.
if [[ ",$EXTRA_FEATURES," == *",test-gpu-sim,"* ]]; then
    mkdir -p "$DISK_ROOT/kronos" && cp "$ROOT/tests/kronos/mic_q8_0.kgr" "$DISK_ROOT/kronos/"
fi
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
# A "Release stick" for usb-pick: the same files + cyber.rw + VERSIUNE (the OS must pick it among several disks).
PICK="$BUILD/stick-pick.img"
cp "$FLOPPY" "$PICK"
echo "scriere permisa" > "$BUILD/cyber.rw.pick" && echo "test usb-pick" > "$BUILD/VERSIUNE.pick"
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$PICK" "$BUILD/cyber.rw.pick" ::/cyber.rw
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$PICK" "$BUILD/VERSIUNE.pick" ::/VERSIUNE
# P6-61 (C-84): a test stick whose settings say "ro" (scenario layout-boot-usb: the stick's settings are applied at start-up).
SETSTICK="$BUILD/stick-setari.img"
cp "$PICK" "$SETSTICK"
MTOOLS_SKIP_CHECK=1 mmd -i "$SETSTICK" ::/config
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$SETSTICK" "$ROOT/tests/disk/layoutboot/config/settings.toml" ::/config/settings.toml
# C-74: a "normal stick" next to the test stick, with the NVIDIA firmware folder (here one stand-in file, NOT an NVIDIA
# file) and cyber.rw: the OS mounts it read-only at /usb2 and reads the firmware from it (scenario usb-fw).
FWSTICK="$BUILD/stick-fw.img"
cp "$FLOPPY" "$FWSTICK"
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$FWSTICK" "$BUILD/cyber.rw.pick" ::/cyber.rw
MTOOLS_SKIP_CHECK=1 mmd -i "$FWSTICK" ::/firmware ::/firmware/nvidia ::/firmware/nvidia/ga102 ::/firmware/nvidia/ga102/gsp
echo "proba C-74, NU fisierul NVIDIA" > "$BUILD/booter_load.proba"
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$FWSTICK" "$BUILD/booter_load.proba" ::/firmware/nvidia/ga102/gsp/booter_load-570.144.bin
# P6-83 (C-113): a 64 GiB stick like Adrian's Kingston (FAT32 from Windows: 32 KiB clusters, ~2 million of them, FAT
# of ~16 MB), as a sparse file (a few MB on disk). FSInfo's "next free" hint points at the last cluster, so the first
# allocations land more than 4 GiB into the disk and then wrap to the start of the FAT (scenario fat-mare).
if [[ ",$EXTRA_FEATURES," == *",test-fat-mare,"* ]]; then
    BIG="$BUILD/fat-mare.img"
    rm -f "$BIG" && truncate -s 64G "$BIG"
    PATH="$PATH:/usr/sbin:/sbin" mkfs.fat -F 32 -s 64 -S 512 -n CYBER-UNIV "$BIG" >/dev/null
    echo "scriere permisa" > "$BUILD/cyber.rw.mare"
    MTOOLS_SKIP_CHECK=1 mcopy -o -i "$BIG" "$BUILD/cyber.rw.mare" ::/cyber.rw
    python3 - "$BIG" <<'PY2'
import struct, sys
with open(sys.argv[1], "r+b") as f:
    bs = f.read(512)
    spc, reserved, fats = bs[13], struct.unpack_from("<H", bs, 14)[0], bs[16]
    total, fat_size = struct.unpack_from("<I", bs, 32)[0], struct.unpack_from("<I", bs, 36)[0]
    fsinfo = struct.unpack_from("<H", bs, 48)[0]
    clusters = (total - reserved - fats * fat_size) // spc
    f.seek(fsinfo * 512 + 492)
    f.write(struct.pack("<I", clusters + 1))  # the last cluster (clusters are numbered from 2)
PY2
fi
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
    # The marker that lets the OS write on its own stick (kernel/src/fs/fat32.rs), needed by /bin/actualizeaza (P6-08).
    printf 'Cyber Universe: sistemul poate scrie pe acest stick (comanda actualizeaza).\n' > "$BUILD/cyber.rw.stick"
    MTOOLS_SKIP_CHECK=1 mcopy -o -i "$USB_IMG@@1M" "$BUILD/cyber.rw.stick" ::/cyber.rw
    # P6-62 (C-83): the test stick carries the tiny control model for I14 (/usb/kronos/mic_q8_0.kgr, 840 KB, random
    # weights from Kronos's control vectors; NOT an NVIDIA file).
    if [[ "$MODE" == "gpu-scrie" ]]; then
        MTOOLS_SKIP_CHECK=1 mmd -i "$USB_IMG@@1M" ::/kronos 2>/dev/null || true
        MTOOLS_SKIP_CHECK=1 mcopy -o -i "$USB_IMG@@1M" "$ROOT/tests/kronos/mic_q8_0.kgr" ::/kronos/mic_q8_0.kgr
    fi
    # P6-84 (C-114): the "single test stick": every file of this image copied onto the 64 GiB FAT32 stick (as Adrian
    # does with the stick-test-complet archive); scenario stick-complet boots from it and writes the journal.
    if [[ ",$EXTRA_FEATURES," == *",test-fat-mare,"* ]]; then
        ONE="$(mktemp -d)"
        MTOOLS_SKIP_CHECK=1 mcopy -s -i "$USB_IMG@@1M" ::/EFI ::/boot "$ONE/"
        MTOOLS_SKIP_CHECK=1 mcopy -s -i "$BUILD/fat-mare.img" "$ONE/EFI" "$ONE/boot" ::/
        rm -rf "$ONE"
    fi
    echo "[build] Gata: build/$(basename "$USB_IMG") (de scris pe stick, vezi docs/USB.md)"
fi
