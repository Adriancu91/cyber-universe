//! RFC 4648 §10 test vectors, then 300 random byte strings checked by round trip
//! (round trip; lengths 0..300).
use cyber_json::base64::{decode, encode};

#[test]
fn rfc4648_vectors() {
    for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
        assert_eq!(encode(plain.as_bytes()), coded);
        assert_eq!(decode(coded).unwrap(), plain.as_bytes());
    }
}

#[test]
fn round_trip_and_errors() {
    let mut x = 0x1234_5678u32;
    for len in 0..300usize {
        let data: Vec<u8> = (0..len).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect();
        let e = encode(&data);
        assert_eq!(e.len(), len.div_ceil(3) * 4);
        assert_eq!(decode(&e).unwrap(), data);
        // GitHub's form: lines of 60 characters.
        let lines: Vec<String> = e.as_bytes().chunks(60).map(|c| String::from_utf8(c.to_vec()).unwrap()).collect();
        assert_eq!(decode(&lines.join("\n")).unwrap(), data);
    }
    for bad in ["Zg=", "Z===", "Zg==Zg==", "Zm9*", "Zm9v\u{e9}"] {
        assert!(decode(bad).is_none(), "{bad}");
    }
}

#[test]
fn all_bytes_like_python() {
    // python3 -c "import base64; print(base64.b64encode(bytes(range(256))).decode())"
    let want = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w==";
    let data: Vec<u8> = (0..=255u8).collect();
    assert_eq!(encode(&data), want);
    assert_eq!(decode(want).unwrap(), data);
}
