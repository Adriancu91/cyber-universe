//! A minimal PNG writer (P6-127, `captura` of the live channel): 8-bit grayscale, no compression ("stored" deflate
//! blocks), so it needs no compressor and stays small enough at a quarter of the screen. Format: the PNG specification
//! (W3C, 2003) and RFC 1950/1951 (zlib, deflate); our own code.

use alloc::vec::Vec;

/// CRC-32 (ISO-HDLC, the one PNG uses), bit by bit — a capture is ~130 KiB, so speed does not matter here.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &b in *part {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// A PNG of a `width` × `height` grayscale image (`pixels` = one byte per pixel, row after row).
pub fn gray(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    assert!(pixels.len() == width * height, "png: marimea imaginii");
    // The raw image: each row starts with filter type 0 (none).
    let mut raw = Vec::with_capacity((width + 1) * height);
    for row in pixels.chunks(width.max(1)).take(height) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    z.extend_from_slice(&[0x78, 0x01]); // deflate, 32 KiB window, no dictionary (31 divides 0x7801)
    let blocks: Vec<&[u8]> = if raw.is_empty() { alloc::vec![&raw[..]] } else { raw.chunks(65535).collect() };
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len())); // BFINAL on the last; BTYPE 00 = stored
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = Vec::with_capacity(z.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8 bits, grayscale, deflate, filter 0, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Self-test: the CRC-32 check value, Adler-32 of "Wikipedia", and a 3 × 2 image byte for byte.
#[cfg_attr(not(feature = "test-ghpush"), allow(dead_code))]
pub fn self_test() {
    assert!(crc32(&[b"123456789"]) == 0xCBF4_3926, "png: crc32");
    assert!(adler32(b"Wikipedia") == 0x11E6_0398, "png: adler32");
    let p = gray(3, 2, &[0, 128, 255, 1, 2, 3]);
    assert!(p.starts_with(b"\x89PNG\r\n\x1a\n") && &p[12..16] == b"IHDR" && p.ends_with(&[0xAE, 0x42, 0x60, 0x82]), "png: forma");
    // IDAT: zlib header, one final stored block of 8 bytes (2 rows × (1 + 3)), the rows, Adler-32.
    let idat = &p[33 + 8..33 + 8 + 2 + 5 + 8 + 4];
    assert!(idat[..7] == [0x78, 0x01, 1, 8, 0, 0xF7, 0xFF] && idat[7..15] == [0, 0, 128, 255, 0, 1, 2, 3], "png: idat {:?}", idat);
}
