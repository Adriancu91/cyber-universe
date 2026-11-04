#!/usr/bin/env bash
# Started by tools/test.sh before the `web` scenario (P6-53): the small test site for /bin/web — HTTPS on 127.0.0.1:18447
# and plain HTTP on 127.0.0.1:18448, reached by the guest as 10.0.2.2.
exec python3 "$(dirname "$0")/web-test-server.py"
