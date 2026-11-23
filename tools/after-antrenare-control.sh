#!/usr/bin/env bash
# After scenario `antrenare-control` (P6-151): the checkpoint on the disk is a whole CYBKCK01 file (SHA-256 right, the
# golden step's 28 tensors), with no `.nou` / `.vechi` left behind by the safe rewrite.
set -uo pipefail
IMG="${1:?disk image}"
fail() { echo "[after-antrenare-control] EȘUAT: $*"; exit 1; }
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
MTOOLS_SKIP_CHECK=1 mcopy -i "$IMG@@1M" ::/control.kck "$TMP/c.kck" 2>/dev/null || fail "lipseste /control.kck"
left="$(MTOOLS_SKIP_CHECK=1 mdir -b -i "$IMG@@1M" ::/ 2>/dev/null | grep -E 'control\.kck\.(nou|vechi)$')"
[[ -z "$left" ]] || fail "au ramas resturi: $left"
python3 -I - "$TMP/c.kck" <<'PY' || exit 1
import hashlib, struct, sys
b = open(sys.argv[1], "rb").read()
assert b[:8] == b"CYBKCK01", "magia"
body, s = b[:-32], b[-32:]
assert hashlib.sha256(body).digest() == s, "SHA-256"
at = 8
def u32():
    global at; v = struct.unpack_from("<I", body, at)[0]; at += 4; return v
def st():
    global at; n = u32(); v = body[at:at + n].decode(); at += n; return v
meta = {}
for _ in range(u32()):
    k = st(); meta[k] = st()
n = u32(); total = 0
for _ in range(n):
    st(); e = struct.unpack_from("<Q", body, at)[0]; at += 8 + 4 * e; total += e
assert at == len(body), "lungimea"
assert meta.get("reteta") == "sgd" and meta.get("pasi") == "10", meta
print(f"[after-antrenare-control] ok: CYBKCK01, {n} tensori, {total} valori, SHA-256 corect, fara resturi")
PY
