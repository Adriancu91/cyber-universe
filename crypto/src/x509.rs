//! X.509 certificates (RFC 5280): reading what a TLS client needs, and verifying a chain from the server's
//! certificate up to a trusted root. Only the common forms are accepted; anything unexpected is refused.

use crate::der::{self, Der, ctx};
use crate::sha2::{Sha256, Sha384, Sha512};
use crate::{p256, p384, rsa};

// Object identifiers (contents of the OID element).
const RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const RSA_PSS: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a];
const SHA256_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const SHA384_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c];
const SHA512_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d];
const MGF1: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x08];
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const SECP384R1: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const ECDSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03];
const ECDSA_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x04];
const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const OID_SHA384: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02];
const OID_SHA512: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03];
const EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const EXT_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const EXT_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const EXT_EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const EXT_SKI: &[u8] = &[0x55, 0x1d, 0x0e];
const EXT_AKI: &[u8] = &[0x55, 0x1d, 0x23];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hash {
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_oid(oid: &[u8]) -> Option<Hash> {
        match oid {
            OID_SHA256 => Some(Hash::Sha256),
            OID_SHA384 => Some(Hash::Sha384),
            OID_SHA512 => Some(Hash::Sha512),
            _ => None,
        }
    }

    fn len(self) -> usize {
        match self {
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
            Hash::Sha512 => 64,
        }
    }
}

/// How a certificate (or a TLS handshake message) is signed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SigAlg {
    RsaPkcs1(Hash),
    RsaPss(Hash, usize), // hash, salt length
    Ecdsa(Hash),
    /// An algorithm we cannot check (e.g. SHA-1 on old roots): the certificate can be read, but nothing it
    /// signed is ever accepted.
    Unknown,
}

