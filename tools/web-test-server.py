#!/usr/bin/env python3
"""Small web site for the `web` QEMU scenario (P6-53, the text browser): the same pages over HTTPS (TLS 1.3,
certificate for test.cyber.local from tls/tests/data) on 127.0.0.1:18447 and over plain HTTP on 127.0.0.1:18448;
the guest reaches them as 10.0.2.2. Pages:
  /                   the start page: title, headings, entities, lists, a table, <pre>, links (absolute, relative,
                      to another folder, fragments and javascript: that must not get a number), a script and a
                      style that must not show
  /doc/pagina2.html   reached by link 1; long, so it has several screen pages; links back with "../"
  /vechi              301 -> /doc/pagina2.html (redirects are followed)
  /text.txt           plain text, shown as it is
  anything else       404
`web-test-server.py --print PATH` prints a page (to look at it on the host)."""
import socket, ssl, sys, threading

D = sys.path[0] + "/../tls/tests/data"

INDEX = """<!DOCTYPE html>
<html lang="ro"><head>
<meta charset="utf-8">
<title>Pagina de  test
 Cyber</title>
<style>body { color: red } h1::before { content: "NU SE VEDE STIL" }</style>
<script>document.write("NU SE VEDE SCRIPT <b>x</b>");</script>
</head>
<body>
<!-- un comentariu care NU SE VEDE -->
<h1>Bine ai venit</h1>
<p>Mere &amp; pere, &lt;3&gt;, &#259;&#x219;&icirc; &mdash; gata.
   Textul acesta    are spatii    multe.</p>
<h2>Liste</h2>
<ul><li>primul element</li><li>al doilea cu <a href="/doc/pagina2.html">documentatia</a></li>
<li>al treilea<ol><li>unu</li><li>doi</li></ol></li></ul>
<p>Legaturi: <a href="relativ.html">relativa</a>, <a href='https://alt.example/x?a=1&amp;b=2'>externa</a>,
<a href="#sus">fragment</a>, <a href="javascript:void(0)">script</a>, <a href=/vechi>veche</a>.</p>
<table><tr><th>Nume</th><th>Valoare</th></tr><tr><td>viteza</td><td>206 simb/s</td></tr></table>
<pre>
  cod   pastrat
    asa cum e
</pre>
<blockquote>un citat</blockquote>
<p><img src="x.png" alt="sigla Cyber"> 5 &lt; 6 si 7 > 3</p>
</body></html>
"""

PAGE2 = (
    "<html><head><title>Documentatie</title></head><body><h1>Documentatie</h1>\n"
    + "".join(f"<p>Paragraful {i}: un text de documentatie destul de lung ca sa se imparta pe mai multe randuri "
              f"pe ecran.</p>\n" for i in range(1, 31))
    + '<p>Inapoi la <a href="../">pagina de start</a> sau <a href="pagina3.html?x=1#y">pagina 3</a>.</p>'
    + "<p>Sfarsitul documentatiei.</p></body></html>\n"
)

# P6-61 step 6 (F6b): two forms. F1: GET to /cauta (text, hidden, checked checkbox, unchecked one, select with a
# selected option); F2: POST to /trimite (textarea with a starting text, password, submit button).
FORMS = """<html><head><title>Formulare</title></head><body><h1>Cautare</h1>
<form action="/cauta" method="get">Cauta: <input type="text" name="q"> <input type=hidden name=lang value=ro>
<input type="checkbox" name="exact" value="da" checked> exact <input type="checkbox" name="vechi"> vechi
<select name="unde"><option value="tot">tot</option><option value="titlu" selected>titlu</option></select>
<input type="submit" value="Cauta"></form>
<form action="trimite" method="POST"><textarea name="mesaj">scrie aici</textarea>
<input type="password" name="parola" value="abc"><button>Trimite</button></form>
</body></html>
"""

# P6-61 step 6 (F6b): basic CSS. The words in capitals must not show; the hidden field is still sent.
STIL = """<html><head><title>Stil</title><style>/* foaia */ .ascuns, #reclama { display: none }
@media print { .tipar { display: none } } div p { display: none }</style></head><body>
<h1>Stil de baza</h1><div id="reclama">RECLAMA <a href="/r">LEGATURA</a></div><p class="ascuns">SECRET</p>
<p hidden>ATRIBUT</p><p style="display: none">INLINE</p><p class="tipar">vizibil pe ecran</p>
<ul><li hidden>ASCUNS<li>element vizibil</ul>
<form action="/cauta"><div class="ascuns"><input name="capcana" value="z">CAMP</div><input name="q" value="a"></form>
<p>sfarsit stil</p></body></html>
"""



