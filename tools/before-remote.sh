#!/usr/bin/env bash
# Started by tools/test.sh before the `remote` scenario (P6-49): the test client talks to the guest's remote service
# through QEMU hostfwd (127.0.0.1:12323 -> guest :2323); tools/after-remote.sh checks its result file.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
rm -f "$ROOT/build/remote-client.txt"
exec python3 "$ROOT/tools/remote-client.py" --test 127.0.0.1 12323 cheie-de-test-remote-0123 "$ROOT/build/remote-client.txt" \
    > "$ROOT/build/remote-client.log" 2>&1
