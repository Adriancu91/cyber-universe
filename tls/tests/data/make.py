# Test certificates for tls/tests (P5-06), made with OpenSSL through Python `cryptography`. TEST ONLY: the private keys
# are public on purpose. Valid 2026-01-01 .. 2036-01-01 (expired.pem: 2020-2021).
import datetime as dt, os
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa, ec, padding
D = os.path.dirname(os.path.abspath(__file__)) + '/'
def name(cn): return x509.Name([x509.NameAttribute(NameOID.ORGANIZATION_NAME, 'Cyber Universe (test)'), x509.NameAttribute(NameOID.COMMON_NAME, cn)])
T0, T1 = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc), dt.datetime(2036, 1, 1, tzinfo=dt.timezone.utc)
def cert(subj, key, issuer, ikey, ca=False, san=None, nb=T0, na=T1):
    b = (x509.CertificateBuilder().subject_name(name(subj)).issuer_name(name(issuer)).public_key(key.public_key())
         .serial_number(x509.random_serial_number()).not_valid_before(nb).not_valid_after(na)
         .add_extension(x509.BasicConstraints(ca=ca, path_length=None), critical=True))
    if san: b = b.add_extension(x509.SubjectAlternativeName([x509.DNSName(s) for s in san]), critical=False)
    return b.sign(ikey, hashes.SHA256())
def pem(c): return c.public_bytes(serialization.Encoding.PEM)
def keypem(k): return k.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
root = ec.generate_private_key(ec.SECP256R1())
rc = cert('Cyber TLS Test Root', root, 'Cyber TLS Test Root', root, ca=True)
open(D + 'root.der', 'wb').write(rc.public_bytes(serialization.Encoding.DER))
other = ec.generate_private_key(ec.SECP256R1())
open(D + 'other_root.der', 'wb').write(cert('Alta radacina', other, 'Alta radacina', other, ca=True).public_bytes(serialization.Encoding.DER))
for fname, key, san, nb, na in [
    ('ec', ec.generate_private_key(ec.SECP256R1()), ['test.cyber.local'], T0, T1),
    ('rsa', rsa.generate_private_key(public_exponent=65537, key_size=2048), ['test.cyber.local'], T0, T1),
    ('expired', ec.generate_private_key(ec.SECP256R1()), ['test.cyber.local'], dt.datetime(2020, 1, 1, tzinfo=dt.timezone.utc), dt.datetime(2021, 1, 1, tzinfo=dt.timezone.utc)),
]:
    c = cert(fname + '.cyber.local', key, 'Cyber TLS Test Root', root, san=san, nb=nb, na=na)
    open(D + fname + '.pem', 'wb').write(pem(c) + pem(rc))  # chain: server, then root
    open(D + fname + '.key', 'wb').write(keypem(key))
