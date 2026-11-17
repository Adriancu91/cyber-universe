#!/usr/bin/env bash
# After scenario `gpu-i9b` (P6-92, C-133): the live journal of `gpu lant --sim --i9 c133` on the written test disk.
#   001 = the whole chain with --i9 c133: one try (A) with a single PROMOTE_CTX of 3 entries (MAIN, PM, PATCH, PA + VA),
#         accepted up to the schedule (stage 5), variant A passed; all `i9 c133` rows after golden and before I9 ok;
#         no C-125 rows (`i9 c125 ...`), and the chain passed.
set -uo pipefail
IMG="${1:?disk image}"
TEXT="$(MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" ::/raport/gpu-jurnal-001.txt 2>/dev/null)"
[[ -n "$TEXT" ]] || { echo "[after-gpu-i9b] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
python3 - "$TEXT" <<'PY'
import re, sys
ev = [m.group(1) for m in (re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l) for l in sys.argv[1].splitlines()[1:]) if m]
def val(prefix):
    return [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + prefix)]
assert val("i9 c133 incercari facute") == [1], val("i9 c133 incercari facute")
assert val("i9 c133 incercarea") == [1], "C-133: prima incercare trebuia sa fie A"
assert val("i9 c133 intrari in PROMOTE_CTX") == [3], val("i9 c133 intrari in PROMOTE_CTX")
assert val("i9 c133 buf id") == [0, 1, 2], val("i9 c133 buf id")
assert all(f & 1 for f in val("i9 c133 buf b0")), "C-133 A: intrari fara PA+init"
assert val("i9 c133 ultimul pas acceptat") == [5], val("i9 c133 ultimul pas acceptat")
assert val("i9 c133 refuz RM") == [], "C-133: refuz neasteptat in SIMULARE"
assert val("i9 c133 varianta reusita") == [1], val("i9 c133 varianta reusita")
assert not [b for b in ev if b.startswith("valoare i9 c125")], "C-133: randuri C-125 in jurnal"
first = next(i for i, b in enumerate(ev) if b.startswith("valoare i9 c133 "))
golden = [i for i, b in enumerate(ev) if b.startswith("pas i8e-golden: ok")]
i9 = [i for i, b in enumerate(ev) if b.startswith("pas i9-canal-lucru: ok")]
assert golden and i9 and golden[0] < first < i9[0], "randurile i9 c133 nu sunt intre golden si pasul I9"
assert any(b.startswith("pas i14-simbol: ok") for b in ev), "lantul nu a trecut pana la I14"
print("[after-gpu-i9b] OK: o incercare (A), PROMOTE_CTX cu 3 intrari PA+VA, programat, varianta A reusita")
PY
