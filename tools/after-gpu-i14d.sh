#!/usr/bin/env bash
# After scenario `gpu-i14d` (P6-94, C-139): `gpu lant --sim --date-vechi` — the GPU model serves stale data until an
# invalidation (Cyber-GPU's hypothesis for os-305). The live journal must hold the ladder's `i14d` rows: the control
# probe (sent as in os-305) = 2 "date vechi", the repair (invalidation before every launch) = 1 "trece", the chosen
# way = the repair, and I14 passes; only the VRAM window, the TLB invalidate and the work-channel doorbell in I14.
set -uo pipefail
IMG="${1:?disk image}"
TEXT="$(MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" ::/raport/gpu-jurnal-001.txt 2>/dev/null)"
[[ -n "$TEXT" ]] || { echo "[after-gpu-i14d] EȘUAT: lipsește /raport/gpu-jurnal-001.txt"; exit 1; }
python3 - "$TEXT" <<'PY'
import re, sys
ev = []
for l in sys.argv[1].splitlines()[1:]:
    m = re.match(r"\[\s*\d+\.\d{6}\] gpu: (.*)$", l)
    if m:
        ev.append(m.group(1))
val = lambda key: [int(b.rsplit("= ", 1)[1], 16) for b in ev if b.startswith("valoare i14d " + key)]
probes = val("proba (")
verdicts = val("rezultat (")
assert probes[:2] == [1, 2], f"scara: probele trebuie 1 (ca os-305) apoi 2 (reparatia), sunt {probes}"
assert verdicts[:2] == [2, 1], f"scara: rezultatele trebuie 2 (date vechi) apoi 1 (trece), sunt {verdicts}"
chosen = [b for b in ev if b.startswith("valoare i14d") and "modul ales" in b]
assert chosen and chosen[-1].endswith("= 0x00000002"), f"felul ales trebuie 2 (invalidare inaintea fiecarei lansari): {chosen}"
step = lambda n: [i for i, b in enumerate(ev) if b.startswith(f"pas {n}:")]
i13, i14 = step("i13-strat"), step("i14-simbol")
assert i13 and i14 and ev[i14[0]].endswith("ok"), "I14 nu a trecut"
seg = [i for i in range(i13[0] + 1, i14[0]) if ev[i].startswith("bar0 w ")]
off = lambda i: int(ev[i].split()[2].rstrip(":"), 16)
# P6-98 (C-148): the last thing in I14 is ONE GSP-RM read of the performance state (`ks`): its GSP-window writes come
# after the last work-channel doorbell and before the `ks` row.
ks = [i for i, b in enumerate(ev) if b.startswith("valoare ks stare de performanta la final")]
assert len(ks) == 1 and i13[0] < ks[0] < i14[0], "ks: un singur rand, in I14"
bell_i = [i for i in seg if off(i) == 0xBB0090]
gsp_rm = [i for i in seg if 0x110000 <= off(i) <= 0x113FFF]
assert all(bell_i[-1] < i < ks[0] for i in gsp_rm), "ks: scrierile GSP trebuie sa fie dupa ultima sonerie si inaintea randului ks"
assert all(off(i) in (0x1700, 0xB830A0, 0xB830A4, 0xB830B0, 0xBB0090) or i in gsp_rm for i in seg), "I14: alte scrieri decat fereastra, TLB, soneria (+ citirea ks)"
bells = len(bell_i)
assert bells == 32, f"I14: trebuie 32 sonerii (urcarea + 2 probe + 29 simboluri), sunt {bells}"
print(f"[after-gpu-i14d] OK: scara = proba os-305 DATE VECHI, reparatia TRECE, textele cu invalidare inaintea fiecarei lansari; I14 ok, {bells} sonerii")
PY
