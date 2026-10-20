//! X25519 (RFC 7748): Diffie-Hellman on Curve25519. Field elements modulo p = 2^255 - 19 are five 51-bit
//! limbs; the Montgomery ladder runs in constant time (the secret scalar never decides a branch or an address).

const MASK: u64 = (1 << 51) - 1;

#[derive(Clone, Copy)]
struct Fe([u64; 5]);

const ZERO: Fe = Fe([0; 5]);
const ONE: Fe = Fe([1, 0, 0, 0, 0]);

fn load8(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

impl Fe {
    /// The low 255 bits of `b` (little-endian); the top bit is ignored, as RFC 7748 asks.
    fn from_bytes(b: &[u8; 32]) -> Fe {
        Fe([
            load8(&b[0..]) & MASK,
            (load8(&b[6..]) >> 3) & MASK,
            (load8(&b[12..]) >> 6) & MASK,
            (load8(&b[19..]) >> 1) & MASK,
            (load8(&b[24..]) >> 12) & MASK,
        ])
    }

    /// Propagates the carries so every limb is below 2^51 (plus a little in limb 0).
    fn carry(mut self) -> Fe {
        let l = &mut self.0;
        for i in 0..4 {
            l[i + 1] += l[i] >> 51;
            l[i] &= MASK;
        }
        l[0] += 19 * (l[4] >> 51);
        l[4] &= MASK;
        l[1] += l[0] >> 51;
        l[0] &= MASK;
        self
    }

    fn add(self, b: Fe) -> Fe {
        Fe(core::array::from_fn(|i| self.0[i] + b.0[i])).carry()
    }

    /// self - b, computed as self + 2p - b so no limb goes below zero.
    fn sub(self, b: Fe) -> Fe {
        let two_p = [0xFFFFFFFFFFFDA, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE];
        Fe(core::array::from_fn(|i| self.0[i] + two_p[i] - b.0[i])).carry()
    }

    fn mul(self, b: Fe) -> Fe {
        let a = self.0.map(|x| x as u128);
        let b = b.0.map(|x| x as u128);
        let b19 = b.map(|x| x * 19);
        let r = [
            a[0] * b[0] + a[1] * b19[4] + a[2] * b19[3] + a[3] * b19[2] + a[4] * b19[1],
            a[0] * b[1] + a[1] * b[0] + a[2] * b19[4] + a[3] * b19[3] + a[4] * b19[2],
            a[0] * b[2] + a[1] * b[1] + a[2] * b[0] + a[3] * b19[4] + a[4] * b19[3],
            a[0] * b[3] + a[1] * b[2] + a[2] * b[1] + a[3] * b[0] + a[4] * b19[4],
            a[0] * b[4] + a[1] * b[3] + a[2] * b[2] + a[3] * b[1] + a[4] * b[0],
        ];
        let mut out = [0u64; 5];
        let mut c: u128 = 0;
        for i in 0..5 {
            let v = r[i] + c;
            out[i] = (v as u64) & MASK;
            c = v >> 51;
        }
        out[0] += (c as u64) * 19;
        Fe(out).carry()
    }

    fn square(self) -> Fe {
        self.mul(self)
    }

    /// self^(p-2) = 1/self (Fermat). p - 2 = 2^255 - 21: every bit from 254 down to 0 is 1 except bits 4 and 2.
    fn invert(self) -> Fe {
        let mut r = ONE;
        for i in (0..255).rev() {
            r = r.square();
            if i != 4 && i != 2 {
                r = r.mul(self);
            }
        }
        r
    }

    /// The unique representative below p, little-endian.
    fn to_bytes(self) -> [u8; 32] {
        let mut h = self.carry().carry().0;
        // If h >= p, then h + 19 overflows 2^255: q is 1 exactly then.
        let mut q = (h[0] + 19) >> 51;
        for limb in &h[1..] {
            q = (limb + q) >> 51;
        }
        h[0] += 19 * q;
        for i in 0..4 {
            h[i + 1] += h[i] >> 51;
            h[i] &= MASK;
        }
        h[4] &= MASK;
        let words = [h[0] | h[1] << 51, h[1] >> 13 | h[2] << 38, h[2] >> 26 | h[3] << 25, h[3] >> 39 | h[4] << 12];
        let mut out = [0u8; 32];
        for (o, w) in out.chunks_mut(8).zip(words) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// Swaps `a` and `b` if `swap` is 1, without a branch.
fn cswap(swap: u64, a: &mut Fe, b: &mut Fe) {
    let mask = 0u64.wrapping_sub(swap);
    for i in 0..5 {
        let t = mask & (a.0[i] ^ b.0[i]);
        a.0[i] ^= t;
        b.0[i] ^= t;
    }
}

/// X25519(scalar, u): the u-coordinate of scalar × the point with u-coordinate `u`.
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let x1 = Fe::from_bytes(u);
    let (mut x2, mut z2, mut x3, mut z3) = (ONE, ZERO, x1, ONE);
    let a24 = Fe([121665, 0, 0, 0, 0]);
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let bit = ((k[t / 8] >> (t % 8)) & 1) as u64;
        swap ^= bit;
        cswap(swap, &mut x2, &mut x3);
        cswap(swap, &mut z2, &mut z3);
        swap = bit;
        let a = x2.add(z2);
        let aa = a.square();
        let b = x2.sub(z2);
        let bb = b.square();
        let e = aa.sub(bb);
        let c = x3.add(z3);
        let d = x3.sub(z3);
        let da = d.mul(a);
        let cb = c.mul(b);
        x3 = da.add(cb).square();
        z3 = x1.mul(da.sub(cb).square());
        x2 = aa.mul(bb);
        z2 = e.mul(aa.add(a24.mul(e)));
    }
    cswap(swap, &mut x2, &mut x3);
    cswap(swap, &mut z2, &mut z3);
    x2.mul(z2.invert()).to_bytes()
}

/// Our public key for the private `scalar` (scalar × the base point u = 9).
pub fn x25519_base(scalar: &[u8; 32]) -> [u8; 32] {
    let mut nine = [0u8; 32];
    nine[0] = 9;
    x25519(scalar, &nine)
}
