#!/usr/bin/env bash
# Started by tools/test.sh before the `update` scenario (P6-08): the imitation of GitHub — API on 127.0.0.1:18445 and file
# storage on 127.0.0.1:18446 (HTTPS), reached by the guest as 10.0.2.2.
exec python3 "$(dirname "$0")/update-test-server.py"
