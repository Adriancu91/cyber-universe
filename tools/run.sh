#!/usr/bin/env bash
# Starts Cyber Universe OS in a QEMU window (for watching it on screen).
#   tools/run.sh            build + run
#   tools/run.sh --no-build run the existing ISO
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/qemu-common.sh"

[[ "${1:-}" == "--no-build" ]] || "$ROOT/tools/build.sh"
find_ovmf
mapfile -t ARGS < <(qemu_args "$BUILD/cyber-universe.iso")
exec qemu-system-x86_64 "${ARGS[@]}" -serial stdio
