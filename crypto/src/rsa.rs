//! RSA signature verification (RFC 8017): RSAVP1, then EMSA-PKCS1-v1_5 or EMSA-PSS (MGF1) with
//! SHA-256/384/512. Public key = (n, e) as big-endian bytes. Anything malformed is refused.

use crate::Digest;
use crate::bignum::{MAX_BYTES, Mont, Num};
use crate::ct_eq;

/// DigestInfo prefixes (RFC 8017 §9.2, note 1), chosen by hash length.
fn digest_info(len: usize) -> Option<&'static [u8]> {
    Some(match len {
        32 => &[0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20],
        48 => &[0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00, 0x04, 0x30],
        64 => &[0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05, 0x00, 0x04, 0x40],
        _ => return None,
    })
}

fn strip(b: &[u8]) -> &[u8] {
    let z = b.iter().take_while(|&&x| x == 0).count();
    &b[z..]
}

/// A public key ready to verify: modulus length k bytes, modulus bits.
pub struct PublicKey<'a> {
    n: Num,
    e: &'a [u8],
    k: usize,
    bits: usize,
    mont: Mont,
}

impl<'a> PublicKey<'a> {
    /// 1024–4096-bit odd modulus, exponent odd and >= 3. `None` otherwise.
    pub fn new(n: &[u8], e: &'a [u8]) -> Option<Self> {
        let (n, e) = (strip(n), strip(e));
        let num = Num::from_be(n)?;
        let bits = num.bits();
        let odd_e = e.last().is_some_and(|&x| x & 1 == 1);
        if !(1024..=MAX_BYTES * 8).contains(&bits) || !odd_e || (e.len() == 1 && e[0] < 3) || e.len() > n.len() {
            return None;
        }
        Some(PublicKey { n: num, e, k: n.len(), bits, mont: Mont::new(&num)? })
    }

    pub fn bits(&self) -> usize {
        self.bits
    }

    /// RSAVP1: sig^e mod n into em[..k]. Refuses a wrong length or sig >= n.
    fn open(&self, sig: &[u8], em: &mut [u8; MAX_BYTES]) -> bool {
        if sig.len() != self.k {
            return false;
        }
        let Some(s) = Num::from_be(sig) else { return false };
        if !s.lt(&self.n) {
            return false;
        }
        self.mont.pow(&s, self.e).to_be(&mut em[..self.k]);
        true
    }

    /// RSASSA-PKCS1-v1_5 verification (RFC 8017 §8.2.2): EM = 00 01 FF..FF 00 || DigestInfo || H(msg).
    pub fn verify_pkcs1v15<D: Digest>(&self, msg: &[u8], sig: &[u8]) -> bool {
        let mut em = [0u8; MAX_BYTES];
        let Some(prefix) = digest_info(D::LEN) else { return false };
        let t = prefix.len() + D::LEN;
        if self.k < t + 11 || !self.open(sig, &mut em) {
            return false;
        }
        // Build the only acceptable encoding and compare it whole.
        let mut want = [0u8; MAX_BYTES];
        let k = self.k;
        want[1] = 1;
        want[2..k - t - 1].fill(0xff);
        want[k - t..k - D::LEN].copy_from_slice(prefix);
        let mut h = D::new();
        h.update(msg);
        h.finish_into(&mut want[k - D::LEN..k]);
        ct_eq(&em[..k], &want[..k])
    }

    /// RSASSA-PSS verification (RFC 8017 §8.1.2, §9.1.2) with MGF1 over the same hash and `salt_len`
    /// bytes of salt (TLS 1.3 and most certificates: salt_len = hash length).
    pub fn verify_pss<D: Digest>(&self, msg: &[u8], sig: &[u8], salt_len: usize) -> bool {
        let mut buf = [0u8; MAX_BYTES];
        if !self.open(sig, &mut buf) {
            return false;
        }
        let h_len = D::LEN;
        let em_bits = self.bits - 1;
        let em_len = em_bits.div_ceil(8);
        // When em_len < k the leading byte of the RSAVP1 output must be zero.
        let skip = self.k - em_len;
        if buf[..skip].iter().any(|&b| b != 0) {
            return false;
        }
        let em = &mut buf[skip..self.k];
        if em_len < h_len + salt_len + 2 || em[em_len - 1] != 0xbc {
            return false;
        }
        let db_len = em_len - h_len - 1;
        let top_mask = 0xffu8 >> (8 * em_len - em_bits);
        if em[0] & !top_mask != 0 {
            return false;
        }
        let mut hh = [0u8; 64];
        hh[..h_len].copy_from_slice(&em[db_len..db_len + h_len]);
        // DB = maskedDB xor MGF1(H, db_len).
        let mut counter = 0u32;
        let mut pos = 0;
        while pos < db_len {
            let mut d = D::new();
            d.update(&hh[..h_len]);
            d.update(&counter.to_be_bytes());
            let mut block = [0u8; 64];
            d.finish_into(&mut block);
            for b in block[..h_len].iter() {
                if pos == db_len {
                    break;
                }
                em[pos] ^= b;
                pos += 1;
            }
            counter += 1;
        }
        em[0] &= top_mask;
        let ps = db_len - salt_len - 1;
        if em[..ps].iter().any(|&b| b != 0) || em[ps] != 1 {
            return false;
        }
        // H' = Hash(00^8 || Hash(msg) || salt).
        let mut m_hash = [0u8; 64];
        let mut d = D::new();
        d.update(msg);
        d.finish_into(&mut m_hash);
        let mut d = D::new();
        d.update(&[0u8; 8]);
        d.update(&m_hash[..h_len]);
        d.update(&em[ps + 1..db_len]);
        let mut h2 = [0u8; 64];
        d.finish_into(&mut h2);
        ct_eq(&h2[..h_len], &hh[..h_len])
    }
}
