//! Encryption of the remote console (task P6-54, F2 step 2): our own small protocol, shaped after the PSK + (EC)DHE
//! mode of TLS 1.3 (RFC 8446 §2.2), built only from pieces of `crypto/` already tested on official vectors
//! (X25519 RFC 7748, HKDF-SHA256 RFC 5869, ChaCha20-Poly1305 RFC 8439). Not TLS itself: the client has to run on
//! Windows with plain Python (no libraries), so the protocol is kept small enough to write twice.
//!
//! 1. Server: `CYBER-REMOTE 2\n`, then 48 bytes: its fresh X25519 public value (32) and a random nonce (16).
//! 2. Client: its own 48 bytes (public value, nonce), then the first record (plaintext `AUTH`).
//! 3. Both: `shared = X25519(own secret, peer public)` (all zero = refused);
//!    `prk = HKDF-Extract(salt = SHA-256("cyber-remote cheie\0" || key), ikm = shared)`;
//!    `th = SHA-256("CYBER-REMOTE 2" || server 48 bytes || client 48 bytes)`;
//!    client→server key = `HKDF-Expand(prk, "c2s" || th, 32)`, server→client key = `HKDF-Expand(prk, "s2c" || th, 32)`.
//! 4. Records: 2 bytes big-endian length of what follows, then ChaCha20-Poly1305 ciphertext + 16-byte tag.
//!    Nonce = 4 zero bytes || 64-bit big-endian counter (one per direction, from 0); the 2 length bytes are the AAD.
//!    At most 4096 bytes of plaintext per record.
//!
//! The key never travels: only someone who has it derives the same record keys, so the first record from the client
//! opening correctly proves the client has the key, and the server's first record opening proves it to the client.
//! Fresh X25519 values per connection: recorded traffic stays secret even if the key leaks later (forward secrecy).
//! A wrong key and a tampered record look the same (the tag does not match): the connection is closed.

use alloc::vec::Vec;
use cyber_crypto::{Digest, aead, hkdf, sha2::Sha256, x25519};

pub const HELLO_LEN: usize = 48;
pub const MAX_PLAIN: usize = 4096;
const TAG: usize = aead::TAG_LEN;
/// The plaintext of the client's first record.
pub const AUTH: &[u8] = b"AUTH";

/// One side's fresh X25519 secret and its 48-byte hello (public value + nonce).
pub struct Hello {
    secret: [u8; 32],
    pub bytes: [u8; HELLO_LEN],
}

impl Hello {
    /// From 48 random bytes: 32 for the secret, 16 for the nonce.
    pub fn from_random(random: &[u8; 48]) -> Hello {
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&random[..32]);
        let mut bytes = [0u8; HELLO_LEN];
        bytes[..32].copy_from_slice(&x25519::x25519_base(&secret));
        bytes[32..].copy_from_slice(&random[32..48]);
        Hello { secret, bytes }
    }

    pub fn new() -> Hello {
        let mut r = [0u8; 48];
        crate::random::fill(&mut r);
        let h = Hello::from_random(&r);
        r.fill(0);
        h
    }
}

impl Drop for Hello {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

/// The record keys of one connection, for one side.
pub struct Channel {
    send: [u8; 32],
    recv: [u8; 32],
    send_seq: u64,
    recv_seq: u64,
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.send.fill(0);
        self.recv.fill(0);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The tag did not match (wrong key or changed bytes).
    Tag,
    /// A length that no record of ours has.
    Length,
}

fn nonce(seq: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&seq.to_be_bytes());
    n
}

/// Derives the record keys. `own` = this side's hello (with its secret), `server`/`client` = the two hello byte
/// strings as sent. `None` if the peer's public value gives the all-zero shared secret (a low-order point).
pub fn derive(key: &[u8], own: &Hello, server: &[u8; HELLO_LEN], client: &[u8; HELLO_LEN], is_server: bool) -> Option<Channel> {
    let peer = if is_server { client } else { server };
    let mut peer_pub = [0u8; 32];
    peer_pub.copy_from_slice(&peer[..32]);
    let mut shared = x25519::x25519(&own.secret, &peer_pub);
    if cyber_crypto::ct_eq(&shared, &[0u8; 32]) {
        return None;
    }
    let mut salt_h = Sha256::new();
    salt_h.update(b"cyber-remote cheie\0");
    salt_h.update(key);
    let salt = salt_h.finish();
    let mut prk = [0u8; 32];
    hkdf::extract::<Sha256>(&salt, &shared, &mut prk);
    shared.fill(0);
    let mut th_h = Sha256::new();
    th_h.update(b"CYBER-REMOTE 2");
    th_h.update(server);
    th_h.update(client);
    let th = th_h.finish();
    let mut info = [0u8; 35];
    info[3..].copy_from_slice(&th);
    let (mut c2s, mut s2c) = ([0u8; 32], [0u8; 32]);
    info[..3].copy_from_slice(b"c2s");
    hkdf::expand::<Sha256>(&prk, &info, &mut c2s);
    info[..3].copy_from_slice(b"s2c");
    hkdf::expand::<Sha256>(&prk, &info, &mut s2c);
    prk.fill(0);
    let (send, recv) = if is_server { (s2c, c2s) } else { (c2s, s2c) };
    Some(Channel { send, recv, send_seq: 0, recv_seq: 0 })
}

