#!/usr/bin/env bash
# Started by tools/test.sh before the `appnet` scenario: the TCP test web server (10.0.2.2:18080 in the guest)
# and a UDP echo server that answers "ecou: <text>" (10.0.2.2:15354).
python3 -c '
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", 15354))
while True:
    m, a = s.recvfrom(2048)
    s.sendto(b"ecou: " + m, a)
' &
echo_pid=$!
"$(dirname "$0")/before-tcp.sh" &
web_pid=$!
trap 'kill $echo_pid $web_pid 2>/dev/null; exit 0' TERM INT
wait
