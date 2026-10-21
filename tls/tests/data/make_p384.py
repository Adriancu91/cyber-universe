# P6-02: a P-384 chain like the ones real sites use (root P-384 -> intermediate P-384 -> server P-384, ECDSA-SHA384),
# made with OpenSSL through Python `cryptography`. TEST ONLY: the private keys are public on purpose.
# Separate from make.py so the existing test root (also used by the kernel's `https` scenario) stays the same.
import datetime as dt, os
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
D = os.path.dirname(os.path.abspath(__file__)) + '/'
def name(cn): return x509.Name([x509.NameAttribute(NameOID.ORGANIZATION_NAME, 'Cyber Universe (test)'), x509.NameAttribute(NameOID.COMMON_NAME, cn)])
T0, T1 = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc), dt.datetime(2036, 1, 1, tzinfo=dt.timezone.utc)
def cert(subj, key, issuer, ikey, ca, san=None):
    b = (x509.CertificateBuilder().subject_name(name(subj)).issuer_name(name(issuer)).public_key(key.public_key())
         .serial_number(x509.random_serial_number()).not_valid_before(T0).not_valid_after(T1)
         .add_extension(x509.BasicConstraints(ca=ca, path_length=None), critical=True))
    if san: b = b.add_extension(x509.SubjectAlternativeName([x509.DNSName(s) for s in san]), critical=False)
    return b.sign(ikey, hashes.SHA384())
pem = lambda c: c.public_bytes(serialization.Encoding.PEM)
root, inter, leaf = (ec.generate_private_key(ec.SECP384R1()) for _ in range(3))
rc = cert('Cyber TLS Test Root P-384', root, 'Cyber TLS Test Root P-384', root, True)
ic = cert('Cyber TLS Test E1', inter, 'Cyber TLS Test Root P-384', root, True)
lc = cert('p384.cyber.local', leaf, 'Cyber TLS Test E1', inter, False, ['test.cyber.local'])
open(D + 'root384.der', 'wb').write(rc.public_bytes(serialization.Encoding.DER))
open(D + 'p384.pem', 'wb').write(pem(lc) + pem(ic))  # chain: server, intermediate (the root is not sent, like real sites)
open(D + 'p384.key', 'wb').write(leaf.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
