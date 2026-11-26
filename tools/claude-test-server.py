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
    # P6-209: prompt caching — the client marks the last block of the last message (a plain text turns into one text
    # block); the system prompt is a list of blocks. Check the marks, then read text-only contents back as plain text.
    if isinstance(m, list) and m:
        last = m[-1].get("content")
        if not (isinstance(last, list) and last and isinstance(last[-1], dict) and last[-1].get("cache_control") == {"type": "ephemeral"}):
            return 400, "lipseste cache_control pe ultimul bloc al ultimului mesaj"
        for x in m:
            c = x.get("content")
            if isinstance(c, list) and c and all(isinstance(b, dict) and b.get("type") == "text" for b in c):
                x["content"] = "".join(b.get("text", "") for b in c)
    sysp = req.get("system")
    if sysp is not None and not (isinstance(sysp, list) and sysp and sysp[-1].get("cache_control") == {"type": "ephemeral"}):
        return 400, "system fara cache_control"
    tl = req.get("tools")
    if tl is not None and not (isinstance(tl, list) and tl and tl[-1].get("cache_control") == {"type": "ephemeral"}):
        return 400, "uneltele fara cache_control pe ultima"
    # A conversation: user, assistant, user, ... ending with the user (what the real API requires).
    if not isinstance(req.get("model"), str) or not isinstance(req.get("max_tokens"), int) or not isinstance(m, list) \
            or len(m) % 2 != 1 or any(x.get("role") != ("user", "assistant")[i % 2] or not isinstance(x.get("content"), (str, list))
                                       for i, x in enumerate(m)):
        return 400, "corp JSON gresit: " + body.decode("utf-8", "replace")
    if m[0]["content"] == "Fa testul uneltelor":
        return 200, tools_step(req, m)
    # P6-203 (C-166 step 2): the agent's read-only tools, each step checking the previous result.
    if m[0]["content"] == "Fa testul agentului":
        return 200, agent_step(req, m, AGENT_STEPS, "agentului")
    # P6-220 (C-166 step 5): the code tools against the GitHub imitation (code repository = its test repository).
    if m[0]["content"] == "Fa testul propunerii":
        return 200, agent_step(req, m, PROPOSAL_STEPS, "propunerii")
    # P6-202 (C-166): `agent ping` -> "pong", with cache tokens in `usage` (the agent prices them).
    if m[0]["content"] == "ping: raspunde doar cu cuvantul pong":
        return 200, ("pong", None)
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


AGENT_STEPS = [
    ("fisiere", {"cale": "/etc/agent"}, None),
    ("citeste", {"cale": "/etc/agent.setari"}, lambda r, e: "cheltuieli.txt" in r and not e),
    ("citeste", {"cale": "/etc/claude.cheie"}, lambda r, e: "plafon_zi" in r and not e),
    ("ruleaza", {"comanda": "mem"}, lambda r, e: e and "sk-test" not in r),
    ("ruleaza", {"comanda": "reboot"}, lambda r, e: not e and len(r) > 0),
    ("citeste_jurnal", {"cuvant": "agent", "n": 5}, lambda r, e: e and "lista alba" in r),
    ("stare", {}, lambda r, e: not e),
    ("tablou", {"n": 5}, lambda r, e: not e and "$ uptime" in r and "$ mem" in r),
    ("citeste", {"cale": "/bin/hello"}, lambda r, e: not e and "tabloului placii video" in r and "nu trimite nicio comanda" in r),
    # P6-219 (C-166 step 4): the white-listed repairs; everything else refused by the OS.
    ("repara", {"actiune": "setare", "nume": "max_tokens", "valoare": "4096"}, lambda r, e: e and "nu e permisa" in r),
    ("repara", {"actiune": "setare", "nume": "plafon_zi", "valoare": "1000"}, lambda r, e: not e and "max_tokens = 4096 scris in /etc/agent.setari" in r),
    ("repara", {"actiune": "setare", "nume": "live_minute", "valoare": "0"}, lambda r, e: e and "nu e in lista aprobata" in r),
    ("repara", {"actiune": "opreste_lucrarea", "nume": "proba-agent"}, lambda r, e: e and "iese din limite" in r),
    # P6-225: taking up a job the OS gave up on; this one never ran, so the kernel refuses.
    ("repara", {"actiune": "reia_lucrarea", "nume": "proba-agent"}, lambda r, e: "lucrare proba-agent:" in r),
    ("repara", {"actiune": "formateaza", "nume": "/disk"}, lambda r, e: e and "reluarea REFUZATA" in r),
    (None, None, lambda r, e: e and "nu e in lista alba a reparatiilor" in r),
]


GOOD_FILES = [{"cale": "note/vechi.txt", "continut": "versiunea propusa\n"}, {"cale": "note/nou.txt", "continut": "fisier nou propus\n"}]
GOOD = {"subiect": "proba", "explicatie": "o proba a propunerilor", "dovada": "testul agent in QEMU", "fisiere": GOOD_FILES}
PROPOSAL_STEPS = [
    ("citeste_cod", {"cale": "note/vechi.txt"}, None),
    ("citeste_cod", {"cale": ""}, lambda r, e: not e and "versiunea veche" in r),
    ("propune", dict(GOOD, fisiere=[{"cale": ".github/workflows/x.yml", "continut": "on: push"}]),
     lambda r, e: not e and "note/" in r and "COMENZI.md" in r),
    ("propune", dict(GOOD, subiect="Proba Mare"), lambda r, e: e and ".github/workflows/x.yml" in r and "nu e permisa" in r),
    ("propune", GOOD, lambda r, e: e and "subiectul" in r),
    ("propune", GOOD, lambda r, e: not e and "ramura agent/" in r and "note/vechi.txt actualizat" in r and "note/nou.txt creat" in r),
    (None, None, lambda r, e: e and "exista deja" in r),
]


def agent_step(req, m, steps, what):
    names = {t.get("name") for t in req.get("tools") or []}
    if names != {"fisiere", "citeste", "citeste_jurnal", "stare", "tablou", "ruleaza", "repara", "citeste_cod", "propune"} \
            or "agentul de bord" not in str(req.get("system")):
        return ("text", f"Testul {what} a esuat: uneltele sau rolul lipsesc")
    results = [x["content"][0] for x in m if x["role"] == "user" and isinstance(x["content"], list)]
    n = len(results)
    if n:
        r = results[-1]
        text = r.get("content") if isinstance(r.get("content"), str) else ""
        if r.get("type") != "tool_result" or r.get("tool_use_id") != f"toolu_{n - 1}" or not steps[n][2](text, bool(r.get("is_error"))):
            return ("text", f"Testul {what} a esuat la pasul {n}: {text[:300]!r}")
    name, inp, _ = steps[n]
    if name is None:
        return ("text", "Gata: uneltele agentului au mers cum trebuie." if what == "agentului" else "Gata: propunerile au mers cum trebuie.")
    return ("tool", {"type": "tool_use", "id": f"toolu_{n}", "name": name, "input": inp})


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
        if status == 200 and isinstance(text, tuple) and text[0] == "pong":
            answer = {"id": "msg_test", "type": "message", "role": "assistant", "model": "test-opus", "content": [{"type": "text", "text": "pong"}],
                      "stop_reason": "end_turn", "usage": {"input_tokens": 20, "output_tokens": 3, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 100}}
            text = "pong"
        elif status == 200 and isinstance(text, tuple):
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