#[derive(Clone, Copy)]
pub enum Key<'a> {
    Rsa { n: &'a [u8], e: &'a [u8] },
    P256(&'a [u8]), // 04 || x || y
    P384(&'a [u8]), // 04 || x || y
    /// A key type we cannot use (e.g. P-521): signatures made with it are refused.
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Error {
    /// The encoding could not be read (or uses a form we do not accept).
    Format,
    /// An algorithm or key type we do not support.
    Unsupported,
    /// A critical extension we do not understand.
    CriticalExtension,
    BadSignature,
    Expired,
    NotYetValid,
    /// An issuer that is not a certificate authority, or may not sign certificates.
    NotCa,
    PathTooLong,
    UnknownIssuer,
    NameMismatch,
    EmptyChain,
}

#[derive(Clone, Copy)]
pub struct Cert<'a> {
    /// The signed part, exactly as encoded.
    pub tbs: &'a [u8],
    pub sig_alg: SigAlg,
    pub signature: &'a [u8],
    /// Issuer and subject names as encoded (compared byte for byte).
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    /// Validity as YYYYMMDDHHMMSS numbers (UTC).
    pub not_before: u64,
    pub not_after: u64,
    pub key: Key<'a>,
    pub is_ca: bool,
    pub path_len: Option<u8>,
    /// keyUsage bits (bit 0 = digitalSignature … bit 5 = keyCertSign), if the extension is present.
    pub key_usage: Option<u16>,
    /// subjectAltName contents (a SEQUENCE body of GeneralNames), if present.
    pub san: Option<&'a [u8]>,
}

fn parse_alg(t: der::Tlv) -> Result<SigAlg, Error> {
    let mut d = Der::new(t.body);
    let oid = d.expect(der::OID).ok_or(Error::Format)?.body;
    // Parameters: absent or NULL (RSA); absent (ECDSA).
    let no_params = |d: &mut Der, a| match d.next() {
        None => Ok(a),
        Some(p) if p.tag == der::NULL && p.body.is_empty() && d.is_empty() => Ok(a),
        _ => Err(Error::Format),
    };
    match oid {
        SHA256_RSA => no_params(&mut d, SigAlg::RsaPkcs1(Hash::Sha256)),
        SHA384_RSA => no_params(&mut d, SigAlg::RsaPkcs1(Hash::Sha384)),
        SHA512_RSA => no_params(&mut d, SigAlg::RsaPkcs1(Hash::Sha512)),
        ECDSA_SHA256 => no_params(&mut d, SigAlg::Ecdsa(Hash::Sha256)),
        ECDSA_SHA384 => no_params(&mut d, SigAlg::Ecdsa(Hash::Sha384)),
        ECDSA_SHA512 => no_params(&mut d, SigAlg::Ecdsa(Hash::Sha512)),
        RSA_PSS => parse_pss(d.expect(der::SEQUENCE).ok_or(Error::Format)?.body),
        _ => Ok(SigAlg::Unknown),
    }
}

/// RSASSA-PSS-params (RFC 4055): we require the hash given explicitly, MGF1 over the same hash, salt length given.
fn parse_pss(body: &[u8]) -> Result<SigAlg, Error> {
    let mut d = Der::new(body);
    let hash_id = d.expect(ctx(0)).ok_or(Error::Format)?;
    let hash_oid = Der::new(Der::new(hash_id.body).expect(der::SEQUENCE).ok_or(Error::Format)?.body)
        .expect(der::OID)
        .ok_or(Error::Format)?
        .body;
    let hash = Hash::from_oid(hash_oid).ok_or(Error::Unsupported)?;
    let mgf = d.expect(ctx(1)).ok_or(Error::Format)?;
    let mut m = Der::new(Der::new(mgf.body).expect(der::SEQUENCE).ok_or(Error::Format)?.body);
    if m.expect(der::OID).ok_or(Error::Format)?.body != MGF1 {
        return Err(Error::Unsupported);
    }
    let mgf_hash = Der::new(m.expect(der::SEQUENCE).ok_or(Error::Format)?.body).expect(der::OID).ok_or(Error::Format)?.body;
    if Hash::from_oid(mgf_hash) != Some(hash) {
        return Err(Error::Unsupported);
    }
    let salt = der::uint(Der::new(d.expect(ctx(2)).ok_or(Error::Format)?.body).expect(der::INTEGER).ok_or(Error::Format)?.body)
        .ok_or(Error::Format)?;
    if salt.len() > 1 {
        return Err(Error::Unsupported);
    }
    // trailerField [3], if present, must be 1.
    if let Some(t) = d.optional(ctx(3)) {
        if Der::new(t.body).expect(der::INTEGER).map(|i| i.body) != Some(&[1]) {
            return Err(Error::Format);
        }
    }
    if !d.is_empty() {
        return Err(Error::Format);
    }
    Ok(SigAlg::RsaPss(hash, salt[0] as usize))
}

fn digits(b: &[u8]) -> Option<u64> {
    b.iter().try_fold(0u64, |a, &c| c.is_ascii_digit().then(|| a * 10 + u64::from(c - b'0')))
}

/// UTCTime YYMMDDHHMMSSZ (years 1950–2049) or GeneralizedTime YYYYMMDDHHMMSSZ -> YYYYMMDDHHMMSS.
fn parse_time(t: der::Tlv) -> Result<u64, Error> {
    let b = t.body;
    let v = match (t.tag, b.len()) {
        (der::UTC_TIME, 13) if b[12] == b'Z' => {
            let v = digits(&b[..12]).ok_or(Error::Format)?;
            let yy = v / 10_000_000_000;
            v + if yy < 50 { 20_000_000_000_000 } else { 19_000_000_000_000 }
        }
        (der::GENERALIZED_TIME, 15) if b[14] == b'Z' => digits(&b[..14]).ok_or(Error::Format)?,
        _ => return Err(Error::Format),
    };
    Ok(v)
}

fn parse_key(spki: &[u8]) -> Result<Key<'_>, Error> {
    let mut d = Der::new(spki);
    let mut alg = Der::new(d.expect(der::SEQUENCE).ok_or(Error::Format)?.body);
    let oid = alg.expect(der::OID).ok_or(Error::Format)?.body;
    let key = der::bits(d.expect(der::BIT_STRING).ok_or(Error::Format)?.body).ok_or(Error::Format)?;
    match oid {
        RSA_ENCRYPTION => {
            let mut k = Der::new(Der::new(key).expect(der::SEQUENCE).ok_or(Error::Format)?.body);
            let n = der::uint(k.expect(der::INTEGER).ok_or(Error::Format)?.body).ok_or(Error::Format)?;
            let e = der::uint(k.expect(der::INTEGER).ok_or(Error::Format)?.body).ok_or(Error::Format)?;
            Ok(Key::Rsa { n, e })
        }
        EC_PUBLIC_KEY => match alg.expect(der::OID).map(|o| o.body) {
            Some(PRIME256V1) => Ok(Key::P256(key)),
            Some(SECP384R1) => Ok(Key::P384(key)),
            _ => Ok(Key::Unknown),
        },
        _ => Ok(Key::Unknown),
    }
}

