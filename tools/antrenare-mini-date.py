#!/usr/bin/env python3
"""P6-157: training data for the `antrenare-mini` scenario, in Kronos's chunk format (as tools/date-bucati-mini.py), on a
vocabulary of 256 symbols so a step fits QEMU without KVM. The sequence is learnable: every symbol is followed by
(symbol + 239) mod 256, except one in 16 positions drawn from a fixed generator (noise).
  tools/antrenare-mini-date.py <out-dir>   -> <out>/antrenare/date + <out>/antrenare/tokenizer.json"""
import hashlib, json, random, struct, sys
from pathlib import Path

out = Path(sys.argv[1]) / "antrenare"
SIZES = [1000, 1000, 700]
tok = b'{"model": "proba P6-157", "vocabular": 256}\n'
out.mkdir(parents=True, exist_ok=True)
(out / "tokenizer.json").write_bytes(tok)
r = random.Random(157)
sym = [13]
for i in range(1, sum(SIZES)):
    sym.append(r.randrange(256) if r.randrange(16) == 0 else (sym[-1] + 239) % 256)
d = out / "date"
d.mkdir(parents=True, exist_ok=True)
tot, bucati, at = hashlib.sha256(), [], 0
for k, n in enumerate(SIZES):
    b = struct.pack("<%dh" % n, *sym[at:at + n])
    tot.update(b)
    bucati.append({"fisier": "bucata_%05d.bin" % k, "inceput": at, "simboluri": n, "sha256": hashlib.sha256(b).hexdigest()})
    (d / bucati[-1]["fisier"]).write_bytes(b)
    at += n
index = {"versiune": 1, "tip": "int16", "ordine_octeti": "little", "marime_bucata": SIZES[0], "simboluri_total": at,
         "sha256_tot": tot.hexdigest(), "partial": False,
         "tokenizer": {"fisier": "tokenizer.json", "sha256": hashlib.sha256(tok).hexdigest()},
         "sursa": "P6-157 proba", "bucati": bucati}
(d / "index.json").write_text(json.dumps(index, indent=2, ensure_ascii=False), encoding="utf-8")
print("simboluri", at, "sha256_tot", tot.hexdigest())
