#!/usr/bin/env bash
# Started by tools/test.sh before the `agent` scenario: the imitation of the Claude API (HTTPS) on 127.0.0.1:18444 and,
# for `agent veghe` (P6-204), the imitation of GitHub on 127.0.0.1:18447 (guest: 10.0.2.2:18444 / :18447).
# Both stop when test.sh stops this script.
D="$(dirname "$0")"
python3 "$D/claude-test-server.py" & a=$!
python3 "$D/ghpush-test-server.py" & b=$!
trap 'kill $a $b 2>/dev/null; exit 0' TERM INT
wait
