#!/usr/bin/env bash
# After scenario `gpu-i9c` (P6-93, C-135): the live journal of `gpu lant --sim --i9 c135` on the written test disk.
#   001 = the whole chain with --i9 c135: one try (N, channel without error notifier) with a single PROMOTE_CTX in form B
#         (4 entries: MAIN + PATCH with PA + VA, FECS_EVENT + PRIV_ACCESS_MAP VA only), token from RM, scheduled (stage 6),
#         variant N passed; all `i9 c135` rows after golden and before I9 ok; no C-125/C-133 rows, and the chain passed.
set -uo pipefail
IMG="${1:?disk image}"
TEXT="$(MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" ::/raport/gpu-jurnal-001.txt 2>/dev/null)"
[[ -n "$TEXT" ]] || { echo "[after-gpu-i9c] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
python3 - "$TEXT" <<'PY'
import re, sys
ev = [m.group(1) for m in (re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l) for l in sys.argv[1].splitlines()[1:]) if m]
def val(prefix):
    return [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + prefix)]
assert val("i9 c135 incercari facute") == [1], val("i9 c135 incercari facute")
assert val("i9 c135 incercarea") == [1], "C-135: prima incercare trebuia sa fie N"
assert val("i9 c135 intrari in PROMOTE_CTX") == [4], val("i9 c135 intrari in PROMOTE_CTX")
flags = val("i9 c135 buf b0")
assert len(flags) == 4 and [f & 1 for f in flags] == [1, 1, 0, 0], ("C-135 forma B: MAIN+PATCH cu PA, globalele doar VA", flags)
assert val("i9 c135 jeton") == [1], ("C-135 N: jetonul trebuia sa vina de la RM", val("i9 c135 jeton"))
assert val("i9 c135 ultimul pas acceptat") == [6], val("i9 c135 ultimul pas acceptat")
assert val("i9 c135 refuz RM") == [], "C-135: refuz neasteptat in SIMULARE"
assert val("i9 c135 varianta reusita") == [1], val("i9 c135 varianta reusita")
assert not [b for b in ev if b.startswith("valoare i9 c125") or b.startswith("valoare i9 c133")], "C-135: randuri C-125/C-133 in jurnal"
first = next(i for i, b in enumerate(ev) if b.startswith("valoare i9 c135 "))
golden = [i for i, b in enumerate(ev) if b.startswith("pas i8e-golden: ok")]
i9 = [i for i, b in enumerate(ev) if b.startswith("pas i9-canal-lucru: ok")]
assert golden and i9 and golden[0] < first < i9[0], "randurile i9 c135 nu sunt intre golden si pasul I9"
assert any(b.startswith("pas i14-simbol: ok") for b in ev), "lantul nu a trecut pana la I14"
print("[after-gpu-i9c] OK: o incercare (N), PROMOTE_CTX forma B cu 4 intrari, jeton de la RM, programat, varianta N reusita")
PY
