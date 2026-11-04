#!/usr/bin/env python3
"""Client for the remote text console of Cyber Universe OS (task P6-49, F2). Python standard library only.

Usage (Windows: `py tools\\remote-client.py ...`, Linux: `python3 tools/remote-client.py ...`):
  remote-client.py 127.0.0.1                 interactive, port 2323, key read from the file `remote.cheie`
  remote-client.py 192.168.1.50 -p 2323 -k D:\\remote.cheie
  remote-client.py 127.0.0.1 -c uptime -c mem   runs these commands, prints their output, then `exit`

The key is never given on the command line (it would stay in the command history): it comes from a file
(-k, default `remote.cheie` in the current folder) or, if that file is missing, it is asked for without echo.

Protocol (our own, line based): the OS sends `CYBER-REMOTE 1` and `cheie: `; we send the key and a newline;
the answer is `OK ...` (then the shell, `cyber> `), `GRESIT` or `REFUZAT ...`. After that every byte we send is a
key typed in that shell, and everything the shell prints comes back.

Exit codes: 0 = ok, 2 = wrong key, 3 = refused (pause after wrong keys), 4 = could not connect / no answer.
"""
import argparse
import getpass
import os
import socket
import sys
import threading
import time

GREETING = b"CYBER-REMOTE 1\n"
KEY_PROMPT = b"cheie: "
PROMPT = b"cyber> "


class Refused(Exception):
    def __init__(self, code, text):
        super().__init__(text)
        self.code = code


def read_key(path):
    if path and os.path.exists(path):
        with open(path, "rb") as f:
            return f.read().strip()
    return getpass.getpass("cheia pentru remote (nu se vede cand scrii): ").strip().encode()


def read_until(sock, marker, timeout, got=b"", start=0):
    """Reads until `marker` is in the data from offset `start` on (returns everything read, `got` included)
    or the connection closes."""
    sock.settimeout(timeout)
    end = time.time() + timeout
    while marker not in got[start:]:
        if time.time() > end:
            raise socket.timeout(f"nu a venit {marker!r}")
        try:
            chunk = sock.recv(4096)
        except socket.timeout:
            continue
        if not chunk:
            break
        got += chunk
    return got


def connect(host, port, key, wait=0.0, timeout=15.0):
    """Connects and logs in. Returns (socket, text received after OK). `wait` = keep retrying for this long
    (the OS may still be starting)."""
    end = time.time() + wait
    while True:
        try:
            sock = socket.create_connection((host, port), timeout=timeout)
            hello = read_until(sock, KEY_PROMPT, timeout)
            if hello.startswith(GREETING):
                break
            sock.close()
            if b"REFUZAT" in hello:
                raise Refused(3, hello.decode("utf-8", "replace").strip())
        except (OSError, socket.timeout):
            pass
        if time.time() >= end:
            raise Refused(4, f"nu ma pot conecta la {host}:{port}")
        time.sleep(0.5)
    if b"REFUZAT" in hello:
        sock.close()
        raise Refused(3, hello[len(GREETING):].decode("utf-8", "replace").strip())
    sock.sendall(key + b"\n")
    answer = read_until(sock, b"\n", timeout)
    if not answer.startswith(b"OK"):
        sock.close()
        raise Refused(2, answer.decode("utf-8", "replace").strip() or "conexiune inchisa")
    return sock, answer


def run_commands(sock, first, commands, timeout=30.0):
    """Sends each command after the prompt; returns the whole conversation as text. Ends with `exit` unless the
    session is already closed (e.g. after `remote opreste`)."""
    text = read_until(sock, PROMPT, timeout, first)
    closed = PROMPT not in text
    for cmd in list(commands) + ["exit"]:
        if closed:
            break
        sock.sendall(cmd.encode() + b"\n")
        before = len(text)
        text = read_until(sock, PROMPT, timeout, text, before)
        closed = PROMPT not in text[before:]
    sock.close()
    return text.decode("utf-8", "replace")


def interactive(sock, first):
    sys.stdout.write(first.decode("utf-8", "replace"))
    sys.stdout.flush()
    sock.settimeout(None)

    def reader():
        while True:
            try:
                data = sock.recv(4096)
            except OSError:
                data = b""
            if not data:
                sys.stdout.write("\n[legatura inchisa]\n")
                sys.stdout.flush()
                os._exit(0)
            # The shell echoes what we type; the backspace character erases on the screen.
            sys.stdout.write(data.decode("utf-8", "replace"))
            sys.stdout.flush()

    threading.Thread(target=reader, daemon=True).start()
    try:
        for line in sys.stdin:
            sock.sendall(line.rstrip("\r\n").encode() + b"\n")
    except KeyboardInterrupt:
        pass
    try:
        sock.sendall(b"exit\n")
        time.sleep(0.5)
    except OSError:
        pass


def main():
    ap = argparse.ArgumentParser(description="Consola text Cyber Universe OS prin retea (P6-49)")
    ap.add_argument("host", help="adresa OS-ului (masina virtuala: 127.0.0.1)")
    ap.add_argument("-p", "--port", type=int, default=2323)
    ap.add_argument("-k", "--cheie-fisier", default="remote.cheie", help="fisierul cu cheia (implicit remote.cheie)")
    ap.add_argument("-c", "--comanda", action="append", default=[], help="ruleaza comanda si iese (se poate repeta)")
    ap.add_argument("--asteapta", type=float, default=0, help="secunde in care reincearca conexiunea (OS-ul porneste)")
    a = ap.parse_args()
    key = read_key(a.cheie_fisier)
    try:
        sock, first = connect(a.host, a.port, key, a.asteapta)
    except Refused as e:
        print(f"remote: {e}", file=sys.stderr)
        return e.code
    first = first.split(b"\n", 1)
    print(first[0].decode("utf-8", "replace"))
    rest = first[1] if len(first) > 1 else b""
    if a.comanda:
        print(run_commands(sock, rest, a.comanda), end="")
        return 0
    interactive(sock, rest)
    return 0


if __name__ == "__main__":
    sys.exit(main())
