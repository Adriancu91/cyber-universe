#!/usr/bin/env python3
"""Measure how long every QEMU scenario took in a CI run (P6-156), for tools/shard.py.

  gh run view <run> --job <job> --log > teste-1.log     (once for every teste-K part)
  tools/durate-ci.py teste-*.log > tests/durate-ci.txt

A scenario's duration = from its "== scenariu: NAME" line to the next one (the last one: to the end of the
part's log), so the build of a new feature set counts for the first scenario that needs it.
"""
import datetime
import re
import sys

STAMP = re.compile(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z (.*)")


def parse(path):
    events = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = STAMP.search(line)
            if m:
                t = datetime.datetime.fromisoformat(m.group(1))
                frac = float(m.group(2) or 0)
                events.append((t.timestamp() + frac, m.group(3)))
    starts = [(t, msg.split(":", 1)[1].strip()) for t, msg in events if msg.startswith("== scenariu:")]
    out = {}
    for i, (t, name) in enumerate(starts):
        if i + 1 < len(starts):
            end = starts[i + 1][0]
        else:
            done = [et for et, msg in events if et > t and msg.startswith("== ") and "scenariu" not in msg]
            end = done[0] if done else events[-1][0]
        out[name] = end - t
    return out


def main(paths):
    if not paths:
        print(__doc__, file=sys.stderr)
        return 2
    all_d = {}
    for p in paths:
        all_d.update(parse(p))
    print("# Durata fiecarui scenariu QEMU in CI, in secunde (tools/durate-ci.py; folosit de tools/shard.py).")
    for name in sorted(all_d):
        print(f"{name} {all_d[name]:.0f}")
    print(f"# {len(all_d)} scenarii, {sum(all_d.values()) / 60:.1f} min in total", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