impl Channel {
    /// One record for `plain` (at most `MAX_PLAIN` bytes; longer input is split by `seal_all`).
    pub fn seal(&mut self, plain: &[u8]) -> Vec<u8> {
        let plain = &plain[..plain.len().min(MAX_PLAIN)];
        let len = ((plain.len() + TAG) as u16).to_be_bytes();
        let mut out = Vec::with_capacity(2 + plain.len() + TAG);
        out.extend_from_slice(&len);
        out.extend_from_slice(plain);
        let tag = aead::seal(&self.send, &nonce(self.send_seq), &len, &mut out[2..]);
        out.extend_from_slice(&tag);
        self.send_seq += 1;
        out
    }

    /// Records for any amount of data, one after the other.
    pub fn seal_all(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for piece in data.chunks(MAX_PLAIN) {
            out.extend_from_slice(&self.seal(piece));
        }
        out
    }

    /// Takes the first complete record from the front of `buf` and returns its plaintext; `Ok(None)` = not all of
    /// it has arrived yet (nothing is taken).
    pub fn open(&mut self, buf: &mut Vec<u8>) -> Result<Option<Vec<u8>>, RecordError> {
        if buf.len() < 2 {
            return Ok(None);
        }
        let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
        if !(TAG..=MAX_PLAIN + TAG).contains(&len) {
            return Err(RecordError::Length);
        }
        if buf.len() < 2 + len {
            return Ok(None);
        }
        let aad = [buf[0], buf[1]];
        let mut data = buf[2..2 + len - TAG].to_vec();
        let ok = aead::open(&self.recv, &nonce(self.recv_seq), &aad, &mut data, &buf[2 + len - TAG..2 + len]);
        if !ok {
            return Err(RecordError::Tag);
        }
        buf.drain(..2 + len);
        self.recv_seq += 1;
        Ok(Some(data))
    }
}

/// Self-test without the network: both sides derive the same keys, records go both ways in order, a wrong key,
/// a changed byte, a replayed record and a bad length are refused, and the fixed vector matches the Python client
/// (`tools/remote-client.py --test` prints the same record).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let key = b"cheie-lunga-de-test-0123456789";
    let mut rs = [0u8; 48];
    let mut rc = [0u8; 48];
    for i in 0..48 {
        rs[i] = i as u8 + 1;
        rc[i] = 0x80 + i as u8;
    }
    let (hs, hc) = (Hello::from_random(&rs), Hello::from_random(&rc));
    let mut server = derive(key, &hs, &hs.bytes, &hc.bytes, true).expect("remote: cheile serverului");
    let mut client = derive(key, &hc, &hs.bytes, &hc.bytes, false).expect("remote: cheile clientului");
    let first = client.seal(AUTH);
    // The fixed vector (the same scalars and nonces in tools/remote-client.py --test).
    let hex: Vec<u8> = first.iter().flat_map(|b| {
        let d = b"0123456789abcdef";
        [d[(b >> 4) as usize], d[(b & 15) as usize]]
    }).collect();
    crate::info!("test: remote: vector AUTH {}", core::str::from_utf8(&hex).unwrap_or("?"));
    assert_eq!(&hex[..], b"00144a1b0bc5e14acd6c45b1600422e7644357b1fa51", "remote: vectorul comun cu remote-client.py");
    let mut buf = first.clone();
    assert_eq!(server.open(&mut buf), Ok(Some(AUTH.to_vec())), "remote: AUTH");
    assert!(buf.is_empty());
    // Several records at once, one split in two, and a long text cut into records.
    let long: Vec<u8> = (0..10_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let mut wire = server.seal(b"OK");
    wire.extend_from_slice(&server.seal_all(&long));
    let mut got = Vec::new();
    let mut inbox = Vec::new();
    for piece in wire.chunks(777) {
        inbox.extend_from_slice(piece);
        while let Some(p) = client.open(&mut inbox).expect("remote: record") {
            got.extend_from_slice(&p);
        }
    }
    assert!(inbox.is_empty() && got.len() == 2 + long.len() && got[..2] == *b"OK" && got[2..] == long[..]);
    // A wrong key: the server cannot open the client's record.
    let mut bad_client = derive(b"alta-cheie-lunga-de-test-012345", &hc, &hs.bytes, &hc.bytes, false).unwrap();
    let mut s2 = derive(key, &hs, &hs.bytes, &hc.bytes, true).unwrap();
    let mut buf = bad_client.seal(AUTH);
    assert_eq!(s2.open(&mut buf), Err(RecordError::Tag), "remote: cheie gresita");
    // A changed byte, a replay, a bad length.
    let mut s3 = derive(key, &hs, &hs.bytes, &hc.bytes, true).unwrap();
    let mut c3 = derive(key, &hc, &hs.bytes, &hc.bytes, false).unwrap();
    let rec = c3.seal(b"uptime\n");
    let mut changed = rec.clone();
    changed[4] ^= 1;
    assert_eq!(s3.open(&mut changed), Err(RecordError::Tag), "remote: octet schimbat");
    let mut ok = rec.clone();
    assert_eq!(s3.open(&mut ok), Ok(Some(b"uptime\n".to_vec())));
    let mut replay = rec.clone();
    assert_eq!(s3.open(&mut replay), Err(RecordError::Tag), "remote: record repetat");
    let mut bad_len = alloc::vec![0xff, 0xff, 0];
    assert_eq!(s3.open(&mut bad_len), Err(RecordError::Length));
    let mut short = alloc::vec![0x00];
    assert_eq!(s3.open(&mut short), Ok(None));
    // A low-order public value (all zero) is refused.
    let mut zero = hc.bytes;
    zero[..32].fill(0);
    assert!(derive(key, &hs, &hs.bytes, &zero, true).is_none(), "remote: punct de ordin mic");
    crate::info!("test: remote: criptarea OK");
}
