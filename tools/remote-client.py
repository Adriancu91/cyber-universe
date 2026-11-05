#!/usr/bin/env python3
"""Client for the remote text console of Cyber Universe OS (tasks P6-49, P6-54; F2). Python standard library only.

Usage (Windows: `py tools\\remote-client.py ...`, Linux: `python3 tools/remote-client.py ...`):
  remote-client.py 127.0.0.1                 interactive, port 2323, key read from the file `remote.cheie`
  remote-client.py 192.168.1.50 -p 2323 -k D:\\remote.cheie
  remote-client.py 127.0.0.1 -c uptime -c mem   runs these commands, prints their output, then `exit`

The key is never given on the command line (it would stay in the command history): it comes from a file
(-k, default `remote.cheie` in the current folder) or, if that file is missing, it is asked for without echo.

  remote-client.py --test                    checks the built-in cryptography on the official RFC vectors

Protocol (our own, encrypted; described in kernel/src/net/remote_crypto.rs): the OS sends `CYBER-REMOTE 2` and
48 bytes (a fresh X25519 public value + nonce); we send ours and a first encrypted record `AUTH`. Both sides derive
the record keys from X25519 and the key (the key itself never travels). The answer is an encrypted `OK ...` (then the
shell, `cyber> `), or in clear `GRESIT` / `REFUZAT ...`. After that every byte we send is a key typed in that shell,
and everything the shell prints comes back, all in ChaCha20-Poly1305 records. The cryptography below is written
from the public RFCs (7748, 8439, 5869) in plain Python, so nothing has to be installed on Windows.

Exit codes: 0 = ok, 2 = wrong key, 3 = refused (pause after wrong keys), 4 = could not connect / no answer.
"""
import argparse
import getpass
import hashlib
import hmac
import os
import socket
import struct
import sys
import threading
import time

GREETING = b"CYBER-REMOTE 2\n"
HELLO_LEN = 48
MAX_PLAIN = 4096
PROMPT = b"cyber> "

# ---- cryptography (pure Python, from the RFCs) ----
P25519 = 2**255 - 19


def x25519(k, u):
    """RFC 7748 section 5: the Montgomery ladder on Curve25519."""
    k = bytearray(k)
    k[0] &= 248
    k[31] &= 127
    k[31] |= 64
    k = int.from_bytes(k, "little")
    u = int.from_bytes(u, "little") & ((1 << 255) - 1)
    x1, x2, z2, x3, z3, swap = u, 1, 0, u, 1, 0
    for t in reversed(range(255)):
        kt = (k >> t) & 1
        swap ^= kt
        if swap:
            x2, x3, z2, z3 = x3, x2, z3, z2
        swap = kt
        a, b = (x2 + z2) % P25519, (x2 - z2) % P25519
        aa, bb = a * a % P25519, b * b % P25519
        e = (aa - bb) % P25519
        c, d = (x3 + z3) % P25519, (x3 - z3) % P25519
        da, cb = d * a % P25519, c * b % P25519
        x3 = (da + cb) ** 2 % P25519
        z3 = x1 * (da - cb) ** 2 % P25519
        x2 = aa * bb % P25519
        z2 = e * (aa + 121665 * e) % P25519
    if swap:
        x2, z2 = x3, z3
    return (x2 * pow(z2, P25519 - 2, P25519) % P25519).to_bytes(32, "little")


def x25519_base(k):
    return x25519(k, (9).to_bytes(32, "little"))


def _rotl(v, c):
    return ((v << c) & 0xFFFFFFFF) | (v >> (32 - c))


def chacha20_block(key, counter, nonce):
    """RFC 8439 section 2.3."""
    s = [0x61707865, 0x3320646E, 0x79622D32, 0x6B206574]
    s += list(struct.unpack("<8I", key)) + [counter] + list(struct.unpack("<3I", nonce))
    w = s[:]
    for _ in range(10):
        for a, b, c, d in ((0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15),
                           (0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)):
            w[a] = (w[a] + w[b]) & 0xFFFFFFFF; w[d] = _rotl(w[d] ^ w[a], 16)
            w[c] = (w[c] + w[d]) & 0xFFFFFFFF; w[b] = _rotl(w[b] ^ w[c], 12)
            w[a] = (w[a] + w[b]) & 0xFFFFFFFF; w[d] = _rotl(w[d] ^ w[a], 8)
            w[c] = (w[c] + w[d]) & 0xFFFFFFFF; w[b] = _rotl(w[b] ^ w[c], 7)
    return struct.pack("<16I", *((w[i] + s[i]) & 0xFFFFFFFF for i in range(16)))


