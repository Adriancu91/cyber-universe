//! Text helpers for programs without a heap.

/// A line of text built without a heap.
pub struct Line {
    buf: [u8; 128],
    len: usize,
}

impl Line {
    pub fn new() -> Line {
        Line { buf: [0; 128], len: 0 }
    }
    pub fn push(&mut self, s: &str) -> &mut Line {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        self
    }
    pub fn num(&mut self, mut v: u64) -> &mut Line {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.push(core::str::from_utf8(&digits[i..]).unwrap_or("?"))
    }
    /// Spaces up to column `col`.
    pub fn pad(&mut self, col: usize) -> &mut Line {
        while self.len < col && self.len < self.buf.len() {
            self.buf[self.len] = b' ';
            self.len += 1;
        }
        self
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}
