//! TLS 1.3 records (RFC 8446 §5): 5-byte header, and records protected with ChaCha20-Poly1305
//! (nonce = iv XOR sequence number, real content type at the end of the plaintext).

use cyber_crypto::aead;
use cyber_crypto::hkdf;
use cyber_crypto::sha2::Sha256;

pub const CHANGE_CIPHER_SPEC: u8 = 20;
pub const ALERT: u8 = 21;
pub const HANDSHAKE: u8 = 22;
pub const APPLICATION_DATA: u8 = 23;

/// Largest plaintext in one record, and the most a protected record may add to it (§5.2).
pub const MAX_PLAIN: usize = 16384;
pub const MAX_RECORD: usize = MAX_PLAIN + 256;

/// A byte pipe: a TCP socket in tests and in the OS.
pub trait Io {
    /// Reads at least one byte; 0 = the other side closed.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()>;
    fn write_all(&mut self, data: &[u8]) -> Result<(), ()>;
}

pub fn read_exact<I: Io>(io: &mut I, mut buf: &mut [u8]) -> Result<bool, ()> {
    while !buf.is_empty() {
        let n = io.read(buf)?;
        if n == 0 {
            return Ok(false);
        }
        buf = &mut buf[n..];
    }
    Ok(true)
}

/// Traffic keys of one direction, derived from a traffic secret (§7.3).
pub struct Keys {
    key: [u8; 32],
    iv: [u8; 12],
    seq: u64,
}

impl Keys {
    pub fn new(secret: &[u8; 32]) -> Keys {
        let mut key = [0u8; 32];
        let mut iv = [0u8; 12];
        hkdf::expand_label::<Sha256>(secret, b"key", &[], &mut key);
        hkdf::expand_label::<Sha256>(secret, b"iv", &[], &mut iv);
        Keys { key, iv, seq: 0 }
    }

    fn next_nonce(&mut self) -> [u8; 12] {
        let mut n = self.iv;
        for (b, s) in n[4..].iter_mut().zip(self.seq.to_be_bytes()) {
            *b ^= s;
        }
        // A sequence number never wraps: the connection would have to carry 2^64 records.
        self.seq += 1;
        n
    }

    /// Protects `buf[5..5 + len]` (plaintext content) as a record of real type `kind`; writes the header and the
    /// tag; returns the record's total length. `buf` must hold 5 + len + 1 + 16 bytes.
    pub fn seal(&mut self, kind: u8, buf: &mut [u8], len: usize) -> usize {
        buf[5 + len] = kind;
        let body = len + 1 + aead::TAG_LEN;
        buf[..5].copy_from_slice(&[APPLICATION_DATA, 3, 3, (body >> 8) as u8, body as u8]);
        let (head, rest) = buf.split_at_mut(5);
        let nonce = self.next_nonce();
        let tag = aead::seal(&self.key, &nonce, head, &mut rest[..len + 1]);
        rest[len + 1..len + 1 + aead::TAG_LEN].copy_from_slice(&tag);
        5 + body
    }

    /// Opens a protected record in place: `head` = its 5-byte header, `body` = ciphertext || tag.
    /// Returns (real type, plaintext length) or `None` if it was not authentic or had no type.
    pub fn open(&mut self, head: &[u8], body: &mut [u8]) -> Option<(u8, usize)> {
        if body.len() < aead::TAG_LEN + 1 {
            return None;
        }
        let n = body.len() - aead::TAG_LEN;
        let (ct, tag) = body.split_at_mut(n);
        let nonce = self.next_nonce();
        if !aead::open(&self.key, &nonce, head, ct, tag) {
            return None;
        }
        // Strip the zero padding; the last non-zero byte is the real type.
        let end = ct.iter().rposition(|&b| b != 0)?;
        Some((ct[end], end))
    }
}
