//! A minimal strict DER reader (ITU-T X.690): one-byte tags, definite minimal lengths. Anything else is refused.

pub const BOOLEAN: u8 = 0x01;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;

/// Context-specific constructed tag [n].
pub const fn ctx(n: u8) -> u8 {
    0xa0 | n
}

#[derive(Clone, Copy)]
pub struct Der<'a> {
    rest: &'a [u8],
}

/// One element: its tag, its contents and the whole encoding (tag + length + contents).
#[derive(Clone, Copy)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub body: &'a [u8],
    pub raw: &'a [u8],
}

impl<'a> Der<'a> {
    pub fn new(data: &'a [u8]) -> Der<'a> {
        Der { rest: data }
    }

    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    pub fn peek_tag(&self) -> Option<u8> {
        self.rest.first().copied()
    }

    pub fn next(&mut self) -> Option<Tlv<'a>> {
        let d = self.rest;
        if d.len() < 2 || d[0] & 0x1f == 0x1f {
            return None;
        }
        // Long form only when needed, with no leading zero byte (minimal DER lengths), up to 3 bytes.
        let (len, head) = match d[1] {
            l @ 0..=0x7f => (l as usize, 2),
            l @ 0x81..=0x83 => {
                let n = (l & 0x7f) as usize;
                if d.len() < 2 + n || d[2] == 0 {
                    return None;
                }
                let len = d[2..2 + n].iter().fold(0usize, |a, &b| a << 8 | b as usize);
                if len < 0x80 {
                    return None;
                }
                (len, 2 + n)
            }
            _ => return None,
        };
        if d.len() - head < len {
            return None;
        }
        let tlv = Tlv { tag: d[0], body: &d[head..head + len], raw: &d[..head + len] };
        self.rest = &d[head + len..];
        Some(tlv)
    }

    /// The next element, which must have `tag`.
    pub fn expect(&mut self, tag: u8) -> Option<Tlv<'a>> {
        let t = self.next()?;
        (t.tag == tag).then_some(t)
    }

    /// The next element if it has `tag`; nothing is consumed otherwise.
    pub fn optional(&mut self, tag: u8) -> Option<Tlv<'a>> {
        if self.peek_tag() == Some(tag) { self.next() } else { None }
    }
}

/// A positive INTEGER's magnitude, without the sign byte. `None` for negative or non-minimal encodings.
pub fn uint(body: &[u8]) -> Option<&[u8]> {
    match body {
        [] => None,
        [b, ..] if b & 0x80 != 0 => None,
        [0, b, ..] if b & 0x80 == 0 => None,
        [0, rest @ ..] if !rest.is_empty() => Some(rest),
        _ => Some(body),
    }
}

/// BIT STRING contents with no unused bits.
pub fn bits(body: &[u8]) -> Option<&[u8]> {
    match body {
        [0, rest @ ..] => Some(rest),
        _ => None,
    }
}