def chacha20(key, counter, nonce, data):
    out = bytearray()
    for i in range(0, len(data), 64):
        block = chacha20_block(key, counter + i // 64, nonce)
        out += bytes(x ^ y for x, y in zip(data[i:i + 64], block))
    return bytes(out)


def poly1305(key, msg):
    """RFC 8439 section 2.5."""
    r = int.from_bytes(key[:16], "little") & 0x0FFFFFFC0FFFFFFC0FFFFFFC0FFFFFFF
    s = int.from_bytes(key[16:], "little")
    p, acc = (1 << 130) - 5, 0
    for i in range(0, len(msg), 16):
        n = int.from_bytes(msg[i:i + 16] + b"\x01", "little")
        acc = (acc + n) * r % p
    return ((acc + s) & ((1 << 128) - 1)).to_bytes(16, "little")


def _pad16(b):
    return b"\x00" * (-len(b) % 16)


def _aead_tag(key, nonce, aad, ct):
    otk = chacha20_block(key, 0, nonce)[:32]
    mac = aad + _pad16(aad) + ct + _pad16(ct) + struct.pack("<QQ", len(aad), len(ct))
    return poly1305(otk, mac)


def aead_seal(key, nonce, aad, plain):
    """RFC 8439 section 2.8: ciphertext + 16-byte tag."""
    ct = chacha20(key, 1, nonce, plain)
    return ct + _aead_tag(key, nonce, aad, ct)


def aead_open(key, nonce, aad, data):
    ct, tag = data[:-16], data[-16:]
    if not hmac.compare_digest(_aead_tag(key, nonce, aad, ct), tag):
        return None
    return chacha20(key, 1, nonce, ct)


def hkdf_extract(salt, ikm):
    return hmac.new(salt, ikm, hashlib.sha256).digest()


def hkdf_expand(prk, info, n):
    out, t, i = b"", b"", 1
    while len(out) < n:
        t = hmac.new(prk, t + info + bytes([i]), hashlib.sha256).digest()
        out += t
        i += 1
    return out[:n]


class Channel:
    """The record layer over a socket, with the socket's recv/sendall/settimeout/close, so the rest of the
    client works the same as before."""

    def __init__(self, sock, send_key, recv_key, inbox=b""):
        self.sock, self.send_key, self.recv_key = sock, send_key, recv_key
        self.send_seq = self.recv_seq = 0
        self.inbox = bytearray(inbox)
        self.plain = bytearray()

    @staticmethod
    def nonce(seq):
        return b"\x00" * 4 + struct.pack(">Q", seq)

    def seal(self, data):
        out = b""
        for i in range(0, max(len(data), 1), MAX_PLAIN):
            piece = data[i:i + MAX_PLAIN]
            head = struct.pack(">H", len(piece) + 16)
            out += head + aead_seal(self.send_key, self.nonce(self.send_seq), head, piece)
            self.send_seq += 1
        return out

    def open_waiting(self):
        """Opens every complete record in the inbox. Raises ConnectionError on a record that does not open."""
        while len(self.inbox) >= 2:
            n = struct.unpack(">H", self.inbox[:2])[0]
            if n < 16 or n > MAX_PLAIN + 16:
                raise ConnectionError("record cu lungime gresita")
            if len(self.inbox) < 2 + n:
                return
            head, body = bytes(self.inbox[:2]), bytes(self.inbox[2:2 + n])
            plain = aead_open(self.recv_key, self.nonce(self.recv_seq), head, body)
            if plain is None:
                raise ConnectionError("record stricat (eticheta gresita)")
            del self.inbox[:2 + n]
            self.recv_seq += 1
            self.plain += plain

    def recv(self, n):
        while not self.plain:
            self.open_waiting()
            if self.plain:
                break
            chunk = self.sock.recv(4096)
            if not chunk:
                return b""
            self.inbox += chunk
        out = bytes(self.plain[:n])
        del self.plain[:n]
        return out

    def sendall(self, data):
        self.sock.sendall(self.seal(data))

    def settimeout(self, t):
        self.sock.settimeout(t)

    def close(self):
        self.sock.close()


def derive(key, secret, server_hello, client_hello):
    """The client's record keys (send = client to server, recv = server to client); None for a bad public value."""
    shared = x25519(secret, server_hello[:32])
    if shared == b"\x00" * 32:
        return None
    prk = hkdf_extract(hashlib.sha256(b"cyber-remote cheie\x00" + key).digest(), shared)
    th = hashlib.sha256(b"CYBER-REMOTE 2" + server_hello + client_hello).digest()
    return hkdf_expand(prk, b"c2s" + th, 32), hkdf_expand(prk, b"s2c" + th, 32)


def client_hello(random48):
    return random48[:32], x25519_base(random48[:32]) + random48[32:48]


def self_test():
    """The official vectors (RFC 7748 5.2 and 6.1, RFC 8439 2.4.2, 2.5.2, 2.8.2, RFC 5869 A.1), then the fixed
    vector the OS prints in its own self-test (`test: remote: vector AUTH ...`)."""
    h = bytes.fromhex
    assert x25519(h("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
                  h("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")) == \
        h("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
    a_pub = x25519_base(h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"))
    assert a_pub == h("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    key = bytes(range(32))
    nonce = h("000000000000004a00000000")
    text = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it."
    assert chacha20(key, 1, nonce, text)[:16] == h("6e2e359a2568f98041ba0728dd0d6981")
    assert poly1305(h("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b"),
                    b"Cryptographic Forum Research Group") == h("a8061dc1305136c6c22b8baf0c0127a9")
    akey = h("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
    anonce = h("070000004041424344454647")
    aad = h("50515253c0c1c2c3c4c5c6c7")
    sealed = aead_seal(akey, anonce, aad, text)
    assert sealed[-16:] == h("1ae10b594f09e26a7e902ecbd0600691"), sealed[-16:].hex()
    assert aead_open(akey, anonce, aad, sealed) == text
    assert aead_open(akey, anonce, aad, sealed[:-1] + bytes([sealed[-1] ^ 1])) is None
    prk = hkdf_extract(h("000102030405060708090a0b0c"), b"\x0b" * 22)
    assert hkdf_expand(prk, h("f0f1f2f3f4f5f6f7f8f9"), 42) == h(
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
    # The fixed vector shared with the OS's self-test (kernel/src/net/remote_crypto.rs).
    rs = bytes(i + 1 for i in range(48))
    rc = bytes(0x80 + i for i in range(48))
    server_hello = x25519_base(rs[:32]) + rs[32:48]
    secret, chello = client_hello(rc)
    send_key, recv_key = derive(b"cheie-lunga-de-test-0123456789", secret, server_hello, chello)
    record = Channel(None, send_key, recv_key).seal(b"AUTH")
    return record.hex()


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
    """Connects, agrees on the record keys and logs in. Returns (channel, text received after OK). `wait` = keep
    retrying for this long (the OS may still be starting)."""
    end = time.time() + wait
    while True:
        try:
            sock = socket.create_connection((host, port), timeout=timeout)
            hello = read_until(sock, b"\n", timeout)
            if hello.startswith(GREETING):
                if b"REFUZAT" in hello:
                    sock.close()
                    raise Refused(3, hello[len(GREETING):].decode("utf-8", "replace").strip())
                hello = read_until_len(sock, len(GREETING) + HELLO_LEN, timeout, hello)
                break
            sock.close()
            if hello.startswith(b"CYBER-REMOTE 1"):
                raise Refused(4, "OS-ul are remote-ul vechi, necriptat: pune pe stick un Release nou")
        except (OSError, socket.timeout):
            pass
        if time.time() >= end:
            raise Refused(4, f"nu ma pot conecta la {host}:{port}")
        time.sleep(0.5)
    server_hello = hello[len(GREETING):len(GREETING) + HELLO_LEN]
    secret, chello = client_hello(os.urandom(48))
    keys = derive(key, secret, server_hello, chello)
    if keys is None:
        sock.close()
        raise Refused(4, "raspuns ciudat de la server (valoare X25519 gresita)")
    ch = Channel(sock, keys[0], keys[1], hello[len(GREETING) + HELLO_LEN:])
    sock.sendall(chello + ch.seal(b"AUTH"))
    # The answer: `GRESIT` in clear (wrong key), or the first record (its first byte is a length, never `G`).
    sock.settimeout(timeout)
    try:
        while len(ch.inbox) < 2:
            chunk = sock.recv(4096)
            if not chunk:
                break
            ch.inbox += chunk
        if ch.inbox.startswith(b"GRESIT") or len(ch.inbox) < 2:
            sock.close()
            raise Refused(2, "GRESIT" if ch.inbox.startswith(b"GRESIT") else "conexiune inchisa")
        answer = read_until(ch, b"\n", timeout)
    except ConnectionError as e:
        sock.close()
        raise Refused(2, f"serverul nu are aceeasi cheie ({e})")
    if not answer.startswith(b"OK"):
        sock.close()
        raise Refused(2, answer.decode("utf-8", "replace").strip() or "conexiune inchisa")
    return ch, answer


def read_until_len(sock, n, timeout, got):
    sock.settimeout(timeout)
    end = time.time() + timeout
    while len(got) < n:
        if time.time() > end:
            raise socket.timeout("salutul serverului e incomplet")
        chunk = sock.recv(4096)
        if not chunk:
            raise socket.timeout("conexiune inchisa in salut")
        got += chunk
    return got


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
    ap.add_argument("host", nargs="?", help="adresa OS-ului (o arata comanda net; testul QEMU: 127.0.0.1)")
    ap.add_argument("--test", action="store_true", help="verifica criptografia pe vectorii oficiali si iese")
    ap.add_argument("-p", "--port", type=int, default=2323)
    ap.add_argument("-k", "--cheie-fisier", default="remote.cheie", help="fisierul cu cheia (implicit remote.cheie)")
    ap.add_argument("-c", "--comanda", action="append", default=[], help="ruleaza comanda si iese (se poate repeta)")
    ap.add_argument("--asteapta", type=float, default=0, help="secunde in care reincearca conexiunea (OS-ul porneste)")
    a = ap.parse_args()
    if a.test:
        print("remote-client: criptografia OK; vector AUTH " + self_test())
        return 0
    if not a.host:
        ap.error("lipseste adresa OS-ului")
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
