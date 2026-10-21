#!/usr/bin/env bash
# Downloads the latest release of the OS (P6-07) the way the update command (P6-08) will — through the GitHub REST API —
# and checks every file against SHA256SUMS. The repository is private, so a token with read access is needed
# (GH_TOKEN or GITHUB_TOKEN). Exit code 0 = all good.
#   GH_TOKEN=... tools/check-release.sh [owner/repo]
set -euo pipefail
REPO="${1:-Adriancu91/cyber-universe}"
TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
AUTH=(); [[ -n "$TOKEN" ]] && AUTH=(-H "Authorization: Bearer $TOKEN")
DIR="$(mktemp -d)"
trap 'rm -rf "$DIR"' EXIT
curl -fsSL "${AUTH[@]}" -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases/latest" > "$DIR/release.json"
python3 - "$DIR/release.json" > "$DIR/assets.txt" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
print(r["tag_name"], file=sys.stderr)
for a in r["assets"]:
    print(a["name"], a["url"])
PY
while read -r name url; do
    curl -fsSL "${AUTH[@]}" -H "Accept: application/octet-stream" -o "$DIR/$name" "$url"
done < "$DIR/assets.txt"
( cd "$DIR" && sha256sum -c SHA256SUMS )
cat "$DIR/VERSIUNE"
