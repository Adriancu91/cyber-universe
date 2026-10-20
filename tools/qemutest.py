#!/usr/bin/env python3
"""Headless QEMU test runner for Cyber Universe OS.

Boots the test ISO, watches the serial output for the expected lines, optionally takes a
screenshot, then acknowledges the kernel (one byte on COM1) so it exits QEMU through
isa-debug-exit.

Exit codes: 0 = pass, 1 = fail (missing text, kernel failure, timeout or QEMU error).
"""
import argparse
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib

QEMU_SUCCESS = 33  # isa-debug-exit: (0x10 << 1) | 1
QEMU_FAILED = 35   # isa-debug-exit: (0x11 << 1) | 1
READY = "TEST READY"


def read_ppm(path):
    with open(path, "rb") as f:
        data = f.read()
    parts, pos = [], 0
    while len(parts) < 4:  # magic, width, height, maxval (comments not produced by QEMU)
        while data[pos:pos + 1].isspace():
            pos += 1
        start = pos
        while not data[pos:pos + 1].isspace():
            pos += 1
        parts.append(data[start:pos])
    pos += 1
    if parts[0] != b"P6":
        raise ValueError("unexpected screenshot format")
    w, h = int(parts[1]), int(parts[2])
    return w, h, data[pos:pos + w * h * 3]


def write_png(path, w, h, rgb):
    raw = b"".join(b"\x00" + rgb[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def check_screen(ppm, png_out):
    w, h, rgb = read_ppm(ppm)
    colors = {rgb[i:i + 3] for i in range(0, len(rgb), 3 * 97)}  # sample
    if png_out:
        os.makedirs(os.path.dirname(png_out) or ".", exist_ok=True)
        write_png(png_out, w, h, rgb)
    if len(colors) < 2:
        return False, f"ecran uniform ({w}x{h}), nimic desenat"
    return True, f"ecran {w}x{h}, {len(colors)} culori in esantion"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iso", required=True)
    ap.add_argument("--expect", action="append", default=[], help="text that must appear on serial (in order)")
    ap.add_argument("--timeout", type=float, default=60)
    ap.add_argument("--screenshot", help="save a PNG of the screen at TEST READY and check it is not blank")
    ap.add_argument("--log", default="build/serial.log")
    ap.add_argument("--keys", help="QEMU key names to type (space-separated) once the kernel prints --keys-when")
    ap.add_argument("--keys-when", default="TASTATURA: astept taste")
    ap.add_argument("--monitor", help="QEMU monitor commands to send at --keys-when, separated by ',' (':' = space)")
    ap.add_argument("--expect-exit", choices=["success", "failed"], default="success",
                    help="how the kernel must end: success (code 33) or a reported failure (code 35)")
    ap.add_argument("qemu_args", nargs=argparse.REMAINDER)
    a = ap.parse_args()

    qemu_args = [x for x in a.qemu_args if x != "--"]
    tmp = tempfile.mkdtemp(prefix="cutest-")
    mon_path = os.path.join(tmp, "monitor.sock")
    cmd = ["qemu-system-x86_64", *qemu_args, "-cdrom", a.iso,
           "-display", "none", "-serial", "stdio",
           "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
           "-monitor", f"unix:{mon_path},server,nowait"]

    os.makedirs(os.path.dirname(a.log) or ".", exist_ok=True)
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
    os.set_blocking(proc.stdout.fileno(), False)

    out = b""
    deadline = time.time() + a.timeout
    acked = False
    keys_sent = not (a.keys or a.monitor)
    screen_msg = None
    screen_ok = True
    while time.time() < deadline and proc.poll() is None:
        try:
            chunk = proc.stdout.read(4096)
        except BlockingIOError:
            chunk = None
        if chunk:
            out += chunk
        elif not acked:
            time.sleep(0.05)
        text = out.decode("utf-8", "replace").replace("\r", "")
        if not keys_sent and a.keys_when in text:
            commands = [f"sendkey {k}" for k in (a.keys or "").split()]
            commands += [c.replace(":", " ") for c in (a.monitor or "").split(",") if c]
            with socket.socket(socket.AF_UNIX) as s:
                s.connect(mon_path)
                for c in commands:
                    s.sendall(f"{c}\n".encode())
                    time.sleep(0.15)
            keys_sent = True
        if not acked and READY in text:
            if a.screenshot:
                ppm = os.path.join(tmp, "screen.ppm")
                with socket.socket(socket.AF_UNIX) as s:
                    s.connect(mon_path)
                    s.sendall(f"screendump {ppm}\n".encode())
                    for _ in range(100):
                        if os.path.exists(ppm) and os.path.getsize(ppm) > 0:
                            time.sleep(0.2)
                            break
                        time.sleep(0.05)
                screen_ok, screen_msg = check_screen(ppm, a.screenshot)
            proc.stdin.write(b"k")  # acknowledge -> kernel exits QEMU
            acked = True
        if acked:
            time.sleep(0.05)

    timed_out = proc.poll() is None
    if timed_out:
        proc.kill()
    try:
        rest = proc.stdout.read()
        if rest:
            out += rest
    except (BlockingIOError, TypeError):
        pass
    code = proc.wait()
    text = out.decode("utf-8", "replace").replace("\r", "")
    with open(a.log, "w") as f:
        f.write(text)

    problems = []
    pos = 0
    for e in a.expect:
        i = text.find(e, pos)
        if i < 0:
            problems.append(f"lipseste textul: {e!r}")
        else:
            pos = i + len(e)
    wanted = QEMU_SUCCESS if a.expect_exit == "success" else QEMU_FAILED
    if timed_out:
        problems.append(f"timeout dupa {a.timeout:.0f}s")
    elif code != wanted:
        names = {QEMU_SUCCESS: "succes (33)", QEMU_FAILED: "esec raportat de nucleu (35)"}
        problems.append(f"QEMU a iesit cu {names.get(code, f'codul {code}')}, asteptat {names[wanted]}")
    if screen_msg:
        print(f"[test] captura: {screen_msg}")
        if not screen_ok:
            problems.append(screen_msg)

    if problems:
        print("[test] ESUAT:\n  - " + "\n  - ".join(problems))
        print("[test] ultimele linii de pe serial:")
        for line in text.strip().splitlines()[-15:]:
            print("   | " + line)
        return 1
    print(f"[test] TRECUT ({len(a.expect)} verificari, cod QEMU {code})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
