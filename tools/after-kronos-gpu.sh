#!/usr/bin/env bash
# After scenario `kronos-gpu` (P6-62 step 3, C-83): `kronos --gpu`.
#   - the first two runs (no `DA`; `DA` in a build that cannot write) never touched the board: exactly ONE journal;
#   - 001 = `kronos --gpu-sim` with `DA`: the whole chain, Bus Master on/off around every write, I14 = the control model
#     (1 upload + 2 ladder probes of C-139 + 29 tokens) + the request (1 upload + 5 prompt positions + 4 greedy = 9 tokens) = 42 work-channel
#     doorbells, nothing but the VRAM window, the TLB invalidate and the doorbell in I14, no write after it.
#   - P6-97 (C-146): the request's routine rows are held in memory and written once per token: the journal still has every
#     access (the same counts as above), plus the `kt` rows (time per token, split) and the shadow read back = 0.
#   - P6-98 (C-148): + the `kp` rows (time per kind), the VRAM → VRAM copy (one more doorbell = 43, `kc` rows, 0 wrong
#     words) and, last in I14, ONE GSP-RM read of the performance state (`ks`): its GSP-window writes come after the last
#     work-channel doorbell and before the `ks` row, and nothing else.
#   - P6-99 (C-148 point 2): the request is checked AFTER the board by /bin/kronos's fast engine: the same 43 doorbells
#     (the board runs the same 9 positions), the `kv` rows of the check after the board instead of the step_cpu ones.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
[[ -n "$(dump gpu-jurnal-001.txt)" ]] || { echo "[after-kronos-gpu] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
[[ -z "$(dump gpu-jurnal-002.txt)" ]] || { echo "[after-kronos-gpu] EȘUAT: un al doilea jurnal: o rulare fara DA a atins placa"; exit 1; }
# P6-98: the journal goes to python as a file (with the `kp`/`kc`/`ks` rows it is too long for one argument).
TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
dump gpu-jurnal-001.txt > "$TMP"
python3 - "$TMP" <<'PY'
import re, sys
ev = []
for l in open(sys.argv[1], encoding="utf-8", errors="replace").read().splitlines()[1:]:
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
# P6-98: the `ks` read (GSP-RM) is the last thing in I14: GSP-window writes only between the last doorbell and its row.
ks = [i for i, b in enumerate(ev) if b.startswith("valoare ks stare de performanta la final")]
assert len(ks) == 1 and i13[0] < ks[0] < i14[0], "ks: un singur rand, in I14"
bell_i = [i for i in seg if off(i) == 0xBB0090]
gsp_rm = [i for i in seg if 0x110000 <= off(i) <= 0x113FFF]
assert gsp_rm and all(bell_i[-1] < i < ks[0] for i in gsp_rm), "ks: scrierile GSP trebuie sa fie dupa ultima sonerie si inaintea randului ks"
assert all(allowed(i) or i in gsp_rm for i in seg), f"I14: alte scrieri: {[ev[i] for i in seg if not allowed(i) and i not in gsp_rm][:3]}"
bells = len(bell_i)
assert bells == 43, f"I14 + kronos --gpu: trebuie 43 sonerii (30 control + 2 scara C-139 + 10 cererea + 1 copierea VRAM), sunt {bells}"
assert not [i for i in gsp if i14[0] < i < cfg[1]], "scriere dupa I14"
assert not [i for i, b in enumerate(ev) if i > cfg[1] and (b.startswith("cfg  w ") or b.startswith("bar0 w "))], "scriere dupa Bus Master oprit"
# P6-95 (C-141): the request's result and speed rows in the journal (they go up to the hub with it), inside I14.
val = lambda name: [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare " + name + " = ")]
kv = [i for i, b in enumerate(ev) if b.startswith("valoare kv ") or b.startswith("valoare kg ") or b.startswith("text kg ")]
assert kv and i13[0] < kv[0] and kv[-1] < i14[0], "randurile kg/kv trebuie sa fie in I14"
assert val("kg simboluri cerute") == [5] and val("kg simboluri generate pe placa (identice cu procesorul)") == [5], "kg: 5 cerute, 5 generate"
assert val("kg primul simbol diferit de procesor (pozitia; ffffffff = niciunul)") == [0xFFFFFFFF], "kg: niciun simbol diferit"
assert val("kv sursa (0 SIMULARE, 1 placa)") == [0] and val("kv simboluri") == [9], ("kv", val("kv simboluri"))
assert val("kv urcare bucati") == [1], "kv: urcarea"
# P6-99 (C-148 point 2): the check after the board by /bin/kronos's fast engine: its own rows, never the step_cpu ones.
assert not val("kv simboluri pe secunda x100 (motorul procesorului)"), "kv: step_cpu nu mai verifica textul cererii"
assert len(val("kv verificarea dupa placa median pe pozitie (us, motorul rapid /bin/kronos)")) == 1, "kv: verificarea dupa placa"
assert len(val("kv pozitii pe secunda x100 (verificarea dupa placa)")) == 1, "kv: pozitii pe secunda la verificare"
wc = val("kv simboluri pe secunda x100 cu verificare (placa + driver + verificarea dupa placa)")
assert len(wc) == 1 and 0 < wc[0] < 0xFFFFFFFF, ("kv: cu verificare", wc)
# P6-97 (C-146): the time per token, split (`kt …`), inside I14; the launch slots read back = the shadow.
kt = [i for i, b in enumerate(ev) if b.startswith("valoare kt ")]
assert kt and i13[0] < kt[0] and kt[-1] < i14[0], "randurile kt trebuie sa fie in I14"
assert val("kt simboluri") == [9] and len(val("kt median jurnal (us)")) == 1 and val("kt median ecran (us)") == [0], "kt: 9 simboluri, jurnal, ecran 0"
assert val("kt umbra lansarilor: cuvinte schimbate de placa in sloturi (0 = umbra corecta)") == [0], "kt: umbra lansarilor"
# P6-98 (C-147 stage 0): kp (9 tokens, every kind), kl (16 empty launches, from I10), kc (3 copies, 0 wrong words).
assert val("kp simboluri") == [9] and len(val("kp toate simbol (us)")) == 1, "kp: 9 simboluri"
assert val("kl lansari goale") == [16], "kl: 16 lansari goale"
assert val("kc copieri") == [3] and len(val("kc copiere (us)")) == 3 and val("kc cuvinte verificate gresite (din 3)") == [0], "kc: 3 copieri, 0 cuvinte gresite"
print(f"[after-kronos-gpu] ok: un singur lant, {len(gsp)} scrieri intre Bus Master pornit/oprit, I14 + cererea = {bells} sonerii, nimic dupa")
PY
