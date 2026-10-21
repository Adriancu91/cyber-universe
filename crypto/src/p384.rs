//! ECDSA signature verification on the NIST P-384 curve (FIPS 186-5, SP 800-186 §3.2.1.4):
//! p = 2^384 − 2^128 − 2^96 + 2^32 − 1. Used by many certificate authorities (ISRG Root X2, Let's Encrypt E…,
//! GTS Root R4, DigiCert G3). The arithmetic is shared with P-256 in `ec`.

use crate::Digest;
use crate::ec::{self, Curve};

pub static CURVE: Curve<6> = Curve {
    p: [0x0000_0000_ffff_ffff, 0xffff_ffff_0000_0000, 0xffff_ffff_ffff_fffe, 0xffff_ffff_ffff_ffff, 0xffff_ffff_ffff_ffff, 0xffff_ffff_ffff_ffff],
    n: [0xecec_196a_ccc5_2973, 0x581a_0db2_48b0_a77a, 0xc763_4d81_f437_2ddf, 0xffff_ffff_ffff_ffff, 0xffff_ffff_ffff_ffff, 0xffff_ffff_ffff_ffff],
    b: [0x2a85_c8ed_d3ec_2aef, 0xc656_398d_8a2e_d19d, 0x0314_088f_5013_875a, 0x181d_9c6e_fe81_4112, 0x988e_056b_e3f8_2d19, 0xb331_2fa7_e23e_e7e4],
    gx: [0x3a54_5e38_7276_0ab7, 0x5502_f25d_bf55_296c, 0x59f7_41e0_8254_2a38, 0x6e1d_3b62_8ba7_9b98, 0x8eb1_c71e_f320_ad74, 0xaa87_ca22_be8b_0537],
    gy: [0x7a43_1d7c_90ea_0e5f, 0x0a60_b1ce_1d7e_819d, 0xe9da_3113_b5f0_b8c0, 0xf8f4_1dbd_289a_147c, 0x5d9e_98bf_9292_dc29, 0x3617_de4a_9626_2c6f],
};

/// Public key = uncompressed point 04 || x || y (97 bytes), checked to lie on the curve.
pub struct PublicKey(ec::PublicKey<6>);

impl PublicKey {
    pub fn new(sec1: &[u8]) -> Option<PublicKey> {
        ec::PublicKey::new(&CURVE, sec1).map(PublicKey)
    }

    /// Verifies (r, s), 48 big-endian bytes each, over `hash` (its leftmost 384 bits are used).
    pub fn verify_prehashed(&self, hash: &[u8], r: &[u8; 48], s: &[u8; 48]) -> bool {
        self.0.verify_prehashed(hash, r, s)
    }

    /// Hashes `msg` with D and verifies a DER signature (SEQUENCE { INTEGER r, INTEGER s }), as in X.509 and TLS.
    pub fn verify<D: Digest>(&self, msg: &[u8], der: &[u8]) -> bool {
        self.0.verify::<D>(msg, der)
    }
}
