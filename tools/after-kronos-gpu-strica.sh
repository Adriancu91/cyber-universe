#!/usr/bin/env bash
# After scenario `kronos-gpu-strica` (P6-99, C-148 point 2): `kronos --gpu-sim --strica-verificarea 6`. /bin/kronos spoils
# its own logits at position 6 on purpose, so the check after the board must refuse exactly that position: one journal,
# the `kg` rows say position 6 (1 token generated before it), the result rows of a passing text are absent.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
[[ -n "$(dump gpu-jurnal-001.txt)" ]] || { echo "[after-kronos-gpu-strica] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
[[ -z "$(dump gpu-jurnal-002.txt)" ]] || { echo "[after-kronos-gpu-strica] EȘUAT: un al doilea jurnal"; exit 1; }
TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
dump gpu-jurnal-001.txt > "$TMP"
python3 - "$TMP" <<'PY'
import re, sys
ev = []
for l in open(sys.argv[1], encoding="utf-8", errors="replace").read().splitlines()[1:]:
    m = re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l)
    assert m, "rand fara ora: " + l
    ev.append(m.group(1))
val = lambda name: [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + name + " = ")]
i14 = [i for i, b in enumerate(ev) if b.startswith("pas i14-simbol:")]
assert i14 and ev[i14[0]].endswith("ok"), "I14 (modelul de control) trebuie sa treaca"
assert val("kg primul simbol diferit de procesor (pozitia; ffffffff = niciunul)") == [6], "kg: refuzul la pozitia 6"
assert val("kg simboluri generate pe placa (identice cu procesorul)") == [1], "kg: un simbol generat inaintea pozitiei 6"
assert not val("kv simboluri pe secunda x100 cu verificare (placa + driver + verificarea dupa placa)"), "kv: niciun rand de text trecut"
print("[after-kronos-gpu-strica] ok: verificarea dupa placa a refuzat pozitia 6 (stricata intentionat), 1 simbol generat inainte")
PY
