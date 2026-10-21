//! The official test vectors (FIPS 180-4 examples, RFC 4231, RFC 5869, RFC 6979, RFC 7748, RFC 8439, RFC 8448) and OpenSSL-made RSA signatures and certificates, run by `self_test`:
//! on the host by `cargo test`, in QEMU by the kernel's `crypto` scenario.

use crate::hkdf;
use crate::hmac::hmac;
use crate::sha2::{Sha256, Sha384, Sha512};
use crate::x25519::{x25519, x25519_base};
use crate::x509::{self, Error as XErr};
use crate::{aead, chacha20, ecp, p256, poly1305::Poly1305, rsa::PublicKey, vectors_rsa};
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

const SUNSCREEN: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

/// RFC 8439 A.3 #5-#11: (key = r || s, message, tag); they push h through the final reduction mod 2^130 - 5.
const POLY_EDGE: [(&str, &str, &str); 7] = [
    ("0200000000000000000000000000000000000000000000000000000000000000", "ffffffffffffffffffffffffffffffff", "03000000000000000000000000000000"),
    ("02000000000000000000000000000000ffffffffffffffffffffffffffffffff", "02000000000000000000000000000000", "03000000000000000000000000000000"),
    ("0100000000000000000000000000000000000000000000000000000000000000", "fffffffffffffffffffffffffffffffff0ffffffffffffffffffffffffffffff11000000000000000000000000000000", "05000000000000000000000000000000"),
    ("0100000000000000000000000000000000000000000000000000000000000000", "fffffffffffffffffffffffffffffffffbfefefefefefefefefefefefefefefe01010101010101010101010101010101", "00000000000000000000000000000000"),
    ("0200000000000000000000000000000000000000000000000000000000000000", "fdffffffffffffffffffffffffffffff", "faffffffffffffffffffffffffffffff"),
    ("0100000000000000040000000000000000000000000000000000000000000000", "e33594d7505e43b900000000000000003394d7505e4379cd01000000000000000000000000000000000000000000000001000000000000000000000000000000", "14000000000000005500000000000000"),
    ("0100000000000000040000000000000000000000000000000000000000000000", "e33594d7505e43b900000000000000003394d7505e4379cd010000000000000000000000000000000000000000000000", "13000000000000000000000000000000"),
];

/// RFC 6979 A.2.5 (P-256): (message, SHA-384?, DER signature).
const P256_SIGS: [(&[u8], bool, &str); 3] = [
    (b"sample", false, "3046022100efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716022100f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8"),
    (b"test", false, "3045022100f1abb023518351cd71d881567b1ea663ed3efcf6c5132b354f28d3b0b7d383670220019f4113742a2b14bd25926b49c649155f267e60d3814b4c0cc84250e46f0083"),
    (b"sample", true, "304402200eafea039b20e9b42309fb1d89e213057cbf973dc0cfc8f129edddc800ef771902204861f0491e6998b9455193e34e7b0d284ddd7149a74b95b9261f13abde940954"),
];

// Test certificates (P5-15), made with OpenSSL; see crypto/src/testcerts/README.md.
const CERT_ROOT_A: &[u8] = include_bytes!("testcerts/root_a.der");
const CERT_INTER_A: &[u8] = include_bytes!("testcerts/inter_a.der");
const CERT_LEAF_A: &[u8] = include_bytes!("testcerts/leaf_a.der");
const CERT_EXPIRED_A: &[u8] = include_bytes!("testcerts/expired_a.der");
const CERT_CRITEXT_A: &[u8] = include_bytes!("testcerts/critext_a.der");
const CERT_UNDER_LEAF: &[u8] = include_bytes!("testcerts/under_leaf.der");
const CERT_INTER2_A: &[u8] = include_bytes!("testcerts/inter2_a.der");
const CERT_DEEP_LEAF: &[u8] = include_bytes!("testcerts/deep_leaf.der");
const CERT_ROOT_B: &[u8] = include_bytes!("testcerts/root_b.der");
const CERT_LEAF_B: &[u8] = include_bytes!("testcerts/leaf_b.der");

