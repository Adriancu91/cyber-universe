#!/usr/bin/env bash
# Started by tools/test.sh before the `tcp` scenario: a web server on 127.0.0.1:18080 (the guest reaches it
# as 10.0.2.2:18080 through QEMU's user network) serving a small text and a 200 000-byte file (byte i = i % 251).
dir="$(mktemp -d)"
printf 'Salut de la serverul de test TCP\n' > "$dir/mic.txt"
python3 -c "import sys; sys.stdout.buffer.write(bytes(i % 251 for i in range(200000)))" > "$dir/mare.bin"
exec python3 -m http.server 18080 --bind 127.0.0.1 --directory "$dir" 2>/dev/null
