//! PNG pictures for the browser (P6-61 step 6, F6b): a DEFLATE decoder (RFC 1950 zlib wrapper, RFC 1951 DEFLATE) and
//! the PNG format (W3C "Portable Network Graphics Specification", second edition): all colour types, bit depths 1–16,
//! the five row filters, transparency composited over white. Interlaced (Adam7) pictures are refused with a clear
//! message. Written from the public specifications; no code from zlib, libpng or other decoders.
//! Included with `#[path]` by /bin/web; it only needs `alloc`.

use alloc::string::String;
use alloc::vec::Vec;

/// The largest picture decoded (pixels): 2048 x 2048.
pub const MAX_PIXELS: usize = 2048 * 2048;

/// A decoded picture: `pixels[y * width + x]` = 0x00RRGGBB.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
    /// "RGB 8 biti", "paleta 4 biti"...
    pub kind: String,
}

/// Reads bits least significant first, as DEFLATE stores them.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    have: u32,
}

impl Bits<'_> {
    fn bits(&mut self, n: u32) -> Result<u32, &'static str> {
        while self.have < n {
            let b = *self.data.get(self.pos).ok_or("date DEFLATE taiate")?;
            self.pos += 1;
            self.acc |= (b as u64) << self.have;
            self.have += 8;
        }
        let v = (self.acc & ((1u64 << n) - 1)) as u32;
        self.acc >>= n;
        self.have -= n;
        Ok(v)
    }

    /// Drops the bits left in the current byte (stored blocks start on a byte).
    fn align(&mut self) {
        let drop = self.have % 8;
        self.acc >>= drop;
        self.have -= drop;
    }
}

/// A canonical Huffman code (RFC 1951 section 3.2.2): how many codes of each length, and the symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, &'static str> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        // Over-subscribed codes are invalid; incomplete ones are allowed (a single distance code, for example).
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = left * 2 - c as i32;
            if left < 0 {
                return Err("cod Huffman invalid");
            }
        }
        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let mut symbols = alloc::vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = s as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    /// One symbol: the code is read bit by bit, most significant first, and compared with each length's range.
    fn decode(&self, b: &mut Bits) -> Result<u16, &'static str> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("cod Huffman necunoscut")
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] =
    [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order in which the code length code lengths are stored (RFC 1951 section 3.2.7).
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Inflates raw DEFLATE data; stops with an error past `limit` bytes of output.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    let mut b = Bits { data, pos: 0, acc: 0, have: 0 };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.bits(1)? == 1;
        match b.bits(2)? {
            0 => {
                b.align();
                let len = b.bits(16)?;
                let nlen = b.bits(16)?;
                if len != !nlen & 0xFFFF {
                    return Err("bloc DEFLATE necomprimat stricat");
                }
                for _ in 0..len {
                    out.push(b.bits(8)? as u8);
                }
            }
            1 => {
                let mut l = [0u8; 288];
                l[..144].fill(8);
                l[144..256].fill(9);
                l[256..280].fill(7);
                l[280..].fill(8);
                let lit = Huffman::new(&l)?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut b, &lit, &dist, &mut out, limit)?;
            }
            2 => {
                let nlen = b.bits(5)? as usize + 257;
                let ndist = b.bits(5)? as usize + 1;
                let ncode = b.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("bloc DEFLATE cu prea multe coduri");
                }
                let mut cl = [0u8; 19];
                for &i in CL_ORDER.iter().take(ncode) {
                    cl[i] = b.bits(3)? as u8;
                }
                let clh = Huffman::new(&cl)?;
                let mut lengths = alloc::vec![0u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = clh.decode(&mut b)?;
                    let (value, times) = match sym {
                        0..=15 => (sym as u8, 1),
                        16 => {
                            let prev = *lengths.get(i.wrapping_sub(1)).filter(|_| i > 0).ok_or("repetare fara lungime")?;
                            (prev, 3 + b.bits(2)? as usize)
                        }
                        17 => (0, 3 + b.bits(3)? as usize),
                        _ => (0, 11 + b.bits(7)? as usize),
                    };
                    if i + times > nlen + ndist {
                        return Err("prea multe lungimi de cod");
                    }
                    lengths[i..i + times].fill(value);
                    i += times;
                }
                if lengths[256] == 0 {
                    return Err("lipseste codul de sfarsit de bloc");
                }
                let lit = Huffman::new(&lengths[..nlen])?;
                let dist = Huffman::new(&lengths[nlen..])?;
                codes(&mut b, &lit, &dist, &mut out, limit)?;
            }
            _ => return Err("tip de bloc DEFLATE invalid"),
        }
        if out.len() > limit {
            return Err("imaginea are mai multe date decat spune antetul");
        }
        if last {
            return Ok(out);
        }
    }
}

