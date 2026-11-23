//! P6-150 (C-161 point 5, C-158 point 10): Kronos's training data in the OS — the chunks `bucata_NNNNN.bin` (int16 or
//! int32 little-endian, no header) + `index.json`, the format of cyberspace `unelte/date_bucati.py` (hub
//! `integrare/kronos/FISA_ANTRENARE.md` §3). Checked exactly as Kronos's `module/date_bucati.py` does at start-up (read
//! only, no code copied): the index version / type / byte order, each chunk's start, size and SHA-256, the whole
//! sequence's SHA-256, the tokenizer's SHA-256; any difference = the data is refused. The fingerprint is Kronos's:
//! SHA-256 of `"<sha256_tot>:<tokenizer sha256>"`, first 16 hex digits. `slice` reads symbols [a, b) across chunks
//! (what the training engine will draw its windows from). Nothing is written anywhere.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// Where `antrenare date` looks for the data when no folder is given (the first with an `index.json`).
pub const DIRS: [&str; 2] = ["/disk/antrenare/date", "/usb/antrenare/date"];

/// The index version Kronos writes (`VERSIUNE_INDEX`).
const VERSION: i64 = 1;

/// Bytes read at a time while hashing (the chunks of #651 are 100 MB each: never whole in memory).
const BLOCK: usize = 1 << 20;

/// One chunk, as the index describes it.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub file: String,
    pub start: u64,
    pub symbols: u64,
    pub sha256: String,
}

/// `index.json`, the fields the check needs.
#[derive(Clone, Debug)]
pub struct Index {
    pub dir: String,
    /// 2 (int16) or 4 (int32) bytes per symbol.
    pub width: u64,
    pub type_name: String,
    pub total: u64,
    pub sha256_tot: String,
    pub partial: bool,
    pub tokenizer_sha256: Option<String>,
    pub chunks: Vec<Chunk>,
}

/// Reads and parses `<dir>/index.json` (no chunk is opened).
pub fn read_index(dir: &str) -> Result<Index, String> {
    let path = format!("{}/index.json", dir);
    let bytes = crate::fs::read_all(&path).map_err(|e| format!("nu pot citi {}: {}", path, e.message()))?;
    let text = core::str::from_utf8(&bytes).map_err(|_| format!("{}: nu e text UTF-8", path))?;
    let v = cyber_json::parse_limited(text, 4 << 20).map_err(|e| format!("{}: JSON gresit ({:?})", path, e))?;
    let int = |k: &str| v.get(k).and_then(|x| x.as_i64());
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    if int("versiune") != Some(VERSION) {
        return Err(format!("index.json: versiune necunoscuta: {:?}", int("versiune")));
    }
    let type_name = s("tip").unwrap_or_default();
    let width = match type_name.as_str() {
        "int16" => 2,
        "int32" => 4,
        _ => 0,
    };
    if width == 0 || s("ordine_octeti").as_deref() != Some("little") {
        return Err(format!("index.json: tip sau ordine de octeti necunoscute: {:?}, {:?}", type_name, s("ordine_octeti")));
    }
    let total = int("simboluri_total").filter(|x| *x >= 0).ok_or("index.json: lipseste simboluri_total")? as u64;
    let sha256_tot = s("sha256_tot").ok_or("index.json: lipseste sha256_tot")?;
    let partial = v.get("partial").and_then(|x| x.as_bool()).unwrap_or(false);
    let tokenizer_sha256 = v.get("tokenizer").and_then(|t| t.get("sha256")).and_then(|x| x.as_str()).map(String::from);
    let mut chunks = Vec::new();
    for (k, b) in v.get("bucati").and_then(|x| x.as_array()).ok_or("index.json: lipseste lista bucati")?.iter().enumerate() {
        let f = |key: &str| b.get(key).and_then(|x| x.as_str()).map(String::from);
        let n = |key: &str| b.get(key).and_then(|x| x.as_i64()).filter(|x| *x >= 0).map(|x| x as u64);
        let (Some(file), Some(start), Some(symbols), Some(sha256)) = (f("fisier"), n("inceput"), n("simboluri"), f("sha256")) else {
            return Err(format!("index.json: bucata {} incompleta", k));
        };
        // A file name only (no folder, no `..`): the chunks live next to the index.
        if file.contains('/') || file.contains('\\') || file == ".." || file.is_empty() {
            return Err(format!("index.json: numele bucatii {} nu e un nume simplu: {}", k, file));
        }
        chunks.push(Chunk { file, start, symbols, sha256 });
    }
    Ok(Index { dir: String::from(dir), width, type_name, total, sha256_tot, partial, tokenizer_sha256, chunks })
}

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// SHA-256 of a file read in blocks; every block also goes to `all` when given (the whole sequence's hash).
fn sha256_file(path: &str, mut all: Option<&mut cyber_crypto::sha2::Sha256>) -> Result<String, String> {
    use cyber_crypto::{sha2::Sha256, Digest};
    let size = crate::fs::size(path).map_err(|e| format!("{}: {}", path, e.message()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; BLOCK];
    let mut at = 0u64;
    while at < size {
        let want = core::cmp::min(BLOCK as u64, size - at) as usize;
        let got = crate::fs::read(path, at, &mut buf[..want]).map_err(|e| format!("{}: {}", path, e.message()))?;
        if got == 0 {
            return Err(format!("{}: citire scurta la {}", path, at));
        }
        h.update(&buf[..got]);
        if let Some(a) = all.as_deref_mut() {
            a.update(&buf[..got]);
        }
        at += got as u64;
    }
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    Ok(hex(&out))
}

/// The outcome of the start-up check: the wrong things (file, problem), as Kronos lists them.
pub struct Check {
    pub wrong: Vec<(String, String)>,
    pub bytes: u64,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.wrong.is_empty()
    }
}