def png_test():
    """P6-61 step 6 (F6b): a 48 x 32 RGBA PNG with every row filter (0..4, in turn) and some transparent pixels,
    built here with zlib (no picture library), so /bin/web's own PNG decoder is checked against known pixels."""
    import struct, zlib
    w, h = 48, 32
    px = [[((x * 5) & 255, (y * 8) & 255, ((x ^ y) * 9) & 255, 255 if (x + y) % 7 else (x * 5) & 255) for x in range(w)] for y in range(h)]
    raw = bytearray()
    prev = bytes(w * 4)
    for y in range(h):
        row = bytes(c for p in px[y] for c in p)
        f = y % 5
        out = bytearray()
        for i in range(len(row)):
            a = row[i - 4] if i >= 4 else 0
            b = prev[i]
            c = prev[i - 4] if i >= 4 else 0
            if f == 0: pred = 0
            elif f == 1: pred = a
            elif f == 2: pred = b
            elif f == 3: pred = (a + b) // 2
            else:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
            out.append((row[i] - pred) & 255)
        raw += bytes([f]) + out
        prev = row
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    # The checksum /bin/web prints: FNV-1a 64 of the pixels composited over white, as little-endian 0x00RRGGBB.
    hsum = 0xcbf29ce484222325
    for y in range(h):
        for (r, g, b, al) in px[y]:
            v = [(c * al + 255 * (255 - al)) // 255 for c in (r, g, b)]
            for byte in (v[2], v[1], v[0], 0):
                hsum = ((hsum ^ byte) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return data, hsum


PNG_TEST, PNG_SUM = png_test()

PAGES = {
    "/x.png": ("200 OK", "image/png", PNG_TEST),
    "/stil": ("200 OK", "text/html", STIL.encode()),
    "/formular": ("200 OK", "text/html", FORMS.encode()),
    "/": ("200 OK", "text/html; charset=utf-8", INDEX.encode()),
    "/doc/pagina2.html": ("200 OK", "text/html", PAGE2.encode()),
    "/text.txt": ("200 OK", "text/plain", "Text simplu, fara HTML.\nRandul doi <nu e eticheta>.\n".encode()),
}


def log(*a):
    print("web-test-server:", *a, flush=True)


def answer(conn):
    req = b""
    while b"\r\n\r\n" not in req:
        d = conn.recv(4096)
        if not d:
            return
        req += d
    line = req.split(b"\r\n")[0].decode(errors="replace")
    parts = line.split(" ")
    path = parts[1] if len(parts) > 1 else "/"
    log(line)
    if path.startswith("/cauta?"):
        body = ("<html><head><title>Rezultate</title></head><body><p>cautare primita: " + path[len("/cauta?"):]
                + "</p></body></html>").encode()
        conn.sendall(f"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
        return
    if parts[0] == "POST" and path == "/trimite":
        head, _, data = req.partition(b"\r\n\r\n")
        hl = head.decode(errors="replace").lower()
        n = next((int(l.split(":", 1)[1]) for l in hl.split("\r\n") if l.startswith("content-length:")), 0)
        while len(data) < n:
            d = conn.recv(4096)
            if not d:
                break
            data += d
        ok = "content-type: application/x-www-form-urlencoded" in hl
        body = ("<html><body><p>POST primit (" + ("tip bun" if ok else "TIP GRESIT") + "): " + data.decode(errors="replace")
                + "</p></body></html>").encode()
        conn.sendall(f"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
        return
    if path == "/vechi":
        head = "HTTP/1.1 301 Moved Permanently\r\nLocation: /doc/pagina2.html\r\nContent-Length: 0\r\n"
        conn.sendall((head + "Connection: close\r\n\r\n").encode())
        return
    status, ctype, body = PAGES.get(path, ("404 Not Found", "text/html", b"<h1>Nu exista</h1>"))
    head = f"HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    conn.sendall(head.encode() + body)


def serve(port, tls):
    ctx = None
    if tls:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.minimum_version = ssl.TLSVersion.TLSv1_3
        ctx.load_cert_chain(D + "/ec.pem", D + "/ec.key")
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(8)
    while True:
        raw, _ = srv.accept()
        raw.settimeout(10)
        try:
            conn = ctx.wrap_socket(raw, server_side=True) if tls else raw
            answer(conn)
            if tls:
                try:
                    conn.unwrap()
                except Exception:
                    pass
            conn.close()
        except Exception as e:
            log("eroare:", e)
            raw.close()


if len(sys.argv) == 2 and sys.argv[1] == "--png-suma":
    print(f"{PNG_SUM:016x}")
    sys.exit(0)

if len(sys.argv) == 3 and sys.argv[1] == "--print":
    sys.stdout.write(PAGES[sys.argv[2]][2].decode())
    sys.exit(0)

threading.Thread(target=serve, args=(18448, False), daemon=True).start()
serve(18447, True)