/// The symbols of one compressed block, up to the end-of-block symbol.
fn codes(b: &mut Bits, lit: &Huffman, dist: &Huffman, out: &mut Vec<u8>, limit: usize) -> Result<(), &'static str> {
    loop {
        let sym = lit.decode(b)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = sym - 257;
            if i >= 29 {
                return Err("lungime DEFLATE invalida");
            }
            let len = LEN_BASE[i] as usize + b.bits(LEN_EXTRA[i] as u32)? as usize;
            let d = dist.decode(b)? as usize;
            if d >= 30 {
                return Err("distanta DEFLATE invalida");
            }
            let back = DIST_BASE[d] as usize + b.bits(DIST_EXTRA[d] as u32)? as usize;
            if back > out.len() {
                return Err("distanta DEFLATE prea mare");
            }
            let start = out.len() - back;
            for k in 0..len {
                out.push(out[start + k]);
            }
        }
        if out.len() > limit {
            return Err("imaginea are mai multe date decat spune antetul");
        }
    }
}

/// zlib (RFC 1950): the 2-byte header, DEFLATE data, then the Adler-32 sum of what came out.
pub fn zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    if data.len() < 6 || data[0] & 0x0F != 8 || (u16::from_be_bytes([data[0], data[1]]) % 31) != 0 {
        return Err("antet zlib invalid");
    }
    if data[1] & 0x20 != 0 {
        return Err("zlib cu dictionar (nu apare in PNG)");
    }
    let out = inflate(&data[2..], limit)?;
    let (mut a, mut s) = (1u32, 0u32);
    for &x in &out {
        a = (a + x as u32) % 65521;
        s = (s + a) % 65521;
    }
    let want = u32::from_be_bytes([data[data.len() - 4], data[data.len() - 3], data[data.len() - 2], data[data.len() - 1]]);
    if (s << 16 | a) != want {
        return Err("suma Adler-32 gresita (date stricate)");
    }
    Ok(out)
}

