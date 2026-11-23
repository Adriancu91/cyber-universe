//! P6-151 (C-161 point 5, second piece): the training engine's checkpoint, our own format, written with the safe rewrite
//! of the own disk (`fs::bucati::write_safe`: a power cut at any moment leaves a whole version). Kronos saves its line's
//! state at the end of every hour, atomically (`.part` then rename — hub `FISA_ANTRENARE.md` §4); this is the same promise
//! in the OS. The format (little-endian):
//!
//! ```text
//! "CYBKCK01"                        8 bytes
//! u32 meta count, then per entry:   u32 len + key (UTF-8), u32 len + value (UTF-8)
//! u32 tensor count, then per tensor: u32 len + name, u64 elements, elements × f32
//! SHA-256 of everything above        32 bytes
//! ```
//!
//! Reading checks the magic, every length against the file, and the SHA-256; any difference = refused.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &[u8; 8] = b"CYBKCK01";

/// A checkpoint: key/value notes (recipe, steps, data fingerprint, …) and named F32 tensors.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    pub meta: Vec<(String, String)>,
    pub tensors: Vec<(String, Vec<f32>)>,
}

fn sha256(b: &[u8]) -> [u8; 32] {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut h = Sha256::new();
    h.update(b);
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    out
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

pub fn encode(c: &Checkpoint) -> Vec<u8> {
    let elems: usize = c.tensors.iter().map(|(_, t)| t.len()).sum();
    let mut out = Vec::with_capacity(64 + 4 * elems + 64 * (c.meta.len() + c.tensors.len()));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(c.meta.len() as u32).to_le_bytes());
    for (k, v) in &c.meta {
        put_str(&mut out, k);
        put_str(&mut out, v);
    }
    out.extend_from_slice(&(c.tensors.len() as u32).to_le_bytes());
    for (name, t) in &c.tensors {
        put_str(&mut out, name);
        out.extend_from_slice(&(t.len() as u64).to_le_bytes());
        for x in t {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    let h = sha256(&out);
    out.extend_from_slice(&h);
    out
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("fisierul se termina prea devreme")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap_or([0; 4])))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap_or([0; 8])))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        core::str::from_utf8(s).map(String::from).map_err(|_| String::from("nume care nu e UTF-8"))
    }
}

pub fn decode(b: &[u8]) -> Result<Checkpoint, String> {
    if b.len() < MAGIC.len() + 8 + 32 || &b[..8] != MAGIC {
        return Err(String::from("nu e un punct de control CYBKCK01"));
    }
    let (body, sum) = b.split_at(b.len() - 32);
    if sha256(body)[..] != sum[..] {
        return Err(String::from("SHA-256 diferit (fisier stricat sau taiat)"));
    }
    let mut r = Reader { b: body, at: 8 };
    let mut meta = Vec::new();
    for _ in 0..r.u32()? {
        let k = r.string()?;
        let v = r.string()?;
        meta.push((k, v));
    }
    let mut tensors = Vec::new();
    for _ in 0..r.u32()? {
        let name = r.string()?;
        let n = r.u64()?;
        let bytes = r.take(usize::try_from(n).ok().and_then(|n| n.checked_mul(4)).ok_or("tensor prea mare")?)?;
        tensors.push((name, bytes.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()));
    }
    if r.at != body.len() {
        return Err(format!("{} octeti in plus dupa ultimul tensor", body.len() - r.at));
    }
    Ok(Checkpoint { meta, tensors })
}

/// Bit-for-bit equality (a NaN equals the same NaN; `==` on f32 would not say so).
pub fn same_bits(a: &Checkpoint, b: &Checkpoint) -> bool {
    a.meta == b.meta
        && a.tensors.len() == b.tensors.len()
        && a.tensors.iter().zip(&b.tensors).all(|((na, ta), (nb, tb))| {
            na == nb && ta.len() == tb.len() && ta.iter().zip(tb).all(|(x, y)| x.to_bits() == y.to_bits())
        })
}

/// Writes `c` to `path` with the safe rewrite, reads it back and checks it bit for bit. Returns (bytes, SHA-256 hex).
pub fn save(path: &str, c: &Checkpoint) -> Result<(usize, String), String> {
    let bytes = encode(c);
    // FAT32: one file below 4 GiB (the training engine's big states will go in chunks, `fs::bucati`).
    if bytes.len() as u64 >= crate::fs::bucati::CHUNK_MAX {
        return Err(format!("{} octeti: peste limita unui fisier FAT32", bytes.len()));
    }
    crate::fs::bucati::write_safe(path, &bytes).map_err(|e| format!("scrierea {}: {}", path, e.message()))?;
    let back = load(path)?;
    if !same_bits(c, &back) {
        return Err(format!("{}: recitit, dar diferit de ce s-a scris", path));
    }
    let h: String = sha256(&bytes).iter().map(|b| format!("{:02x}", b)).collect();
    Ok((bytes.len(), h))
}

/// Reads a checkpoint (after putting a cut rewrite back in order).
pub fn load(path: &str) -> Result<Checkpoint, String> {
    let _ = crate::fs::bucati::recover(path);
    let b = crate::fs::read_all(path).map_err(|e| format!("nu pot citi {}: {}", path, e.message()))?;
    decode(&b).map_err(|e| format!("{}: {}", path, e))
}

/// Checks without a disk: a round trip, a flipped bit, a cut file, a NaN kept bit for bit.
pub fn self_test() {
    let c = Checkpoint {
        meta: alloc::vec![(String::from("reteta"), String::from("sgd")), (String::from("pasi"), String::from("10"))],
        tensors: alloc::vec![(String::from("a"), alloc::vec![1.0, -2.5, f32::from_bits(0x7fc0_1234)]), (String::from("b"), Vec::new())],
    };
    let mut b = encode(&c);
    assert!(decode(&b).is_ok_and(|d| same_bits(&c, &d)), "test: punct de control: dus-intors");
    b[20] ^= 1;
    assert!(decode(&b).is_err(), "test: punct de control: un bit schimbat");
    let b = encode(&c);
    assert!(decode(&b[..b.len() - 5]).is_err(), "test: punct de control: taiat");
    crate::info!("test: punct de control: formatul CYBKCK01 OK");
}
