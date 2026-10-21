//! ECDSA signature verification on the NIST prime curves with a = −3 (FIPS 186-5 §6.4.2, SEC 1, SP 800-186):
//! y² = x³ − 3x + b over a prime p. Field and scalar arithmetic in Montgomery form on L 64-bit limbs (4 for
//! P-256, 6 for P-384), points in Jacobian coordinates. Verification only works on public data, so it need
//! not be constant time. `p256` and `p384` give each curve its constants.

use crate::Digest;

/// The constants of one curve, as little-endian limbs.
pub struct Curve<const L: usize> {
    pub p: [u64; L],
    pub n: [u64; L],
    pub b: [u64; L],
    pub gx: [u64; L],
    pub gy: [u64; L],
}

/// Big-endian 8·L bytes -> little-endian limbs.
fn from_be<const L: usize>(b: &[u8]) -> [u64; L] {
    let n = 8 * L;
    core::array::from_fn(|i| u64::from_be_bytes(b[n - 8 - 8 * i..n - 8 * i].try_into().unwrap()))
}

fn one<const L: usize>() -> [u64; L] {
    core::array::from_fn(|i| u64::from(i == 0))
}

fn ge<const L: usize>(a: &[u64; L], b: &[u64; L]) -> bool {
    for i in (0..L).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn is_zero<const L: usize>(a: &[u64; L]) -> bool {
    a.iter().all(|&x| x == 0)
}

fn sub<const L: usize>(a: &[u64; L], b: &[u64; L]) -> ([u64; L], bool) {
    let mut r = [0u64; L];
    let mut borrow = false;
    for i in 0..L {
        let (d, b1) = a[i].overflowing_sub(b[i]);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        r[i] = d;
        borrow = b1 | b2;
    }
    (r, borrow)
}

fn add<const L: usize>(a: &[u64; L], b: &[u64; L]) -> ([u64; L], bool) {
    let mut r = [0u64; L];
    let mut carry = false;
    for i in 0..L {
        let (s, c1) = a[i].overflowing_add(b[i]);
        let (s, c2) = s.overflowing_add(u64::from(carry));
        r[i] = s;
        carry = c1 | c2;
    }
    (r, carry)
}

/// Arithmetic modulo an odd m of 64·L bits, values kept in Montgomery form (x·2^(64L) mod m).
struct Mod<const L: usize> {
    m: [u64; L],
    n0inv: u64,
    rr: [u64; L],
}

impl<const L: usize> Mod<L> {
    fn new(m: [u64; L]) -> Mod<L> {
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        // 2^(128L) mod m by doubling 1, 128·L times.
        let mut x = one::<L>();
        for _ in 0..128 * L {
            let (d, carry) = add(&x, &x);
            x = if carry || ge(&d, &m) { sub(&d, &m).0 } else { d };
        }
        Mod { m, n0inv: inv.wrapping_neg(), rr: x }
    }

    fn add(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let (s, carry) = add(a, b);
        if carry || ge(&s, &self.m) { sub(&s, &self.m).0 } else { s }
    }

    fn sub(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let (d, borrow) = sub(a, b);
        if borrow { add(&d, &self.m).0 } else { d }
    }

    /// a·b·2^(-64L) mod m (CIOS).
    fn mul(&self, a: &[u64; L], b: &[u64; L]) -> [u64; L] {
        let mut t = [0u64; 8]; // L + 2 words are used (L ≤ 6)
        for i in 0..L {
            let mut c: u128 = 0;
            for j in 0..L {
                let s = u128::from(t[j]) + u128::from(a[i]) * u128::from(b[j]) + c;
                t[j] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[L]) + c;
            t[L] = s as u64;
            t[L + 1] = (s >> 64) as u64;
            let q = t[0].wrapping_mul(self.n0inv);
            let mut c = (u128::from(t[0]) + u128::from(q) * u128::from(self.m[0])) >> 64;
            for j in 1..L {
                let s = u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + c;
                t[j - 1] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[L]) + c;
            t[L - 1] = s as u64;
            t[L] = t[L + 1] + (s >> 64) as u64;
        }
        let r: [u64; L] = core::array::from_fn(|i| t[i]);
        if t[L] != 0 || ge(&r, &self.m) { sub(&r, &self.m).0 } else { r }
    }

    fn to_mont(&self, a: &[u64; L]) -> [u64; L] {
        self.mul(a, &self.rr)
    }

    fn from_mont(&self, a: &[u64; L]) -> [u64; L] {
        self.mul(a, &one())
    }

    /// a^-1 = a^(m-2) (m prime), Montgomery in and out.
    fn inv(&self, a: &[u64; L]) -> [u64; L] {
        let mut two = [0u64; L];
        two[0] = 2;
        let e = sub(&self.m, &two).0;
        let mut x = self.to_mont(&one());
        for i in (0..64 * L).rev() {
            x = self.mul(&x, &x);
            if (e[i / 64] >> (i % 64)) & 1 == 1 {
                x = self.mul(&x, a);
            }
        }
        x
    }
}

/// A point (X, Y, Z) in Jacobian coordinates, Montgomery form: x = X/Z², y = Y/Z³; Z = 0 is infinity.
#[derive(Clone, Copy)]
struct Pt<const L: usize> {
    x: [u64; L],
    y: [u64; L],
    z: [u64; L],
}

impl<const L: usize> Pt<L> {
    fn inf() -> Pt<L> {
        Pt { x: [0; L], y: [0; L], z: [0; L] }
    }
}

fn dbl<const L: usize>(f: &Mod<L>, p: &Pt<L>) -> Pt<L> {
    if is_zero(&p.z) || is_zero(&p.y) {
        return Pt::inf();
    }
    // a = −3: alpha = 3(X − Z²)(X + Z²).
    let delta = f.mul(&p.z, &p.z);
    let gamma = f.mul(&p.y, &p.y);
    let beta = f.mul(&p.x, &gamma);
    let t = f.mul(&f.sub(&p.x, &delta), &f.add(&p.x, &delta));
    let alpha = f.add(&f.add(&t, &t), &t);
    let beta2 = f.add(&beta, &beta);
    let beta4 = f.add(&beta2, &beta2);
    let x3 = f.sub(&f.mul(&alpha, &alpha), &f.add(&beta4, &beta4));
    let yz = f.add(&p.y, &p.z);
    let z3 = f.sub(&f.sub(&f.mul(&yz, &yz), &gamma), &delta);
    let g2 = f.mul(&gamma, &gamma);
    let g4 = f.add(&g2, &g2);
    let g8 = f.add(&g4, &g4);
    let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &f.add(&g8, &g8));
    Pt { x: x3, y: y3, z: z3 }
}

