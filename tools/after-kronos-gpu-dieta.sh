#!/usr/bin/env bash
# After scenario `kronos-gpu-dieta` (P6-107, C-154): the same `kronos --gpu-sim` text twice in one power-on, first with
# the driver's diet (`gpu dieta-sim on`: the pushbuffer through a shadow, the records not zeroed), then without it.
#   - two journals (001 = with the diet, 002 = without), each with I14 passed and the same 43 work-channel doorbells;
#   - `kt dieta` 1 / 0; the logits the same bit for bit (`kt amprenta …` and the worst deviation's f32 bits equal);
#   - with the diet the driver writes fewer VRAM words per token (median) than without it.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
for n in 001 002; do
    dump "gpu-jurnal-$n.txt" > "$TMP/$n" || true
    [[ -s "$TMP/$n" ]] || { echo "[after-kronos-gpu-dieta] EȘUAT: lipsește /raport/gpu-jurnal-$n.txt"; exit 1; }
done
[[ -z "$(dump gpu-jurnal-003.txt)" ]] || { echo "[after-kronos-gpu-dieta] EȘUAT: un al treilea jurnal"; exit 1; }
python3 - "$TMP/001" "$TMP/002" <<'PY'
import re, sys
def load(path):
    ev = []
    for l in open(path, encoding="utf-8", errors="replace").read().splitlines()[1:]:
        m = re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l)
        assert m, "rand fara ora: " + l
        ev.append(m.group(1))
    return ev
def val(ev, name):
    return [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + name + " = ")]
def bells(ev):
    i14 = [i for i, b in enumerate(ev) if b.startswith("pas i14-simbol:")]
    i13 = [i for i, b in enumerate(ev) if b.startswith("pas i13-strat:")]
    assert i13 and i14 and ev[i14[0]].endswith("ok"), "I14 nu a trecut"
    return len([i for i in range(i13[0], i14[0]) if ev[i].startswith("bar0 w 0xbb0090")])
on, off = load(sys.argv[1]), load(sys.argv[2])
assert val(on, "kt dieta (0/1)") == [1] and val(off, "kt dieta (0/1)") == [0], ("kt dieta", val(on, "kt dieta (0/1)"), val(off, "kt dieta (0/1)"))
b_on, b_off = bells(on), bells(off)
assert b_on == b_off == 43, f"sonerii: cu dieta {b_on}, fara {b_off} (trebuie 43 si 43)"
for name in ["kt amprenta logit-urilor (FNV-1a, toate pozitiile)", "kt abaterea maxima a logit-urilor (f32, biti)"]:
    a, b = val(on, name), val(off, name)
    assert len(a) == 1 and a == b, f"{name}: cu dieta {a}, fara {b} (trebuie identice)"
assert val(on, "kg simboluri generate pe placa (identice cu procesorul)") == [5] == val(off, "kg simboluri generate pe placa (identice cu procesorul)")
w_on, w_off = val(on, "kt cuvinte VRAM pe simbol (median)"), val(off, "kt cuvinte VRAM pe simbol (median)")
assert len(w_on) == 1 and len(w_off) == 1 and 0 < w_on[0] < w_off[0], f"cuvinte VRAM pe simbol: cu dieta {w_on}, fara {w_off}"
for name in ["kt median citirea logit-urilor (us)", "kt median argmax (us)"]:
    assert len(val(on, name)) == 1 and len(val(off, name)) == 1, name
print(f"[after-kronos-gpu-dieta] ok: aceleasi logit-uri (amprenta {val(on, 'kt amprenta logit-urilor (FNV-1a, toate pozitiile)')[0]:#010x}), {b_on} sonerii fiecare; cuvinte VRAM pe simbol {w_on[0]} cu dieta, {w_off[0]} fara")
PY
