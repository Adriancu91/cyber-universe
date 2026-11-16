#!/usr/bin/env bash
# After scenario `gpu-i9` (P6-91, C-125): the live journal of `gpu lant --sim --i9 c125` on the written test disk.
#   001 = the whole chain with I9Options::C125: the option row (= 3: V2 + V3), RM's answer to GR_GET_CTX_BUFFER_INFO
#         (status 0 and the list on the gpu-sim model), every context buffer of ours mapped privileged (V2), all the `i9`
#         rows after the I9 step's messages and before I10, and the chain passed.
set -uo pipefail
IMG="${1:?disk image}"
TEXT="$(MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" ::/raport/gpu-jurnal-001.txt 2>/dev/null)"
[[ -n "$TEXT" ]] || { echo "[after-gpu-i9] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
python3 - "$TEXT" <<'PY'
import re, sys
ev = [m.group(1) for m in (re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l) for l in sys.argv[1].splitlines()[1:]) if m]
def val(prefix):
    return [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + prefix)]
assert val("i9 c125 optiuni") == [3], val("i9 c125 optiuni")
assert val("i9 v3 GR_GET_CTX_BUFFER_INFO stare") == [0], "V3: raspunsul RM lipseste"
n = val("i9 v3 buffere in lista RM")
assert n and n[0] > 0, "V3: lista RM goala"
priv = val("i9 ctx buf privilegiat (V2)")
assert priv and all(p == 1 for p in priv), f"V2: buffere neprivilegiate {priv}"
first = next(i for i, b in enumerate(ev) if b.startswith("valoare i9 "))
golden = [i for i, b in enumerate(ev) if b.startswith("pas i8e-golden: ok")]
i9 = [i for i, b in enumerate(ev) if b.startswith("pas i9-canal-lucru: ok")]
assert golden and i9 and golden[0] < first < i9[0], "randurile i9 nu sunt intre golden si pasul I9"
assert any(b.startswith("pas i14-simbol: ok") for b in ev), "lantul nu a trecut pana la I14"
print(f"[after-gpu-i9] OK: optiuni 3, lista RM {n[0]} buffere, {len(priv)} buffere de context privilegiate")
PY