impl<'a> Cert<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Cert<'a>, Error> {
        let mut outer = Der::new(data);
        let cert = outer.expect(der::SEQUENCE).ok_or(Error::Format)?;
        if !outer.is_empty() {
            return Err(Error::Format);
        }
        let mut c = Der::new(cert.body);
        let tbs = c.expect(der::SEQUENCE).ok_or(Error::Format)?;
        let sig_alg = parse_alg(c.expect(der::SEQUENCE).ok_or(Error::Format)?)?;
        let signature = der::bits(c.expect(der::BIT_STRING).ok_or(Error::Format)?.body).ok_or(Error::Format)?;
        if !c.is_empty() {
            return Err(Error::Format);
        }

        let mut t = Der::new(tbs.body);
        // Only version 3 certificates ([0] INTEGER 2).
        let v = t.expect(ctx(0)).ok_or(Error::Format)?;
        if Der::new(v.body).expect(der::INTEGER).map(|i| i.body) != Some(&[2]) {
            return Err(Error::Unsupported);
        }
        t.expect(der::INTEGER).ok_or(Error::Format)?; // serial number
        let inner_alg = parse_alg(t.expect(der::SEQUENCE).ok_or(Error::Format)?)?;
        if inner_alg != sig_alg {
            return Err(Error::Format);
        }
        let issuer = t.expect(der::SEQUENCE).ok_or(Error::Format)?.raw;
        let mut validity = Der::new(t.expect(der::SEQUENCE).ok_or(Error::Format)?.body);
        let not_before = parse_time(validity.next().ok_or(Error::Format)?)?;
        let not_after = parse_time(validity.next().ok_or(Error::Format)?)?;
        let subject = t.expect(der::SEQUENCE).ok_or(Error::Format)?.raw;
        let key = parse_key(t.expect(der::SEQUENCE).ok_or(Error::Format)?.body)?;
        t.optional(0x81); // issuerUniqueID
        t.optional(0x82); // subjectUniqueID

