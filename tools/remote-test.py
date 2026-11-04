#!/usr/bin/env python3
"""The harness side of the `remote` scenario (P6-49): talks to the OS through QEMU's port forward
(127.0.0.1:12323 -> guest 2323) with tools/remote-client.py and writes what it saw to a log.
Started by tools/before-remote.sh; tools/after-remote.sh checks the log.
  remote-test.py <key file> <log file>
"""
import importlib.util
import sys
sys.dont_write_bytecode = True  # no __pycache__ next to the tools
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("remote_client", os.path.join(HERE, "remote-client.py"))
rc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rc)

HOST, PORT = "127.0.0.1", 12323


def main():
    key_file, log_file = sys.argv[1], sys.argv[2]
    with open(key_file, "rb") as f:
        key = f.read().strip()
    log = open(log_file, "w", buffering=1)
    problems = []

    def check(ok, what):
        log.write(("OK      " if ok else "ESUAT   ") + what + "\n")
        if not ok:
            problems.append(what)

    def attempt(k, wait=0.0):
        try:
            sock, first = rc.connect(HOST, PORT, k, wait)
            return 0, sock, first
        except rc.Refused as e:
            return e.code, None, str(e)

    # 1. Three wrong keys (the first attempt also waits for the OS to boot and start the service).
    for i in range(3):
        code, _, text = attempt(b"cheie-gresita-" + str(i).encode() * 8, wait=45 if i == 0 else 5)
        check(code == 2 and "GRESIT" in text, f"cheie gresita {i + 1} refuzata (cod {code}: {text})")
    # 2. Now even the right key is refused: the pause.
    code, _, text = attempt(key, wait=5)
    check(code == 3 and "prea multe chei gresite" in text, f"pauza dupa 3 chei gresite (cod {code}: {text})")
    time.sleep(3.5)
    # 3. The right key: allowed commands answer, the others are refused, the key is never shown.
    code, sock, first = attempt(key, wait=5)
    check(code == 0, f"cheia buna acceptata (cod {code})")
    if code == 0:
        text = rc.run_commands(sock, first.split(b"\n", 1)[1],
                               ["uptime", "help", "gpu i6 --scrie", "reboot", "cat /disk/remote.cheie", "remote opreste"])
        log.write("---- conversatia ----\n" + text + "\n---- sfarsit ----\n")
        check("pornit de" in text, "uptime a raspuns")
        check("comenzi:" in text, "help a raspuns")
        check("REFUZAT: scrierile in placa video" in text, "gpu i6 --scrie refuzat")
        check("REFUZAT: repornirea" in text, "reboot refuzat")
        check("fisier secret" in text, "cat pe cheie: fisier secret")
        check(key.decode() not in text, "cheia nu apare in conversatie")
        check("serviciul a fost oprit" in text, "remote opreste a oprit serviciul")
    log.write("CLIENT: TOATE VERIFICARILE AU TRECUT\n" if not problems else f"CLIENT: {len(problems)} verificari esuate\n")
    log.close()


if __name__ == "__main__":
    main()
