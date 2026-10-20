"""TLS 1.3 test server (OpenSSL through Python's ssl), for tls/tests and the `https` QEMU scenario.
Usage: server.py CHAIN.pem KEY [count] [port] [text]. Prints the port it listens on (0 = any free one), then answers
`count` connections with an HTTP page (binary bytes at the end, or only text with `text`) and close_notify."""
import socket, ssl, sys
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(sys.argv[1], sys.argv[2])
count = int(sys.argv[3]) if len(sys.argv) > 3 else 1
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(('127.0.0.1', int(sys.argv[4]) if len(sys.argv) > 4 else 0))
srv.listen(4)
print(srv.getsockname()[1], flush=True)
body = b'Salut prin TLS 1.3 de la serverul de test!\n'
if len(sys.argv) > 5 and sys.argv[5] == 'text':
    body += b'linie de umplutura 0123456789\n' * 600 + b'Sfarsit pagina HTTPS.\n'
else:
    body += bytes(range(256)) * 100
for _ in range(count):
    raw, _ = srv.accept()
    raw.settimeout(10)
    try:
        conn = ctx.wrap_socket(raw, server_side=True)
        req = b''
        while b'\r\n\r\n' not in req:
            d = conn.recv(4096)
            if not d:
                break
            req += d
        conn.sendall(b'HTTP/1.0 200 OK\r\nContent-Length: %d\r\n\r\n' % len(body) + body)
        conn.unwrap()
    except Exception as e:
        print('server:', type(e).__name__, e, file=sys.stderr, flush=True)
    finally:
        raw.close()
