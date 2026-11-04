#!/usr/bin/env python3
"""Client for the remote text service of Cyber Universe (P6-49, C-68 F2).

Interactive:   python3 tools/remote-client.py [host] [port]          (asks for the key, then command lines)
Test (tools/before-remote.sh):  python3 tools/remote-client.py --test 127.0.0.1 12323 <key> <result file>
"""
import socket
import sys
import time


def read_until(s, marks, timeout=30):
    s.settimeout(timeout)
    data = b""
    end = time.time() + timeout
    while time.time() < end:
        if any(m in data for m in marks):
            return data.decode("utf-8", "replace")
        try:
            chunk = s.recv(4096)
        except socket.timeout:
            break
        if not chunk:
            break
        data += chunk
    return data.decode("utf-8", "replace")


def connect(host, port, wait=120):
    """The guest listens only after boot: QEMU accepts on the host side and then closes; retry until a banner comes."""
    end = time.time() + wait
    while time.time() < end:
        try:
            s = socket.create_connection((host, port), timeout=5)
            banner = read_until(s, [b"Cheia: "], timeout=5)
            if "Cheia: " in banner:
                return s, banner
            s.close()
        except OSError:
            pass
        time.sleep(1)
    raise SystemExit("nu ma pot conecta la %s:%d" % (host, port))


def test(host, port, key, out):
    log = []
    s, banner = connect(host, port)
    s.sendall(b"cheie-gresita-0000000000\r\n")
    r = read_until(s, [b"gresita"])
    log.append("cheie gresita -> " + r.strip())
    assert "cheie gresita" in r, r
    s.close()
    s, banner = connect(host, port)
    s.sendall(key.encode() + b"\r\n")
    r = read_until(s, [b"cyber-remote> "])
    assert "autentificat" in r, r
    log.append("cheie buna -> autentificat")
    for cmd, expect in [("uptime", "pornit de"), ("reboot", "REFUZAT"), ("gpu i6 --scrie", "REFUZAT"),
                        ("run editor", "REFUZAT"), ("help", "iesire")]:
        s.sendall(cmd.encode() + b"\n")
        r = read_until(s, [b"cyber-remote> "])
        assert expect in r, (cmd, r)
        log.append("%s -> %s" % (cmd, r.replace("cyber-remote> ", "").strip().splitlines()[0]))
    s.sendall(b"iesire\r\n")
    r = read_until(s, [b"la revedere"])
    assert "la revedere" in r, r
    log.append("CLIENT OK")
    with open(out, "w") as f:
        f.write("\n".join(log) + "\n")
    print("\n".join(log))
    s.close()


def interactive(host, port):
    s, banner = connect(host, port, wait=10)
    sys.stdout.write(banner)
    s.sendall(input().encode() + b"\r\n")
    while True:
        r = read_until(s, [b"cyber-remote> ", b"gresita", b"la revedere"], timeout=600)
        sys.stdout.write(r)
        sys.stdout.flush()
        if not r or "gresita" in r or "la revedere" in r:
            break
        try:
            line = input()
        except EOFError:
            line = "iesire"
        s.sendall(line.encode() + b"\r\n")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--test":
        test(sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5])
    else:
        interactive(sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1", int(sys.argv[2]) if len(sys.argv) > 2 else 2323)