        let mut cert = Cert {
            tbs: tbs.raw,
            sig_alg,
            signature,
            issuer,
            subject,
            not_before,
            not_after,
            key,
            is_ca: false,
            path_len: None,
            key_usage: None,
            san: None,
        };
        if let Some(ext) = t.optional(ctx(3)) {
            let mut list = Der::new(Der::new(ext.body).expect(der::SEQUENCE).ok_or(Error::Format)?.body);
            while !list.is_empty() {
                cert.extension(list.expect(der::SEQUENCE).ok_or(Error::Format)?.body)?;
            }
        }
        if !t.is_empty() {
            return Err(Error::Format);
        }
        Ok(cert)
    }

    fn extension(&mut self, body: &'a [u8]) -> Result<(), Error> {
        let mut e = Der::new(body);
        let oid = e.expect(der::OID).ok_or(Error::Format)?.body;
        let critical = match e.optional(der::BOOLEAN) {
            Some(b) if b.body == [0xff] => true,
            Some(_) => return Err(Error::Format), // DER: FALSE is the default and must be left out
            None => false,
        };
        let value = e.expect(der::OCTET_STRING).ok_or(Error::Format)?.body;
        if !e.is_empty() {
            return Err(Error::Format);
        }
        let mut v = Der::new(value);
        match oid {
            EXT_BASIC_CONSTRAINTS => {
                let mut s = Der::new(v.expect(der::SEQUENCE).ok_or(Error::Format)?.body);
                if let Some(b) = s.optional(der::BOOLEAN) {
                    self.is_ca = b.body == [0xff];
                    if !self.is_ca {
                        return Err(Error::Format);
                    }
                }
                if let Some(n) = s.optional(der::INTEGER) {
                    match der::uint(n.body) {
                        Some(&[x]) => self.path_len = Some(x),
                        _ => return Err(Error::Format),
                    }
                }
            }
            EXT_KEY_USAGE => {
                let b = v.expect(der::BIT_STRING).ok_or(Error::Format)?.body;
                // First byte = unused bits; bit 0 is the most significant bit of the next byte.
                let (b1, b2) = (b.get(1).copied().unwrap_or(0), b.get(2).copied().unwrap_or(0));
                self.key_usage = Some(u16::from(b1.reverse_bits()) | u16::from(b2.reverse_bits()) << 8);
            }
            EXT_SAN => self.san = Some(v.expect(der::SEQUENCE).ok_or(Error::Format)?.body),
            EXT_EXT_KEY_USAGE | EXT_SKI | EXT_AKI => {}
            _ if critical => return Err(Error::CriticalExtension),
            _ => {}
        }
        Ok(())
    }

    /// Checks that `issuer_key` signed this certificate.
    pub fn signed_by(&self, issuer_key: &Key) -> Result<(), Error> {
        if verify_signature(issuer_key, self.sig_alg, self.tbs, self.signature) { Ok(()) } else { Err(Error::BadSignature) }
    }

    /// Does `host` match one of the DNS names (RFC 6125: case-insensitive; `*` only as the whole leftmost label)?
    pub fn matches_host(&self, host: &[u8]) -> bool {
        let Some(san) = self.san else { return false };
        let mut d = Der::new(san);
        while let Some(name) = d.next() {
            if name.tag == 0x82 && host_matches(name.body, host) {
                return true;
            }
        }
        false
    }
}

fn host_matches(pattern: &[u8], host: &[u8]) -> bool {
    if let Some(rest) = pattern.strip_prefix(b"*.") {
        // The wildcard covers exactly one label, and needs at least two labels after it.
        let Some(dot) = host.iter().position(|&c| c == b'.') else { return false };
        return dot > 0 && rest.contains(&b'.') && host[dot + 1..].eq_ignore_ascii_case(rest);
    }
    !pattern.is_empty() && pattern.eq_ignore_ascii_case(host)
}

/// Verifies `sig` over `msg` with `key` and `alg` (used for certificates and for TLS CertificateVerify).
pub fn verify_signature(key: &Key, alg: SigAlg, msg: &[u8], sig: &[u8]) -> bool {
    match (key, alg) {
        (Key::Rsa { n, e }, SigAlg::RsaPkcs1(h)) => {
            let Some(k) = rsa::PublicKey::new(n, e) else { return false };
            match h {
                Hash::Sha256 => k.verify_pkcs1v15::<Sha256>(msg, sig),
                Hash::Sha384 => k.verify_pkcs1v15::<Sha384>(msg, sig),
                Hash::Sha512 => k.verify_pkcs1v15::<Sha512>(msg, sig),
            }
        }
        (Key::Rsa { n, e }, SigAlg::RsaPss(h, salt)) => {
            let Some(k) = rsa::PublicKey::new(n, e) else { return false };
            if salt != h.len() {
                return false; // we only accept the usual salt = hash length
            }
            match h {
                Hash::Sha256 => k.verify_pss::<Sha256>(msg, sig, salt),
                Hash::Sha384 => k.verify_pss::<Sha384>(msg, sig, salt),
                Hash::Sha512 => k.verify_pss::<Sha512>(msg, sig, salt),
            }
        }
        (Key::P256(point), SigAlg::Ecdsa(h)) => {
            let Some(k) = p256::PublicKey::new(point) else { return false };
            match h {
                Hash::Sha256 => k.verify::<Sha256>(msg, sig),
                Hash::Sha384 => k.verify::<Sha384>(msg, sig),
                Hash::Sha512 => k.verify::<Sha512>(msg, sig),
            }
        }
        (Key::P384(point), SigAlg::Ecdsa(h)) => {
            let Some(k) = p384::PublicKey::new(point) else { return false };
            match h {
                Hash::Sha256 => k.verify::<Sha256>(msg, sig),
                Hash::Sha384 => k.verify::<Sha384>(msg, sig),
                Hash::Sha512 => k.verify::<Sha512>(msg, sig),
            }
        }
        _ => false,
    }
}

