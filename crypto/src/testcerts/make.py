import datetime as dt
from cryptography import x509
from cryptography.x509.oid import NameOID, ObjectIdentifier
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa, ec, padding
import os
OUT=os.path.dirname(os.path.abspath(__file__))+'/'
def name(cn): return x509.Name([x509.NameAttribute(NameOID.COUNTRY_NAME,'RO'),x509.NameAttribute(NameOID.ORGANIZATION_NAME,'Cyber Universe (test)'),x509.NameAttribute(NameOID.COMMON_NAME,cn)])
T0=dt.datetime(2026,1,1,tzinfo=dt.timezone.utc); T1=dt.datetime(2036,1,1,tzinfo=dt.timezone.utc)
def cert(subj, key, issuer, ikey, ca=False, pathlen=None, san=None, h=hashes.SHA256(), pss=False, nb=T0, na=T1, extra=None):
    b=(x509.CertificateBuilder().subject_name(name(subj)).issuer_name(name(issuer)).public_key(key.public_key())
       .serial_number(x509.random_serial_number()).not_valid_before(nb).not_valid_after(na))
    b=b.add_extension(x509.BasicConstraints(ca=ca,path_length=pathlen),critical=True)
    b=b.add_extension(x509.KeyUsage(digital_signature=not ca,content_commitment=False,key_encipherment=False,data_encipherment=False,key_agreement=False,key_cert_sign=ca,crl_sign=ca,encipher_only=False,decipher_only=False),critical=True)
    if san: b=b.add_extension(x509.SubjectAlternativeName([x509.DNSName(s) for s in san]),critical=False)
    b=b.add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()),critical=False)
    if extra: b=b.add_extension(x509.UnrecognizedExtension(ObjectIdentifier('1.3.6.1.4.1.99999.1'),b'\x05\x00'),critical=True)
    kw={}
    if pss: kw['rsa_padding']=padding.PSS(mgf=padding.MGF1(h),salt_length=padding.PSS.DIGEST_LENGTH)
    return b.sign(ikey,h,**kw)
def save(n,c): open(OUT+n+'.der','wb').write(c.public_bytes(serialization.Encoding.DER))
rootA=rsa.generate_private_key(public_exponent=65537,key_size=2048)
inter=ec.generate_private_key(ec.SECP256R1()); leafk=ec.generate_private_key(ec.SECP256R1())
save('root_a',cert('Cyber Test Root A',rootA,'Cyber Test Root A',rootA,ca=True))
save('inter_a',cert('Cyber Test Inter A',inter,'Cyber Test Root A',rootA,ca=True,pathlen=0,pss=True))
save('leaf_a',cert('test.cyber.local',leafk,'Cyber Test Inter A',inter,san=['test.cyber.local','*.exemplu.ro']))
save('expired_a',cert('test.cyber.local',leafk,'Cyber Test Inter A',inter,san=['test.cyber.local'],nb=dt.datetime(2020,1,1,tzinfo=dt.timezone.utc),na=dt.datetime(2021,1,1,tzinfo=dt.timezone.utc)))
save('critext_a',cert('test.cyber.local',leafk,'Cyber Test Inter A',inter,san=['test.cyber.local'],extra=True))
k2=ec.generate_private_key(ec.SECP256R1())
save('under_leaf',cert('rau.cyber.local',k2,'test.cyber.local',leafk,san=['rau.cyber.local']))
inter2=ec.generate_private_key(ec.SECP256R1())
save('inter2_a',cert('Cyber Test Inter A2',inter2,'Cyber Test Inter A',inter,ca=True,pathlen=0))
save('deep_leaf',cert('adanc.cyber.local',k2,'Cyber Test Inter A2',inter2,san=['adanc.cyber.local']))
rootB=ec.generate_private_key(ec.SECP256R1()); leafB=rsa.generate_private_key(public_exponent=65537,key_size=2048)
save('root_b',cert('Cyber Test Root B',rootB,'Cyber Test Root B',rootB,ca=True,h=hashes.SHA384()))
save('leaf_b',cert('server.b',leafB,'Cyber Test Root B',rootB,san=['server.b'],h=hashes.SHA384()))
print('ok')
