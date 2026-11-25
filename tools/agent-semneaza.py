#!/usr/bin/env python3
"""Writes one signed board line for the agent's command channel (C-166 step 6, part 2; docs/agent/COMENZI.md).

    tools/agent-semneaza.py <cheie> <nr> test-placa --bariera
    -> - 8 · nou · test-placa --bariera · semn: <64 hex>

<cheie> = the file with the live channel's key (the same bytes as /usb/hub/live.cheie, at least 32).
The line goes into live/os-agent-comenzi.md of the hub (branch main). The number must be larger than every signed
number used before (the OS keeps them in /usb/hub/agent-placa.txt).
"""
import hmac, sys

FORM = "cyber-agent-1"


def sign(key: bytes, nr: int, command: str) -> str:
    return hmac.new(key, f"{FORM}\n{nr}\n{command}".encode(), "sha256").hexdigest()


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    key = open(sys.argv[1], "rb").read()
    if len(key) < 32:
        print("cheia are sub 32 de octeti", file=sys.stderr)
        return 1
    nr = int(sys.argv[2])
    command = " ".join(sys.argv[3:]).strip()
    if nr <= 0 or not command.startswith("test-placa"):
        print("nr > 0 si doar comenzi `test-placa ...`", file=sys.stderr)
        return 1
    print(f"- {nr} · nou · {command} · semn: {sign(key, nr, command)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
