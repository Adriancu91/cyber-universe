//! The official test vectors (FIPS 180-4 examples, RFC 4231, RFC 5869, RFC 8448), run by `self_test`:
//! on the host by `cargo test`, in QEMU by the kernel's `crypto` scenario.

use crate::hkdf;
use crate::hmac::hmac;
use crate::sha2::{Sha256, Sha384, Sha512};
use crate::{Digest, ct_eq};

/// Hex text -> bytes in `out`; returns the length.
fn unhex(s: &str, out: &mut [u8]) -> usize {
    let v = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,
    };
    let b = s.as_bytes();
    for i in 0..b.len() / 2 {
        out[i] = v(b[2 * i]) << 4 | v(b[2 * i + 1]);
    }
    b.len() / 2
}

fn same(got: &[u8], hex: &str) -> bool {
    let mut want = [0u8; 128];
    let n = unhex(hex, &mut want);
    ct_eq(got, &want[..n])
}

fn million_a<D: Digest>(out: &mut [u8]) {
    let mut d = D::new();
    let chunk = [b'a'; 1000];
    for _ in 0..1000 {
        d.update(&chunk);
    }
    d.finish_into(out);
}

const LONG256: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
const LONG384: &[u8] = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";

/// HMAC test cases 1, 2, 3, 4, 6, 7 of RFC 4231: (key, data, SHA-256 tag, SHA-384 tag). Keys given as
/// (byte, count) or literal text.
const HMAC_CASES: [(&[u8], &[u8], &str, &str); 6] = [
    (&[0x0b; 20], b"Hi There",
     "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
     "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6"),
    (b"Jefe", b"what do ya want for nothing?",
     "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
     "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649"),
    (&[0xaa; 20], &[0xdd; 50],
     "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
     "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b2a5ab39dc13814b94e3ab6e101a34f27"),
    (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25], &[0xcd; 50],
     "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
     "3e8a69b7783c25851933ab6290af6ca77a9981480850009cc5577c6e1f573b4e6801dd23c4a7d679ccf8a386c674cffb"),
    (&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First",
     "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
     "4ece084485813e9088d2c63a041bc5b44f9ef1012a2b588f3cd11f05033ac4c60c2ef6ab4030fe8296248df163f44952"),
    (&[0xaa; 131], b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.",
     "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
     "6617178e941f020d351e2f254e8fd32c602420feb0b8fb9adccebb82461e99c5a678cc31e799176d3860e6110c46523e"),
];

/// Runs every vector. `Ok(count)` or `Err(name of the first one that failed)`.
pub fn self_test() -> Result<usize, &'static str> {
    let mut n = 0;
    let mut check = |ok: bool, name: &'static str| -> Result<(), &'static str> {
        n += 1;
        if ok { Ok(()) } else { Err(name) }
    };
    // SHA-2 (FIPS 180-4 examples).
    check(same(&Sha256::digest(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"), "sha256 gol")?;
    check(same(&Sha256::digest(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"), "sha256 abc")?;
    check(same(&Sha256::digest(LONG256), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"), "sha256 448 biti")?;
    let mut out = [0u8; 64];
    million_a::<Sha256>(&mut out);
    check(same(&out[..32], "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"), "sha256 milion de a")?;
    check(same(&Sha384::digest(b""), "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"), "sha384 gol")?;
    check(same(&Sha384::digest(b"abc"), "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"), "sha384 abc")?;
    check(same(&Sha384::digest(LONG384), "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"), "sha384 896 biti")?;
    million_a::<Sha384>(&mut out);
    check(same(&out[..48], "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985"), "sha384 milion de a")?;
    check(same(&Sha512::digest(b"abc"), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"), "sha512 abc")?;
    // Incremental input split at awkward places gives the same digest.
    let mut s = Sha256::new();
    for part in LONG256.chunks(7) {
        s.update(part);
    }
    check(same(&s.finish(), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"), "sha256 pe bucati")?;

    // HMAC (RFC 4231).
    for (key, data, t256, t384) in HMAC_CASES {
        hmac::<Sha256>(key, data, &mut out);
        check(same(&out[..32], t256), "hmac-sha256 (RFC 4231)")?;
        hmac::<Sha384>(key, data, &mut out);
        check(same(&out[..48], t384), "hmac-sha384 (RFC 4231)")?;
    }

    // HKDF (RFC 5869 cases 1-3, SHA-256).
    let mut prk = [0u8; 32];
    let mut okm = [0u8; 82];
    let salt1: [u8; 13] = core::array::from_fn(|i| i as u8);
    let info1: [u8; 10] = core::array::from_fn(|i| 0xf0 + i as u8);
    hkdf::extract::<Sha256>(&salt1, &[0x0b; 22], &mut prk);
    check(same(&prk, "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"), "hkdf caz 1 (extract)")?;
    check(hkdf::expand::<Sha256>(&prk, &info1, &mut okm[..42]), "hkdf caz 1 (lungime)")?;
    check(same(&okm[..42], "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"), "hkdf caz 1")?;
    let ikm2: [u8; 80] = core::array::from_fn(|i| i as u8);
    let salt2: [u8; 80] = core::array::from_fn(|i| 0x60 + i as u8);
    let info2: [u8; 80] = core::array::from_fn(|i| 0xb0 + i as u8);
    hkdf::extract::<Sha256>(&salt2, &ikm2, &mut prk);
    hkdf::expand::<Sha256>(&prk, &info2, &mut okm);
    check(same(&okm, "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71cc30c58179ec3e87c14c01d5c1f3434f1d87"), "hkdf caz 2")?;
    hkdf::extract::<Sha256>(&[], &[0x0b; 22], &mut prk);
    hkdf::expand::<Sha256>(&prk, &[], &mut okm[..42]);
    check(same(&okm[..42], "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"), "hkdf caz 3")?;
    check(!hkdf::expand::<Sha256>(&prk, &[], &mut [0u8; 255 * 32 + 1]), "hkdf: lungime prea mare acceptata")?;

    // TLS 1.3 key schedule start (RFC 8448 §3): early secret, then Derive-Secret(., "derived", "").
    hkdf::extract::<Sha256>(&[], &[0u8; 32], &mut prk);
    check(same(&prk, "33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a"), "tls13 early secret")?;
    let mut derived = [0u8; 32];
    check(hkdf::expand_label::<Sha256>(&prk, b"derived", &Sha256::digest(b""), &mut derived), "tls13 expand_label")?;
    check(same(&derived, "6f2615a108c702c5678f54fc9dbab69716c076189c48250cebeac3576c3611ba"), "tls13 derived")?;

    // Constant-time comparison.
    check(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"abc", b"ab"), "ct_eq")?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn official_vectors() {
        assert_eq!(super::self_test().map(|n| n > 30), Ok(true));
    }
}
