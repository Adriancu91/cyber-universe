//! JSON (RFC 8259): `parse` reads a document into a `Value` tree, `Value::to_string` writes one back (compact).
//! `no_std` with `alloc`, no dependencies. Objects keep their keys in the order they were written; numbers keep
//! their exact text (so big integers and decimals are never rounded) and can be read as i64 or f64.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// The deepest nesting of arrays and objects accepted (protects the stack).
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// A number exactly as written (already checked against the JSON grammar).
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Key/value pairs in document order. A repeated key keeps its first place; `get` returns the last value.
    Object(Vec<(String, Value)>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ErrorKind {
    UnexpectedEnd,
    /// A character that cannot start or continue what was expected.
    Unexpected,
    BadNumber,
    BadEscape,
    /// A control character (below U+0020) inside a string.
    ControlChar,
    /// \uD800–\uDFFF not forming a valid pair.
    BadSurrogate,
    TooDeep,
    TooLarge,
    /// Something after the end of the document.
    Trailing,
}

/// What went wrong, and at which byte of the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub at: usize,
}

/// Reads a whole JSON document. `max_len` limits the text size (the caller knows its memory).
pub fn parse_limited(text: &str, max_len: usize) -> Result<Value, Error> {
    if text.len() > max_len {
        return Err(Error { kind: ErrorKind::TooLarge, at: max_len });
    }
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err(ErrorKind::Trailing));
    }
    Ok(v)
}

/// Reads a whole JSON document (at most 16 MiB).
pub fn parse(text: &str) -> Result<Value, Error> {
    parse_limited(text, 16 << 20)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, kind: ErrorKind) -> Error {
        Error { kind, at: self.i }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn literal(&mut self, word: &[u8], v: Value) -> Result<Value, Error> {
        if self.s[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else if word.starts_with(&self.s[self.i..]) {
            self.i = self.s.len();
            Err(self.err(ErrorKind::UnexpectedEnd))
        } else {
            Err(self.err(ErrorKind::Unexpected))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        match self.peek() {
            None => Err(self.err(ErrorKind::UnexpectedEnd)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'[') => {
                if depth >= MAX_DEPTH {
                    return Err(self.err(ErrorKind::TooDeep));
                }
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Array(items));
                        }
                        None => return Err(self.err(ErrorKind::UnexpectedEnd)),
                        _ => return Err(self.err(ErrorKind::Unexpected)),
                    }
                }
            }
            Some(b'{') => {
                if depth >= MAX_DEPTH {
                    return Err(self.err(ErrorKind::TooDeep));
                }
                self.i += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Value::Object(pairs));
                }
                loop {
                    self.ws();
                    match self.peek() {
                        Some(b'"') => {}
                        None => return Err(self.err(ErrorKind::UnexpectedEnd)),
                        _ => return Err(self.err(ErrorKind::Unexpected)),
                    }
                    let key = self.string()?;
                    self.ws();
                    match self.peek() {
                        Some(b':') => self.i += 1,
                        None => return Err(self.err(ErrorKind::UnexpectedEnd)),
                        _ => return Err(self.err(ErrorKind::Unexpected)),
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    pairs.push((key, v));
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Object(pairs));
                        }
                        None => return Err(self.err(ErrorKind::UnexpectedEnd)),
                        _ => return Err(self.err(ErrorKind::Unexpected)),
                    }
                }
            }
            _ => Err(self.err(ErrorKind::Unexpected)),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - start
    }

    /// number = [ minus ] int [ frac ] [ exp ]; int = zero / ( digit1-9 *DIGIT )
    fn number(&mut self) -> Result<Value, Error> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.err(ErrorKind::BadNumber));
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            None => return Err(self.err(ErrorKind::UnexpectedEnd)),
            _ => return Err(self.err(ErrorKind::BadNumber)),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.err(ErrorKind::BadNumber));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err(ErrorKind::BadNumber));
            }
        }
        // The bytes are ASCII digits and signs (checked above), so they are valid UTF-8.
        Ok(Value::Number(String::from(core::str::from_utf8(&self.s[start..self.i]).unwrap())))
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        if self.i + 4 > self.s.len() {
            self.i = self.s.len();
            return Err(self.err(ErrorKind::UnexpectedEnd));
        }
        let mut v = 0;
        for _ in 0..4 {
            let d = match self.s[self.i] {
                c @ b'0'..=b'9' => c - b'0',
                c @ b'a'..=b'f' => c - b'a' + 10,
                c @ b'A'..=b'F' => c - b'A' + 10,
                _ => return Err(self.err(ErrorKind::BadEscape)),
            };
            v = v * 16 + u32::from(d);
            self.i += 1;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, Error> {
        self.i += 1; // the opening quote
        let mut out = String::new();
        loop {
            // Copy a run of plain characters at once (the input is valid UTF-8: it came from a &str).
            let start = self.i;
            while let Some(c) = self.peek() {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(core::str::from_utf8(&self.s[start..self.i]).unwrap());
            match self.peek() {
                None => return Err(self.err(ErrorKind::UnexpectedEnd)),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(e) = self.peek() else { return Err(self.err(ErrorKind::UnexpectedEnd)) };
                    self.i += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let at = self.i - 2;
                            let hi = self.hex4()?;
                            let code = match hi {
                                0xD800..=0xDBFF => {
                                    if !self.s[self.i..].starts_with(b"\\u") {
                                        return Err(Error { kind: ErrorKind::BadSurrogate, at });
                                    }
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&lo) {
                                        return Err(Error { kind: ErrorKind::BadSurrogate, at });
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                }
                                0xDC00..=0xDFFF => return Err(Error { kind: ErrorKind::BadSurrogate, at }),
                                v => v,
                            };
                            char::from_u32(code).ok_or(Error { kind: ErrorKind::BadSurrogate, at })?
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.err(ErrorKind::BadEscape));
                        }
                    };
                    out.push(c);
                }
                Some(_) => return Err(self.err(ErrorKind::ControlChar)),
            }
        }
    }
}

impl Value {
    /// The value of `key` in an object (the last one, if repeated).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(pairs) => pairs.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The item `i` of an array.
    pub fn at(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self { Some(s) } else { None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self { Some(a) } else { None }
    }

    /// A number without fraction or exponent that fits in an i64.
    pub fn as_i64(&self) -> Option<i64> {
        let Value::Number(n) = self else { return None };
        n.parse().ok()
    }

    /// A number as the nearest f64 (core's correctly rounded decimal reader).
    pub fn as_f64(&self) -> Option<f64> {
        let Value::Number(n) = self else { return None };
        n.parse().ok()
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Builders for writing documents.
    pub fn str(s: &str) -> Value {
        Value::String(String::from(s))
    }

    pub fn int(n: i64) -> Value {
        let mut s = String::new();
        let _ = write!(s, "{}", n);
        Value::Number(s)
    }

    /// An object from (key, value) pairs.
    pub fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (String::from(k), v)).collect())
    }

    /// Writes the value as compact JSON (no spaces); non-ASCII text is written as UTF-8, control characters escaped.
    pub fn write_to(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(n),
            Value::String(s) => write_string(s, out),
            Value::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_to(out);
                }
                out.push(']');
            }
            Value::Object(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write_to(out);
                }
                out.push('}');
            }
        }
    }
}

impl core::fmt::Display for Value {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let mut s = String::new();
        self.write_to(&mut s);
        f.write_str(&s)
    }
}

/// A JSON string literal: quotes, backslashes and control characters escaped (RFC 8259 §7).
pub fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
