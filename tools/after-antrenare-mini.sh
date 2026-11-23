#!/usr/bin/env bash
# After scenario `antrenare-mini` (P6-157): both checkpoints are whole CYBKCK01 files (SHA-256 right) of kind
# `antrenare-mini`, the second one continuing the first (pas 20 -> 30, AdamW t 20 -> 30), each with the weights and the
# AdamW state (3 x 28 tensors), and no `.nou` / `.vechi` left behind by the safe rewrite.
set -uo pipefail
IMG="${1:?disk image}"
fail() { echo "[after-antrenare-mini] EȘUAT: $*"; exit 1; }
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
for f in mini mini2; do
    MTOOLS_SKIP_CHECK=1 mcopy -i "$IMG@@1M" "::/$f.kck" "$TMP/$f.kck" 2>/dev/null || fail "lipseste /$f.kck"
done
left="$(MTOOLS_SKIP_CHECK=1 mdir -b -i "$IMG@@1M" ::/ 2>/dev/null | grep -E '\.kck\.(nou|vechi)$')"
[[ -z "$left" ]] || fail "au ramas resturi: $left"
python3 -I - "$TMP/mini.kck" "$TMP/mini2.kck" <<'PY' || exit 1
import hashlib, struct, sys
def read(path):
    b = open(path, "rb").read()
    assert b[:8] == b"CYBKCK01", "magia"
    body, s = b[:-32], b[-32:]
    assert hashlib.sha256(body).digest() == s, "SHA-256"
    at = 8
    def u32():
        nonlocal at; v = struct.unpack_from("<I", body, at)[0]; at += 4; return v
    def st():
        nonlocal at; n = u32(); v = body[at:at + n].decode(); at += n; return v
    meta = {}
    for _ in range(u32()):
        k = st(); meta[k] = st()
    names = []
    for _ in range(u32()):
        names.append(st()); e = struct.unpack_from("<Q", body, at)[0]; at += 8 + 4 * e
    assert at == len(body), "lungimea"
    return meta, names
(m1, n1), (m2, n2) = read(sys.argv[1]), read(sys.argv[2])
for m in (m1, m2):
    assert m.get("fel") == "antrenare-mini", m
assert (m1["pas"], m1["adam_t"], m2["pas"], m2["adam_t"]) == ("20", "20", "30", "30"), (m1, m2)
assert n1 == n2 and len(n1) == 84 and sum(x.startswith("adam.m.") for x in n1) == 28, len(n1)
print(f"[after-antrenare-mini] ok: 2 puncte de control CYBKCK01 (pas 20 -> 30), {len(n1)} tensori, SHA-256 corect, fara resturi")
PY