fn add_pt<const L: usize>(f: &Mod<L>, a: &Pt<L>, b: &Pt<L>) -> Pt<L> {
    if is_zero(&a.z) {
        return *b;
    }
    if is_zero(&b.z) {
        return *a;
    }
    let z1z1 = f.mul(&a.z, &a.z);
    let z2z2 = f.mul(&b.z, &b.z);
    let u1 = f.mul(&a.x, &z2z2);
    let u2 = f.mul(&b.x, &z1z1);
    let s1 = f.mul(&a.y, &f.mul(&b.z, &z2z2));
    let s2 = f.mul(&b.y, &f.mul(&a.z, &z1z1));
    if u1 == u2 {
        return if s1 == s2 { dbl(f, a) } else { Pt::inf() };
    }
    let h = f.sub(&u2, &u1);
    let r = f.sub(&s2, &s1);
    let hh = f.mul(&h, &h);
    let hhh = f.mul(&h, &hh);
    let v = f.mul(&u1, &hh);
    let x3 = f.sub(&f.sub(&f.mul(&r, &r), &hhh), &f.add(&v, &v));
    let y3 = f.sub(&f.mul(&r, &f.sub(&v, &x3)), &f.mul(&s1, &hhh));
    let z3 = f.mul(&h, &f.mul(&a.z, &b.z));
    Pt { x: x3, y: y3, z: z3 }
}

/// A public key: a point checked to lie on the curve.
pub struct PublicKey<const L: usize> {
    curve: &'static Curve<L>,
    q: Pt<L>,
}

impl<const L: usize> PublicKey<L> {
    /// Uncompressed point 04 || x || y (1 + 16·L bytes).
    pub fn new(curve: &'static Curve<L>, sec1: &[u8]) -> Option<PublicKey<L>> {
        let s = 8 * L;
        if sec1.len() != 1 + 2 * s || sec1[0] != 4 {
            return None;
        }
        let (x, y) = (from_be::<L>(&sec1[1..1 + s]), from_be::<L>(&sec1[1 + s..]));
        if ge(&x, &curve.p) || ge(&y, &curve.p) {
            return None;
        }
        let f = Mod::new(curve.p);
        let (xm, ym) = (f.to_mont(&x), f.to_mont(&y));
        // y² == x³ − 3x + b
        let x3 = f.mul(&f.mul(&xm, &xm), &xm);
        let three_x = f.add(&f.add(&xm, &xm), &xm);
        let rhs = f.add(&f.sub(&x3, &three_x), &f.to_mont(&curve.b));
        if f.mul(&ym, &ym) != rhs {
            return None;
        }
        Some(PublicKey { curve, q: Pt { x: xm, y: ym, z: f.to_mont(&one()) } })
    }

