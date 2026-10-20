//! Fixed-size big numbers for RSA verification (up to 4096 bits), no heap: little-endian 64-bit limbs,
//! Montgomery multiplication (CIOS) and modular exponentiation. Used with public values only.

pub const LIMBS: usize = 64; // 4096 bits
pub const MAX_BYTES: usize = LIMBS * 8;

#[derive(Clone, Copy)]
pub struct Num {
    pub l: [u64; LIMBS],
}

impl Num {
    pub const ZERO: Num = Num { l: [0; LIMBS] };

    /// Big-endian bytes (at most MAX_BYTES) -> number. `None` if too long.
    pub fn from_be(b: &[u8]) -> Option<Num> {
        if b.len() > MAX_BYTES {
            return None;
        }
        let mut n = Num::ZERO;
        for (i, &byte) in b.iter().rev().enumerate() {
            n.l[i / 8] |= u64::from(byte) << (8 * (i % 8));
        }
        Some(n)
    }

    /// Writes the low `out.len()` bytes, big-endian.
    pub fn to_be(&self, out: &mut [u8]) {
        let len = out.len();
        for (i, o) in out.iter_mut().enumerate() {
            let pos = len - 1 - i;
            *o = if pos < MAX_BYTES { (self.l[pos / 8] >> (8 * (pos % 8))) as u8 } else { 0 };
        }
    }

    /// Number of significant bits.
    pub fn bits(&self) -> usize {
        for i in (0..LIMBS).rev() {
            if self.l[i] != 0 {
                return 64 * i + 64 - self.l[i].leading_zeros() as usize;
            }
        }
        0
    }

    /// self >= other, over the first `len` limbs.
    fn ge(&self, other: &Num, len: usize) -> bool {
        for i in (0..len).rev() {
            if self.l[i] != other.l[i] {
                return self.l[i] > other.l[i];
            }
        }
        true
    }

    /// self -= other over `len` limbs; returns the borrow.
    fn sub_assign(&mut self, other: &Num, len: usize) -> u64 {
        let mut borrow = 0u64;
        for i in 0..len {
            let (d, b1) = self.l[i].overflowing_sub(other.l[i]);
            let (d, b2) = d.overflowing_sub(borrow);
            self.l[i] = d;
            borrow = u64::from(b1 | b2);
        }
        borrow
    }

    pub fn lt(&self, other: &Num) -> bool {
        !self.ge(other, LIMBS)
    }
}

/// A Montgomery context for an odd modulus n: R = 2^(64·len).
pub struct Mont {
    n: Num,
    len: usize,
    n0inv: u64, // -n^-1 mod 2^64
    rr: Num,    // R^2 mod n
}

impl Mont {
    /// `None` if n is even or below 3.
    pub fn new(n: &Num) -> Option<Mont> {
        if n.l[0] & 1 == 0 || n.bits() < 2 {
            return None;
        }
        let len = n.bits().div_ceil(64);
        // Newton iteration doubles the correct low bits each step: 1 -> 2 -> ... -> 64.
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(n.l[0].wrapping_mul(inv)));
        }
        // R^2 mod n by doubling 1 (2·64·len times), reducing after every step.
        let mut x = Num::ZERO;
        x.l[0] = 1;
        for _ in 0..2 * 64 * len {
            let top = x.l[len - 1] >> 63;
            for i in (1..len).rev() {
                x.l[i] = (x.l[i] << 1) | (x.l[i - 1] >> 63);
            }
            x.l[0] <<= 1;
            if top == 1 || x.ge(n, len) {
                x.sub_assign(n, len);
            }
        }
        Some(Mont { n: *n, len, n0inv: inv.wrapping_neg(), rr: x })
    }

    /// a·b·R^-1 mod n (CIOS). Inputs below n.
    pub fn mul(&self, a: &Num, b: &Num) -> Num {
        let len = self.len;
        let mut t = [0u64; LIMBS + 2];
        for i in 0..len {
            let mut c: u128 = 0;
            for j in 0..len {
                let s = u128::from(t[j]) + u128::from(a.l[i]) * u128::from(b.l[j]) + c;
                t[j] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[len]) + c;
            t[len] = s as u64;
            t[len + 1] = (s >> 64) as u64;
            let m = t[0].wrapping_mul(self.n0inv);
            let s = u128::from(t[0]) + u128::from(m) * u128::from(self.n.l[0]);
            let mut c = s >> 64;
            for j in 1..len {
                let s = u128::from(t[j]) + u128::from(m) * u128::from(self.n.l[j]) + c;
                t[j - 1] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[len]) + c;
            t[len - 1] = s as u64;
            t[len] = t[len + 1] + (s >> 64) as u64;
        }
        let mut r = Num::ZERO;
        r.l[..len].copy_from_slice(&t[..len]);
        // t < 2n: one conditional subtraction (also when the carry limb t[len] is set).
        if t[len] != 0 || r.ge(&self.n, len) {
            r.sub_assign(&self.n, len);
        }
        r
    }

    /// base^exp mod n; `exp` big-endian bytes. `base` must be below n.
    pub fn pow(&self, base: &Num, exp: &[u8]) -> Num {
        let b = self.mul(base, &self.rr); // into Montgomery form
        let mut one = Num::ZERO;
        one.l[0] = 1;
        let mut x = self.mul(&one, &self.rr); // R mod n
        for &byte in exp {
            for bit in (0..8).rev() {
                x = self.mul(&x, &x);
                if (byte >> bit) & 1 == 1 {
                    x = self.mul(&x, &b);
                }
            }
        }
        self.mul(&x, &one) // out of Montgomery form
    }
}
