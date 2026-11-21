#!/usr/bin/env python3
"""Writes one signed command file of the live channel (C-159, docs/LIVE.md).

    tools/live-semneaza.py <cheie> <sesiune> <nr> <fel> [text] > c-0001.json

<cheie> = the file with the shared key (the same bytes as /usb/hub/live.cheie, at least 32).
"""
import hmac, json, sys

FORM = "cyber-live-1"

def sign(key: bytes, session: str, nr: int, kind: str, text: str) -> str:
    return hmac.new(key, f"{FORM}\n{session}\n{nr}\n{kind}\n{text}".encode(), "sha256").hexdigest()

def main() -> int:
    if len(sys.argv) not in (5, 6):
        print(__doc__, file=sys.stderr)
        return 2
    key = open(sys.argv[1], "rb").read()
    if len(key) < 32:
        print("cheia are sub 32 de octeti", file=sys.stderr)
        return 1
    session, nr, kind = sys.argv[2], int(sys.argv[3]), sys.argv[4]
    text = sys.argv[5] if len(sys.argv) == 6 else ""
    print(json.dumps({"nr": nr, "sesiune": session, "fel": kind, "text": text, "semnatura": sign(key, session, nr, kind, text)}, ensure_ascii=False))
    return 0

if __name__ == "__main__":
    sys.exit(main())