/// ECDSA P-384 (P6-02): RFC 6979 A.2.6 key; deterministic (RFC 6979) signatures made with OpenSSL (Python `cryptography`).
const P384_Q: &str = "04ec3a4e415b4e19a4568618029f427fa5da9a8bc4ae92e02e06aae5286b300c64def8f0ea9055866064a254515480bc138015d9b72d7d57244ea8ef9ac0c621896708a59367f9dfb9f54ca84b3f1c9db1288b231c3ae0d4fe7344fd2533264720";
const P384_SIGS: [(&[u8], u16, &str); 6] = [
    (b"sample", 256, "3065023021b13d1e013c7fa1392d03c5f99af8b30c570c6f98d4ea8e354b63a21d3daa33bde1e888e63355d92fa2b3c36d8fb2cd023100f3aa443fb107745bf4bd77cb3891674632068a10ca67e3d45db2266fa7d1feebefdc63eccd1ac42ec0cb8668a4fa0ab0"),
    (b"sample", 384, "306602310094edbb92a5ecb8aad4736e56c691916b3f88140666ce9fa73d64c4ea95ad133c81a648152e44acf96e36dd1e80fabe4602310099ef4aeb15f178cea1fe40db2603138f130e740a19624526203b6351d0a3a94fa329c145786e679e7b82c71a38628ac8"),
    (b"sample", 512, "3065023100ed0959d5880ab2d869ae7f6c2915c6d60f96507f9cb3e047c0046861da4a799cfe30f35cc900056d7c99cd78824337090230512c8cceee3890a84058ce1e22dbc2198f42323ce8aca9135329f03c068e5112dc7cc3ef3446defceb01a45c2667fdd5"),
    (b"test", 256, "306402306d6defac9ab64dabafe36c6bf510352a4cc27001263638e5b16d9bb51d451559f918eedaf2293be5b475cc8f0188636b02302d46f3becbcc523d5f1a1256bf0c9b024d879ba9e838144c8ba6baeb4b53b47d51ab373f9845c0514eefb14024787265"),
    (b"test", 384, "30660231008203b63d3c853e8d77227fb377bcf7b7b772e97892a80f36ab775d509d7a5feb0542a7f0812998da8f1dd3ca3cf023db023100ddd0760448d42d8a43af45af836fce4de8be06b485e9b61b827c2f13173923e06a739f040649a667bf3b828246baa5a5"),
    (b"test", 512, "3066023100a0d5d090c9980faf3c2ce57b7ae951d31977dd11c775d314af55f76c676447d06fb6495cd21b4b6e340fc236584fb277023100976984e59b4c77b0e8e4460dca3d9f20e07b9bb1f63beefaf576f6b2e8b224634a2092cd3792e0159ad9cee37659c736"),
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

    // X25519 (RFC 7748 §5.2 and §6.1).
    let hex32 = |s: &str| {
        let mut b = [0u8; 32];
        unhex(s, &mut b);
        b
    };
    let r = x25519(&hex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"), &hex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c"));
    check(same(&r, "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"), "x25519 vector 1")?;
    let r = x25519(&hex32("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d"), &hex32("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493"));
    check(same(&r, "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"), "x25519 vector 2")?;
    let (mut k, mut u) = ([0u8; 32], [0u8; 32]);
    k[0] = 9;
    u[0] = 9;
    for i in 0..1000 {
        let next = x25519(&k, &u);
        u = k;
        k = next;
        if i == 0 {
            check(same(&k, "422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079"), "x25519 o iteratie")?;
        }
    }
    check(same(&k, "684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51"), "x25519 1000 de iteratii")?;
    let alice = hex32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let bob = hex32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
    let (pa, pb) = (x25519_base(&alice), x25519_base(&bob));
    check(same(&pa, "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"), "x25519 cheia publica Alice")?;
    check(same(&pb, "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"), "x25519 cheia publica Bob")?;
    let (sa, sb) = (x25519(&alice, &pb), x25519(&bob, &pa));
    check(sa == sb && same(&sa, "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"), "x25519 secret comun")?;

    // ChaCha20 (RFC 8439 §2.3.2, §2.4.2, A.1 #1 and #3).
    let k0: [u8; 32] = core::array::from_fn(|i| i as u8);
    let mut n12 = [0u8; 12];
    unhex("000000090000004a00000000", &mut n12);
    check(same(&chacha20::block(&k0, 1, &n12), "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4ed2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e"), "chacha20 bloc (2.3.2)")?;
    check(same(&chacha20::block(&[0; 32], 0, &[0; 12]), "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586"), "chacha20 bloc zero (A.1)")?;
    let mut k1 = [0u8; 32];
    k1[31] = 1;
    check(same(&chacha20::block(&k1, 1, &[0; 12]), "3aeb5224ecf849929b9d828db1ced4dd832025e8018b8160b82284f3c949aa5a8eca00bbb4a73bdad192b5c42f73f2fd4e273644c8b36125a64addeb006c13a0"), "chacha20 bloc cheie 1 (A.1)")?;
    let mut buf = [0u8; 128];
    let len = SUNSCREEN.len();
    buf[..len].copy_from_slice(SUNSCREEN);
    unhex("000000000000004a00000000", &mut n12);
    chacha20::xor(&k0, 1, &n12, &mut buf[..len]);
    check(same(&buf[..len], "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d"), "chacha20 criptare (2.4.2)")?;

    // Poly1305 (RFC 8439 §2.5.2, the reduction edge cases of A.3 #5-#11), then split input.
    let pk = hex32("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b");
    check(same(&Poly1305::mac(&pk, b"Cryptographic Forum Research Group"), "a8061dc1305136c6c22b8baf0c0127a9"), "poly1305 (2.5.2)")?;
    for (key, msg, want) in POLY_EDGE {
        let n = unhex(msg, &mut buf);
        check(same(&Poly1305::mac(&hex32(key), &buf[..n]), want), "poly1305 caz limita (A.3)")?;
    }
    let big: [u8; 1000] = core::array::from_fn(|i| (i * 7 + 3) as u8);
    let mut p = Poly1305::new(&pk);
    for part in big.chunks(13) {
        p.update(part);
    }
    check(same(&p.finish(), "1a0426ce6ec6e1323ffcca66044d6815"), "poly1305 pe bucati")?;

    // ChaCha20-Poly1305 AEAD (RFC 8439 §2.6.2 one-time key, §2.8.2), a 1000-byte message, tampering.
    let k8: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
    unhex("000000000001020304050607", &mut n12);
    check(same(&chacha20::block(&k8, 0, &n12)[..32], "8ad5a08b905f81cc815040274ab29471a833b637e3fd0da508dbb8e2fdd1a646"), "poly1305 cheia unica (2.6.2)")?;
    unhex("070000004041424344454647", &mut n12);
    let mut aad = [0u8; 12];
    unhex("50515253c0c1c2c3c4c5c6c7", &mut aad);
    buf[..len].copy_from_slice(SUNSCREEN);
    let tag = aead::seal(&k8, &n12, &aad, &mut buf[..len]);
    check(same(&buf[..len], "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116"), "aead text criptat (2.8.2)")?;
    check(same(&tag, "1ae10b594f09e26a7e902ecbd0600691"), "aead eticheta (2.8.2)")?;
    check(aead::open(&k8, &n12, &aad, &mut buf[..len], &tag) && &buf[..len] == SUNSCREEN, "aead deschidere")?;
    let mut msg = big;
    let aad2: [u8; 37] = core::array::from_fn(|i| i as u8);
    let tag = aead::seal(&k8, &n12, &aad2, &mut msg);
    check(same(&Sha256::digest(&msg), "5ba834c792459a1800e686605a5018ed7c1be47269247a01e00b3350c4c55184") && same(&tag, "f8d37df945a700d43a416719671d106f"), "aead 1000 de octeti")?;
    msg[500] ^= 0x10;
    let before = msg;
    check(!aead::open(&k8, &n12, &aad2, &mut msg, &tag) && msg == before, "aead: bit schimbat in text acceptat")?;
    msg[500] ^= 0x10;
    let mut bad = tag;
    bad[15] ^= 1;
    check(!aead::open(&k8, &n12, &aad2, &mut msg, &bad), "aead: eticheta gresita acceptata")?;
    check(!aead::open(&k8, &n12, &aad2[..36], &mut msg, &tag), "aead: date aditionale schimbate acceptate")?;
    check(aead::open(&k8, &n12, &aad2, &mut msg, &tag) && msg == big, "aead 1000 de octeti deschidere")?;

    // RSA (P5-13): PKCS#1 v1.5 and PSS, 2047 … 4096 bits, e = 65537 and e = 3; then everything that must be refused.
    for &(bits, n_hex, e_hex, sigs) in vectors_rsa::KEYS {
        let (mut n_b, mut e_b, mut sig) = ([0u8; 512], [0u8; 4], [0u8; 512]);
        let nl = unhex(n_hex, &mut n_b);
        let el = unhex(e_hex, &mut e_b);
        let Some(key) = PublicKey::new(&n_b[..nl], &e_b[..el]) else { return Err("rsa: cheie refuzata") };
        check(key.bits() == bits, "rsa: lungimea cheii")?;
        for &(scheme, hash, sig_hex) in sigs {
            let sl = unhex(sig_hex, &mut sig);
            let verify = |msg: &[u8], sig: &[u8]| match (scheme, hash) {
                ("v15", 256) => key.verify_pkcs1v15::<Sha256>(msg, sig),
                ("v15", 384) => key.verify_pkcs1v15::<Sha384>(msg, sig),
                ("v15", _) => key.verify_pkcs1v15::<Sha512>(msg, sig),
                (_, 256) => key.verify_pss::<Sha256>(msg, sig, 32),
                (_, 384) => key.verify_pss::<Sha384>(msg, sig, 48),
                _ => key.verify_pss::<Sha512>(msg, sig, 64),
            };
            check(verify(vectors_rsa::MSG, &sig[..sl]), if scheme == "v15" { "rsa pkcs1 v1.5" } else { "rsa pss" })?;
            let mut bad = sig;
            bad[sl / 2] ^= 4;
            let wrong_len = sl - 1;
            check(!verify(vectors_rsa::MSG, &bad[..sl]) && !verify(b"alt mesaj", &sig[..sl]) && !verify(vectors_rsa::MSG, &sig[1..sl]) && !verify(vectors_rsa::MSG, &n_b[..nl]) && !verify(vectors_rsa::MSG, &sig[..wrong_len]), "rsa: semnatura gresita acceptata")?;
        }
        if bits == 2048 && el == 1 && e_b[0] == 3 {
            // The wrong salt length or the other scheme with the same signature is refused too.
            let sl = unhex(sigs[0].2, &mut sig);
            check(!key.verify_pss::<Sha256>(vectors_rsa::MSG, &sig[..sl], 32) && !key.verify_pkcs1v15::<Sha384>(vectors_rsa::MSG, &sig[..sl]), "rsa: schema gresita acceptata")?;
        }
    }
    check(PublicKey::new(&[0xff; 64], &[1, 0, 1]).is_none() && PublicKey::new(&[0xfe; 256], &[1, 0, 1]).is_none() && PublicKey::new(&[0xff; 256], &[2]).is_none(), "rsa: cheie invalida acceptata")?;

    // ECDSA P-256 (P5-14): RFC 6979 A.2.5 key and signatures (SHA-256 and SHA-384), then what must be refused.
    let mut q = [0u8; 65];
    unhex("0460fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb67903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299", &mut q);
    let Some(key) = p256::PublicKey::new(&q) else { return Err("p256: cheie refuzata") };
    let mut der = [0u8; 72];
    for (msg, sha384, sig) in P256_SIGS {
        let dl = unhex(sig, &mut der);
        let ok = |m: &[u8], d: &[u8]| if sha384 { key.verify::<Sha384>(m, d) } else { key.verify::<Sha256>(m, d) };
        check(ok(msg, &der[..dl]), "ecdsa p256 (RFC 6979)")?;
        let mut bad = der;
        bad[dl - 3] ^= 0x20;
        check(!ok(msg, &bad[..dl]) && !ok(b"altceva", &der[..dl]), "ecdsa p256: semnatura gresita acceptata")?;
    }
    // r = 0, s = n, a point off the curve, a non-minimal DER integer.
    let order = hex32("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
    let h = Sha256::digest(b"sample");
    check(!key.verify_prehashed(&h, &[0; 32], &order) && !key.verify_prehashed(&h, &order, &hex32("01")), "ecdsa p256: r sau s in afara intervalului")?;
    q[64] ^= 1;
    check(p256::PublicKey::new(&q).is_none() && p256::PublicKey::new(&q[..33]).is_none(), "ecdsa p256: punct in afara curbei acceptat")?;
    let mut nm = [0u8; 8];
    unhex("3006020200010201", &mut nm);
    check(p256::parse_der_sig(&nm[..8]).is_none() && p256::parse_der_sig(&[0x30, 6, 2, 1, 1, 2, 1, 1]).is_some(), "ecdsa p256: DER necanonic acceptat")?;

    // ECDSA P-384 (P6-02): the same checks on the generic curve code (ecp.rs).
    let mut q384 = [0u8; 97];
    unhex(P384_Q, &mut q384);
    let Some(key) = ecp::PublicKey::new(&ecp::P384, &q384) else { return Err("p384: cheie refuzata") };
    let mut der = [0u8; 110];
    for (msg, hash, sig) in P384_SIGS {
        let dl = unhex(sig, &mut der);
        let ok = |m: &[u8], d: &[u8]| match hash {
            256 => key.verify::<Sha256>(m, d),
            384 => key.verify::<Sha384>(m, d),
            _ => key.verify::<Sha512>(m, d),
        };
        check(ok(msg, &der[..dl]), "ecdsa p384 (RFC 6979)")?;
        let mut bad = der;
        bad[dl - 3] ^= 0x20;
        check(!ok(msg, &bad[..dl]) && !ok(b"altceva", &der[..dl]), "ecdsa p384: semnatura gresita acceptata")?;
    }
    let n384 = ecp::P384.n;
    let mut order = [0u8; 48];
    for i in 0..6 {
        order[48 - 8 * (i + 1)..48 - 8 * i].copy_from_slice(&n384[i].to_be_bytes());
    }
    let mut one = [0u8; 48];
    one[47] = 1;
    let h = Sha384::digest(b"sample");
    check(!key.verify_prehashed(&h, &[0; 48], &order) && !key.verify_prehashed(&h, &order, &one), "ecdsa p384: r sau s in afara intervalului")?;
    q384[96] ^= 1;
    check(ecp::PublicKey::new(&ecp::P384, &q384).is_none() && ecp::PublicKey::new(&ecp::P384, &q384[..49]).is_none(), "ecdsa p384: punct in afara curbei acceptat")?;
    let mut rs = [0u8; 96];
    check(!ecp::parse_der_sig(&[0x30, 7, 2, 2, 0, 1, 2, 1, 1], 48, &mut rs) && ecp::parse_der_sig(&[0x30, 6, 2, 1, 1, 2, 1, 1], 48, &mut rs), "ecdsa p384: DER necanonic acceptat")?;

    // X.509 chains (P5-15): certificates made with OpenSSL; the good chains pass, each broken one is refused
    // for the right reason.
    let now = 20261020120000;
    let (ra, rb) = (CERT_ROOT_A, CERT_ROOT_B);
    let roots: &[&[u8]] = &[ra, rb];
    let chain = |c: &[&[u8]], host: &[u8]| x509::verify_chain(c, roots, host, now);
    check(chain(&[CERT_LEAF_A, CERT_INTER_A], b"test.cyber.local") == Ok(()), "x509 lant RSA-PSS + P-256")?;
    check(chain(&[CERT_LEAF_A, CERT_INTER_A, ra], b"Ceva.Exemplu.RO") == Ok(()), "x509 lant cu radacina si nume *.")?;
    check(chain(&[CERT_LEAF_B], b"server.b") == Ok(()), "x509 lant P-256 -> RSA (SHA-384)")?;
    check(chain(&[CERT_LEAF_A, CERT_INTER_A], b"exemplu.ro") == Err(XErr::NameMismatch)
        && chain(&[CERT_LEAF_A, CERT_INTER_A], b"a.b.exemplu.ro") == Err(XErr::NameMismatch)
        && chain(&[CERT_LEAF_A, CERT_INTER_A], b"server.b") == Err(XErr::NameMismatch), "x509: nume gresit acceptat")?;
    check(chain(&[CERT_LEAF_A], b"test.cyber.local") == Err(XErr::UnknownIssuer)
        && x509::verify_chain(&[CERT_LEAF_A, CERT_INTER_A], &[rb], b"test.cyber.local", now) == Err(XErr::UnknownIssuer), "x509: radacina necunoscuta acceptata")?;
    check(chain(&[CERT_EXPIRED_A, CERT_INTER_A], b"test.cyber.local") == Err(XErr::Expired)
        && x509::verify_chain(&[CERT_LEAF_A, CERT_INTER_A], roots, b"test.cyber.local", 20251231235959) == Err(XErr::NotYetValid), "x509: certificat expirat acceptat")?;
    check(chain(&[CERT_CRITEXT_A, CERT_INTER_A], b"test.cyber.local") == Err(XErr::CriticalExtension), "x509: extensie critica necunoscuta acceptata")?;
    check(chain(&[CERT_UNDER_LEAF, CERT_LEAF_A, CERT_INTER_A], b"rau.cyber.local") == Err(XErr::NotCa), "x509: certificat de server folosit ca autoritate")?;
    check(chain(&[CERT_DEEP_LEAF, CERT_INTER2_A, CERT_INTER_A], b"adanc.cyber.local") == Err(XErr::PathTooLong), "x509: lant prea lung acceptat")?;
    let mut bad = [0u8; 1024];
    let len = CERT_LEAF_A.len();
    bad[..len].copy_from_slice(CERT_LEAF_A);
    bad[len - 5] ^= 1; // inside the signature
    check(chain(&[&bad[..len], CERT_INTER_A], b"test.cyber.local") == Err(XErr::BadSignature), "x509: semnatura stricata acceptata")?;
    bad[len - 5] ^= 1;
    bad[60] ^= 0x40; // inside the signed part
    check(chain(&[&bad[..len], CERT_INTER_A], b"test.cyber.local").is_err(), "x509: certificat modificat acceptat")?;
    check(x509::Cert::parse(&CERT_LEAF_A[..len - 1]).is_err() && x509::Cert::parse(&[]).is_err(), "x509: certificat trunchiat acceptat")?;

    // Constant-time comparison.
    check(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"abc", b"ab"), "ct_eq")?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn official_vectors() {
        let r = super::self_test();
        assert!(matches!(r, Ok(n) if n > 30), "{:?}", r);
        println!("vectori: {:?}", r);
    }
}
