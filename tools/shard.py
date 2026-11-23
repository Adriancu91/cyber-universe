#!/usr/bin/env python3
"""Split the QEMU scenarios into N CI parts of about the same duration (P6-156).

  tools/shard.py K/N [scenarios.txt] [durations.txt]   -> the scenario names of part K, one per line
  tools/shard.py --table N [scenarios.txt] [durations.txt] -> the estimated minutes of every part

Durations come from tests/durate-ci.txt (seconds per scenario, measured from a CI run with tools/durate-ci.py).
A scenario without a measured duration counts as the median of the known ones. Assignment: longest first, each
to the part with the smallest total so far (ties: lowest part number) - deterministic, so every CI part computes
the same split independently.
"""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def scenario_names(path):
    names = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            name = line.split("|", 1)[0].strip()
            if name and not name.startswith("#"):
                names.append(name)
    return names


def durations(path):
    known = {}
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.split("#", 1)[0].strip()
                if not line:
                    continue
                name, secs = line.split()
                known[name] = float(secs)
    return known


def split(names, known, n):
    vals = sorted(known[x] for x in names if x in known)
    default = vals[len(vals) // 2] if vals else 60.0
    order = sorted(range(len(names)), key=lambda i: (-known.get(names[i], default), i))
    parts = [[] for _ in range(n)]
    totals = [0.0] * n
    for i in order:
        k = min(range(n), key=lambda p: (totals[p], p))
        parts[k].append(i)
        totals[k] += known.get(names[i], default)
    # keep the file order inside each part (same order as before, only fewer scenarios)
    return [[names[i] for i in sorted(p)] for p in parts], totals


def main(argv):
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    table = argv[1] == "--table"
    rest = argv[2:]
    spec = rest.pop(0) if table and rest else (None if table else argv[1])
    if not table:
        rest = argv[2:]
    scen = rest[0] if len(rest) > 0 else os.path.join(ROOT, "tests", "scenarios.txt")
    dur = rest[1] if len(rest) > 1 else os.path.join(ROOT, "tests", "durate-ci.txt")
    try:
        if table:
            k, n = 0, int(spec)
        else:
            k, n = (int(x) for x in spec.split("/"))
        if n < 1 or (not table and not 1 <= k <= n):
            raise ValueError
    except (ValueError, AttributeError, TypeError):
        print("shard.py: cere K/N cu 1 <= K <= N sau --table N", file=sys.stderr)
        return 2
    names = scenario_names(scen)
    parts, totals = split(names, durations(dur), n)
    if table:
        for p in range(n):
            print(f"partea {p + 1}/{n}: {len(parts[p])} scenarii, ~{totals[p] / 60:.1f} min")
    else:
        print("\n".join(parts[k - 1]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
