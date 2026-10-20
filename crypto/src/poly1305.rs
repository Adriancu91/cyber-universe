//! Poly1305 one-time authenticator (RFC 8439 §2.5): arithmetic modulo p = 2^130 - 5 on five 26-bit limbs,
//! products in u64. No branches or table lookups depend on the key or the message.

const M26: u32 = 0x3ff_ffff;

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub struct Poly1305 {
    r: [u32; 5],
    h: [u32; 5],
    pad: [u32; 4],
    buf: [u8; 16],
    len: usize,
}

impl Poly1305 {
    /// `key` = r (clamped here) || s. A key must never be used for two messages.
    pub fn new(key: &[u8; 32]) -> Self {
        // Clamping r &= 0x0ffffffc0ffffffc0ffffffc0fffffff, split into 26-bit limbs.
        let r = [
            le32(&key[0..]) & 0x3ff_ffff,
            (le32(&key[3..]) >> 2) & 0x3ff_ff03,
            (le32(&key[6..]) >> 4) & 0x3ff_c0ff,
            (le32(&key[9..]) >> 6) & 0x3f0_3fff,
            (le32(&key[12..]) >> 8) & 0x00f_ffff,
        ];
        let pad = [le32(&key[16..]), le32(&key[20..]), le32(&key[24..]), le32(&key[28..])];
        Poly1305 { r, h: [0; 5], pad, buf: [0; 16], len: 0 }
    }

    /// h = (h + block + hibit * 2^128) * r mod p (partially reduced).
    fn block(&mut self, m: &[u8; 16], hibit: u32) {
        let [r0, r1, r2, r3, r4] = self.r.map(u64::from);
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
        let h = &mut self.h;
        let h0 = u64::from(h[0] + (le32(&m[0..]) & M26));
        let h1 = u64::from(h[1] + ((le32(&m[3..]) >> 2) & M26));
        let h2 = u64::from(h[2] + ((le32(&m[6..]) >> 4) & M26));
        let h3 = u64::from(h[3] + ((le32(&m[9..]) >> 6) & M26));
        let h4 = u64::from(h[4] + ((le32(&m[12..]) >> 8) | (hibit << 24)));
        // Limb i of the product; terms that pass 2^130 wrap around multiplied by 5 (2^130 = 5 mod p).
        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let mut d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let mut d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let mut d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let mut d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;
        d1 += d0 >> 26;
        h[0] = d0 as u32 & M26;
        d2 += d1 >> 26;
        h[1] = d1 as u32 & M26;
        d3 += d2 >> 26;
        h[2] = d2 as u32 & M26;
        d4 += d3 >> 26;
        h[3] = d3 as u32 & M26;
        h[4] = d4 as u32 & M26;
        // Carry from the top limb wraps around times 5; kept in u64 so it can never overflow.
        let t = u64::from(h[0]) + (d4 >> 26) * 5;
        h[0] = t as u32 & M26;
        h[1] += (t >> 26) as u32;
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if self.len > 0 {
            let take = (16 - self.len).min(data.len());
            self.buf[self.len..self.len + take].copy_from_slice(&data[..take]);
            self.len += take;
            data = &data[take..];
            if self.len < 16 {
                return;
            }
            let b = self.buf;
            self.block(&b, 1);
            self.len = 0;
        }
        let mut chunks = data.chunks_exact(16);
        for c in &mut chunks {
            self.block(c.try_into().unwrap(), 1);
        }
        let rest = chunks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.len = rest.len();
    }

    pub fn finish(mut self) -> [u8; 16] {
        if self.len > 0 {
            // Last partial block: append the byte 1, pad with zeros, no 2^128 bit.
            let mut b = [0u8; 16];
            b[..self.len].copy_from_slice(&self.buf[..self.len]);
            b[self.len] = 1;
            self.block(&b, 0);
        }
        let mut h = self.h;
        // Full carry, so every limb is below 2^26.
        for i in 1..5 {
            h[i] += h[i - 1] >> 26;
            h[i - 1] &= M26;
        }
        h[0] += (h[4] >> 26) * 5;
        h[4] &= M26;
        h[1] += h[0] >> 26;
        h[0] &= M26;
        // g = h + 5 - 2^130; if it did not go negative, h >= p and the result is g. Chosen by mask.
        let mut g = [0u32; 5];
        let mut c = 5;
        for i in 0..4 {
            g[i] = h[i] + c;
            c = g[i] >> 26;
            g[i] &= M26;
        }
        g[4] = h[4].wrapping_add(c).wrapping_sub(1 << 26);
        let mask = (g[4] >> 31).wrapping_sub(1); // all ones when g >= 0
        for i in 0..5 {
            h[i] = (h[i] & !mask) | (g[i] & mask);
        }
        // h mod 2^128 as four 32-bit words, then + s mod 2^128.
        let w = [
            h[0] | (h[1] << 26),
            (h[1] >> 6) | (h[2] << 20),
            (h[2] >> 12) | (h[3] << 14),
            (h[3] >> 18) | (h[4] << 8),
        ];
        let mut out = [0u8; 16];
        let mut f = 0u64;
        for i in 0..4 {
            f = u64::from(w[i]) + u64::from(self.pad[i]) + (f >> 32);
            out[4 * i..4 * i + 4].copy_from_slice(&(f as u32).to_le_bytes());
        }
        out
    }

    pub fn mac(key: &[u8; 32], data: &[u8]) -> [u8; 16] {
        let mut p = Poly1305::new(key);
        p.update(data);
        p.finish()
    }
}
