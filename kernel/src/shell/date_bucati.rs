//! P6-150 (C-161 point 5, C-158 point 10): Kronos's training data in the OS — the chunks `bucata_NNNNN.bin` (int16 or
//! int32 little-endian, no header) + `index.json`, the format of cyberspace `unelte/date_bucati.py` (hub
//! `integrare/kronos/FISA_ANTRENARE.md` §3). Checked exactly as Kronos's `module/date_bucati.py` does at start-up (read
//! only, no code copied): the index version / type / byte order, each chunk's start, size and SHA-256, the whole
//! sequence's SHA-256, the tokenizer's SHA-256; any difference = the data is refused. The fingerprint is Kronos's:
//! SHA-256 of `"<sha256_tot>:<tokenizer sha256>"`, first 16 hex digits. `slice` reads symbols [a, b) across chunks
//! (what the training engine will draw its windows from). Nothing is written anywhere.
//!
//! P6-153: the batch windows drawn exactly as Kronos's training loop draws them (`DateBucati.lot`,
//! `torch.randint(len - context - 1, (numar,), generator=g)` with `g = torch.Generator("cpu").manual_seed(seed)`):
//! `Mt19937` is the standard Mersenne Twister (Matsumoto & Nishimura 1998, the C++ `std::mt19937` definition) that
//! PyTorch's CPU generator uses; `randint` follows PyTorch's documented CPU rule (one 32-bit draw `% range` when the
//! range is below 2^28, otherwise two draws joined high-then-low). Checked against PyTorch itself, not copied.

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

/// The Mersenne Twister MT19937 (32-bit outputs), seeded like `std::mt19937(seed)` with the low 32 bits of the seed.
pub struct Mt19937 {
    state: [u32; 624],
    next: usize,
}

impl Mt19937 {
    pub fn new(seed: u64) -> Mt19937 {
        let mut state = [0u32; 624];
        state[0] = seed as u32;
        for j in 1..624 {
            state[j] = 1_812_433_253u32.wrapping_mul(state[j - 1] ^ (state[j - 1] >> 30)).wrapping_add(j as u32);
        }
        Mt19937 { state, next: 624 }
    }

    fn twist(&mut self) {
        for i in 0..624 {
            let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7fff_ffff);
            let mut v = self.state[(i + 397) % 624] ^ (y >> 1);
            if y & 1 != 0 {
                v ^= 0x9908_b0df;
            }
            self.state[i] = v;
        }
        self.next = 0;
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.next >= 624 {
            self.twist();
        }
        let mut y = self.state[self.next];
        self.next += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// One value in [0, range), as PyTorch's CPU `randint(range)`: from 2^28 up (all of Kronos's real data) two
    /// 32-bit draws joined high-then-low, below that one draw (checked against torch 2.15 CPU, every range tried).
    pub fn randint(&mut self, range: u64) -> u64 {
        if range >= 1 << 28 {
            let hi = self.next_u32() as u64;
            let lo = self.next_u32() as u64;
            ((hi << 32) | lo) % range
        } else {
            self.next_u32() as u64 % range
        }
    }
}

/// Values PyTorch 2.15 CPU gave for `torch.Generator().manual_seed(24000079)` then `torch.randint(range, (4,))`: one
/// range on the 32-bit path, one on the 64-bit path (the size of Kronos's real data, ~2.7e9 symbols).
const TORCH_REFERENCE: [(u64, [u64; 4]); 2] =
    [(2633, [2337, 1996, 58, 256]), (2_700_000_000, [706_797_272, 1_616_238_451, 1_669_814_552, 1_245_281_307])];

/// The generator checked against the PyTorch reference values (both paths).
pub fn generator_matches_torch() -> bool {
    TORCH_REFERENCE.iter().all(|(range, want)| {
        let mut g = Mt19937::new(24_000_079);
        want.iter().all(|w| g.randint(*range) == *w)
    })
}

/// Kronos's batch seed for one hour of training (`_samanta_loturi`): `samanta` on the first hour (`pasi_total == 0`),
/// otherwise `samanta * 1_000_003 + pasi_total`.
pub fn batch_seed(samanta: u64, pasi_total: u64) -> u64 {
    if pasi_total == 0 {
        samanta
    } else {
        samanta.wrapping_mul(1_000_003).wrapping_add(pasi_total)
    }
}

/// The start positions of `count` windows drawn by one generator (`torch.randint(total - context - 1, (count,), g)`).
pub fn positions(g: &mut Mt19937, total: u64, count: usize, context: u64) -> Result<Vec<u64>, String> {
    if total <= context + 1 {
        return Err(format!("sirul ({} simboluri) e mai scurt decat contextul {}", total, context));
    }
    Ok((0..count).map(|_| g.randint(total - context - 1)).collect())
}

