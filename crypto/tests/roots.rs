//! The real trusted roots (boot/certs/, from tools/roots.py): every one can be read, its file name matches its
//! subject (so /bin/fetch finds it), and its key is one we can use (P-521 is the only one we cannot).

use cyber_crypto::x509::{Cert, Key, subject_id};

#[test]
fn mozilla_roots() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../boot/certs");
    let (mut count, mut unusable) = (0, Vec::new());
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if name.ends_with(".md") {
            continue;
        }
        let der = std::fs::read(&path).unwrap();
        let cert = Cert::parse(&der).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let id = subject_id(cert.subject);
        assert_eq!(&name.as_bytes()[..16], &id[..], "{name}: numele nu se potriveste cu subject");
        assert!(cert.is_ca, "{name}: nu e autoritate");
        if matches!(cert.key, Key::Unknown) {
            unusable.push(name);
        }
        count += 1;
    }
    println!("radacini: {count}, cu chei pe care nu le folosim: {unusable:?}");
    assert!(count > 100, "prea putine radacini: {count}");
    assert!(unusable.len() <= 2, "chei necunoscute: {unusable:?}");
}
