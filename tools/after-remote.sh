#!/usr/bin/env bash
# After the `remote` scenario (P6-49): the harness's client saw every expected answer, and the key never reached
# the serial log.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
log="$ROOT/build/remote-client.log"
cat "$log" 2>/dev/null | grep -v '^\(cyber>\| \)' | sed 's/^/[remote] /' | head -40
grep -q "CLIENT: TOATE VERIFICARILE AU TRECUT" "$log" || { echo "[remote] ESUAT: clientul nu a vazut tot ce trebuia"; exit 1; }
if grep -qF "$(cat "$ROOT/build/remote-test.cheie")" "$ROOT/build/serial-remote.log"; then
    echo "[remote] ESUAT: cheia apare pe portul serial"; exit 1
fi
echo "[remote] clientul: toate verificarile au trecut; cheia nu apare pe serial"
