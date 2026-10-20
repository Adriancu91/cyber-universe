//! Cyber Universe's own cryptography (decision D-011): written from the public standards, checked against
//! the official test vectors. `no_std`, no heap, no dependencies. Not audited: see crypto/README.md.
#![cfg_attr(not(test), no_std)]

pub mod aead;
pub mod bignum;
pub mod chacha20;
pub mod der;
pub mod hkdf;
pub mod hmac;
pub mod p256;
pub mod poly1305;
pub mod rsa;
pub mod sha2;
pub mod vectors;
pub mod x509;
mod vectors_rsa;
pub mod x25519;

/// A hash function usable by HMAC and HKDF.
pub trait Digest: Sized {
    /// Block size in bytes (64 for SHA-256, 128 for SHA-384/512).
    const BLOCK: usize;
    /// Output size in bytes.
    const LEN: usize;
    fn new() -> Self;
    fn update(&mut self, data: &[u8]);
    /// Writes the digest into `out[..LEN]`.
    fn finish_into(self, out: &mut [u8]);
}

/// Equality of two byte strings in time that does not depend on where they differ (for tags and MACs).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y));
    // black_box keeps the compiler from turning the fold into an early exit.
    core::hint::black_box(diff) == 0
}
