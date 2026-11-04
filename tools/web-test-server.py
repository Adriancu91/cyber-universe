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

PAGES = {
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


if len(sys.argv) == 3 and sys.argv[1] == "--print":
    sys.stdout.write(PAGES[sys.argv[2]][2].decode())
    sys.exit(0)

threading.Thread(target=serve, args=(18448, False), daemon=True).start()
serve(18447, True)
