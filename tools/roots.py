#!/usr/bin/env python3
"""P6-02: the real trust roots, as DER files for /etc/certs (boot/initrd/etc/certs/).

Source: Mozilla's list of root certificates trusted for websites (Mozilla CA Certificate Program, published through
CCADB). The list is used only as data (MPL 2.0 allows that); the certificates themselves are public.
  tools/roots.py                 download the current list and rewrite boot/initrd/etc/certs/
  tools/roots.py mozilla.pem     use a file downloaded before
Writes boot/CERTS-SURSA.md (date, count, the name of every file). Run by hand when the list should be refreshed;
the build itself never needs the internet.
"""
import datetime, os, re, sys, urllib.request
from cryptography import x509
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa

URL = "https://ccadb.my.salesforce-sites.com/mozilla/IncludedRootsPEMTxt?TrustBitsInclude=Websites"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "boot/initrd/etc/certs")

data = open(sys.argv[1], "rb").read() if len(sys.argv) > 1 else urllib.request.urlopen(URL, timeout=60).read()
certs = x509.load_pem_x509_certificates(data)
os.makedirs(OUT, exist_ok=True)
for f in os.listdir(OUT):
    if f.endswith(".der"):
        os.remove(os.path.join(OUT, f))

def kind(c):
    k = c.public_key()
    if isinstance(k, rsa.RSAPublicKey):
        return f"RSA {k.key_size}"
    if isinstance(k, ec.EllipticCurvePublicKey):
        return {"secp256r1": "P-256", "secp384r1": "P-384"}.get(k.curve.name, k.curve.name + " (nu e suportată)")
    return type(k).__name__

rows = []
for i, c in enumerate(sorted(certs, key=lambda c: c.subject.rfc4514_string())):
    cn = c.subject.get_attributes_for_oid(x509.NameOID.COMMON_NAME)
    label = cn[0].value if cn else c.subject.rfc4514_string()
    slug = re.sub(r"[^a-z0-9]+", "-", label.lower()).strip("-")[:20].strip("-")
    fname = f"{i + 1:03d}-{slug}.der"
    open(os.path.join(OUT, fname), "wb").write(c.public_bytes(serialization.Encoding.DER))
    rows.append(f"| `{fname}` | {label} | {kind(c)} | {c.not_valid_after_utc.date()} |")

today = datetime.date.today().isoformat()
open(os.path.join(ROOT, "boot/CERTS-SURSA.md"), "w").write(f"""# Rădăcinile de încredere din `/etc/certs` (P6-02)

- **Sursa:** lista Mozilla a rădăcinilor de încredere pentru site-uri (Mozilla CA Certificate Program, publicată prin CCADB:
  `{URL}`). Folosită doar ca date (licența MPL 2.0 permite); certificatele sunt publice.
- **Descărcată:** {today} · **{len(rows)} rădăcini** · generată de `tools/roots.py` (se rulează de mână când vrem lista nouă).
- Rădăcinile cu chei pe care nu le suportăm (de ex. P-521) sunt păstrate, dar sărite la verificare.

| Fișier | Nume | Cheie | Expiră |
|---|---|---|---|
""" + "\n".join(rows) + "\n")
print(f"{len(rows)} radacini in {OUT}")
