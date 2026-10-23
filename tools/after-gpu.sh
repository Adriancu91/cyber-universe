#!/usr/bin/env bash
# After scenario `gpu` (P6-11): the `gpu` path must contain no write to a card. Checks the source of
# kernel/src/drivers/gpu.rs (everything before the test-only block of scenario `gpu-ro`):
# no volatile writes, no PCI configuration writes, no Command/power changes, no writable mapping.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/kernel/src/drivers/gpu.rs"
# P6-19: `gpu identifica --sim` (test builds only) sizes the BARs of the gpu-sim MODEL; that block is left out here,
# and BAR sizing must not appear anywhere else.
body="$(sed '/#\[cfg(feature = "test-gpu-write")\]/,$d' "$SRC" \
    | sed '/^#\[cfg(feature = "test-gpu-sim")\]/,/^}/d' | grep -v '^\s*//')"
# P6-19: the Cyber-GPU adapters must implement `write32` / `write32_raw` of the hardware traits; their bodies may only
# count and log the refusal. Each must be exactly: signature, `self.refused += 1;`, one `crate::warn!` line, `}`.
for f in write32 write32_raw; do
    impl="$(echo "$body" | grep -A3 "fn $f(&mut self" || true)"
    if [[ -z "$impl" ]] || ! echo "$impl" | sed -n 2p | grep -qE '^\s*self\.refused \+= 1;$' \
        || ! echo "$impl" | sed -n 3p | grep -qE '^\s*crate::warn!\("gpu: scriere refuzata' \
        || ! echo "$impl" | sed -n 4p | grep -qE '^\s*\}$'; then
        echo "[after-gpu] EȘUAT: $f din adaptorul cyber-gpu nu doar refuză scrierea:"; echo "$impl"; exit 1
    fi
done
body="$(echo "$body" | grep -vE 'fn write32(_raw)?\(&mut self' | grep -vE 'crate::warn!\("gpu: scriere refuzata')"
bad="$(echo "$body" | grep -nE 'write_volatile|write32|write_bytes|\.enable|enable_as|power_on|set_command|WRITABLE|as \*mut|size_bars|SizingPermit|checked_write32' || true)"
if [[ -n "$bad" ]]; then
    echo "[after-gpu] EȘUAT: scriere posibilă în calea gpu:"; echo "$bad"; exit 1
fi
echo "[after-gpu] ok: nicio scriere în calea gpu (sursa verificată)"
