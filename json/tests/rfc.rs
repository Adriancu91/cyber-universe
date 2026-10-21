//! JSON (RFC 8259) on the host: the RFC's examples, what must be refused (with the position), the limits, and a
//! cross-check against Python's `json` module on generated documents (when python3 is available).

use cyber_json::{Error, ErrorKind, MAX_DEPTH, Value, parse, parse_limited};

const RFC_IMAGE: &str = r#"{
  "Image": {
    "Width":  800,
    "Height": 600,
    "Title":  "View from 15th Floor",
    "Thumbnail": {
      "Url":    "http://www.example.com/image/481989943",
      "Height": 125,
      "Width":  100
    },
    "Animated" : false,
    "IDs": [116, 943, 234, 38793]
  }
}"#;

const RFC_LIST: &str = r#"[
  {
     "precision": "zip",
     "Latitude":  37.7668,
     "Longitude": -122.3959,
     "Address":   "",
     "City":      "SAN FRANCISCO",
     "State":     "CA",
     "Zip":       "94107",
     "Country":   "US"
  },
  {
     "precision": "zip",
     "Latitude":  37.371991,
     "Longitude": -122.026020,
     "Address":   "",
     "City":      "SUNNYVALE",
     "State":     "CA",
     "Zip":       "94085",
     "Country":   "US"
  }
]"#;

