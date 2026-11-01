#!/usr/bin/env bash
# After scenario `gpu-i6` (P6-40, C-52): the live journals on the written test disk. File 002 is `gpu i6 --scrie --sim`:
# header, every line with its time, Bus Master on (first config write) before the first GSP write, the falcon start,
# then Bus Master back (last config write) after the last GSP write, times never going back. File 003 (FWSEC error)
# must also end with Bus Master back.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
for f in gpu-jurnal-001.txt gpu-jurnal-002.txt gpu-jurnal-003.txt; do
    [[ -n "$(dump "$f")" ]] || { echo "[after-gpu-i6] EȘUAT: lipsește /raport/$f"; exit 1; }
done
python3 - "$(dump gpu-jurnal-002.txt)" "$(dump gpu-jurnal-003.txt)" <<'PY'
import re, sys
for k, text in enumerate(sys.argv[1:]):
    lines = text.splitlines()
    assert lines[0].startswith("# jurnal gpu P6-40"), lines[0]
    t_prev, kinds = -1.0, []
    for l in lines[1:]:
        m = re.match(r"\[\s*(\d+)\.(\d{6})\] gpu: (.*)$", l)
        assert m, "rand fara ora: " + l
        t = int(m.group(1)) + int(m.group(2)) / 1e6
        assert t >= t_prev, "ora merge inapoi: " + l
        t_prev = t
        body = m.group(3)
        if body.startswith("cfg  w 0x004"): kinds.append(("cfg", body))
        elif body.startswith("bar0 w "): kinds.append(("bar0", body))
    cfg = [i for i, (w, _) in enumerate(kinds) if w == "cfg"]
    gsp = [i for i, (w, _) in enumerate(kinds) if w == "bar0"]
    assert len(cfg) == 2 and cfg[0] < gsp[0] and cfg[1] > gsp[-1], "Bus Master nu inconjoara I6"
    assert kinds[cfg[0]][1].endswith("<- 0x00000007 => 0x00100007"), kinds[cfg[0]][1]
    assert kinds[cfg[1]][1].endswith("<- 0x00000003 => 0x00100003"), kinds[cfg[1]][1]
    assert all(0x110000 <= int(b.split()[2].rstrip(":"), 16) <= 0x113FFF for _, b in [kinds[i] for i in gsp])
    assert kinds[gsp[-1]][1].startswith("bar0 w 0x110100"), "ultima scriere GSP nu e pornirea falcon-ului"
    print(f"[after-gpu-i6] ok: jurnalul {2 + k}: {len(lines) - 1} randuri cu ora, Bus Master pornit/oprit in jurul a {len(gsp)} scrieri GSP")
PY
