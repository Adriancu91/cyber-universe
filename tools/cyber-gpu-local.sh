#!/usr/bin/env bash
# B-005 (2026-10-28): `cyber-gpu` is private and the scheduled OS session cannot clone it. The command centre keeps a
# git bundle of it in the hub (integrare/cyber-gpu/snapshot/cyber-gpu.bundle, refreshed by Cyber-GPU, command C-34).
# This script turns that bundle into a local bare mirror and tells git (and cargo, through the git CLI) to fetch
# https://github.com/Adriancu91/cyber-gpu from it. Nothing in Cargo.toml changes; on a machine with access the
# script is not needed.
#   tools/cyber-gpu-local.sh [path-to-hub]     (default: ../cyber-hub next to this repo)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HUB="${1:-$ROOT/../cyber-hub}"
BUNDLE="$HUB/integrare/cyber-gpu/snapshot/cyber-gpu.bundle"
MIRROR="${CYBER_GPU_MIRROR:-$HOME/.cache/cyber-gpu-mirror.git}"
URL="https://github.com/Adriancu91/cyber-gpu"

if [[ ! -f "$BUNDLE" ]]; then
    echo "Lipseste copia cyber-gpu din hub: $BUNDLE" >&2
    exit 1
fi
git bundle verify -q "$BUNDLE" >/dev/null 2>&1
if [[ -d "$MIRROR" ]]; then
    git -C "$MIRROR" fetch -q --force "$BUNDLE" 'refs/heads/*:refs/heads/*'
else
    mkdir -p "$(dirname "$MIRROR")"
    git clone -q --bare "$BUNDLE" "$MIRROR"
fi
git -C "$MIRROR" symbolic-ref HEAD refs/heads/g0-g2-fundatie   # cargo fetches HEAD for a short rev
# Every fetch of the GitHub URL (with or without .git) goes to the mirror.
git config --global --unset-all "url.file://$MIRROR.insteadOf" 2>/dev/null || true
git config --global --add "url.file://$MIRROR.insteadOf" "$URL"
git config --global --add "url.file://$MIRROR.insteadOf" "$URL.git"
mkdir -p "$HOME/.cargo"
if ! grep -q 'git-fetch-with-cli' "$HOME/.cargo/config.toml" 2>/dev/null; then
    printf '\n[net]\ngit-fetch-with-cli = true\n' >> "$HOME/.cargo/config.toml"
fi
echo "cyber-gpu: copie locala din hub ($(cat "$HUB/integrare/cyber-gpu/snapshot/VERSIUNE.txt" 2>/dev/null || echo '?')) -> $MIRROR"
