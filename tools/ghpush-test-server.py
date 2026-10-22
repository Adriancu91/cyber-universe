#!/usr/bin/env python3
"""Imitation of the GitHub "contents" API for the `ghpush` QEMU scenario (P6-09): HTTPS (TLS 1.3, certificate for
test.cyber.local from tls/tests/data) on 127.0.0.1:18447 (guest: 10.0.2.2:18447). Files live in memory; one is there
from the start (note/vechi.txt). GET answers like GitHub (sha = git blob sha, content in base64 split into lines of 60);
PUT checks the request like GitHub would (token, JSON, base64, message, and the sha of the current version when the
file exists — a wrong or missing one gets 409/422) and stores the file. Token "tok-gh" is the good one."""
import base64, hashlib, json, socket, ssl, sys, urllib.parse

D = sys.path[0] + "/../tls/tests/data"
REPO = "Adriancu91/note-test"
TOKEN = "tok-gh"
FILES = {"note/vechi.txt": b"versiunea veche\n"}
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(D + "/ec.pem", D + "/ec.key")
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 18447))
srv.listen(4)
commits = 0


def blob_sha(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def handle(method, path, h, body):
    global commits
    if h.get("host") != "test.cyber.local" or not h.get("user-agent") or h.get("accept") != "application/vnd.github+json":
        return 400, {"message": "antete gresite"}
    if h.get("authorization") != "Bearer " + TOKEN:
        return 401, {"message": "Bad credentials"}
    prefix = f"/repos/{REPO}/contents/"
    if not path.startswith(prefix):
        return 404, {"message": "Not Found"}
    name = urllib.parse.unquote(path[len(prefix):])
    if method == "GET":
        if name not in FILES:
            return 404, {"message": "Not Found"}
        b64 = base64.b64encode(FILES[name]).decode()
        return 200, {"type": "file", "name": name.split("/")[-1], "path": name, "sha": blob_sha(FILES[name]), "size": len(FILES[name]),
                     "encoding": "base64", "content": "\n".join(b64[i:i + 60] for i in range(0, len(b64), 60)) + "\n"}
    if method != "PUT":
        return 404, {"message": "Not Found"}
    if h.get("content-type") != "application/json" or int(h.get("content-length", -1)) != len(body):
        return 400, {"message": "Content-Type sau Content-Length gresit"}
    try:
        req = json.loads(body)
        data = base64.b64decode(req["content"], validate=True)
    except Exception as e:
        return 400, {"message": "Problems parsing JSON: " + str(e)}
    if not isinstance(req.get("message"), str) or not req["message"]:
        return 422, {"message": "Invalid request: message lipseste"}
    if name in FILES and req.get("sha") != blob_sha(FILES[name]):
        return 409 if "sha" in req else 422, {"message": f"{name} does not match" if "sha" in req else "\"sha\" wasn't supplied."}
    created = name not in FILES
    FILES[name] = data
    commits += 1
    print(f"ghpush-test-server: {'creat' if created else 'actualizat'} {name} ({len(data)} octeti): {req['message']!r}", flush=True)
    return (201 if created else 200), {"content": {"path": name, "sha": blob_sha(data), "html_url": f"https://github.com/{REPO}/blob/main/{name}"},
                                       "commit": {"sha": hashlib.sha1(b"commit %d" % commits).hexdigest(), "message": req["message"]}}


while True:
    raw, _ = srv.accept()
    raw.settimeout(10)
    try:
        conn = ctx.wrap_socket(raw, server_side=True)
        data = b""
        while b"\r\n\r\n" not in data:
            d = conn.recv(4096)
            if not d:
                break
            data += d
        head, _, body = data.partition(b"\r\n\r\n")
        lines = head.decode("latin-1").split("\r\n")
        h = {}
        for l in lines[1:]:
            k, _, v = l.partition(":")
            h[k.strip().lower()] = v.strip()
        want = int(h.get("content-length", 0))
        while len(body) < want:
            d = conn.recv(65536)
            if not d:
                break
            body += d
        method, path, _ = lines[0].split(" ")
        status, answer = handle(method, path, h, body)
        payload = json.dumps(answer).encode()
        reason = {200: "OK", 201: "Created", 400: "Bad Request", 401: "Unauthorized", 404: "Not Found", 409: "Conflict", 422: "Unprocessable Entity"}[status]
        conn.sendall(b"HTTP/1.1 %d %s\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: %d\r\nConnection: close\r\n\r\n"
                     % (status, reason.encode(), len(payload)) + payload)
        print("ghpush-test-server:", method, path, status, flush=True)
        conn.unwrap()
    except Exception as e:
        print("ghpush-test-server:", type(e).__name__, e, file=sys.stderr, flush=True)
    finally:
        raw.close()
