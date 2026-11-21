#!/usr/bin/env python3
"""P6-122 (C-158 point 3): a KGR1 model with MORE layers, made from a small one by repeating its layers in order.

For the QEMU test of a deep model on the simulated board (#651 has 24 layers; the real file is not in Git): the
launch lists, the pushbuffer area and the records' stride grow with the number of layers, not with the sizes, so the
tiny control model with 24 layers is enough to check that the OS takes the limits from the list that runs.
Written by us from the KGR1 specification in the hub (`integrare/kronos/format_greutati.md`); nothing else changes
(same header keys, tokenizer, embeddings and final LayerNorm). Deterministic.

    tools/kgr-straturi.py <in.kgr> <out.kgr> <layers>
writes <out.kgr> and <out.kgr>.sha256 (the hex digest).
"""
import hashlib
import struct
import sys


def main():
    src, dst, layers = sys.argv[1], sys.argv[2], int(sys.argv[3])
    d = open(src, "rb").read()
    if d[:4] != b"KGR1":
        sys.exit("nu e KGR1")
    ver, nkv, nt = struct.unpack_from("<IQQ", d, 4)
    p = 24

    def txt():
        nonlocal p
        (n,) = struct.unpack_from("<Q", d, p)
        p += 8
        s = d[p : p + n]
        p += n
        return s

    kv = []
    for _ in range(nkv):
        k = txt()
        (f,) = struct.unpack_from("<I", d, p)
        p += 4
        if f in (1, 2):
            raw = d[p : p + 8]
            p += 8
        else:
            s = txt()
            raw = struct.pack("<Q", len(s)) + s
        kv.append((k, f, raw))
    tens = []
    for _ in range(nt):
        name = txt().decode()
        (nd,) = struct.unpack_from("<I", d, p)
        p += 4
        shape = struct.unpack_from("<%dQ" % nd, d, p)
        p += 8 * nd
        fel, off = struct.unpack_from("<IQ", d, p)
        p += 12
        tens.append([name, shape, fel, off])
    base = p + (-p) % 32
    # The bytes of every tensor (up to the next one's offset, or the end).
    offs = sorted(t[3] for t in tens) + [len(d) - base]
    size = {o: offs[i + 1] - o for i, o in enumerate(offs[:-1])}
    old = None
    for k, f, raw in kv:
        if k == b"n_straturi":
            (old,) = struct.unpack("<q", raw)
    if not old:
        sys.exit("lipseste n_straturi")
    by = {t[0]: t for t in tens}
    out = []
    for t in tens:
        if t[0].startswith("straturi."):
            continue
        out.append(t)
        if t[0] == "emb_pozitie.weight":
            for i in range(layers):
                for u in tens:
                    pre = "straturi.%d." % (i % old)
                    if u[0].startswith(pre):
                        out.append(["straturi.%d.%s" % (i, u[0][len(pre):]), u[1], u[2], u[3]])
    head = b"KGR1" + struct.pack("<IQQ", ver, len(kv), len(out))
    for k, f, raw in kv:
        if k == b"n_straturi":
            raw = struct.pack("<q", layers)
        head += struct.pack("<Q", len(k)) + k + struct.pack("<I", f) + raw
    blobs, off = [], 0
    meta = b""
    for name, shape, fel, o in out:
        data = d[base + o : base + o + size[o]]
        meta += struct.pack("<Q", len(name)) + name.encode() + struct.pack("<I", len(shape))
        meta += b"".join(struct.pack("<Q", x) for x in shape) + struct.pack("<IQ", fel, off)
        blobs.append(data)
        off += len(data)
    head += meta
    head += b"\0" * ((-len(head)) % 32)
    data = head + b"".join(blobs)
    open(dst, "wb").write(data)
    open(dst + ".sha256", "w").write(hashlib.sha256(data).hexdigest() + "\n")
    del by
    print("%s: %d straturi (din %d), %d tensori, %d octeti" % (dst, layers, old, len(out), len(data)))


if __name__ == "__main__":
    main()
