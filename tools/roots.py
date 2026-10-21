#!/usr/bin/env python3
"""Extracts the trusted web roots from Mozilla's public list (NSS certdata.txt, MPL 2.0, used only as data)
into boot/certs/ as DER files named after their subject: <first 8 bytes of SHA-256(subject), hex>.<n>.

Only roots with CKA_TRUST_SERVER_AUTH = CKT_NSS_TRUSTED_DELEGATOR are kept; a root with a "distrust after" date
for servers (CKA_NSS_SERVER_DISTRUST_AFTER) is left out, because we cannot apply that rule.

    tools/roots.py <certdata.txt> [revision]      (the file comes from github.com/nss-dev/nss, lib/ckfw/builtins/)
"""
import datetime
import hashlib
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "boot", "certs")


def objects(path):
    """Yields every object of certdata.txt as {attribute: value}; octal values become bytes."""
    obj, lines = {}, open(path, encoding="utf-8").read().splitlines()
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        i += 1
        if not line or line.startswith("#"):
            if not line and obj:
                yield obj
                obj = {}
            continue
        if line.startswith("BEGINDATA"):
            continue
        parts = line.split(" ", 2)
        name = parts[0]
        if len(parts) >= 2 and parts[1] == "MULTILINE_OCTAL":
            data = bytearray()
            while lines[i].strip() != "END":
                for o in lines[i].strip().split("\\")[1:]:
                    data.append(int(o, 8))
                i += 1
            i += 1
            obj[name] = bytes(data)
        else:
            obj[name] = line[len(name) + 1:]
    if obj:
        yield obj


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    source = sys.argv[1]
    revision = sys.argv[2] if len(sys.argv) > 2 else "?"
    certs, trust = {}, {}
    for o in objects(source):
        cls = o.get("CKA_CLASS")
        if cls == "CK_OBJECT_CLASS CKO_CERTIFICATE":
            certs[(o["CKA_ISSUER"], o["CKA_SERIAL_NUMBER"])] = o
        elif cls == "CK_OBJECT_CLASS CKO_NSS_TRUST":
            trust[(o["CKA_ISSUER"], o["CKA_SERIAL_NUMBER"])] = o.get("CKA_TRUST_SERVER_AUTH", "")
    kept, skipped = [], []
    for key, c in certs.items():
        label = c.get("CKA_LABEL", "?").removeprefix("UTF8 ").strip('"')
        if trust.get(key) != "CK_TRUST CKT_NSS_TRUSTED_DELEGATOR":
            continue
        if isinstance(c.get("CKA_NSS_SERVER_DISTRUST_AFTER"), bytes):
            skipped.append(label)
            continue
        kept.append((label, c["CKA_SUBJECT"], c["CKA_VALUE"]))
    if os.path.isdir(OUT):
        for f in os.listdir(OUT):
            if f != "SURSA.md":
                os.remove(os.path.join(OUT, f))
    os.makedirs(OUT, exist_ok=True)
    names, total = [], 0
    for label, subject, der in sorted(kept, key=lambda k: k[0]):
        h = hashlib.sha256(subject).hexdigest()[:16]
        n = 0
        while os.path.exists(os.path.join(OUT, f"{h}.{n}")):
            n += 1
        with open(os.path.join(OUT, f"{h}.{n}"), "wb") as f:
            f.write(der)
        names.append(f"| `{h}.{n}` | {label} |")
        total += len(der)
    sha = hashlib.sha256(open(source, "rb").read()).hexdigest()
    today = datetime.date.today().isoformat()
    with open(os.path.join(OUT, "SURSA.md"), "w", encoding="utf-8") as f:
        f.write(f"""# Rădăcinile de încredere pentru site-uri (HTTPS)

Generate de `tools/roots.py` — **nu edita de mână**. Pentru o listă nouă: descarcă `certdata.txt` și rulează scriptul.

- **Sursa:** lista publică Mozilla (Mozilla CA Certificate Program), fișierul `lib/ckfw/builtins/certdata.txt` din NSS
  (oglinda oficială `github.com/nss-dev/nss`, revizia `{revision}`), descărcat pe {today}.
- **SHA-256 al fișierului sursă:** `{sha}`
- **Licența sursei:** Mozilla Public License 2.0 (https://mozilla.org/MPL/2.0/). Folosim lista doar ca date
  (certificatele publice ale autorităților), fără cod din NSS.
- **Ce s-a păstrat:** rădăcinile cu `CKA_TRUST_SERVER_AUTH = CKT_NSS_TRUSTED_DELEGATOR` (de încredere pentru site-uri):
  {len(kept)} certificate, {total} octeți. Lăsate deoparte (au o dată „neîncredere după” pentru servere): {len(skipped)}
  ({", ".join(skipped) if skipped else "—"}).
- **Numele fișierelor:** primii 8 octeți din SHA-256 al numelui (subject, DER), în hexazecimal, plus `.0`, `.1` …
  dacă două rădăcini au același nume. `fetch` deschide doar fișierul emitentului căutat.

| Fișier | Rădăcina |
|---|---|
""")
        f.write("\n".join(names) + "\n")
    print(f"roots: {len(kept)} radacini ({total} octeti) in boot/certs/, {len(skipped)} lasate deoparte")
    return 0


if __name__ == "__main__":
    sys.exit(main())