/// Kronos's start-up check (`DateBucati._verifica`): every chunk's start, size and SHA-256, the total, the whole
/// sequence's SHA-256 (only when every chunk is right), the tokenizer (when a path is given).
pub fn check(ix: &Index, tokenizer: Option<&str>) -> Check {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut wrong: Vec<(String, String)> = Vec::new();
    let mut bad = |f: &str, p: String| wrong.push((String::from(f), p));
    if ix.partial {
        bad("index.json", String::from("indexul e partial (lipsesc bucati la coada)"));
    }
    let mut all = Sha256::new();
    let mut expected = 0u64;
    let mut bytes = 0u64;
    for c in &ix.chunks {
        let path = format!("{}/{}", ix.dir, c.file);
        if c.start != expected {
            bad(&c.file, format!("incepe la {}, trebuia {}", c.start, expected));
        }
        expected = c.start + c.symbols;
        let size = match crate::fs::size(&path) {
            Ok(s) => s,
            Err(_) => {
                bad(&c.file, String::from("lipseste"));
                continue;
            }
        };
        if size != c.symbols * ix.width {
            bad(&c.file, format!("alta marime decat in index ({} octeti, asteptat {})", size, c.symbols * ix.width));
            continue;
        }
        match sha256_file(&path, Some(&mut all)) {
            Ok(h) if h == c.sha256 => bytes += size,
            Ok(_) => bad(&c.file, String::from("SHA diferit")),
            Err(e) => bad(&c.file, e),
        }
    }
    if expected != ix.total {
        bad("index.json", format!("bucatile au {} simboluri, indexul zice {}", expected, ix.total));
    }
    let mut out = [0u8; 32];
    all.finish_into(&mut out);
    let no_wrong = wrong.is_empty();
    if no_wrong && hex(&out) != ix.sha256_tot {
        wrong.push((String::from("(tot sirul)"), String::from("SHA diferit")));
    }
    if let Some(t) = tokenizer {
        match (&ix.tokenizer_sha256, crate::fs::size(t)) {
            (_, Err(_)) => wrong.push((String::from(t), String::from("tokenizerul lipseste"))),
            (None, Ok(_)) => wrong.push((String::from("index.json"), String::from("indexul nu spune cu ce tokenizer s-au facut simbolurile"))),
            (Some(want), Ok(_)) => match sha256_file(t, None) {
                Ok(h) if &h == want => {}
                Ok(_) => wrong.push((String::from(t), String::from("alt tokenizer decat cel cu care s-au facut simbolurile"))),
                Err(e) => wrong.push((String::from(t), e)),
            },
        }
    }
    Check { wrong, bytes }
}

/// Kronos's data fingerprint (`DateBucati.amprenta`): from the index alone.
pub fn fingerprint(ix: &Index) -> String {
    let t = ix.tokenizer_sha256.as_deref().unwrap_or("");
    let h = super::antrenare::sha256_hex(format!("{}:{}", ix.sha256_tot, t).as_bytes());
    h[..16].to_string()
}

