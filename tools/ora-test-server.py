#!/usr/bin/env python3
"""SNTP test server for the `ora` QEMU scenario (P6-61): UDP 127.0.0.1:18123 (guest: 10.0.2.2:18123); answers every
48-byte client request (mode 3) with stratum 2 and the fixed time 2026-11-06 12:00:00 UTC."""
import socket, struct, sys
T = 1793966400 + 2208988800  # 2026-11-06 12:00:00 UTC, seconds since 1900
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", 18123))
while True:
    data, addr = s.recvfrom(512)
    if len(data) < 48 or data[0] & 7 != 3:
        continue
    reply = bytearray(48)
    reply[0] = (0 << 6) | (4 << 3) | 4  # LI 0, version 4, mode 4 (server)
    reply[1] = 2
    struct.pack_into(">II", reply, 32, T, 0)  # receive
    struct.pack_into(">II", reply, 40, T, 0)  # transmit
    s.sendto(bytes(reply), addr)
    print("ora-test-server: raspuns trimis", addr, flush=True)
