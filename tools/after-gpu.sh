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
# P6-21: there are several adapters (ConfigSpace, the BAR0 page, the 4 MiB window): every one is checked.
for f in write32 write32_raw; do
    starts="$(echo "$body" | grep -n "fn $f(&mut self" | cut -d: -f1 || true)"
    if [[ -z "$starts" ]]; then echo "[after-gpu] EȘUAT: $f lipsește din adaptoarele cyber-gpu"; exit 1; fi
    for n in $starts; do
        impl="$(echo "$body" | sed -n "${n},$((n + 3))p")"
        if ! echo "$impl" | sed -n 2p | grep -qE '^\s*self\.refused \+= 1;$' \
            || ! echo "$impl" | sed -n 3p | grep -qE '^\s*crate::warn!\("gpu: scriere refuzata' \
            || ! echo "$impl" | sed -n 4p | grep -qE '^\s*\}$'; then
            echo "[after-gpu] EȘUAT: $f din adaptorul cyber-gpu nu doar refuză scrierea:"; echo "$impl"; exit 1
        fi
    done
done
body="$(echo "$body" | grep -vE 'fn write32(_raw)?\(&mut self' | grep -vE 'crate::warn!\("gpu: scriere refuzata')"
bad="$(echo "$body" | grep -nE 'write_volatile|write32|write_bytes|\.enable|enable_as|power_on|set_command|WRITABLE|as \*mut|size_bars|SizingPermit|checked_write32' || true)"
if [[ -n "$bad" ]]; then
    echo "[after-gpu] EȘUAT: scriere posibilă în calea gpu:"; echo "$bad"; exit 1
fi
echo "[after-gpu] ok: nicio scriere în calea gpu (sursa verificată)"

# P6-40 (C-52): kernel/src/drivers/gpu_i6.rs. Without the `gpu-scrie` build option it must not be able to write to a card:
# everything that can (the `hw` module, `HardwareWritePermit::new`) sits behind `#[cfg(feature = "gpu-scrie")]`; the rest
# (minus the test-only SIMULARE functions) has no write at all. Inside `hw`: configuration writes only to Command,
# BAR0 writes only behind the GSP-window check.
I6="$ROOT/kernel/src/drivers/gpu_i6.rs"
# P6-42: `hw` is `pub(super)` (gpu_tur.rs uses it, also only in the gpu-scrie build; checked below).
if ! grep -B1 -nE '^(pub\(super\) )?mod hw \{' "$I6" | head -1 | grep -q '#\[cfg(feature = "gpu-scrie")\]'; then
    echo "[after-gpu] EȘUAT: modulul hw din gpu_i6.rs nu e doar în build-ul gpu-scrie"; exit 1
fi
rest="$(sed -E '/^(pub\(super\) )?mod hw \{/,/^}/d' "$I6" \
    | sed '/^#\[cfg(feature = "test-gpu-sim")\]/,/^}/d' \
    | sed '/^#\[cfg(feature = "gpu-scrie")\]/,/^}/d' | grep -v '^\s*//')"
bad="$(echo "$rest" | grep -nE 'write_volatile|write32|write_bytes|\.enable|enable_as|power_on|set_command|WRITABLE|as \*mut|size_bars|SizingPermit|checked_write32|HardwareWritePermit::new|hw::' || true)"
if [[ -n "$bad" ]]; then
    echo "[after-gpu] EȘUAT: scriere posibilă în gpu_i6.rs în afara build-ului gpu-scrie:"; echo "$bad"; exit 1
fi
hw="$(sed -nE '/^(pub\(super\) )?mod hw \{/,/^}/p' "$I6" | grep -v '^\s*//')"
# P6-56 step 2b: a second BAR0 fence, `VramFence` (golden, I9): only the VRAM window's control registers and PRAMIN.
if [[ "$(echo "$hw" | grep -c 'write_volatile')" != 3 ]] || [[ "$(echo "$hw" | grep -c 'dev\.write32(')" != 1 ]] \
    || ! echo "$hw" | grep -q 'self.dev.write32(PCI_COMMAND, value)' \
    || ! echo "$hw" | grep -q '(GSP_LO..=GSP_HI - 3).contains(&offset)' \
    || ! echo "$hw" | grep -q 'vram_window::CONTROL_REGISTERS.iter().any(|&r| u64::from(r) == offset)' \
    || ! echo "$hw" | grep -q '(PRAMIN_LO..=PRAMIN_HI - 3).contains(&offset)' \
    || ! echo "$hw" | grep -q 'pub(crate) const PRAMIN_LO: u64 = 0x70_0000;' \
    || ! echo "$hw" | grep -q 'pub(crate) const PRAMIN_HI: u64 = 0x7F_FFFF;' \
    || ! echo "$hw" | grep -q 'if offset != nvidia_ga104::push::DOORBELL || value != self.token {'; then
    echo "[after-gpu] EȘUAT: gardul scrierilor din gpu_i6.rs (Command / fereastra GSP / fereastra VRAM) s-a schimbat"; exit 1
fi
if grep -v '^\s*//' "$ROOT/kernel/src/drivers/gpu_golden.rs" | grep -nE 'write_volatile|as \*mut|HardwareWritePermit::new|WRITABLE'; then
    echo "[after-gpu] EȘUAT: gpu_golden.rs scrie direct (trebuie doar prin bibliotecă și gardurile din gpu_i6.rs)"; exit 1
fi
# P6-56 step 3 (I10): gpu_i10.rs writes nothing itself (only through the library, the VRAM fence and the doorbell fence),
# and its program `add64` is the file Cyber-GPU compiled (same bytes as their tools/nak/out/add64.sm86.sass).
if grep -v '^\s*//' "$ROOT/kernel/src/drivers/gpu_i10.rs" | grep -nE 'write_volatile|as \*mut|HardwareWritePermit::new|WRITABLE|checked_write32'; then
    echo "[after-gpu] EȘUAT: gpu_i10.rs scrie direct (trebuie doar prin bibliotecă și gardurile din gpu_i6.rs)"; exit 1
fi
ADD64="$ROOT/kernel/src/drivers/gpu_prog/add64.sm86.sass"
ADD64_SHA256="a890b75c4207b216c2f959da3b59745dbb43f3cbd58490b37b7b4c1dd12698ce"  # cyber-gpu 20b4a3b (identic in 60740a3, 657e209)
if [[ "$(sha256sum "$ADD64" | cut -d' ' -f1)" != "$ADD64_SHA256" ]]; then
    echo "[after-gpu] EȘUAT: add64.sm86.sass nu e fișierul cyber-gpu (SHA-256 diferit)"; exit 1
fi
rev="$(grep -oE 'nvidia-ga104 = \{ git = "[^"]+", rev = "[0-9a-f]+"' "$ROOT/kernel/Cargo.toml" | grep -oE '[0-9a-f]{7,}"$' | tr -d '"')"
for d in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/cyber-gpu-*/"$rev"*/; do
    if [[ -f "$d/tools/nak/out/add64.sm86.sass" ]] && ! cmp -s "$d/tools/nak/out/add64.sm86.sass" "$ADD64"; then
        echo "[after-gpu] EȘUAT: add64.sm86.sass diferă de cel din cyber-gpu $rev ($d)"; exit 1
    fi
done
echo "[after-gpu] ok: gpu_i6.rs scrie doar în build-ul gpu-scrie, doar Command (Bus Master), fereastra GSP, fereastra VRAM și soneria canalului (I10)"

# P6-42 (C-55): kernel/src/drivers/gpu_tur.rs (`gpu tur`, `gpu lant`). Every item or match arm that can write (the `hw`
# adapters, Bus Master, the falcon load, the write permit) must carry `#[cfg(feature = "gpu-scrie")]` (or be a test-only
# SIMULARE item, `test-gpu-sim`); what is left after removing those items has no write at all.
TUR="$ROOT/kernel/src/drivers/gpu_tur.rs"
python3 - "$TUR" <<'PY' || exit 1
import re, sys
src = open(sys.argv[1]).read().splitlines()
out, i = [], 0
while i < len(src):
    l = src[i]
    if re.match(r'\s*#\[cfg\(feature = "(gpu-scrie|test-gpu-sim)"\)\]\s*$', l):
        # Skip the next item / statement / match arm: until brackets balance and the line ends it.
        depth, j = 0, i + 1
        while j < len(src):
            code = re.sub(r'"(\\.|[^"\\])*"', '""', src[j].split("//")[0])
            depth += code.count("{") + code.count("(") + code.count("[") - code.count("}") - code.count(")") - code.count("]")
            if depth <= 0 and re.search(r'[};,]\s*$', code):
                break
            j += 1
        i = j + 1
        continue
    out.append(l)
    i += 1
body = "\n".join(x for x in out if not x.strip().startswith("//"))
bad = [m.group(0) for m in re.finditer(r'write_volatile|write32|write_bytes|WRITABLE|as \*mut|HardwareWritePermit::new|hw::|set_bus_master|restore_command|i6_falcon|dma_put', body)]
if bad:
    print("[after-gpu] EȘUAT: scriere posibilă în gpu_tur.rs în afara build-ului gpu-scrie:", sorted(set(bad)))
    sys.exit(1)
print("[after-gpu] ok: gpu_tur.rs scrie doar în build-ul gpu-scrie (turul și lanțul doar-citire nu au nicio scriere)")
PY
