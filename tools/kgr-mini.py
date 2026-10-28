#!/usr/bin/env python3
"""P6-27 (C-25): a tiny KGR1 model with random weights + its reference outputs, for the OS engine's test.

Written by us from the KGR1 specification in the hub (`integrare/kronos/format_greutati.md`), without the Kronos code.
The reference forward pass runs in float64 on the weights exactly as stored in the file (F32 values, or Q8_0
scale * number), so it checks the OS engine (float32, any thread count, scalar or vector) against the specification.

    tools/kgr-mini.py <output dir>   (the test files: tools/kgr-mini.py tests/kronos)
writes mini-f32.kgr, mini-q8.kgr (each with a .sha256 file: the hex digest) and their references mini-f32.ref, mini-q8.ref (our own small format "KREF"):
    "KREF", u32 version (1), u32 vocabulary, u32 sequences; per sequence: u32 length, u32 generated,
    u32 ids[length], f32 logits[length * vocabulary], u32 greedy continuation[generated].
Deterministic (fixed seed): running it again gives the same bytes.
"""
import hashlib
import json
import math
import os
import struct
import sys

import numpy as np

CFG = dict(n_straturi=2, n_capete=4, n_emb=64, context=32, vocabular=96)
EPS = 1e-5
PROMPTS = [[1, 2, 3, 4, 5], [17, 42, 7, 7, 90, 0, 63, 12, 5, 33, 80], [95]]
GENERATE = 8
MIN_GAP = 1e-3  # greedy steps must not be near-ties, or float32 rounding could pick another token


def text(s):
    b = s.encode("utf-8")
    return struct.pack("<Q", len(b)) + b


def tensors(rng):
    v, e, c, layers = CFG["vocabular"], CFG["n_emb"], CFG["context"], CFG["n_straturi"]
    t = {}
    t["emb_token.weight"] = rng.normal(0, 0.5, (v, e))
    t["emb_pozitie.weight"] = rng.normal(0, 0.2, (c, e))
    for i in range(layers):
        p = f"straturi.{i}."
        t[p + "ln1.weight"] = 1 + rng.normal(0, 0.1, e)
        t[p + "ln1.bias"] = rng.normal(0, 0.1, e)
        t[p + "atentie.in_proj_weight"] = rng.normal(0, 0.3, (3 * e, e))
        t[p + "atentie.in_proj_bias"] = rng.normal(0, 0.1, 3 * e)
        t[p + "atentie.out_proj.weight"] = rng.normal(0, 0.15, (e, e))
        t[p + "atentie.out_proj.bias"] = rng.normal(0, 0.1, e)
        t[p + "ln2.weight"] = 1 + rng.normal(0, 0.1, e)
        t[p + "ln2.bias"] = rng.normal(0, 0.1, e)
        t[p + "mlp.0.weight"] = rng.normal(0, 0.2, (4 * e, e))
        t[p + "mlp.0.bias"] = rng.normal(0, 0.1, 4 * e)
        t[p + "mlp.2.weight"] = rng.normal(0, 0.1, (e, 4 * e))
        t[p + "mlp.2.bias"] = rng.normal(0, 0.1, e)
    t["ln.weight"] = 1 + rng.normal(0, 0.1, e)
    t["ln.bias"] = rng.normal(0, 0.1, e)
    return {k: x.astype(np.float32) for k, x in t.items()}


