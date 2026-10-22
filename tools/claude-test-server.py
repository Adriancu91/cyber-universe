#!/usr/bin/env python3
"""Imitation of the Anthropic Messages API for the `claude-cli` QEMU scenario (P6-04): HTTPS (TLS 1.3, certificate
for test.cyber.local from tls/tests/data) on 127.0.0.1:18444, which the guest reaches as 10.0.2.2:18444.
It checks the request the way the real API would need it (method, path, headers, JSON body) and answers in the API's
format, with chunked transfer encoding and \\u escapes, so /bin/claude has to handle both. Key "sk-test-cyber" is the
good one; any other gets 401. A request with something missing gets 400 and says what."""
import json, socket, ssl, sys

D = sys.path[0] + "/../tls/tests/data"
GOOD_KEY = "sk-test-cyber"
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(D + "/ec.pem", D + "/ec.key")
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 18444))
srv.listen(4)


def check(head, body):
    lines = head.split("\r\n")
    if lines[0] != "POST /v1/messages HTTP/1.1":
        return 400, "cerere gresita: " + lines[0]
    h = {}
    for l in lines[1:]:
        k, _, v = l.partition(":")
        h[k.strip().lower()] = v.strip()
    for name, want in [("anthropic-version", "2023-06-01"), ("content-type", "application/json"), ("host", "test.cyber.local")]:
        if h.get(name) != want:
            return 400, f"antet {name} gresit: {h.get(name)!r}"
    if int(h.get("content-length", "-1")) != len(body):
        return 400, "content-length gresit"
    if h.get("x-api-key") != GOOD_KEY:
        return 401, "invalid x-api-key"
    try:
        req = json.loads(body.decode("utf-8"))
    except Exception as e:
        return 400, "JSON invalid: " + str(e)
    m = req.get("messages")
    # A conversation: user, assistant, user, ... ending with the user (what the real API requires).
    if not isinstance(req.get("model"), str) or not isinstance(req.get("max_tokens"), int) or not isinstance(m, list) \
            or len(m) % 2 != 1 or any(x.get("role") != ("user", "assistant")[i % 2] or not isinstance(x.get("content"), (str, list))
                                       for i, x in enumerate(m)):
        return 400, "corp JSON gresit: " + body.decode("utf-8", "replace")
    if m[0]["content"] == "Fa testul uneltelor":
        return 200, tools_step(req, m)
    return 200, m[-1]["content"] + ("" if len(m) == 1 else f" (mesaje in conversatie: {len(m)})")


# The tools test (P6-06): Claude asks for each tool in turn; each step checks the result of the previous one.
STEPS = [
    ("list_dir", {"path": "/disk"}, None),
    ("read_file", {"path": "/disk/salut.txt"}, lambda r, e: "salut.txt" in r and not e),
    ("write_file", {"path": "/disk/claude.txt", "content": "scris de Claude\n"}, lambda r, e: r.startswith("salut de pe disc") and not e),
    ("write_file", {"path": "/disk/claude.txt", "content": "scris de Claude\n"}, lambda r, e: e and "refuzat" in r),
    ("run_command", {"command": "ls /disk"}, lambda r, e: not e and "scris /disk/claude.txt" in r),
    ("github_push", {"file": "/disk/claude.txt", "repo": "Adriancu91/note-test", "path": "note/claude.txt"},
     lambda r, e: not e and "claude.txt" in r and "salut.txt" in r),
    # No token on the test disk: gh-trimite runs and says so (P6-09).
    ("write_file", {"path": "/etc/rau.txt", "content": "nu"}, lambda r, e: not e and "gh-trimite: lipseste tokenul GitHub" in r),
    (None, None, lambda r, e: e and "nu e permisa" in r),
]


def tools_step(req, m):
    """Returns ("tool", block) or ("text", text) for the next answer."""
    if not isinstance(req.get("tools"), list) or {t.get("name") for t in req["tools"]} != {"read_file", "list_dir", "write_file", "run_command", "github_push"}:
        return ("text", "Testul uneltelor a esuat: uneltele nu au fost declarate")
    results = [x["content"][0] for x in m if x["role"] == "user" and isinstance(x["content"], list)]
    n = len(results)
    if n:
        r = results[-1]
        if r.get("type") != "tool_result" or r.get("tool_use_id") != f"toolu_{n - 1}":
            return ("text", f"Testul uneltelor a esuat la pasul {n}: tool_result gresit {r}")
        text = r.get("content") if isinstance(r.get("content"), str) else ""
        if not STEPS[n][2](text, bool(r.get("is_error"))):
            return ("text", f"Testul uneltelor a esuat la pasul {n}: {text!r}")
    name, inp, _ = STEPS[n]
    if name is None:
        return ("text", "Gata: toate uneltele au mers cum trebuie.")
    return ("tool", {"type": "tool_use", "id": f"toolu_{n}", "name": name, "input": inp})


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
        head = head.decode("latin-1")
        want = 0
        for l in head.split("\r\n"):
            if l.lower().startswith("content-length:"):
                want = int(l.split(":")[1])
        while len(body) < want:
            d = conn.recv(4096)
            if not d:
                break
            body += d
        status, text = check(head, body)
        if status == 200 and isinstance(text, tuple):
            kind, value = text
            if kind == "tool":
                content = [{"type": "text", "text": f"Folosesc {value['name']}."}, value]
                answer = {"id": "msg_test", "type": "message", "role": "assistant", "model": "test", "content": content,
                          "stop_reason": "tool_use", "usage": {"input_tokens": 10, "output_tokens": 12}}
                text = "tool_use " + value["name"]
            else:
                answer = {"id": "msg_test", "type": "message", "role": "assistant", "model": "test",
                          "content": [{"type": "text", "text": value}], "stop_reason": "end_turn", "usage": {"input_tokens": 10, "output_tokens": 12}}
                text = value
        elif status == 200:
            answer = {"id": "msg_test", "type": "message", "role": "assistant", "model": "test",
                      "content": [{"type": "text", "text": "Salut! Am primit: " + text + " (cu diacritice: ăîșț)"}],
                      "stop_reason": "end_turn", "usage": {"input_tokens": 10, "output_tokens": 12}}
        else:
            answer = {"type": "error", "error": {"type": "authentication_error" if status == 401 else "invalid_request_error", "message": text}}
        payload = json.dumps(answer).encode()  # ensure_ascii: diacritics travel as \u escapes
        chunks = b"".join(b"%x\r\n" % len(payload[i:i + 7]) + payload[i:i + 7] + b"\r\n" for i in range(0, len(payload), 7))
        reason = {200: "OK", 400: "Bad Request", 401: "Unauthorized"}[status]
        conn.sendall(b"HTTP/1.1 %d %s\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
                     % (status, reason.encode()) + chunks + b"0\r\n\r\n")
        print("claude-test-server:", status, text, flush=True)
        conn.unwrap()
    except Exception as e:
        print("claude-test-server:", type(e).__name__, e, file=sys.stderr, flush=True)
    finally:
        raw.close()
