#!/usr/bin/env python3
"""P6-150: tiny training data in Kronos's chunk format (index.json + bucata_NNNNN.bin, int16 little-endian, no header;
the same fields as cyberspace `unelte/date_bucati.py`), for the `antrenare-date` scenario.
  tools/date-bucati-mini.py <out-dir>   -> <out>/antrenare/date (good), <out>/antrenare/stricat (one byte changed),
                                           <out>/antrenare/tokenizer.json; prints the expected fingerprint."""
import hashlib, json, struct, sys
from pathlib import Path

out = Path(sys.argv[1]) / "antrenare"
SIZES = [1000, 1000, 700]
tok = b'{"model": "proba P6-150", "vocabular": 16384}\n'
(out).mkdir(parents=True, exist_ok=True)
(out / "tokenizer.json").write_bytes(tok)
sym = [((i * 7919 + 13) % 16384) for i in range(sum(SIZES))]
for name, broken in (("date", False), ("stricat", True)):
    d = out / name
    d.mkdir(parents=True, exist_ok=True)
    tot = hashlib.sha256()
    bucati, at = [], 0
    for k, n in enumerate(SIZES):
        b = struct.pack("<%dh" % n, *sym[at:at + n])
        tot.update(b)
        bucati.append({"fisier": "bucata_%05d.bin" % k, "inceput": at, "simboluri": n, "sha256": hashlib.sha256(b).hexdigest()})
        if broken and k == 1:
            b = b[:10] + bytes([b[10] ^ 1]) + b[11:]
        (d / bucati[-1]["fisier"]).write_bytes(b)
        at += n
    index = {"versiune": 1, "tip": "int16", "ordine_octeti": "little", "marime_bucata": SIZES[0], "simboluri_total": at,
             "sha256_tot": tot.hexdigest(), "partial": False,
             "tokenizer": {"fisier": "tokenizer.json", "sha256": hashlib.sha256(tok).hexdigest()},
             "sursa": "P6-150 proba", "bucati": bucati}
    (d / "index.json").write_text(json.dumps(index, indent=2, ensure_ascii=False), encoding="utf-8")
amprenta = hashlib.sha256(f"{tot.hexdigest()}:{hashlib.sha256(tok).hexdigest()}".encode("ascii")).hexdigest()[:16]
print("simboluri", sum(SIZES), "amprenta", amprenta, "sha256_tot", tot.hexdigest())
print("felie 995..1005:", sym[995:1005])
