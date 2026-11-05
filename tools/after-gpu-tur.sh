#!/usr/bin/env bash
# After scenario `gpu-tur` (P6-42, C-55): the live journals on the written test disk.
#   001 = `gpu tur --sim`: no write at all.
#   002 = `gpu lant --sim`: Bus Master on before the first GSP write, back after the last one, every write inside the
#         GSP window 0x110000..0x113FFF or (P6-56, I7 only) the SEC2 window 0x840000..0x843FFF, the step lines in
#         chain order, I7 passed (SEC2 mailbox 0 = the GspFwWprMeta address), I8 passed with only the GSP doorbell, golden stopped
#         honestly, the tour after.
#   003 = `gpu lant --sim --eroare`: the same shape, I6 failed, Bus Master still back, the tour still after it.
#   004 = `gpu lant --sim --doar-citire`: no write at all, the chain stopped before Bus Master, the tour still ran.
set -uo pipefail
IMG="${1:?disk image}"
dump() { MTOOLS_SKIP_CHECK=1 mtype -i "$IMG@@1M" "::/raport/$1" 2>/dev/null; }
for f in gpu-jurnal-001.txt gpu-jurnal-002.txt gpu-jurnal-003.txt gpu-jurnal-004.txt; do
    [[ -n "$(dump "$f")" ]] || { echo "[after-gpu-tur] EȘUAT: lipsește /raport/$f"; exit 1; }
done
python3 - "$(dump gpu-jurnal-001.txt)" "$(dump gpu-jurnal-002.txt)" "$(dump gpu-jurnal-003.txt)" "$(dump gpu-jurnal-004.txt)" <<'PY'
import re, sys

def parse(text):
    lines = text.splitlines()
    assert lines[0].startswith("# jurnal gpu"), lines[0]
    out, t_prev = [], -1.0
    for l in lines[1:]:
        m = re.match(r"\[\s*(\d+)\.(\d{6})\] gpu: (.*)$", l)
        assert m, "rand fara ora: " + l
        t = int(m.group(1)) + int(m.group(2)) / 1e6
        assert t >= t_prev, "ora merge inapoi: " + l
        t_prev = t
        out.append(m.group(3))
    return out

def writes(ev):
    return [i for i, b in enumerate(ev) if b.startswith("cfg  w ") or b.startswith("bar0 w ")]

def step(ev, name):
    return [i for i, b in enumerate(ev) if b.startswith(f"pas {name}:")]

tour, ok, err, ro = (parse(t) for t in sys.argv[1:])
assert not writes(tour), "turul a scris"
assert any(b.startswith("bar0 r 0x1fa828") for b in tour), "turul nu a citit WPR2"
print(f"[after-gpu-tur] ok: tur: {len(tour)} randuri, 0 scrieri")