/// The symbols [a, b) (clamped to the sequence), read across chunks — `SirBucati.felie`. A negative int16 (a symbol
/// Kronos's vocabulary never has) is an error.
pub fn slice(ix: &Index, a: u64, b: u64) -> Result<Vec<u32>, String> {
    let (a, b) = (a.min(ix.total), b.min(ix.total));
    let mut out = Vec::with_capacity(b.saturating_sub(a) as usize);
    for c in &ix.chunks {
        let (lo, hi) = (a.max(c.start), b.min(c.start + c.symbols));
        if lo >= hi {
            continue;
        }
        let path = format!("{}/{}", ix.dir, c.file);
        let mut buf = vec![0u8; ((hi - lo) * ix.width) as usize];
        let got = crate::fs::read(&path, (lo - c.start) * ix.width, &mut buf).map_err(|e| format!("{}: {}", path, e.message()))?;
        if got != buf.len() {
            return Err(format!("{}: citire scurta", path));
        }
        for w in buf.chunks(ix.width as usize) {
            let v = if ix.width == 2 { i16::from_le_bytes([w[0], w[1]]) as i64 } else { i32::from_le_bytes([w[0], w[1], w[2], w[3]]) as i64 };
            if v < 0 {
                return Err(format!("{}: simbol negativ {}", path, v));
            }
            out.push(v as u32);
        }
    }
    if out.len() as u64 != b.saturating_sub(a) {
        return Err(format!("felia [{}, {}) a dat {} simboluri", a, b, out.len()));
    }
    Ok(out)
}

/// `antrenare date [dosar] [--tokenizer <cale>] [--felie <a> <n>]`: the check, the fingerprint, optionally a slice.
pub fn command(args: &[&str]) -> Vec<String> {
    let mut rows = Vec::new();
    let (mut dir, mut tok, mut sl): (Option<&str>, Option<&str>, Option<(u64, u64)>) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--tokenizer" if i + 1 < args.len() => {
                tok = Some(args[i + 1]);
                i += 2;
            }
            "--felie" if i + 2 < args.len() => match (args[i + 1].parse::<u64>(), args[i + 2].parse::<u64>()) {
                (Ok(a), Ok(n)) if n <= 4096 => {
                    sl = Some((a, n));
                    i += 3;
                }
                _ => return vec![String::from("antrenare date: --felie <inceput> <numar> (numar cel mult 4096)")],
            },
            x if !x.starts_with("--") && dir.is_none() => {
                dir = Some(x);
                i += 1;
            }
            _ => return vec![String::from("folosire: antrenare date [dosar] [--tokenizer <cale>] [--felie <inceput> <numar>]")],
        }
    }
    let dir = match dir {
        Some(d) => String::from(d.trim_end_matches('/')),
        None => match DIRS.iter().find(|d| crate::fs::size(&format!("{}/index.json", d)).is_ok()) {
            Some(d) => String::from(*d),
            None => return vec![format!("antrenare date: NU A RULAT: niciun index.json in {} sau {}", DIRS[0], DIRS[1])],
        },
    };
    let ix = match read_index(&dir) {
        Ok(ix) => ix,
        Err(e) => return vec![format!("antrenare date: REFUZAT: {}", e)],
    };
    rows.push(format!(
        "antrenare date: {}: index versiunea {}, {} little-endian, {} bucati, {} simboluri, sha256_tot {}",
        dir,
        VERSION,
        ix.type_name,
        ix.chunks.len(),
        ix.total,
        ix.sha256_tot
    ));
    let t0 = crate::time::since_boot_us();
    let c = check(&ix, tok);
    let ms = t0.zip(crate::time::since_boot_us()).map_or(0, |(a, b)| b.saturating_sub(a) / 1000);
    for (f, p) in &c.wrong {
        rows.push(format!("antrenare date: GRESIT: {}: {}", f, p));
    }
    if c.ok() {
        rows.push(format!(
            "antrenare date: TRECE verificarea (ca Kronos la pornire): {} bucati, {} octeti verificati in {} ms; tokenizer: {}; amprenta {}",
            ix.chunks.len(),
            c.bytes,
            ms,
            if tok.is_some() { "verificat" } else { "neverificat (fara --tokenizer)" },
            fingerprint(&ix)
        ));
    } else {
        rows.push(format!("antrenare date: NU TRECE verificarea: {} probleme; datele sunt refuzate (ca la Kronos)", c.wrong.len()));
        return rows;
    }
    if let Some((a, n)) = sl {
        match slice(&ix, a, a.saturating_add(n)) {
            Ok(v) => {
                let shown: Vec<String> = v.iter().take(16).map(|x| format!("{}", x)).collect();
                rows.push(format!(
                    "antrenare date: felia [{}, {}): {} simboluri: [{}]{}",
                    a,
                    a.saturating_add(n).min(ix.total),
                    v.len(),
                    shown.join(", "),
                    if v.len() > 16 { " ..." } else { "" }
                ));
            }
            Err(e) => rows.push(format!("antrenare date: felia: EROARE: {}", e)),
        }
    }
    rows
}
