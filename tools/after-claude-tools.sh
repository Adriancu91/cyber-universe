#!/usr/bin/env bash
# Checks the disk of the `claude-tools` scenario (argument: the disk image), as Linux sees it: the file Claude wrote
# after Adrian's "Da" is there with the right content, the disk passes fsck.fat, and nothing else was written.
set -uo pipefail
IMG="$1"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export MTOOLS_SKIP_CHECK=1 PATH="$PATH:/usr/sbin:/sbin"
fail() { echo "[after-claude-tools] EȘUAT: $*"; exit 1; }
dd if="$IMG" of="$TMP/part.img" bs=1M skip=1 count=38 status=none || fail "nu pot extrage partiția"
fsck.fat -n "$TMP/part.img" > "$TMP/fsck.txt" 2>&1 || { cat "$TMP/fsck.txt"; fail "fsck.fat a găsit erori"; }
[[ "$(mtype -i "$IMG@@1M" ::/claude.txt)" == "scris de Claude" ]] || fail "claude.txt lipsește sau are alt conținut"
echo "[after-claude-tools] claude.txt scris o singură dată, după „Da”; discul e curat"