for k, ev, i6_ok in ((2, ok, True), (3, err, False)):
    cfg = [i for i, b in enumerate(ev) if b.startswith("cfg  w 0x004")]
    gsp = [i for i, b in enumerate(ev) if b.startswith("bar0 w ")]
    assert len(cfg) == 2 and gsp and cfg[0] < gsp[0] and cfg[1] > gsp[-1], f"{k}: Bus Master nu inconjoara I6"
    assert ev[cfg[0]].endswith("<- 0x00000007 => 0x00100007"), ev[cfg[0]]
    assert ev[cfg[1]].endswith("<- 0x00000003 => 0x00100003"), ev[cfg[1]]
    off = lambda i: int(ev[i].split()[2].rstrip(":"), 16)
    sec2 = [i for i in gsp if 0x840000 <= off(i) <= 0x843FFF]
    vwin = [i for i in gsp if off(i) == 0x1700]  # P6-56 step 2b: the VRAM window (golden, I9)
    assert all(0x110000 <= off(i) <= 0x113FFF or i in sec2 or i in vwin for i in gsp), f"{k}: scriere in afara GSP/SEC2/fereastra VRAM"
    order = ["pregatire-i6", "bus-master-pornit", "i6-fwsec-frts", "bus-master-oprit", "tur-dupa"]
    at = [step(ev, n) for n in order]
    assert all(len(a) == 1 for a in at), f"{k}: pasi lipsa/dubli: {at}"
    assert [a[0] for a in at] == sorted(a[0] for a in at), f"{k}: pasii nu sunt in ordine"
    assert ev[at[2][0]].endswith("ok" if i6_ok else "ESEC"), ev[at[2][0]]
    if i6_ok:
        i7 = step(ev, "i7-booter-gsp")
        assert i7 and ev[i7[0]].endswith("ok"), f"{k}: I7 nu a trecut"
        assert sec2 and sec2[0] > step(ev, "i6-wpr2-citit")[0] and sec2[-1] < i7[0], f"{k}: scrieri SEC2 in afara pasului I7"
        mb0 = [i for i in sec2 if off(i) == 0x840040]
        assert mb0 and not ev[mb0[0]].split("<- ")[1].startswith("0x00000000"), f"{k}: cutia SEC2 0 fara GspFwWprMeta"
        assert ev[step(ev, "i7-citit")[0]].endswith("ok"), f"{k}: i7-citit"
        # P6-56 step 2: I8 passed with exactly one write, the GSP doorbell 0x110C00, then the chain stops at golden.
        r7, i8 = step(ev, "i7-citit")[0], step(ev, "i8-gsp-init")
        assert i8 and ev[i8[0]].endswith("ok"), f"{k}: I8 nu a trecut"
        bell = [i for i in gsp if r7 < i < i8[0]]
        assert len(bell) == 1 and off(bell[0]) == 0x110C00, f"{k}: I8 trebuie sa scrie doar soneria GSP: {[ev[i] for i in bell]}"
        # P6-56 step 2b: golden + I9 pass with only GSP doorbells (one per RM message) and VRAM-window moves, the
        # window put back to its first value at the end; the chain stops at I10 (not in the OS) with no write after I9.
        gold, i9, i10 = step(ev, "i8e-golden"), step(ev, "i9-canal-lucru"), step(ev, "i10-add64")
        assert gold and ev[gold[0]].endswith("ok") and i9 and ev[i9[0]].endswith("ok"), f"{k}: golden/I9 nu au trecut"
        assert i10 and ev[i10[0]].endswith("ESEC"), f"{k}: I10 nu e oprit"
        after = [i for i in gsp if i8[0] < i < i9[0]]
        bells = [i for i in after if off(i) == 0x110C00]
        assert after and all(i in bells or i in vwin for i in after), f"{k}: golden/I9: alte scrieri decat soneria si fereastra VRAM"
        assert len(bells) >= 20, f"{k}: prea putine sonerii RM: {len(bells)}"
        assert vwin and ev[vwin[0]].split("=> ")[0] and ev[vwin[-1]].split("<- ")[1].startswith("0x00000000"), f"{k}: fereastra VRAM nu e pusa la loc"
        assert not [i for i in gsp if i > i9[0]], f"{k}: scriere dupa I9"
    else:
        assert not sec2, f"{k}: SEC2 scris desi I6 a esuat"
    tour_reads = [i for i, b in enumerate(ev) if b.startswith("bar0 r 0x1fa828")]
    assert tour_reads and tour_reads[-1] > cfg[1], f"{k}: turul nu e dupa Bus Master oprit"
    assert not writes(ev[cfg[1] + 1:]), f"{k}: scriere dupa Bus Master oprit"
    print(f"[after-gpu-tur] ok: lantul {k}: Bus Master pornit/oprit in jurul a {len(gsp)} scrieri GSP/SEC2/fereastra VRAM, I6 {'ok' if i6_ok else 'ESEC'}, turul dupa")

assert not writes(ro), "lantul doar-citire a scris"
assert step(ro, "pregatire-i6") and step(ro, "tur-dupa"), "lantul doar-citire: lipseste pregatirea sau turul"
assert ro[step(ro, "bus-master-pornit")[0]].endswith("ESEC"), "lantul doar-citire nu s-a oprit la Bus Master"
assert not step(ro, "i6-fwsec-frts"), "lantul doar-citire a pornit I6"
print(f"[after-gpu-tur] ok: lantul doar-citire: 0 scrieri, oprit inainte de Bus Master, turul a rulat")
PY
