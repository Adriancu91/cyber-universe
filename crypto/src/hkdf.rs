//! HKDF (RFC 5869) and TLS 1.3's HKDF-Expand-Label (RFC 8446 §7.1).

use crate::Digest;
use crate::hmac::Hmac;

/// HKDF-Extract: the pseudorandom key `prk[..D::LEN]` from `salt` and input keying material `ikm`.
pub fn extract<D: Digest>(salt: &[u8], ikm: &[u8], prk: &mut [u8]) {
    let zeros = [0u8; 64];
    let salt = if salt.is_empty() { &zeros[..D::LEN] } else { salt };
    let mut h = Hmac::<D>::new(salt);
    h.update(ikm);
    h.finish_into(prk);
}

/// HKDF-Expand: fills `okm` (at most 255 × D::LEN bytes) from `prk` and `info`. False if `okm` is too long.
pub fn expand<D: Digest>(prk: &[u8], info: &[u8], okm: &mut [u8]) -> bool {
    if okm.len() > 255 * D::LEN {
        return false;
    }
    let mut t = [0u8; 64];
    let mut t_len = 0;
    for (i, chunk) in okm.chunks_mut(D::LEN).enumerate() {
        let mut h = Hmac::<D>::new(prk);
        h.update(&t[..t_len]);
        h.update(info);
        h.update(&[(i + 1) as u8]);
        h.finish_into(&mut t);
        t_len = D::LEN;
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
    true
}

/// TLS 1.3 HKDF-Expand-Label(secret, label, context, length): `label` without the "tls13 " prefix.
/// False if the label or context is too long or `out` is longer than 255 bytes.
pub fn expand_label<D: Digest>(secret: &[u8], label: &[u8], context: &[u8], out: &mut [u8]) -> bool {
    let full = 6 + label.len();
    if full > 255 || context.len() > 255 || out.len() > 255 {
        return false;
    }
    let mut info = [0u8; 2 + 1 + 255 + 1 + 255];
    info[..2].copy_from_slice(&(out.len() as u16).to_be_bytes());
    info[2] = full as u8;
    info[3..9].copy_from_slice(b"tls13 ");
    info[9..9 + label.len()].copy_from_slice(label);
    let mut n = 9 + label.len();
    info[n] = context.len() as u8;
    info[n + 1..n + 1 + context.len()].copy_from_slice(context);
    n += 1 + context.len();
    expand::<D>(secret, &info[..n], out)
}
