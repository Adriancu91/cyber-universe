#!/usr/bin/env bash
# Makes a module package (.cum = ustar archive) from a folder with module.toml (docs/specs/MODULE.md).
#   tools/mkmodule.sh <module folder> <folder with built user programs> <output.cum>
# If module.toml has `program_source = "x"`, the built program x is put at the path named by `start`.
set -euo pipefail
SRC="$1"; BIN="$2"; OUT="$3"
[[ -f "$SRC/module.toml" ]] || { echo "EROARE: $SRC/module.toml lipsește" >&2; exit 1; }
value() { sed -n "s/^$1 *= *\"\\(.*\\)\" *$/\\1/p" "$SRC/module.toml" | head -1; }
STAGE="$(mktemp -d)"; trap 'rm -rf "$STAGE"' EXIT
cp -r "$SRC/." "$STAGE/"
program="$(value program_source)"; start="$(value start)"
if [[ -n "$program" ]]; then
    [[ -n "$start" ]] || { echo "EROARE: program_source fără start în $SRC/module.toml" >&2; exit 1; }
    mkdir -p "$STAGE/$(dirname "$start")"
    cp "$BIN/$program" "$STAGE/$start"
fi
( cd "$STAGE" && tar --format=ustar --owner=0 --group=0 --sort=name -cf "$OUT" module.toml $(ls | grep -v '^module.toml$') )
