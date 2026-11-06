#!/usr/bin/env bash
# Started by tools/test.sh before the `ora` scenario (P6-61): the SNTP test server on 127.0.0.1:18123.
exec python3 "$(dirname "$0")/ora-test-server.py"
