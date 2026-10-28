//! P6-27b: a JSON reader without a tree, for the big documents of the Kronos engine (the tokenizer: 16k merges; the
//! official vectors: 98k logits). The caller walks the document with `object` / `array` and reads the values it needs
//! straight from the text, so a number costs no allocation (our heap is first-fit: 100k small blocks are slow).
//! Only RFC 8259 JSON; at most `MAX_DEPTH` levels when skipping values.

use alloc::string::String;

const MAX_DEPTH: usize = 64;

pub struct Scan<'a> {
    s: &'a [u8],
    pub i: usize,
}

pub type R<T> = Result<T, String>;

impl<'a> Scan<'a> {
    pub fn new(text: &'a str) -> Scan<'a> {
        Scan { s: text.as_bytes(), i: 0 }
    }

    fn err<T>(&self, what: &str) -> R<T> {
        Err(alloc::format!("JSON: {} la octetul {}", what, self.i))
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> R<()> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&alloc::format!("astept `{}`", c as char))
        }
    }

    /// The end of the document (only spaces may follow).
    pub fn end(&mut self) -> R<()> {
        if self.peek().is_some() { self.err("text dupa sfarsit") } else { Ok(()) }
    }

    /// `{ "key": value, ... }`: `f` is called with each key and must read (or `skip`) its value.
    pub fn object(&mut self, mut f: impl FnMut(&mut Scan<'a>, &str) -> R<()>) -> R<()> {
        self.eat(b'{')?;
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(());
        }
        loop {
            let k = self.string()?;
            self.eat(b':')?;
            f(self, &k)?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return self.err("astept `,` sau `}`"),
            }
        }
    }

    /// `[ value, ... ]`: `f` is called for each element and must read (or `skip`) it.
    pub fn array(&mut self, mut f: impl FnMut(&mut Scan<'a>) -> R<()>) -> R<()> {
        self.eat(b'[')?;
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(());
        }
        loop {
            f(self)?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return self.err("astept `,` sau `]`"),
            }
        }
    }

    /// The text of a number, checked against the JSON grammar.
    pub fn number(&mut self) -> R<&'a str> {
        self.ws();
        let s = self.i;
        let b = self.s;
        let digits = |i: &mut usize| {
            let st = *i;
            while *i < b.len() && b[*i].is_ascii_digit() {
                *i += 1;
            }
            *i > st
        };
        let mut i = s;
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        if b.get(i) == Some(&b'0') {
            i += 1;
        } else if !digits(&mut i) {
            return self.err("numar gresit");
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            if !digits(&mut i) {
                return self.err("numar gresit");
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            if !digits(&mut i) {
                return self.err("numar gresit");
            }
        }
        self.i = i;
        Ok(core::str::from_utf8(&b[s..i]).unwrap_or("0"))
    }

    pub fn int(&mut self) -> R<i64> {
        let n = self.number()?;
        n.parse().or_else(|_| self.err("astept un numar intreg"))
    }

    pub fn float(&mut self) -> R<f32> {
        let n = self.number()?;
        n.parse().or_else(|_| self.err("numar gresit"))
    }

    fn hex4(&mut self) -> R<u32> {
        let h = self.s.get(self.i..self.i + 4).and_then(|h| core::str::from_utf8(h).ok());
        match h.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(v) => {
                self.i += 4;
                Ok(v)
            }
            None => self.err("\\u gresit"),
        }
    }

    pub fn string(&mut self) -> R<String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let st = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' && self.s[self.i] >= 0x20 {
                self.i += 1;
            }
            // The input is a &str and we stop only at ASCII bytes, so the slice is valid UTF-8.
            out.push_str(core::str::from_utf8(&self.s[st..self.i]).unwrap_or(""));
            match self.s.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let e = self.s.get(self.i).copied();
                    self.i += 1;
                    let c = match e {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.s.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return self.err("surogat fara pereche");
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("surogat fara pereche");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            match char::from_u32(cp) {
                                Some(c) => c,
                                None => return self.err("surogat fara pereche"),
                            }
                        }
                        _ => return self.err("escape gresit"),
                    };
                    out.push(c);
                }
                Some(_) => return self.err("caracter de control in text"),
                None => return self.err("text neterminat"),
            }
        }
    }

    /// Reads past any value.
    pub fn skip(&mut self) -> R<()> {
        self.skip_depth(0)
    }

    fn skip_depth(&mut self, depth: usize) -> R<()> {
        if depth > MAX_DEPTH {
            return self.err("prea adanc");
        }
        match self.peek() {
            Some(b'{') => self.object(|s, _| s.skip_depth(depth + 1)),
            Some(b'[') => self.array(|s| s.skip_depth(depth + 1)),
            Some(b'"') => self.string().map(|_| ()),
            Some(b't') | Some(b'f') | Some(b'n') => {
                for w in [&b"true"[..], b"false", b"null"] {
                    if self.s[self.i..].starts_with(w) {
                        self.i += w.len();
                        return Ok(());
                    }
                }
                self.err("valoare necunoscuta")
            }
            _ => self.number().map(|_| ()),
        }
    }
}
