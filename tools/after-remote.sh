#!/usr/bin/env bash
# After the `remote` scenario (P6-49): the test client must have finished every step.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if grep -q "CLIENT OK" "$ROOT/build/remote-client.txt" 2>/dev/null; then
    sed 's/^/  [client] /' "$ROOT/build/remote-client.txt"
    exit 0
fi
echo "  [client] EROARE:"; cat "$ROOT/build/remote-client.log" 2>/dev/null | tail -20
exit 1
