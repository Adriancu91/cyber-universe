//! ECDSA signature verification on the NIST P-256 curve (FIPS 186-5 §6.4.2, SEC 1): y² = x³ − 3x + b over
//! p = 2^256 − 2^224 + 2^192 + 2^96 − 1. Field and scalar arithmetic in Montgomery form on 4×64-bit limbs,
//! points in Jacobian coordinates. Verification only works on public data, so it need not be constant time.

use crate::Digest;

type W = [u64; 4];

/// Big-endian 32 bytes -> little-endian limbs.
fn from_be(b: &[u8]) -> W {
    core::array::from_fn(|i| u64::from_be_bytes(b[24 - 8 * i..32 - 8 * i].try_into().unwrap()))
}

fn ge(a: &W, b: &W) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn is_zero(a: &W) -> bool {
    a.iter().all(|&x| x == 0)
}

fn sub(a: &W, b: &W) -> (W, bool) {
    let mut r = [0u64; 4];
    let mut borrow = false;
    for i in 0..4 {
        let (d, b1) = a[i].overflowing_sub(b[i]);
        let (d, b2) = d.overflowing_sub(u64::from(borrow));
        r[i] = d;
        borrow = b1 | b2;
    }
    (r, borrow)
}

fn add(a: &W, b: &W) -> (W, bool) {
    let mut r = [0u64; 4];
    let mut carry = false;
    for i in 0..4 {
        let (s, c1) = a[i].overflowing_add(b[i]);
        let (s, c2) = s.overflowing_add(u64::from(carry));
        r[i] = s;
        carry = c1 | c2;
    }
    (r, carry)
}

/// Arithmetic modulo an odd 256-bit m, values kept in Montgomery form (x·2^256 mod m).
struct Mod {
    m: W,
    n0inv: u64,
    rr: W,
}

impl Mod {
    fn new(m: W) -> Mod {
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        // 2^512 mod m by doubling 1, 512 times.
        let mut x: W = [1, 0, 0, 0];
        for _ in 0..512 {
            let (d, carry) = add(&x, &x);
            x = if carry || ge(&d, &m) { sub(&d, &m).0 } else { d };
        }
        Mod { m, n0inv: inv.wrapping_neg(), rr: x }
    }

    fn add(&self, a: &W, b: &W) -> W {
        let (s, carry) = add(a, b);
        if carry || ge(&s, &self.m) { sub(&s, &self.m).0 } else { s }
    }

    fn sub(&self, a: &W, b: &W) -> W {
        let (d, borrow) = sub(a, b);
        if borrow { add(&d, &self.m).0 } else { d }
    }

    /// a·b·2^-256 mod m (CIOS).
    fn mul(&self, a: &W, b: &W) -> W {
        let mut t = [0u64; 6];
        for i in 0..4 {
            let mut c: u128 = 0;
            for j in 0..4 {
                let s = u128::from(t[j]) + u128::from(a[i]) * u128::from(b[j]) + c;
                t[j] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[4]) + c;
            t[4] = s as u64;
            t[5] = (s >> 64) as u64;
            let q = t[0].wrapping_mul(self.n0inv);
            let mut c = (u128::from(t[0]) + u128::from(q) * u128::from(self.m[0])) >> 64;
            for j in 1..4 {
                let s = u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + c;
                t[j - 1] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[4]) + c;
            t[3] = s as u64;
            t[4] = t[5] + (s >> 64) as u64;
        }
        let r = [t[0], t[1], t[2], t[3]];
        if t[4] != 0 || ge(&r, &self.m) { sub(&r, &self.m).0 } else { r }
    }

    fn to_mont(&self, a: &W) -> W {
        self.mul(a, &self.rr)
    }

    fn from_mont(&self, a: &W) -> W {
        self.mul(a, &[1, 0, 0, 0])
    }

    /// a^-1 = a^(m-2) (m prime), Montgomery in and out.
    fn inv(&self, a: &W) -> W {
        let e = sub(&self.m, &[2, 0, 0, 0]).0;
        let mut x = self.to_mont(&[1, 0, 0, 0]);
        for i in (0..256).rev() {
            x = self.mul(&x, &x);
            if (e[i / 64] >> (i % 64)) & 1 == 1 {
                x = self.mul(&x, a);
            }
        }
        x
    }
}

const P: W = [0xffff_ffff_ffff_ffff, 0x0000_0000_ffff_ffff, 0, 0xffff_ffff_0000_0001];
const N: W = [0xf3b9_cac2_fc63_2551, 0xbce6_faad_a717_9e84, 0xffff_ffff_ffff_ffff, 0xffff_ffff_0000_0000];
const B: W = [0x3bce_3c3e_27d2_604b, 0x651d_06b0_cc53_b0f6, 0xb3eb_bd55_7698_86bc, 0x5ac6_35d8_aa3a_93e7];
const GX: W = [0xf4a1_3945_d898_c296, 0x7703_7d81_2deb_33a0, 0xf8bc_e6e5_63a4_40f2, 0x6b17_d1f2_e12c_4247];
const GY: W = [0xcbb6_4068_37bf_51f5, 0x2bce_3357_6b31_5ece, 0x8ee7_eb4a_7c0f_9e16, 0x4fe3_42e2_fe1a_7f9b];

