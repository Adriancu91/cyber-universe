//! HMAC (RFC 2104) over any `Digest`.

use crate::Digest;

/// Largest block and output among our hashes (SHA-512: 128 and 64 bytes).
const MAX_BLOCK: usize = 128;
const MAX_LEN: usize = 64;

/// An HMAC computation in progress.
pub struct Hmac<D: Digest> {
    inner: D,
    outer_key: [u8; MAX_BLOCK],
}

impl<D: Digest> Hmac<D> {
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; MAX_BLOCK];
        if key.len() > D::BLOCK {
            let mut h = D::new();
            h.update(key);
            h.finish_into(&mut k);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0u8; MAX_BLOCK];
        let mut opad = [0u8; MAX_BLOCK];
        for i in 0..D::BLOCK {
            ipad[i] = k[i] ^ 0x36;
            opad[i] = k[i] ^ 0x5c;
        }
        let mut inner = D::new();
        inner.update(&ipad[..D::BLOCK]);
        Hmac { inner, outer_key: opad }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// Writes the tag into `out[..D::LEN]`.
    pub fn finish_into(self, out: &mut [u8]) {
        let mut inner = [0u8; MAX_LEN];
        self.inner.finish_into(&mut inner);
        let mut outer = D::new();
        outer.update(&self.outer_key[..D::BLOCK]);
        outer.update(&inner[..D::LEN]);
        outer.finish_into(out);
    }
}

/// HMAC of `data` with `key`, written into `out[..D::LEN]`.
pub fn hmac<D: Digest>(key: &[u8], data: &[u8], out: &mut [u8]) {
    let mut h = Hmac::<D>::new(key);
    h.update(data);
    h.finish_into(out);
}