#[test]
fn rfc_examples() {
    let v = parse(RFC_IMAGE).unwrap();
    let img = v.get("Image").unwrap();
    assert_eq!(img.get("Width").unwrap().as_i64(), Some(800));
    assert_eq!(img.get("Title").unwrap().as_str(), Some("View from 15th Floor"));
    assert_eq!(img.get("Thumbnail").unwrap().get("Url").unwrap().as_str(), Some("http://www.example.com/image/481989943"));
    assert_eq!(img.get("Animated").unwrap().as_bool(), Some(false));
    assert_eq!(img.get("IDs").unwrap().at(3).unwrap().as_i64(), Some(38793));
    let l = parse(RFC_LIST).unwrap();
    assert_eq!(l.as_array().unwrap().len(), 2);
    assert_eq!(l.at(1).unwrap().get("Longitude").unwrap().as_f64(), Some(-122.026020));
    // Keys keep their order and numbers their exact text when written back.
    assert_eq!(
        parse(r#"{ "b" : 1.50, "a":[true,null,"x"] , "e":-0e+10}"#).unwrap().to_string(),
        r#"{"b":1.50,"a":[true,null,"x"],"e":-0e+10}"#
    );
    // Scalars are documents too (RFC 8259 §2).
    for (t, v) in [("\"Hello world!\"", Value::str("Hello world!")), ("42", Value::int(42)), ("true", Value::Bool(true)), (" null ", Value::Null)] {
        assert_eq!(parse(t).unwrap(), v);
    }
}

#[test]
fn strings() {
    let v = parse(r#""a\"b\\c\/d\b\f\n\r\t \u0041\u00e9\u021B \ud83d\ude00 ăîșț""#).unwrap();
    assert_eq!(v.as_str(), Some("a\"b\\c/d\u{8}\u{c}\n\r\t Aéț 😀 ăîșț"));
    // Written back: control characters escaped, the rest kept as UTF-8, and read again unchanged.
    let w = v.to_string();
    assert_eq!(w, "\"a\\\"b\\\\c/d\\u0008\\u000c\\n\\r\\t Aéț 😀 ăîșț\"");
    assert_eq!(parse(&w).unwrap(), v);
    let o = Value::object([("model", Value::str("claude")), ("max_tokens", Value::int(1024)), ("n", Value::int(-7))]);
    assert_eq!(o.to_string(), r#"{"model":"claude","max_tokens":1024,"n":-7}"#);
    // A repeated key: `get` gives the last value.
    assert_eq!(parse(r#"{"a":1,"a":2}"#).unwrap().get("a").unwrap().as_i64(), Some(2));
}

fn err(t: &str) -> Error {
    parse(t).expect_err(t)
}

#[test]
fn refused() {
    use ErrorKind::*;
    let cases: &[(&str, ErrorKind, usize)] = &[
        ("", UnexpectedEnd, 0),
        ("[1,2,]", Unexpected, 5),
        ("{\"a\":1,}", Unexpected, 7),
        ("{\"a\" 1}", Unexpected, 5),
        ("{a:1}", Unexpected, 1),
        ("[1 2]", Unexpected, 3),
        ("\"abc", UnexpectedEnd, 4),
        ("\"a\nb\"", ControlChar, 2),
        ("\"\\x\"", BadEscape, 2),
        ("\"\\u12g4\"", BadEscape, 5),
        ("\"\\ud800\"", BadSurrogate, 1),
        ("\"\\ud800\\u0041\"", BadSurrogate, 1),
        ("\"\\udc00\"", BadSurrogate, 1),
        ("01", BadNumber, 1),
        ("-", UnexpectedEnd, 1),
        ("1.", BadNumber, 2),
        ("1e", BadNumber, 2),
        ("+1", Unexpected, 0),
        (".5", Unexpected, 0),
        ("tru", UnexpectedEnd, 3),
        ("nul1", Unexpected, 0),
        ("[1] x", Trailing, 4),
        ("{} {}", Trailing, 3),
        ("'a'", Unexpected, 0),
        ("[NaN]", Unexpected, 1),
    ];
    for &(t, kind, at) in cases {
        assert_eq!(err(t), Error { kind, at }, "{t:?}");
    }
    // Depth: exactly MAX_DEPTH levels pass, one more is refused.
    let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
    assert!(parse(&ok).is_ok());
    let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
    assert_eq!(parse(&deep).unwrap_err().kind, TooDeep);
    assert_eq!(parse(&"{\"a\":".repeat(100)).unwrap_err().kind, TooDeep);
    assert_eq!(parse_limited("[1,2,3]", 6).unwrap_err().kind, TooLarge);
    assert!(parse_limited("[1,2,3]", 7).is_ok());
}

/// ~200 random documents written by Python's json (ensure_ascii on and off): our reading, written back and read by
/// Python again, must give the same data; and the error/success verdict on broken documents must match.
#[test]
fn python_cross_check() {
    let script = r#"
import json, random, sys
random.seed(8259)
def rnd(d):
    k = random.random()
    if d > 4 or k < 0.3:
        return random.choice([None, True, False, random.randint(-10**18, 10**18), random.random() * 10**random.randint(-5, 5),
                              ''.join(random.choice('aăîșțé"\\/\n\t\x01😀€ z{}[]:,') for _ in range(random.randint(0, 12)))])
    if k < 0.65:
        return [rnd(d + 1) for _ in range(random.randint(0, 5))]
    return {''.join(random.choice('abcșț"\\') for _ in range(random.randint(0, 5))): rnd(d + 1) for _ in range(random.randint(0, 5))}
for i in range(200):
    doc = json.dumps(rnd(0), ensure_ascii=(i % 2 == 0), indent=(None if i % 3 else 1))
    if i % 4 == 3:  # broken: cut it or change one character
        if random.random() < 0.5 and len(doc) > 1:
            doc = doc[:random.randrange(len(doc))]
        else:
            j = random.randrange(len(doc)); doc = doc[:j] + random.choice(',:[]{}"\\ x') + doc[j+1:]
    try:
        json.loads(doc); ok = 1
    except Exception:
        ok = 0
    sys.stdout.write(json.dumps([doc, ok]) + "\n")
"#;
    let Ok(out) = std::process::Command::new("python3").args(["-c", script]).output() else {
        println!("sarit: fara python3");
        return;
    };
    let text = String::from_utf8(out.stdout).unwrap();
    let mut written = String::new();
    let (mut n, mut good) = (0, 0);
    for line in text.lines() {
        let case = parse(line).unwrap();
        let doc = case.at(0).unwrap().as_str().unwrap();
        let ok = case.at(1).unwrap().as_i64() == Some(1);
        let ours = parse(doc);
        // Python accepts NaN/Infinity and lone surrogates; our generator never makes them, so verdicts must agree.
        assert_eq!(ours.is_ok(), ok, "verdict diferit pentru {doc:?}: {ours:?}");
        if let Ok(v) = ours {
            written.push_str(&Value::Array(vec![Value::str(doc), v]).to_string());
            written.push('\n');
            good += 1;
        }
        n += 1;
    }
    // Python reads what we wrote and compares it with its own reading of the original text.
    let check = r#"
import json, sys
bad = 0
for line in sys.stdin:
    doc, ours = json.loads(line)
    if json.loads(doc) != ours: bad += 1
print(bad)
"#;
    use std::io::Write;
    let mut child = std::process::Command::new("python3")
        .args(["-c", check])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(written.as_bytes()).unwrap();
    let res = child.wait_with_output().unwrap();
    let bad = String::from_utf8(res.stdout).unwrap();
    println!("documente: {n}, valide: {good}, diferente fata de Python: {}", bad.trim());
    assert!(n >= 200 && good >= 100);
    assert_eq!(bad.trim(), "0");
}
