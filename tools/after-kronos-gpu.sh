#!/usr/bin/env bash
# After scenario `kronos-gpu` (P6-62 step 3, C-83): `kronos --gpu`.
#   - the first two runs (no `DA`; `DA` in a build that cannot write) never touched the board: exactly ONE journal;
#   - 001 = `kronos --gpu-sim` with `DA`: the whole chain, Bus Master on/off around every write, I14 = the control model
#     (1 upload + 29 tokens) + the request (1 upload + 5 prompt positions + 4 greedy = 9 tokens) = 40 work-channel
#     doorbells, nothing but the VRAM window, the TLB invalidate and the doorbell in I14, no write after it.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
[[ -n "$(dump gpu-jurnal-001.txt)" ]] || { echo "[after-kronos-gpu] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
[[ -z "$(dump gpu-jurnal-002.txt)" ]] || { echo "[after-kronos-gpu] EȘUAT: un al doilea jurnal: o rulare fara DA a atins placa"; exit 1; }
python3 - "$(dump gpu-jurnal-001.txt)" <<'PY'
import re, sys
ev = []
for l in sys.argv[1].splitlines()[1:]:
    m = re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l)
    assert m, "rand fara ora: " + l
    ev.append(m.group(1))
step = lambda n: [i for i, b in enumerate(ev) if b.startswith(f"pas {n}:")]
cfg = [i for i, b in enumerate(ev) if b.startswith("cfg  w 0x004")]
gsp = [i for i, b in enumerate(ev) if b.startswith("bar0 w ")]
assert len(cfg) == 2 and gsp and cfg[0] < gsp[0] and cfg[1] > gsp[-1], "Bus Master nu inconjoara scrierile"
off = lambda i: int(ev[i].split()[2].rstrip(":"), 16)
allowed = lambda i: off(i) in (0x1700, 0xB830A0, 0xB830A4, 0xB830B0, 0xBB0090)
i13, i14 = step("i13-strat"), step("i14-simbol")
assert i13 and i14 and ev[i14[0]].endswith("ok"), "I14 nu a trecut"
seg = [i for i in gsp if i13[0] < i < i14[0]]
assert all(allowed(i) for i in seg), f"I14: alte scrieri: {[ev[i] for i in seg if not allowed(i)][:3]}"
bells = len([i for i in seg if off(i) == 0xBB0090])
assert bells == 40, f"I14 + kronos --gpu: trebuie 40 sonerii (30 control + 10 cererea), sunt {bells}"
assert not [i for i in gsp if i14[0] < i < cfg[1]], "scriere dupa I14"
assert not [i for i, b in enumerate(ev) if i > cfg[1] and (b.startswith("cfg  w ") or b.startswith("bar0 w "))], "scriere dupa Bus Master oprit"
print(f"[after-kronos-gpu] ok: un singur lant, {len(gsp)} scrieri intre Bus Master pornit/oprit, I14 + cererea = {bells} sonerii, nimic dupa")
PY
