#!/usr/bin/env bash
# Started by tools/test.sh before the `ghpush` scenario (P6-09): the imitation of the GitHub contents API on 127.0.0.1:18447.
exec python3 "$(dirname "$0")/ghpush-test-server.py"