def quantize(x):
    """Q8_0 bytes of a 2-D float32 array, and the values they decode to."""
    rows, cols = x.shape
    assert cols % 32 == 0
    out, dec = bytearray(), np.zeros_like(x, dtype=np.float64)
    for r in range(rows):
        for b in range(cols // 32):
            blk = x[r, b * 32:(b + 1) * 32]
            scale = np.float32(np.abs(blk).max() / 127)
            q = np.zeros(32, np.int8) if scale == 0 else np.clip(np.round(blk / scale), -127, 127).astype(np.int8)
            out += struct.pack("<f", scale) + q.tobytes()
            dec[r, b * 32:(b + 1) * 32] = np.float64(scale) * q.astype(np.float64)
    return bytes(out), dec


def write_kgr(path, ts, kind):
    """Writes the file; returns the float64 weights exactly as the file holds them."""
    kv = [("arhitectura", 3, "kronos_gpt"), ("fel", 3, kind), ("sursa", 3, "cyber-universe mini (aleator, P6-27)")]
    kv += [(k, 1, val) for k, val in CFG.items()]
    kv += [("ln_eps", 2, EPS), ("cap_legat", 1, 1)]
    kv += [("tokenizer.bpe", 3, json.dumps({"versiune": 0, "imbinari": [], "speciale": {}, "reguli": "test"}))]
    blobs, meta, held = [], [], {}
    off = 0
    for name, x in ts.items():
        if kind == "q8_0" and x.ndim == 2:
            data, held[name] = quantize(x)
            fel = 1
        else:
            data, held[name] = x.astype("<f4").tobytes(), x.astype(np.float64)
            fel = 0
        meta.append((name, x.shape, fel, off))
        pad = (-len(data)) % 32
        blobs.append(data + b"\0" * pad)
        off += len(data) + pad
    head = b"KGR1" + struct.pack("<IQQ", 1, len(kv), len(meta))
    for k, f, val in kv:
        head += text(k) + struct.pack("<I", f)
        head += {1: lambda v: struct.pack("<q", v), 2: lambda v: struct.pack("<d", v), 3: text}[f](val)
    for name, shape, fel, o in meta:
        head += text(name) + struct.pack("<I", len(shape)) + b"".join(struct.pack("<Q", d) for d in shape)
        head += struct.pack("<IQ", fel, o)
    head += b"\0" * ((-len(head)) % 32)
    data = head + b"".join(blobs)
    with open(path, "wb") as f:
        f.write(data)
    with open(path + ".sha256", "w") as f:  # what Adrian puts next to a real model: the sum from MODEL.md
        f.write(hashlib.sha256(data).hexdigest() + "\n")
    return held


def layernorm(x, w, b):
    m = x.mean(-1, keepdims=True)
    v = ((x - m) ** 2).mean(-1, keepdims=True)
    return (x - m) / np.sqrt(v + EPS) * w + b


def gelu(x):
    return 0.5 * x * (1 + np.vectorize(math.erf)(x / math.sqrt(2)))


def forward(w, ids):
    e, h = CFG["n_emb"], CFG["n_capete"]
    d = e // h
    t = len(ids)
    x = w["emb_token.weight"][ids] + w["emb_pozitie.weight"][:t]
    mask = np.triu(np.ones((t, t), bool), 1)
    for i in range(CFG["n_straturi"]):
        p = f"straturi.{i}."
        a = layernorm(x, w[p + "ln1.weight"], w[p + "ln1.bias"])
        qkv = a @ w[p + "atentie.in_proj_weight"].T + w[p + "atentie.in_proj_bias"]
        q, k, v = qkv[:, :e], qkv[:, e:2 * e], qkv[:, 2 * e:]
        heads = []
        for j in range(h):
            s = q[:, j * d:(j + 1) * d] @ k[:, j * d:(j + 1) * d].T / math.sqrt(d)
            s[mask] = -np.inf
            s = np.exp(s - s.max(-1, keepdims=True))
            s /= s.sum(-1, keepdims=True)
            heads.append(s @ v[:, j * d:(j + 1) * d])
        x = x + np.concatenate(heads, -1) @ w[p + "atentie.out_proj.weight"].T + w[p + "atentie.out_proj.bias"]
        a = layernorm(x, w[p + "ln2.weight"], w[p + "ln2.bias"])
        x = x + gelu(a @ w[p + "mlp.0.weight"].T + w[p + "mlp.0.bias"]) @ w[p + "mlp.2.weight"].T + w[p + "mlp.2.bias"]
    return layernorm(x, w["ln.weight"], w["ln.bias"]) @ w["emb_token.weight"].T


def gap(row):
    s = np.sort(row)
    return s[-1] - s[-2]


def write_ref(path, w):
    out = b"KREF" + struct.pack("<III", 1, CFG["vocabular"], len(PROMPTS))
    for ids in PROMPTS:
        logits = forward(w, ids)
        seq, gen = list(ids), []
        for _ in range(GENERATE):
            last = forward(w, seq)[-1]
            if gap(last) < MIN_GAP:
                raise SystemExit(f"{path}: greedy near-tie ({gap(last)}), change the seed")
            nxt = int(np.argmax(last))
            gen.append(nxt)
            seq.append(nxt)
        out += struct.pack("<II", len(ids), len(gen)) + struct.pack(f"<{len(ids)}I", *ids)
        out += logits.astype("<f4").tobytes() + struct.pack(f"<{len(gen)}I", *gen)
        print(f"{os.path.basename(path)}: {ids} -> {gen}, |logit| max {np.abs(logits).max():.3f}")
    with open(path, "wb") as f:
        f.write(out)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    ts = tensors(np.random.default_rng(27))
    for kind, tag in (("f32", "f32"), ("q8_0", "q8")):
        held = write_kgr(os.path.join(out, f"mini-{tag}.kgr"), ts, kind)
        write_ref(os.path.join(out, f"mini-{tag}.ref"), held)


if __name__ == "__main__":
    main()
