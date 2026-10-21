//! ECDSA signature verification on the NIST P-256 curve (FIPS 186-5, SP 800-186 §3.2.1.3):
//! p = 2^256 − 2^224 + 2^192 + 2^96 − 1. The arithmetic is shared with P-384 in `ec`.

use crate::Digest;
use crate::ec::{self, Curve};

pub static CURVE: Curve<4> = Curve {
    p: [0xffff_ffff_ffff_ffff, 0x0000_0000_ffff_ffff, 0, 0xffff_ffff_0000_0001],
    n: [0xf3b9_cac2_fc63_2551, 0xbce6_faad_a717_9e84, 0xffff_ffff_ffff_ffff, 0xffff_ffff_0000_0000],
    b: [0x3bce_3c3e_27d2_604b, 0x651d_06b0_cc53_b0f6, 0xb3eb_bd55_7698_86bc, 0x5ac6_35d8_aa3a_93e7],
    gx: [0xf4a1_3945_d898_c296, 0x7703_7d81_2deb_33a0, 0xf8bc_e6e5_63a4_40f2, 0x6b17_d1f2_e12c_4247],
    gy: [0xcbb6_4068_37bf_51f5, 0x2bce_3357_6b31_5ece, 0x8ee7_eb4a_7c0f_9e16, 0x4fe3_42e2_fe1a_7f9b],
};

/// Public key = uncompressed point 04 || x || y (65 bytes), checked to lie on the curve.
pub struct PublicKey(ec::PublicKey<4>);

impl PublicKey {
    pub fn new(sec1: &[u8]) -> Option<PublicKey> {
        ec::PublicKey::new(&CURVE, sec1).map(PublicKey)
    }

    /// Verifies (r, s), 32 big-endian bytes each, over `hash` (its leftmost 256 bits are used).
    pub fn verify_prehashed(&self, hash: &[u8], r: &[u8; 32], s: &[u8; 32]) -> bool {
        self.0.verify_prehashed(hash, r, s)
    }

    /// Hashes `msg` with D and verifies a DER signature (SEQUENCE { INTEGER r, INTEGER s }), as in X.509 and TLS.
    pub fn verify<D: Digest>(&self, msg: &[u8], der: &[u8]) -> bool {
        self.0.verify::<D>(msg, der)
    }
}

/// Strict DER: 30 len (02 len r) (02 len s), minimal positive integers of at most 32 bytes.
pub fn parse_der_sig(der: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    let (mut r, mut s) = ([0u8; 32], [0u8; 32]);
    ec::parse_der_sig(der, &mut r, &mut s).then_some((r, s))
}