const MAX_CHAIN: usize = 8;

/// Where trusted roots come from: a list in memory, or files looked up by name (see /bin/fetch).
pub trait TrustStore {
    /// Calls `f` with each trusted root (DER) whose subject may be `subject` (DER), until `f` returns true;
    /// returns whether it did. Giving extra roots is harmless (they are checked), missing one is not.
    fn find(&mut self, subject: &[u8], f: &mut dyn FnMut(&[u8]) -> bool) -> bool;
}

/// A plain list of roots.
impl TrustStore for &[&[u8]] {
    fn find(&mut self, _subject: &[u8], f: &mut dyn FnMut(&[u8]) -> bool) -> bool {
        self.iter().any(|r| f(r))
    }
}

/// The first 8 bytes of SHA-256 of a subject name, in lowercase hex: the file name of a root in /etc/certs
/// (tools/roots.py names them the same way, plus `.0`, `.1` …).
pub fn subject_id(subject: &[u8]) -> [u8; 16] {
    let h = Sha256::digest(subject);
    let hex = b"0123456789abcdef";
    core::array::from_fn(|i| hex[usize::from(h[i / 2] >> (4 - 4 * (i % 2) as u8)) & 15])
}

/// Verifies a server's chain: `chain[0]` is the server certificate, each next one its issuer (as sent in TLS);
/// it must end at one of `roots` (DER certificates). `now` is YYYYMMDDHHMMSS (UTC).
pub fn verify_chain(chain: &[&[u8]], mut roots: &[&[u8]], host: &[u8], now: u64) -> Result<(), Error> {
    verify_chain_in(chain, &mut roots, host, now)
}

/// The same, with the roots taken from a `TrustStore`.
pub fn verify_chain_in(chain: &[&[u8]], roots: &mut dyn TrustStore, host: &[u8], now: u64) -> Result<(), Error> {
    if chain.is_empty() {
        return Err(Error::EmptyChain);
    }
    if chain.len() > MAX_CHAIN {
        return Err(Error::PathTooLong);
    }
    let leaf = Cert::parse(chain[0])?;
    if !leaf.matches_host(host) {
        return Err(Error::NameMismatch);
    }
    let mut cur = leaf;
    // When `cur` = chain[i], its issuer has i intermediate certificates below it (for pathLenConstraint).
    for i in 0..chain.len() {
        check_time(&cur, now)?;
        // A trusted root that issued `cur` ends the chain (also when the root itself was sent along).
        // Roots we cannot read are skipped.
        let mut found = None;
        roots.find(cur.issuer, &mut |r| match Cert::parse(r) {
            Ok(root) if root.subject == cur.issuer && cur.signed_by(&root.key).is_ok() => {
                found = Some(check_time(&root, now).and_then(|_| check_issuer(&root, i)));
                true
            }
            _ => false,
        });
        if let Some(result) = found {
            return result;
        }
        let Some(&next) = chain.get(i + 1) else { return Err(Error::UnknownIssuer) };
        let issuer = Cert::parse(next)?;
        if issuer.subject != cur.issuer {
            return Err(Error::UnknownIssuer);
        }
        check_issuer(&issuer, i)?;
        cur.signed_by(&issuer.key)?;
        cur = issuer;
    }
    Err(Error::UnknownIssuer)
}

fn check_time(c: &Cert, now: u64) -> Result<(), Error> {
    if now < c.not_before {
        Err(Error::NotYetValid)
    } else if now > c.not_after {
        Err(Error::Expired)
    } else {
        Ok(())
    }
}

/// An issuer must be a CA allowed to sign certificates, with room for the intermediates below it.
fn check_issuer(c: &Cert, below: usize) -> Result<(), Error> {
    if !c.is_ca || c.key_usage.is_some_and(|u| u & (1 << 5) == 0) {
        return Err(Error::NotCa);
    }
    if c.path_len.is_some_and(|p| below > p as usize) {
        return Err(Error::PathTooLong);
    }
    Ok(())
}
