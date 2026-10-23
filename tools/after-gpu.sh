#!/usr/bin/env bash
# After scenario `gpu` (P6-11): the `gpu` path must contain no write to a card. Checks the source of
# kernel/src/drivers/gpu.rs (everything before the test-only block of scenario `gpu-ro`):
# no volatile writes, no PCI configuration writes, no Command/power changes, no writable mapping.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/kernel/src/drivers/gpu.rs"
body="$(sed '/#\[cfg(feature = "test-gpu-write")\]/,$d' "$SRC" | grep -v '^\s*//')"
bad="$(echo "$body" | grep -nE 'write_volatile|write32|write_bytes|\.enable|enable_as|power_on|set_command|WRITABLE|as \*mut' || true)"
if [[ -n "$bad" ]]; then
    echo "[after-gpu] EȘUAT: scriere posibilă în calea gpu:"; echo "$bad"; exit 1
fi
echo "[after-gpu] ok: nicio scriere în calea gpu (sursa verificată)"
