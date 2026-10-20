#!/usr/bin/env bash
# Automated headless test: builds the test ISO, boots it in QEMU and checks the serial output.
#   tools/test.sh                  standard boot test
#   tools/test.sh --no-build       reuse the existing test ISO
#   tools/test.sh --expect "TEXT"  additional text that must appear (can repeat)
#   tools/test.sh --screenshot tests/screens/boot.png   save the screen and check it is not blank
# Exit code: 0 = pass, 1 = fail.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/qemu-common.sh"

BUILD_FIRST=1
EXTRA=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build) BUILD_FIRST=0; shift ;;
        --expect) EXTRA+=(--expect "$2"); shift 2 ;;
        --screenshot) EXTRA+=(--screenshot "$2"); shift 2 ;;
        *) echo "Opțiune necunoscută: $1" >&2; exit 2 ;;
    esac
done

if (( BUILD_FIRST )); then
    mkdir -p "$BUILD"
    if ! "$ROOT/tools/build.sh" --test > "$BUILD/build-test.log" 2>&1; then
        echo "[test] build eșuat:"; tail -30 "$BUILD/build-test.log"; exit 1
    fi
fi

find_ovmf
mapfile -t ARGS < <(qemu_args "$BUILD/cyber-universe-test.iso")
# qemutest.py adds -cdrom itself; drop it from the common arguments.
FILTERED=()
skip=0
for a in "${ARGS[@]}"; do
    if (( skip )); then skip=0; continue; fi
    if [[ "$a" == "-cdrom" ]]; then skip=1; continue; fi
    FILTERED+=("$a")
done

python3 "$ROOT/tools/qemutest.py" \
    --iso "$BUILD/cyber-universe-test.iso" \
    --log "$BUILD/serial.log" \
    --timeout 60 \
    --expect "CYBER UNIVERSE OS: BOOT OK" \
    "${EXTRA[@]}" \
    --expect "TEST READY" \
    -- "${FILTERED[@]}"
