#!/usr/bin/env python3
"""Imitation of GitHub for the `update` QEMU scenario (P6-08): the REST API that /bin/actualizeaza talks to and the file
storage the API redirects to, both HTTPS (TLS 1.3, certificate for test.cyber.local from tls/tests/data).
  - API on 127.0.0.1:18445 (guest: 10.0.2.2:18445): GET /repos/Adriancu91/cyber-universe/releases/latest (JSON, sent
    chunked) and GET /repos/.../releases/assets/<n> (with Accept: application/octet-stream) -> 302 to the storage.
    Like GitHub for a private repository: no token -> 404, unknown token -> 401.
  - storage on 127.0.0.1:18446: the file, with Content-Length; refuses requests that still carry the token (the real
    storage refuses them too) and the whole stick image (the update must not download it).
Token "tok-bun" gets the right files; "tok-stricat" gets a kernel with one byte changed (its sum must not match).
`update-test-server.py --files DIR` only writes the release files to DIR (used by tools/after-update.sh)."""
import hashlib, json, os, socket, ssl, sys, threading

D = sys.path[0] + "/../tls/tests/data"
REPO = "Adriancu91/cyber-universe"
TAG = "os-42"
TOKENS = {"tok-bun", "tok-stricat"}


def release_files():
    """The files of the test release: deterministic bytes, a few clusters to a few thousand of them."""
    def data(seed, size):
        out, block = bytearray(), seed.encode()
        while len(out) < size:
            block = hashlib.sha256(block).digest()
            out += block
        return bytes(out[:size])
    files = {
        "BOOTX64.EFI": data("efi", 210_000),
        "kernel": data("kernel", 1_300_007),
        "initrd.tar": data("initrd", 700_416),
        "limine.conf": b"timeout: 0\n\n/Cyber Universe OS (actualizat)\n    protocol: limine\n    kernel_path: boot():/boot/kernel\n",
        "limine-uefi-cd.bin": data("cd", 300_000),
        "cyber-universe-usb.img": b"nu se descarca",
        "VERSIUNE": b"commit test-actualizare-42\ndata 2026-10-22T00:00:00Z\n",
    }
    sums = "".join(f"{hashlib.sha256(v).hexdigest()}  {k}\n" for k, v in files.items())
    files["SHA256SUMS"] = sums.encode()
    return files


if len(sys.argv) == 3 and sys.argv[1] == "--files":
    os.makedirs(sys.argv[2], exist_ok=True)
    for name, body in release_files().items():
        open(os.path.join(sys.argv[2], name), "wb").write(body)
    sys.exit(0)

FILES = release_files()
NAMES = list(FILES)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(D + "/ec.pem", D + "/ec.key")


def log(*a):
    print("update-test-server:", *a, flush=True)


def reply(conn, status, body, ctype="application/octet-stream", extra="", chunked=False):
    reason = {200: "OK", 302: "Found", 400: "Bad Request", 401: "Unauthorized", 404: "Not Found", 500: "Server Error"}[status]
    if chunked:
        framed = b"".join(b"%x\r\n" % len(body[i:i + 1000]) + body[i:i + 1000] + b"\r\n" for i in range(0, len(body), 1000)) + b"0\r\n\r\n"
        head = f"HTTP/1.1 {status} {reason}\r\ncontent-type: {ctype}\r\ntransfer-encoding: chunked\r\n{extra}connection: close\r\n\r\n"
        conn.sendall(head.encode() + framed)
    else:
        head = f"HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {len(body)}\r\n{extra}Connection: close\r\n\r\n"
        conn.sendall(head.encode() + body)


def request(conn):
    data = b""
    while b"\r\n\r\n" not in data:
        d = conn.recv(4096)
        if not d:
            break
        data += d
    lines = data.partition(b"\r\n\r\n")[0].decode("latin-1").split("\r\n")
    h = {}
    for l in lines[1:]:
        k, _, v = l.partition(":")
        h[k.strip().lower()] = v.strip()
    return lines[0], h


def api(conn):
    line, h = request(conn)
    if h.get("host") != "test.cyber.local" or not h.get("user-agent"):
        return reply(conn, 400, b"antet Host sau User-Agent gresit"), "400 antete"
    auth = h.get("authorization", "")
    if not auth:
        return reply(conn, 404, b'{"message":"Not Found"}', "application/json"), "404 fara token"
    token = auth.removeprefix("Bearer ")
    if token not in TOKENS:
        return reply(conn, 401, b'{"message":"Bad credentials"}', "application/json"), "401 token necunoscut"
    if line == f"GET /repos/{REPO}/releases/latest HTTP/1.1":
        if h.get("accept") != "application/vnd.github+json":
            return reply(conn, 400, b"Accept gresit"), "400 accept"
        assets = [{"url": f"https://test.cyber.local:18445/repos/{REPO}/releases/assets/{100 + i}", "id": 100 + i,
                   "name": n, "label": "", "uploader": {"login": "github-actions[bot]"}, "content_type": "application/octet-stream",
                   "state": "uploaded", "size": len(FILES[n]), "browser_download_url": f"https://github.com/{REPO}/releases/download/{TAG}/{n}"}
                  for i, n in enumerate(NAMES)]
        body = json.dumps({"tag_name": TAG, "name": TAG, "draft": False, "prerelease": False,
                           "body": "Construit și testat de CI (test).", "assets": assets}).encode()
        return reply(conn, 200, body, "application/json", chunked=True), "200 release"
    prefix = f"GET /repos/{REPO}/releases/assets/"
    if line.startswith(prefix) and line.endswith(" HTTP/1.1"):
        n = int(line[len(prefix):-9]) - 100
        if h.get("accept") != "application/octet-stream":
            return reply(conn, 400, b"Accept gresit"), "400 accept"
        sig = "stricat" if token == "tok-stricat" else "bun"
        loc = f"https://test.cyber.local:18446/stocare/{NAMES[n]}?semnatura={sig}"
        return reply(conn, 302, b"", extra=f"Location: {loc}\r\n"), f"302 {NAMES[n]}"
    return reply(conn, 404, b'{"message":"Not Found"}', "application/json"), "404 " + line


def storage(conn):
    line, h = request(conn)
    if "authorization" in h:
        return reply(conn, 400, b"<Error>Only one auth mechanism allowed</Error>", "application/xml"), "400 token trimis la stocare"
    path = line.split(" ")[1]
    name, _, sig = path.removeprefix("/stocare/").partition("?semnatura=")
    if name not in FILES:
        return reply(conn, 404, b"nu exista"), "404 " + name
    if name == "cyber-universe-usb.img":
        return reply(conn, 500, b"imaginea intreaga nu trebuia ceruta"), "500 imaginea ceruta"
    body = FILES[name]
    if name == "kernel" and sig == "stricat":
        body = body[:1000] + bytes([body[1000] ^ 1]) + body[1001:]
    return reply(conn, 200, body), f"200 {name} ({len(body)} octeti)"


def serve(port, handler):
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(4)
    while True:
        raw, _ = srv.accept()
        raw.settimeout(20)
        try:
            conn = ctx.wrap_socket(raw, server_side=True)
            _, what = handler(conn)
            log(port, what)
            conn.unwrap()
        except Exception as e:
            print("update-test-server:", port, type(e).__name__, e, file=sys.stderr, flush=True)
        finally:
            raw.close()


threading.Thread(target=serve, args=(18446, storage), daemon=True).start()
serve(18445, api)
