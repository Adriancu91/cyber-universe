#!/usr/bin/env python3
"""A tiny DNS server for the `dns` test scenario (P5-03). Listens on 127.0.0.1:<port> (UDP);
QEMU's user network maps the guest's 10.0.2.2:<port> to it. Fixed answers, no internet needed:
  test.cyber   -> A 10.1.2.3, TTL 60
  alias.cyber  -> CNAME test.cyber + A 10.1.2.3 (compressed names)
  anything else -> NXDOMAIN
Usage: dns-test-server.py <port>   (runs until killed)
"""
import socket
import struct
import sys

A_ADDR = bytes([10, 1, 2, 3])


def qname(msg: bytes, i: int):
    labels = []
    while msg[i]:
        n = msg[i]
        labels.append(msg[i + 1:i + 1 + n].decode("ascii", "replace").lower())
        i += 1 + n
    return ".".join(labels), i + 1


def answer(msg: bytes) -> bytes:
    ident, _flags, qd = struct.unpack(">HHH", msg[:6])
    name, end = qname(msg, 12)
    question = msg[12:end + 4]
    records = b""
    rcode, count = 0, 0
    if name == "test.cyber":
        records = b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 60, 4) + A_ADDR
        count = 1
    elif name == "alias.cyber":
        target = b"\x04test\x05cyber\x00"
        records = b"\xc0\x0c" + struct.pack(">HHIH", 5, 1, 60, len(target)) + target
        cname_at = 12 + len(question) + 12  # where the CNAME's target name starts
        records += struct.pack(">H", 0xC000 | cname_at) + struct.pack(">HHIH", 1, 1, 60, 4) + A_ADDR
        count = 2
    else:
        rcode = 3
    header = struct.pack(">HHHHHH", ident, 0x8180 | rcode, qd, count, 0, 0)
    return header + question + records


def main():
    port = int(sys.argv[1])
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", port))
    while True:
        msg, addr = s.recvfrom(512)
        try:
            s.sendto(answer(msg), addr)
        except (IndexError, struct.error, UnicodeError):
            pass


if __name__ == "__main__":
    main()