/// One batch: for each position p the input `[p, p + context)` and the target `[p + 1, p + context + 1)`, read across
/// chunks (`DateBucati.ferestre`). Returned as (x, y), each `count * context` symbols, row after row.
pub fn batch(ix: &Index, pos: &[u64], context: u64) -> Result<(Vec<u32>, Vec<u32>), String> {
    let (mut x, mut y) = (Vec::with_capacity(pos.len() * context as usize), Vec::with_capacity(pos.len() * context as usize));
    for &p in pos {
        let w = slice(ix, p, p + context + 1)?;
        if w.len() as u64 != context + 1 {
            return Err(format!("fereastra de la {} iese din sir", p));
        }
        x.extend_from_slice(&w[..context as usize]);
        y.extend_from_slice(&w[1..]);
    }
    Ok((x, y))
}

/// The batch's fingerprint: SHA-256 over x then y, each symbol as int64 little-endian (the tensors Kronos builds),
/// first 16 hex digits.
pub fn batch_fingerprint(x: &[u32], y: &[u32]) -> String {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut h = Sha256::new();
    for v in x.iter().chain(y.iter()) {
        h.update(&(*v as i64).to_le_bytes());
    }
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    hex(&out)[..16].to_string()
}

const USAGE: &str = "folosire: antrenare date [dosar] [--tokenizer <cale>] [--felie <inceput> <numar>] [--lot <samanta> <loturi> <numar> <context> [--pasi-total <p>]]";
const LOT_USAGE: &str = "antrenare date: --lot <samanta> <loturi 1..64> <numar 1..64> <context 1..4096> [--pasi-total <p>]";

/// `antrenare date [dosar] [--tokenizer <cale>] [--felie <a> <n>]`: the check, the fingerprint, optionally a slice.
pub fn command(args: &[&str]) -> Vec<String> {
    let mut rows = Vec::new();
    let (mut dir, mut tok, mut sl): (Option<&str>, Option<&str>, Option<(u64, u64)>) = (None, None, None);
    // --lot <samanta> <loturi> <numar> <context> [--pasi-total <p>]
    let (mut lot, mut pasi_total): (Option<(u64, u64, u64, u64)>, u64) = (None, 0);
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
            "--lot" if i + 4 < args.len() => {
                let n: Vec<Option<u64>> = args[i + 1..i + 5].iter().map(|a| a.parse::<u64>().ok()).collect();
                match (n[0], n[1], n[2], n[3]) {
                    (Some(s), Some(l), Some(k), Some(c)) if (1..=64).contains(&l) && (1..=64).contains(&k) && (1..=4096).contains(&c) => {
                        lot = Some((s, l, k, c));
                        i += 5;
                    }
                    _ => return vec![String::from(LOT_USAGE)],
                }
            }
            "--pasi-total" if i + 1 < args.len() => match args[i + 1].parse::<u64>() {
                Ok(p) => {
                    pasi_total = p;
                    i += 2;
                }
                Err(_) => return vec![String::from(LOT_USAGE)],
            },
            x if !x.starts_with("--") && dir.is_none() => {
                dir = Some(x);
                i += 1;
            }
            _ => return vec![String::from(USAGE)],
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
    if let Some((samanta, loturi, numar, context)) = lot {
        let seed = batch_seed(samanta, pasi_total);
        rows.push(format!(
            "antrenare date: loturi ca Kronos: samanta {} (pasi_total {} -> generatorul cu {}), {} loturi x {} ferestre x context {}",
            samanta, pasi_total, seed, loturi, numar, context
        ));
        if !generator_matches_torch() {
            rows.push(String::from("antrenare date: REFUZAT: generatorul nu da valorile PyTorch de referinta"));
            return rows;
        }
        rows.push(String::from("antrenare date: generatorul MT19937 = PyTorch (valorile de referinta, caile pe 32 si pe 64 de biti)"));
        let mut g = Mt19937::new(seed);
        for l in 0..loturi {
            let r = positions(&mut g, ix.total, numar as usize, context).and_then(|p| batch(&ix, &p, context).map(|b| (p, b)));
            match r {
                Ok((p, (x, y))) => {
                    let shown: Vec<String> = p.iter().take(8).map(|v| format!("{}", v)).collect();
                    rows.push(format!(
                        "antrenare date: lotul {}: pozitii [{}]{}; x[0][..4] = {:?}; amprenta {}",
                        l,
                        shown.join(", "),
                        if p.len() > 8 { " ..." } else { "" },
                        &x[..4.min(x.len())],
                        batch_fingerprint(&x, &y)
                    ));
                }
                Err(e) => {
                    rows.push(format!("antrenare date: lotul {}: REFUZAT: {}", l, e));
                    break;
                }
            }
        }
    }
    rows
}
