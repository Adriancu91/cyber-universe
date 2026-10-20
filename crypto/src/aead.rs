//! ChaCha20-Poly1305 authenticated encryption with additional data (RFC 8439 §2.8), as used by the
//! TLS 1.3 suite TLS_CHACHA20_POLY1305_SHA256.

use crate::chacha20;
use crate::ct_eq;
use crate::poly1305::Poly1305;

pub const TAG_LEN: usize = 16;

/// Tag over aad || pad16 || ciphertext || pad16 || len(aad) || len(ciphertext), keyed by block 0.
fn tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let block0 = chacha20::block(key, 0, nonce);
    let mut p = Poly1305::new(block0[..32].try_into().unwrap());
    let zeros = [0u8; 16];
    p.update(aad);
    p.update(&zeros[..(16 - aad.len() % 16) % 16]);
    p.update(ct);
    p.update(&zeros[..(16 - ct.len() % 16) % 16]);
    p.update(&(aad.len() as u64).to_le_bytes());
    p.update(&(ct.len() as u64).to_le_bytes());
    p.finish()
}

/// Encrypts `data` in place and returns the tag.
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &mut [u8]) -> [u8; TAG_LEN] {
    chacha20::xor(key, 1, nonce, data);
    tag(key, nonce, aad, data)
}

/// Checks the tag (in constant time) and only then decrypts `data` in place. On a wrong tag returns
/// `false` and leaves `data` as it was (still encrypted), so no unauthenticated text is released.
pub fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &mut [u8], tag_in: &[u8]) -> bool {
    if !ct_eq(&tag(key, nonce, aad, data), tag_in) {
        return false;
    }
    chacha20::xor(key, 1, nonce, data);
    true
}
