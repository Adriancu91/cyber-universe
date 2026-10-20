#!/usr/bin/env bash
# Checks (and, with --install, installs) the tools needed to build and test Cyber Universe OS.
# Exit code: 0 = everything present, 1 = something missing.
set -uo pipefail

INSTALL=0
[[ "${1:-}" == "--install" ]] && INSTALL=1

SUDO=""
if [[ $EUID -ne 0 ]] && command -v sudo >/dev/null 2>&1; then SUDO="sudo"; fi

missing=0
need_cmd() {
    if command -v "$1" >/dev/null 2>&1; then
        printf '  %-22s %s\n' "$1" "$("$1" --version 2>/dev/null | head -n1)"
    else
        printf '  %-22s LIPSEȘTE\n' "$1"
        missing=1
    fi
}

ovmf_code() {
    for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/qemu/OVMF.fd; do
        [[ -f "$f" ]] && { echo "$f"; return 0; }
    done
    return 1
}

if (( INSTALL )); then
    if ! command -v qemu-system-x86_64 >/dev/null || ! command -v xorriso >/dev/null || ! ovmf_code >/dev/null \
        || ! command -v mkfs.fat >/dev/null || ! command -v mcopy >/dev/null || ! command -v sfdisk >/dev/null; then
        echo "Instalez QEMU, OVMF, xorriso, unelte FAT..."
        $SUDO apt-get update -qq && $SUDO apt-get install -y -qq qemu-system-x86 ovmf xorriso make curl python3 \
            dosfstools mtools fdisk >/dev/null
    fi
    if ! command -v rustup >/dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi
    rustup toolchain install nightly --profile minimal -c rust-src -t x86_64-unknown-none >/dev/null 2>&1
fi

echo "Unelte:"
export PATH="$PATH:/usr/sbin:/sbin"   # mkfs.fat and sfdisk live there
for c in git curl make xorriso qemu-system-x86_64 python3 rustup cargo mkfs.fat mcopy sfdisk; do need_cmd "$c"; done

if command -v rustup >/dev/null 2>&1; then
    if rustup run nightly rustc --version >/dev/null 2>&1; then
        printf '  %-22s %s\n' "rustc (nightly)" "$(rustup run nightly rustc --version)"
    else
        printf '  %-22s LIPSEȘTE\n' "rustc (nightly)"; missing=1
    fi
fi

if f=$(ovmf_code); then
    printf '  %-22s %s\n' "OVMF" "$f"
else
    printf '  %-22s LIPSEȘTE\n' "OVMF"; missing=1
fi

if (( missing )); then
    echo "REZULTAT: lipsesc unelte. Rulează: tools/env-check.sh --install"
    exit 1
fi
echo "REZULTAT: mediul e complet."
