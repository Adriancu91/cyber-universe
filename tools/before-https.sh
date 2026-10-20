#!/usr/bin/env bash
# Started by tools/test.sh before the `https` scenario: the TLS 1.3 test server (OpenSSL through Python's ssl) on
# 127.0.0.1:18443, which the guest reaches as 10.0.2.2:18443. Certificate for test.cyber.local (tls/tests/data).
D="$(dirname "$0")/../tls/tests"
exec python3 "$D/server.py" "$D/data/ec.pem" "$D/data/ec.key" 100 18443 text
