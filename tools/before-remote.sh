#!/usr/bin/env bash
# Started by tools/test.sh before the `remote` scenario (P6-49): a fresh random key goes on the test disk as
# /remote.cheie (never in the repository), then tools/remote-test.py talks to the OS through QEMU's port
# forward 127.0.0.1:12323 -> 2323. tools/after-remote.sh checks build/remote-client.log.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
key="$ROOT/build/remote-test.cheie"
python3 -c "import secrets; print(secrets.token_urlsafe(32))" > "$key"
MTOOLS_SKIP_CHECK=1 mcopy -o -i "$DISK_IMG@@1M" "$key" ::/remote.cheie
exec python3 "$ROOT/tools/remote-test.py" "$key" "$ROOT/build/remote-client.log"