/// Whether the bytes start like a PNG file.
pub fn is_png(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Decodes a PNG file.
pub fn decode(data: &[u8]) -> Result<Picture, String> {
    if !is_png(data) {
        return Err(String::from("nu e un fisier PNG"));
    }
    let mut pos = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut alpha: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_header = false;
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let Some(body) = data.get(pos + 8..pos + 8 + len) else { return Err(String::from("fisier PNG taiat")) };
        pos += 12 + len;
        match kind {
            b"IHDR" if body.len() == 13 => {
                w = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
                h = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                (depth, ctype, interlace) = (body[8], body[9], body[12]);
                if body[10] != 0 || body[11] != 0 {
                    return Err(String::from("PNG cu metoda necunoscuta"));
                }
                seen_header = true;
            }
            b"PLTE" => palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"tRNS" => alpha = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
    }
    if !seen_header || w == 0 || h == 0 {
        return Err(String::from("PNG fara antet IHDR"));
    }
    if w > 2048 || h > 2048 || w * h > MAX_PIXELS {
        return Err(alloc::format!("imagine prea mare ({}x{}, cel mult 2048x2048)", w, h));
    }
    if interlace != 0 {
        return Err(String::from("PNG intretesut (Adam7) - inca nu stiu sa-l arat"));
    }
    let channels = match (ctype, depth) {
        (0, 1 | 2 | 4 | 8 | 16) => 1,
        (2, 8 | 16) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(alloc::format!("PNG cu tip de culoare {} si {} biti necunoscut", ctype, depth)),
    };
    if ctype == 3 && palette.is_empty() {
        return Err(String::from("PNG cu paleta, dar fara PLTE"));
    }
    let bits_px = channels * depth as usize;
    let stride = (w * bits_px).div_ceil(8);
    let bpp = bits_px.div_ceil(8).max(1);
    let mut raw = zlib(&idat, h * (stride + 1)).map_err(String::from)?;
    if raw.len() != h * (stride + 1) {
        return Err(alloc::format!("date PNG incomplete ({} din {} octeti)", raw.len(), h * (stride + 1)));
    }
    // Undo the row filters in place (each row starts with its filter type).
    for y in 0..h {
        let (done, rest) = raw.split_at_mut(y * (stride + 1));
        let prev: &[u8] = if y == 0 { &[] } else { &done[done.len() - stride..] };
        let filter = rest[0];
        let row = &mut rest[1..stride + 1];
        let up = |i: usize| if y == 0 { 0 } else { prev[i] };
        for i in 0..stride {
            let a = if i >= bpp { row[i - bpp] } else { 0 };
            let b = up(i);
            let c = if i >= bpp && y > 0 { prev[i - bpp] } else { 0 };
            let add = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(alloc::format!("filtru PNG necunoscut {} pe randul {}", filter, y)),
            };
            row[i] = row[i].wrapping_add(add);
        }
    }
    let mut pixels = alloc::vec![0u32; w * h];
    for y in 0..h {
        let row = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        // Sample `k` of the row (1–16 bits), scaled to 0–255; `raw_s` = the unscaled value (palette index).
        let sample = |k: usize| -> (u8, usize) {
            match depth {
                16 => (row[k * 2], u16::from_be_bytes([row[k * 2], row[k * 2 + 1]]) as usize),
                8 => (row[k], row[k] as usize),
                d => {
                    let d = d as usize;
                    let bit = k * d;
                    let v = (row[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1) as u8;
                    ((v as usize * 255 / ((1 << d) - 1)) as u8, v as usize)
                }
            }
        };
        for x in 0..w {
            let k = x * channels;
            let (r, g, b, a) = match ctype {
                0 => {
                    let (v, raw_v) = sample(k);
                    let transparent = alpha.len() >= 2 && raw_v == u16::from_be_bytes([alpha[0], alpha[1]]) as usize;
                    (v, v, v, if transparent { 0 } else { 255 })
                }
                2 => (sample(k).0, sample(k + 1).0, sample(k + 2).0, 255),
                3 => {
                    let i = sample(k).1;
                    let [r, g, b] = *palette.get(i).ok_or("PNG: indice in afara paletei")?;
                    (r, g, b, alpha.get(i).copied().unwrap_or(255))
                }
                4 => (sample(k).0, sample(k).0, sample(k).0, sample(k + 1).0),
                _ => (sample(k).0, sample(k + 1).0, sample(k + 2).0, sample(k + 3).0),
            };
            let over = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) & 0xFF;
            pixels[y * w + x] = over(r) << 16 | over(g) << 8 | over(b);
        }
    }
    let name = match ctype {
        0 => "gri",
        2 => "RGB",
        3 => "paleta",
        4 => "gri + alfa",
        _ => "RGBA",
    };
    Ok(Picture { width: w, height: h, pixels, kind: alloc::format!("{} {} biti", name, depth) })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// FNV-1a (64 bits) of the pixels, as little-endian 0x00RRGGBB words: the tests compare it.
pub fn checksum(p: &Picture) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for px in &p.pixels {
        for byte in px.to_le_bytes() {
            h = (h ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}
