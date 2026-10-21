//! The TLS 1.3 client handshake (RFC 8446 §4, §7.1) with one suite (TLS_CHACHA20_POLY1305_SHA256) and one group
//! (X25519), then application data. All buffers come from the caller; nothing is allocated.

use crate::record::{self, Io, Keys, MAX_PLAIN, MAX_RECORD};
use cyber_crypto::sha2::Sha256;
use cyber_crypto::x509::{self, Cert, Hash, SigAlg};
use cyber_crypto::x25519::{x25519, x25519_base};
use cyber_crypto::{Digest, ct_eq, hkdf, hmac::hmac};

const CHACHA20_POLY1305_SHA256: u16 = 0x1303;
const X25519: u16 = 0x001d;
const TLS13: u16 = 0x0304;
/// signature_algorithms: what we can check in CertificateVerify, then the RSA PKCS#1 ones (certificates only).
const SIG_ALGS: [u16; 8] = [0x0403, 0x0503, 0x0804, 0x0805, 0x0806, 0x0401, 0x0501, 0x0601];
/// ServerHello.random of a HelloRetryRequest (§4.1.3).
const HRR_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91, 0xc2, 0xa2, 0x11, 0x16,
    0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];
const MAX_CERTS: usize = 8;

/// Handshake message types.
const SERVER_HELLO: u8 = 2;
const NEW_SESSION_TICKET: u8 = 4;
const ENCRYPTED_EXTENSIONS: u8 = 8;
const CERTIFICATE: u8 = 11;
const CERTIFICATE_VERIFY: u8 = 15;
const FINISHED: u8 = 20;
const KEY_UPDATE: u8 = 24;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Error {
    /// The pipe failed, or closed in the middle of something.
    Io,
    /// The server broke the protocol (the text says where).
    Protocol(&'static str),
    /// The server chose something we do not support.
    Unsupported(&'static str),
    /// The server sent an alert (its code).
    Alert(u8),
    /// The certificate chain was refused.
    Certificate(x509::Error),
    BadSignature,
    BadFinished,
    /// The caller's buffer is too small for what the server sent.
    BufferTooSmall,
}

/// Minimal size of the work buffer given to `connect`: one record plus room for the server's handshake.
pub const MIN_BUFFER: usize = 5 + MAX_RECORD + 16 * 1024;

/// An open TLS connection. `buf` holds one record (and, during the handshake, the handshake messages after it).
pub struct Tls<'b> {
    buf: &'b mut [u8],
    read: Keys,
    write: Keys,
    /// Received application data not yet returned by `recv`: buf[pending.0..pending.1].
    pending: (usize, usize),
    closed: bool,
}

struct Writer<'a> {
    b: &'a mut [u8],
    n: usize,
}

impl Writer<'_> {
    fn put(&mut self, d: &[u8]) {
        self.b[self.n..self.n + d.len()].copy_from_slice(d);
        self.n += d.len();
    }
    fn u8(&mut self, v: u8) {
        self.put(&[v]);
    }
    fn u16(&mut self, v: u16) {
        self.put(&v.to_be_bytes());
    }
    /// Reserves a length field of `size` bytes; `end_len` fills it in.
    fn start_len(&mut self, size: usize) -> (usize, usize) {
        let at = self.n;
        self.n += size;
        (at, size)
    }
    fn end_len(&mut self, (at, size): (usize, usize)) {
        let len = self.n - at - size;
        for i in 0..size {
            self.b[at + i] = (len >> (8 * (size - 1 - i))) as u8;
        }
    }
}

