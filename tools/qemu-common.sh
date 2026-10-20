#!/usr/bin/env bash
# Shared QEMU settings for run.sh and test.sh. Source this file; do not run it.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$ROOT/build"

find_ovmf() {
    local code vars
    for pair in \
        "/usr/share/OVMF/OVMF_CODE_4M.fd:/usr/share/OVMF/OVMF_VARS_4M.fd" \
        "/usr/share/OVMF/OVMF_CODE.fd:/usr/share/OVMF/OVMF_VARS.fd" \
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd:/usr/share/edk2/x64/OVMF_VARS.4m.fd"; do
        code="${pair%%:*}"; vars="${pair##*:}"
        if [[ -f "$code" && -f "$vars" ]]; then
            OVMF_CODE="$code"; OVMF_VARS_TEMPLATE="$vars"; return 0
        fi
    done
    echo "EROARE: nu găsesc firmware-ul OVMF. Rulează tools/env-check.sh --install" >&2
    return 1
}

# Prints the common QEMU arguments (one per line) for the given ISO.
qemu_args() {
    local iso="$1"
    mkdir -p "$BUILD/ovmf"
    cp "$OVMF_VARS_TEMPLATE" "$BUILD/ovmf/vars.fd"   # the firmware writes to this file at boot
    printf '%s\n' \
        -M q35 -m 256M -no-reboot \
        -drive "if=pflash,unit=0,format=raw,file=$OVMF_CODE,readonly=on" \
        -drive "if=pflash,unit=1,format=raw,file=$BUILD/ovmf/vars.fd" \
        -cdrom "$iso"
    # The test disk (made by build.sh) as a SATA disk on the q35 AHCI controller. Changes are thrown
    # away (snapshot), unless DISK_IMG names a copy the test may change (test.sh, `disk=` in scenarios).
    local disk="${DISK_IMG:-$BUILD/disk.img}" snapshot=on
    [[ -n "${DISK_IMG:-}" ]] && snapshot=off
    if [[ -f "$disk" ]]; then
        printf '%s\n' -drive "if=none,id=sata0,format=raw,snapshot=$snapshot,file=$disk" -device ide-hd,drive=sata0,bus=ide.0
    fi
    # Hardware acceleration when available (Linux/WSL2 with KVM); the cloud usually has none.
    if [[ -w /dev/kvm ]]; then printf '%s\n' -enable-kvm -cpu host; fi
}
