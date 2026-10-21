#!/usr/bin/env bash
# Started by tools/test.sh before the `claude-cli` scenario: the imitation of the Claude API (HTTPS) on 127.0.0.1:18444,
# reached by the guest as 10.0.2.2:18444.
exec python3 "$(dirname "$0")/claude-test-server.py"