    /// Verifies (r, s), 8·L big-endian bytes each, over `hash` (its leftmost 64·L bits are used; the curve
    /// orders here have exactly 64·L bits, so no bit shift is needed).
    pub fn verify_prehashed(&self, hash: &[u8], r: &[u8], s: &[u8]) -> bool {
        let size = 8 * L;
        if r.len() != size || s.len() != size || hash.is_empty() {
            return false;
        }
        let c = self.curve;
        let (r, s) = (from_be::<L>(r), from_be::<L>(s));
        if is_zero(&r) || is_zero(&s) || ge(&r, &c.n) || ge(&s, &c.n) {
            return false;
        }
        // e = leftmost min(hash, size) bytes as a number.
        let mut eb = [0u8; 48];
        let take = hash.len().min(size);
        eb[size - take..size].copy_from_slice(&hash[..take]);
        let mut e = from_be::<L>(&eb[..size]);
        if ge(&e, &c.n) {
            e = sub(&e, &c.n).0;
        }
        let fn_ = Mod::new(c.n);
        let w = fn_.inv(&fn_.to_mont(&s));
        let u1 = fn_.from_mont(&fn_.mul(&fn_.to_mont(&e), &w));
        let u2 = fn_.from_mont(&fn_.mul(&fn_.to_mont(&r), &w));
        // u1·G + u2·Q with one shared doubling chain (Shamir's trick).
        let f = Mod::new(c.p);
        let g = Pt { x: f.to_mont(&c.gx), y: f.to_mont(&c.gy), z: f.to_mont(&one()) };
        let gq = add_pt(&f, &g, &self.q);
        let mut acc = Pt::inf();
        for i in (0..64 * L).rev() {
            acc = dbl(&f, &acc);
            let (b1, b2) = ((u1[i / 64] >> (i % 64)) & 1, (u2[i / 64] >> (i % 64)) & 1);
            match (b1, b2) {
                (1, 1) => acc = add_pt(&f, &acc, &gq),
                (1, 0) => acc = add_pt(&f, &acc, &g),
                (0, 1) => acc = add_pt(&f, &acc, &self.q),
                _ => {}
            }
        }
        if is_zero(&acc.z) {
            return false;
        }
        let zi = f.inv(&acc.z);
        let x = f.from_mont(&f.mul(&acc.x, &f.mul(&zi, &zi)));
        let xn = if ge(&x, &c.n) { sub(&x, &c.n).0 } else { x };
        xn == r
    }

    /// Hashes `msg` with D and verifies a DER signature (SEQUENCE { INTEGER r, INTEGER s }), as in X.509 and TLS.
    pub fn verify<D: Digest>(&self, msg: &[u8], der: &[u8]) -> bool {
        let mut r = [0u8; 48];
        let mut s = [0u8; 48];
        let size = 8 * L;
        if !parse_der_sig(der, &mut r[..size], &mut s[..size]) {
            return false;
        }
        let mut h = [0u8; 64];
        let mut d = D::new();
        d.update(msg);
        d.finish_into(&mut h);
        self.verify_prehashed(&h[..D::LEN], &r[..size], &s[..size])
    }
}

/// Strict DER: 30 len (02 len r) (02 len s), minimal positive integers of at most `r.len()` bytes, written
/// right-aligned into `r` and `s` (which must be zero).
pub fn parse_der_sig(der: &[u8], r: &mut [u8], s: &mut [u8]) -> bool {
    let size = r.len();
    if der.len() < 8 || der[0] != 0x30 || der[1] as usize != der.len() - 2 || der[1] >= 0x80 {
        return false;
    }
    let mut rest = &der[2..];
    for v in [r, s] {
        if rest.len() < 3 || rest[0] != 2 {
            return false;
        }
        let len = rest[1] as usize;
        if len == 0 || len > size + 1 || rest.len() < 2 + len {
            return false;
        }
        let int = &rest[2..2 + len];
        // Negative numbers, and a leading zero that is not needed, are not DER.
        if int[0] & 0x80 != 0 || (len > 1 && int[0] == 0 && int[1] & 0x80 == 0) {
            return false;
        }
        let int = if int[0] == 0 { &int[1..] } else { int };
        if int.len() > size {
            return false;
        }
        v[size - int.len()..].copy_from_slice(int);
        rest = &rest[2 + len..];
    }
    rest.is_empty()
}
