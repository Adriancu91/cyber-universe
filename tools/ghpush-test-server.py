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
FILES = {"note/vechi.txt": b"versiunea veche\n",
         # P6-61 (C-84 item 2): a small Cyber Hub for `gh-ia --hub` (same repository, `hub =` in the guest's github.conf).
         "COMENZI.md": "# Comenzi\n\n## Active\n\n### C-1 · kronos · Ceva pentru Kronos\n- stare: nou\n\n"
                       "### C-2 · cyber-universe · Rapoartele pleaca singure\n- ce: ...\n- stare: preluat (P6-61)\n\n"
                       "### C-3 · cyber-gpu + cyber-universe · Pasi dupa I10\n- stare: nou\n- stare (cyber-universe): preluat\n\n"
                       "## Arhiva\n- stare: nu se arata\n".encode(),
         "stare/cyber-universe.md": "# Stare: Cyber Universe OS\nTotul merge in QEMU (proba).\n".encode(),
         "DECIZII.md": "".join(f"- decizia {i}\n" for i in range(1, 51)).encode(),
         # P6-64 (C-87 item 2): Adrian's commands for the OS: one to run, one refused (writes to the card), one done by hand.
         "integrare/cyber-universe/os-comenzi.md": "# Comenzi pentru OS\n\n- [ ] K-1: uptime\n- [ ] K-2: `gpu lant`\n"
                                                   "- [x] K-3: hw\n- [ ] K-4: mem\n".encode()}
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(D + "/ec.pem", D + "/ec.key")
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 18447))
srv.listen(4)
commits = 0
# P6-61 step 4 (F8): branches. "main" holds FILES; another branch starts as a copy of main when it is made.
BRANCHES = {"main": FILES}


def blob_sha(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def handle(method, path, h, body):
    global commits
    if h.get("host") != "test.cyber.local" or not h.get("user-agent") or h.get("accept") != "application/vnd.github+json":
        return 400, {"message": "antete gresite"}
    if h.get("authorization") != "Bearer " + TOKEN:
        return 401, {"message": "Bad credentials"}
    path, _, query = path.partition("?")
    ref = urllib.parse.unquote(urllib.parse.parse_qs(query).get("ref", ["main"])[0])
    refs = f"/repos/{REPO}/git/ref/heads/"
    if method == "GET" and path.startswith(refs):
        b = urllib.parse.unquote(path[len(refs):])
        if b not in BRANCHES:
            return 404, {"message": "Not Found"}
        return 200, {"ref": "refs/heads/" + b, "object": {"type": "commit", "sha": hashlib.sha1(b.encode()).hexdigest()}}
    if method == "POST" and path == f"/repos/{REPO}/git/refs":
        req = json.loads(body)
        b = req.get("ref", "").removeprefix("refs/heads/")
        if b in BRANCHES:
            return 422, {"message": "Reference already exists"}
        if req.get("sha") != hashlib.sha1(b"main").hexdigest():
            return 422, {"message": "Object does not exist"}
        BRANCHES[b] = dict(FILES)
        print(f"ghpush-test-server: ramura noua {b}", flush=True)
        return 201, {"ref": "refs/heads/" + b, "object": {"sha": req["sha"]}}
    prefix = f"/repos/{REPO}/contents"
    if not (path == prefix or path.startswith(prefix + "/")):
        return 404, {"message": "Not Found"}
    name = urllib.parse.unquote(path[len(prefix) + 1:])
    if method == "GET":
        files = BRANCHES.get(ref)
        if files is None:
            return 404, {"message": "No commit found for the ref " + ref}
        if name not in files:
            d = name + "/" if name else ""
            names = sorted({k[len(d):].split("/")[0] + ("/" if "/" in k[len(d):] else "") for k in files if k.startswith(d)})
            if not names:
                return 404, {"message": "Not Found"}
            return 200, [{"name": n.rstrip("/"), "path": d + n.rstrip("/"), "type": "dir" if n.endswith("/") else "file",
                          "size": 0 if n.endswith("/") else len(files[d + n])} for n in names]
        b64 = base64.b64encode(files[name]).decode()
        return 200, {"type": "file", "name": name.split("/")[-1], "path": name, "sha": blob_sha(files[name]), "size": len(files[name]),
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
    branch = req.get("branch", "main")
    if branch == "main" and "branch" in req:
        return 403, {"message": "testul: OS-ul nu scrie niciodata pe main cu --ramura"}
    if branch not in BRANCHES:
        return 404, {"message": "Branch " + branch + " not found"}
    FILES_B = BRANCHES[branch]
    if name in FILES_B and req.get("sha") != blob_sha(FILES_B[name]):
        return 409 if "sha" in req else 422, {"message": f"{name} does not match" if "sha" in req else "\"sha\" wasn't supplied."}
    created = name not in FILES_B
    FILES_B[name] = data
    commits += 1
    print(f"ghpush-test-server: {'creat' if created else 'actualizat'} {name} ({len(data)} octeti, ramura {branch}): {req['message']!r}", flush=True)
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
        reason = {200: "OK", 201: "Created", 400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found", 409: "Conflict", 422: "Unprocessable Entity"}[status]
        conn.sendall(b"HTTP/1.1 %d %s\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: %d\r\nConnection: close\r\n\r\n"
                     % (status, reason.encode(), len(payload)) + payload)
        print("ghpush-test-server:", method, path, status, flush=True)
        conn.unwrap()
    except Exception as e:
        print("ghpush-test-server:", type(e).__name__, e, file=sys.stderr, flush=True)
    finally:
        raw.close()