/// A point (X, Y, Z) in Jacobian coordinates, Montgomery form: x = X/Z², y = Y/Z³; Z = 0 is infinity.
#[derive(Clone, Copy)]
struct Pt {
    x: W,
    y: W,
    z: W,
}

impl Pt {
    fn inf() -> Pt {
        Pt { x: [0; 4], y: [0; 4], z: [0; 4] }
    }
}

fn dbl(f: &Mod, p: &Pt) -> Pt {
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

fn add_pt(f: &Mod, a: &Pt, b: &Pt) -> Pt {
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

/// Public key = uncompressed point 04 || x || y (65 bytes), checked to lie on the curve.
pub struct PublicKey {
    q: Pt,
}

impl PublicKey {
    pub fn new(sec1: &[u8]) -> Option<PublicKey> {
        if sec1.len() != 65 || sec1[0] != 4 {
            return None;
        }
        let (x, y) = (from_be(&sec1[1..33]), from_be(&sec1[33..65]));
        if ge(&x, &P) || ge(&y, &P) {
            return None;
        }
        let f = Mod::new(P);
        let (xm, ym) = (f.to_mont(&x), f.to_mont(&y));
        // y² == x³ − 3x + b
        let x3 = f.mul(&f.mul(&xm, &xm), &xm);
        let three_x = f.add(&f.add(&xm, &xm), &xm);
        let rhs = f.add(&f.sub(&x3, &three_x), &f.to_mont(&B));
        if f.mul(&ym, &ym) != rhs {
            return None;
        }
        Some(PublicKey { q: Pt { x: xm, y: ym, z: f.to_mont(&[1, 0, 0, 0]) } })
    }

    /// Verifies (r, s), 32 big-endian bytes each, over `hash` (its leftmost 256 bits are used).
    pub fn verify_prehashed(&self, hash: &[u8], r: &[u8; 32], s: &[u8; 32]) -> bool {
        let (r, s) = (from_be(r), from_be(s));
        if is_zero(&r) || is_zero(&s) || ge(&r, &N) || ge(&s, &N) || hash.len() < 32 {
            return false;
        }
        let fn_ = Mod::new(N);
        let mut e = from_be(&hash[..32]);
        if ge(&e, &N) {
            e = sub(&e, &N).0;
        }
        let w = fn_.inv(&fn_.to_mont(&s));
        let u1 = fn_.from_mont(&fn_.mul(&fn_.to_mont(&e), &w));
        let u2 = fn_.from_mont(&fn_.mul(&fn_.to_mont(&r), &w));
        // u1·G + u2·Q with one shared doubling chain (Shamir's trick).
        let f = Mod::new(P);
        let one = f.to_mont(&[1, 0, 0, 0]);
        let g = Pt { x: f.to_mont(&GX), y: f.to_mont(&GY), z: one };
        let gq = add_pt(&f, &g, &self.q);
        let mut acc = Pt::inf();
        for i in (0..256).rev() {
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
        let xn = if ge(&x, &N) { sub(&x, &N).0 } else { x };
        xn == r
    }

    /// Hashes `msg` with D and verifies a DER signature (SEQUENCE { INTEGER r, INTEGER s }), as in X.509 and TLS.
    pub fn verify<D: Digest>(&self, msg: &[u8], der: &[u8]) -> bool {
        let Some((r, s)) = parse_der_sig(der) else { return false };
        let mut h = [0u8; 64];
        let mut d = D::new();
        d.update(msg);
        d.finish_into(&mut h);
        self.verify_prehashed(&h[..D::LEN], &r, &s)
    }
}

/// Strict DER: 30 len (02 len r) (02 len s), minimal positive integers of at most 32 bytes.
pub fn parse_der_sig(der: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    if der.len() < 8 || der[0] != 0x30 || der[1] as usize != der.len() - 2 || der[1] >= 0x80 {
        return None;
    }
    let mut rest = &der[2..];
    let mut out = [[0u8; 32]; 2];
    for v in out.iter_mut() {
        if rest.len() < 3 || rest[0] != 2 {
            return None;
        }
        let len = rest[1] as usize;
        if len == 0 || len > 33 || rest.len() < 2 + len {
            return None;
        }
        let int = &rest[2..2 + len];
        // Negative numbers, and a leading zero that is not needed, are not DER.
        if int[0] & 0x80 != 0 || (len > 1 && int[0] == 0 && int[1] & 0x80 == 0) {
            return None;
        }
        let int = if int[0] == 0 { &int[1..] } else { int };
        if int.len() > 32 {
            return None;
        }
        v[32 - int.len()..].copy_from_slice(int);
        rest = &rest[2 + len..];
    }
    if !rest.is_empty() {
        return None;
    }
    Some((out[0], out[1]))
}
