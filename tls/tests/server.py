"""TLS 1.3 test server for tls/tests (OpenSSL through Python's ssl). Usage: server.py CHAIN.pem KEY [count].
Prints the port it listens on, then answers `count` connections with a small HTTP page and close_notify."""
import socket, ssl, sys
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(sys.argv[1], sys.argv[2])
count = int(sys.argv[3]) if len(sys.argv) > 3 else 1
srv = socket.socket()
srv.bind(('127.0.0.1', 0))
srv.listen(4)
print(srv.getsockname()[1], flush=True)
body = b'Salut prin TLS 1.3 de la serverul de test!\n' + bytes(range(256)) * 100
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