/// A cursor over received bytes.
struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.b.len() < n {
            return Err(Error::Protocol("mesaj trunchiat"));
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> Result<usize, Error> {
        let b = self.take(3)?;
        Ok((b[0] as usize) << 16 | (b[1] as usize) << 8 | b[2] as usize)
    }
    fn vec8(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn vec16(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u16()? as usize;
        self.take(n)
    }
}

fn derive_secret(secret: &[u8; 32], label: &[u8], hash: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf::expand_label::<Sha256>(secret, label, hash, &mut out);
    out
}

fn finished_mac(base: &[u8; 32], hash: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    hkdf::expand_label::<Sha256>(base, b"finished", &[], &mut key);
    let mut mac = [0u8; 32];
    hmac::<Sha256>(&key, hash, &mut mac);
    mac
}

fn snapshot(t: &Sha256) -> [u8; 32] {
    t.clone().finish()
}

/// Reads one record into buf[at..]: returns (type, start, end) of its plaintext. Protected records are opened
/// with `keys`; a ChangeCipherSpec record during the handshake is skipped by the caller.
fn read_record<I: Io>(io: &mut I, buf: &mut [u8], at: usize, keys: Option<&mut Keys>) -> Result<(u8, usize, usize), Error> {
    if buf.len() < at + 5 + MAX_RECORD {
        return Err(Error::BufferTooSmall);
    }
    let mut head = [0u8; 5];
    if !record::read_exact(io, &mut head).map_err(|_| Error::Io)? {
        return Err(Error::Io);
    }
    let len = (head[3] as usize) << 8 | head[4] as usize;
    if len > MAX_RECORD || head[1] != 3 {
        return Err(Error::Protocol("antet de inregistrare"));
    }
    let body = &mut buf[at..at + len];
    if !record::read_exact(io, body).map_err(|_| Error::Io)? {
        return Err(Error::Io);
    }
    let protected = keys.is_some();
    match (head[0], keys) {
        (record::APPLICATION_DATA, Some(k)) => {
            let (kind, n) = k.open(&head, body).ok_or(Error::Protocol("inregistrare neautentica"))?;
            Ok((kind, at, at + n))
        }
        (record::ALERT, _) if len == 2 => Err(Error::Alert(body[1])),
        // Once keys are in place, handshake messages must come protected; only the compatibility CCS may not.
        (record::HANDSHAKE, _) if !protected && len <= MAX_PLAIN => Ok((record::HANDSHAKE, at, at + len)),
        (record::CHANGE_CIPHER_SPEC, _) if len == 1 => Ok((record::CHANGE_CIPHER_SPEC, at, at + 1)),
        _ => Err(Error::Protocol("tip de inregistrare neasteptat")),
    }
}

/// Handshake messages of the server, gathered in buf[hs..hs_end] from consecutive records (a message may span
/// records and a record may hold several). The records are read at buf[0..] and copied after.
struct Handshake<'a, 'b, I: Io> {
    io: &'a mut I,
    buf: &'a mut [u8],
    keys: Option<&'b mut Keys>,
    pos: usize,
    end: usize,
}

impl<I: Io> Handshake<'_, '_, I> {
    /// The next message: (type, start, end) of the whole message (header included) in buf.
    fn next(&mut self) -> Result<(u8, usize, usize), Error> {
        loop {
            let avail = self.end - self.pos;
            if avail >= 4 {
                let b = &self.buf[self.pos..];
                let len = (b[1] as usize) << 16 | (b[2] as usize) << 8 | b[3] as usize;
                if avail >= 4 + len {
                    let start = self.pos;
                    self.pos += 4 + len;
                    return Ok((b[0], start, self.pos));
                }
            }
            let (kind, s, e) = read_record(self.io, self.buf, 0, self.keys.as_deref_mut())?;
            match kind {
                record::CHANGE_CIPHER_SPEC if e - s == 1 && self.buf[s] == 1 => continue,
                record::HANDSHAKE => {}
                record::ALERT if e - s == 2 => return Err(Error::Alert(self.buf[s + 1])),
                _ => return Err(Error::Protocol("se astepta handshake")),
            }
            let n = e - s;
            if self.end + n > self.buf.len() {
                return Err(Error::BufferTooSmall);
            }
            self.buf.copy_within(s..e, self.end);
            self.end += n;
        }
    }

    /// True if no partial message is left (required before keys change).
    fn drained(&self) -> bool {
        self.pos == self.end
    }
}

impl<'b> Tls<'b> {
    /// Opens a TLS 1.3 connection over `io` to `host`, trusting `roots` (DER certificates). `now` is the time as
    /// YYYYMMDDHHMMSS (UTC); `random` = 64 unpredictable bytes (client random, then the X25519 private key).
    /// `buf` must have at least `MIN_BUFFER` bytes.
    pub fn connect<I: Io>(io: &mut I, host: &str, roots: &[&[u8]], now: u64, random: &[u8; 64], buf: &'b mut [u8]) -> Result<Tls<'b>, Error> {
        if buf.len() < MIN_BUFFER || host.is_empty() || host.len() > 255 {
            return Err(Error::BufferTooSmall);
        }
        let secret: [u8; 32] = random[32..].try_into().unwrap();
        let public = x25519_base(&secret);

        // ClientHello, written after a 5-byte record header.
        let mut w = Writer { b: &mut buf[..], n: 5 };
        w.u8(1);
        let body = w.start_len(3);
        w.u16(0x0303);
        w.put(&random[..32]);
        w.u8(32);
        w.put(&random[..32]); // legacy_session_id: any value (middlebox compatibility mode)
        w.u16(2);
        w.u16(CHACHA20_POLY1305_SHA256);
        w.put(&[1, 0]); // compression: null only
        let exts = w.start_len(2);
        // server_name
        w.u16(0);
        let e = w.start_len(2);
        let list = w.start_len(2);
        w.u8(0);
        w.u16(host.len() as u16);
        w.put(host.as_bytes());
        w.end_len(list);
        w.end_len(e);
        // supported_groups
        w.put(&[0, 10, 0, 4, 0, 2]);
        w.u16(X25519);
        // signature_algorithms
        w.u16(13);
        w.u16(2 + 2 * SIG_ALGS.len() as u16);
        w.u16(2 * SIG_ALGS.len() as u16);
        for a in SIG_ALGS {
            w.u16(a);
        }
        // supported_versions: TLS 1.3 only
        w.put(&[0, 43, 0, 3, 2]);
        w.u16(TLS13);
        // key_share: X25519
        w.put(&[0, 51, 0, 38, 0, 36]);
        w.u16(X25519);
        w.u16(32);
        w.put(&public);
        w.end_len(exts);
        w.end_len(body);
        let n = w.n;
        let len = n - 5;
        buf[..5].copy_from_slice(&[record::HANDSHAKE, 3, 1, (len >> 8) as u8, len as u8]);
        let mut transcript = Sha256::new();
        transcript.update(&buf[5..n]);
        io.write_all(&buf[..n]).map_err(|_| Error::Io)?;

        // ServerHello (plaintext).
        let hs_start = 5 + MAX_RECORD;
        let mut hs = Handshake { io: &mut *io, buf: &mut buf[..], keys: None, pos: hs_start, end: hs_start };
        let (kind, s, e) = hs.next()?;
        if kind != SERVER_HELLO {
            return Err(Error::Protocol("se astepta ServerHello"));
        }
        let mut r = Reader { b: &hs.buf[s + 4..e] };
        r.u16()?; // legacy_version
        if r.take(32)? == HRR_RANDOM {
            return Err(Error::Unsupported("HelloRetryRequest"));
        }
        if r.vec8()? != &random[..32] {
            return Err(Error::Protocol("session id diferit"));
        }
        if r.u16()? != CHACHA20_POLY1305_SHA256 || r.u8()? != 0 {
            return Err(Error::Unsupported("suita"));
        }
        let mut ext = Reader { b: r.vec16()? };
        let (mut version, mut server_key) = (0, None);
        while !ext.b.is_empty() {
            let typ = ext.u16()?;
            let mut v = Reader { b: ext.vec16()? };
            match typ {
                43 => version = v.u16()?,
                51 => {
                    if v.u16()? != X25519 {
                        return Err(Error::Unsupported("grupul"));
                    }
                    let k = v.vec16()?;
                    server_key = Some(<[u8; 32]>::try_from(k).map_err(|_| Error::Protocol("cheia serverului"))?);
                }
                _ => return Err(Error::Protocol("extensie neasteptata in ServerHello")),
            }
        }
        if version != TLS13 {
            return Err(Error::Unsupported("versiunea"));
        }
        let server_key = server_key.ok_or(Error::Protocol("lipseste key_share"))?;
        transcript.update(&hs.buf[s..e]);
        if !hs.drained() {
            return Err(Error::Protocol("date dupa ServerHello"));
        }

        // Key schedule (§7.1).
        let shared = x25519(&secret, &server_key);
        if shared == [0; 32] {
            return Err(Error::Protocol("cheie X25519 slaba"));
        }
        let mut early = [0u8; 32];
        hkdf::extract::<Sha256>(&[], &[0; 32], &mut early);
        let empty = Sha256::digest(&[]);
        let mut hs_secret = [0u8; 32];
        hkdf::extract::<Sha256>(&derive_secret(&early, b"derived", &empty), &shared, &mut hs_secret);
        let th = snapshot(&transcript);
        let c_hs = derive_secret(&hs_secret, b"c hs traffic", &th);
        let s_hs = derive_secret(&hs_secret, b"s hs traffic", &th);
        let mut master = [0u8; 32];
        hkdf::extract::<Sha256>(&derive_secret(&hs_secret, b"derived", &empty), &[0; 32], &mut master);
        let mut read = Keys::new(&s_hs);
        hs.keys = Some(&mut read);

        // EncryptedExtensions, Certificate, CertificateVerify, Finished.
        let (kind, s, e) = hs.next()?;
        if kind != ENCRYPTED_EXTENSIONS {
            return Err(Error::Protocol("se astepta EncryptedExtensions"));
        }
        transcript.update(&hs.buf[s..e]);
        let (kind, s, e) = hs.next()?;
        if kind != CERTIFICATE {
            return Err(Error::Unsupported("server fara certificat"));
        }
        let mut certs: [&[u8]; MAX_CERTS] = [&[]; MAX_CERTS];
        let mut count = 0;
        {
            let mut r = Reader { b: &hs.buf[s + 4..e] };
            if r.u8()? != 0 {
                return Err(Error::Protocol("context de certificat"));
            }
            let list_len = r.u24()?;
            let mut list = Reader { b: r.take(list_len)? };
            while !list.b.is_empty() {
                let n = list.u24()?;
                let der = list.take(n)?;
                list.vec16()?; // per-certificate extensions (OCSP, SCT): not used
                if count == MAX_CERTS {
                    return Err(Error::Certificate(x509::Error::PathTooLong));
                }
                certs[count] = der;
                count += 1;
            }
            x509::verify_chain(&certs[..count], roots, host.as_bytes(), now).map_err(Error::Certificate)?;
        }
        transcript.update(&hs.buf[s..e]);
        let leaf_range = {
            let base = hs.buf.as_ptr() as usize;
            let p = certs[0].as_ptr() as usize - base;
            (p, p + certs[0].len())
        };
        let th_cert = snapshot(&transcript);

        let (kind, s, e) = hs.next()?;
        if kind != CERTIFICATE_VERIFY {
            return Err(Error::Protocol("se astepta CertificateVerify"));
        }
        {
            let mut r = Reader { b: &hs.buf[s + 4..e] };
            let scheme = r.u16()?;
            let alg = match scheme {
                0x0403 => SigAlg::Ecdsa(Hash::Sha256),
                0x0503 => SigAlg::Ecdsa(Hash::Sha384),
                0x0804 => SigAlg::RsaPss(Hash::Sha256, 32),
                0x0805 => SigAlg::RsaPss(Hash::Sha384, 48),
                0x0806 => SigAlg::RsaPss(Hash::Sha512, 64),
                _ => return Err(Error::Unsupported("algoritmul CertificateVerify")),
            };
            let sig = r.vec16()?;
            let mut content = [0x20u8; 64 + 33 + 1 + 32];
            content[64..97].copy_from_slice(b"TLS 1.3, server CertificateVerify");
            content[97] = 0;
            content[98..].copy_from_slice(&th_cert);
            let leaf = Cert::parse(&hs.buf[leaf_range.0..leaf_range.1]).map_err(Error::Certificate)?;
            // In TLS 1.3 the ECDSA schemes also fix the curve (RFC 8446 §4.2.3).
            let curve_ok = match scheme {
                0x0403 => matches!(leaf.key, x509::Key::P256(_)),
                0x0503 => matches!(leaf.key, x509::Key::P384(_)),
                _ => true,
            };
            if !curve_ok || !x509::verify_signature(&leaf.key, alg, &content, sig) {
                return Err(Error::BadSignature);
            }
        }
        transcript.update(&hs.buf[s..e]);
        let th_cv = snapshot(&transcript);

        let (kind, s, e) = hs.next()?;
        if kind != FINISHED || e - s != 4 + 32 {
            return Err(Error::Protocol("se astepta Finished"));
        }
        if !ct_eq(&hs.buf[s + 4..e], &finished_mac(&s_hs, &th_cv)) {
            return Err(Error::BadFinished);
        }
        if !hs.drained() {
            return Err(Error::Protocol("date dupa Finished"));
        }
        transcript.update(&hs.buf[s..e]);
        let th_sf = snapshot(&transcript);

        // Our Finished: a compatibility ChangeCipherSpec, then Finished protected with the handshake keys.
        let c_ap = derive_secret(&master, b"c ap traffic", &th_sf);
        let s_ap = derive_secret(&master, b"s ap traffic", &th_sf);
        io.write_all(&[record::CHANGE_CIPHER_SPEC, 3, 3, 0, 1, 1]).map_err(|_| Error::Io)?;
        let mut write = Keys::new(&c_hs);
        buf[5..9].copy_from_slice(&[FINISHED, 0, 0, 32]);
        buf[9..41].copy_from_slice(&finished_mac(&c_hs, &th_sf));
        let n = write.seal(record::HANDSHAKE, buf, 36);
        io.write_all(&buf[..n]).map_err(|_| Error::Io)?;

        Ok(Tls { buf, read: Keys::new(&s_ap), write: Keys::new(&c_ap), pending: (0, 0), closed: false })
    }

    /// Sends application data (split into records).
    pub fn send<I: Io>(&mut self, io: &mut I, data: &[u8]) -> Result<(), Error> {
        for chunk in data.chunks(MAX_PLAIN) {
            self.buf[5..5 + chunk.len()].copy_from_slice(chunk);
            let n = self.write.seal(record::APPLICATION_DATA, self.buf, chunk.len());
            io.write_all(&self.buf[..n]).map_err(|_| Error::Io)?;
        }
        Ok(())
    }

    /// Receives application data into `out`; Ok(0) once the server has closed (close_notify).
    pub fn recv<I: Io>(&mut self, io: &mut I, out: &mut [u8]) -> Result<usize, Error> {
        loop {
            let (a, b) = self.pending;
            if a < b {
                let n = (b - a).min(out.len());
                out[..n].copy_from_slice(&self.buf[a..a + n]);
                self.pending.0 += n;
                return Ok(n);
            }
            if self.closed {
                return Ok(0);
            }
            let (kind, s, e) = read_record(io, self.buf, 0, Some(&mut self.read))?;
            match kind {
                record::APPLICATION_DATA => self.pending = (s, e),
                // Post-handshake messages: tickets are ignored (no resumption); KeyUpdate is not supported.
                record::HANDSHAKE if e - s >= 4 && self.buf[s] == NEW_SESSION_TICKET => {}
                record::HANDSHAKE if e - s >= 4 && self.buf[s] == KEY_UPDATE => return Err(Error::Unsupported("KeyUpdate")),
                record::ALERT if e - s == 2 => {
                    if self.buf[s + 1] == 0 {
                        self.closed = true; // close_notify
                    } else {
                        return Err(Error::Alert(self.buf[s + 1]));
                    }
                }
                _ => return Err(Error::Protocol("inregistrare neasteptata")),
            }
        }
    }

    /// Tells the server we are done (close_notify).
    pub fn close<I: Io>(&mut self, io: &mut I) -> Result<(), Error> {
        self.buf[5..7].copy_from_slice(&[1, 0]);
        let n = self.write.seal(record::ALERT, self.buf, 2);
        io.write_all(&self.buf[..n]).map_err(|_| Error::Io)
    }
}
