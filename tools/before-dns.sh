#!/usr/bin/env bash
# Started by tools/test.sh before the `dns` scenario: the test DNS server on 127.0.0.1:15353
# (the guest reaches it as 10.0.2.2:15353 through QEMU's user network).
exec python3 "$(dirname "$0")/dns-test-server.py" 15353
