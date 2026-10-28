//! P6-27b (command C-25, part 2): the KGR1 tokenizer, the `tokenizer.bpe` key of a model file. Written by us from the
//! description in the hub (`integrare/kronos/format_greutati.md`, "Tokenizerul"); no tokenizer code was read or ported.
//!
//! Vocabulary: ids 0–255 are the bytes, 256 + k joins the bytes of `imbinari[k] = [a, b]`, then the special texts.
//! Encoding: cut around the special texts, cut every remaining piece with the six-way pre-tokenizer (the Python
//! classes come from `unicode.rs`), give 1–3 ASCII digits their fixed id (rules `ws`), join the rest with BPE
//! (always the pair with the lowest merge position, every occurrence left to right). Decoding: the bytes of every id,
//! read as UTF-8 with invalid sequences replaced by U+FFFD ("maximal subpart", as `String::from_utf8_lossy`).

use alloc::{string::String, vec::Vec};

use super::jscan::Scan;
use super::unicode::{is_decimal, is_letter, is_space};

/// The tokenizer rules named by `versiune` / `reguli`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rules {
    /// versiune 1, no `reguli`: digits as ` ?\d+`, no fixed digit groups.
    Gpt2,
    /// versiune 2, `reguli` = "ws": digits as `\d{1,3}`, the first 1100 merges are the fixed digit groups.
    Ws,
}

pub struct Tokenizer {
    pub rules: Rules,
    /// (a << 32 | b, merge position k), sorted by pair; the joined id is 256 + k. A flat list, not a map: the program
    /// heap is first-fit and the tokenizer must load fast (16k merges).
    ranks: Vec<(u64, u32)>,
    /// The bytes of every id: `data[offs[id]..offs[id + 1]]`.
    data: Vec<u8>,
    offs: Vec<u32>,
    /// Special texts with their ids, longest first.
    specials: Vec<(String, u32)>,
    pub merges: usize,
}

impl Tokenizer {
    /// Reads the JSON text of `tokenizer.bpe`; `vocab` is the model's `vocabular` key (must match).
    pub fn from_json(text: &str, vocab: usize) -> Result<Tokenizer, String> {
        if text.is_empty() {
            return Err("fisierul nu are tokenizer (cheia `tokenizer.bpe`)".into());
        }
        let (mut version, mut rules_name, mut pairs, mut special_texts) = (None, None, None, None);
        let mut sc = Scan::new(text);
        sc.object(|sc, key| match key {
            "versiune" => sc.int().map(|v| version = Some(v)),
            "reguli" => sc.string().map(|r| rules_name = Some(r)),
            "imbinari" => {
                let mut list: Vec<(u32, u32)> = Vec::with_capacity(vocab);
                sc.array(|sc| {
                    let mut p = [0i64; 3];
                    let mut n = 0;
                    sc.array(|sc| {
                        let v = sc.int()?;
                        if n < 3 {
                            p[n] = v;
                        }
                        n += 1;
                        Ok(())
                    })?;
                    let ok = |v: i64| (0..=u32::MAX as i64).contains(&v);
                    if n != 2 || !ok(p[0]) || !ok(p[1]) {
                        return Err("tokenizer: o imbinare nu e o pereche de numere".into());
                    }
                    list.push((p[0] as u32, p[1] as u32));
                    Ok(())
                })?;
                pairs = Some(list);
                Ok(())
            }
            "speciale" => {
                let mut list = Vec::new();
                sc.array(|sc| sc.string().map(|s| list.push(s)))?;
                special_texts = Some(list);
                Ok(())
            }
            _ => sc.skip(),
        })
        .and_then(|_| sc.end())
        .map_err(|e| alloc::format!("tokenizer: {}", e))?;
        let rules = match (version, rules_name.as_deref()) {
            (Some(1), None) | (Some(1), Some("gpt2")) => Rules::Gpt2,
            (Some(2), Some("ws")) => Rules::Ws,
            (v, r) => return Err(alloc::format!("tokenizer: versiune {:?} cu reguli {:?} necunoscute (stiu 1/gpt2 si 2/ws)", v, r)),
        };
        let list = pairs.ok_or("tokenizer: lipseste `imbinari`")?;
        let special_texts = special_texts.ok_or("tokenizer: lipseste `speciale`")?;
        let mut data: Vec<u8> = (0..=255u8).collect();
        let mut offs: Vec<u32> = (0..=256u32).collect();
        offs.reserve(list.len() + special_texts.len());
        let mut ranks = Vec::with_capacity(list.len());
        for (k, &(a, b)) in list.iter().enumerate() {
            let id = 256 + k as u32;
            if a >= id || b >= id {
                return Err(alloc::format!("tokenizer: imbinarea {} foloseste un simbol de dupa ea", k));
            }
            for x in [a, b] {
                let (s, e) = (offs[x as usize] as usize, offs[x as usize + 1] as usize);
                data.extend_from_within(s..e);
            }
            offs.push(data.len() as u32);
            ranks.push(((a as u64) << 32 | b as u64, k as u32));
        }
        // The same pair twice: the first (lowest) position is the one that ever applies.
        ranks.sort_unstable();
        ranks.dedup_by_key(|r| r.0);
        let merges = list.len();
        if rules == Rules::Ws {
            // Merges 0–99: "xy" from the two ASCII digits; 100–1099: [256 + k, digit] = "xyz".
            for k in 0..1100usize {
                let want = if k < 100 {
                    (b'0' as u32 + (k / 10) as u32, b'0' as u32 + (k % 10) as u32)
                } else {
                    let i = k - 100;
                    (256 + (i / 10) as u32, b'0' as u32 + (i % 10) as u32)
                };
                if list.get(k) != Some(&want) {
                    return Err(alloc::format!("tokenizer `ws`: imbinarea {} nu e grupa fixa de cifre", k));
                }
            }
        }
        let mut specials = Vec::new();
        for s in special_texts {
            if s.is_empty() {
                return Err("tokenizer: un simbol special e gol".into());
            }
            data.extend_from_slice(s.as_bytes());
            specials.push((s, offs.len() as u32 - 1));
            offs.push(data.len() as u32);
        }
        let total = offs.len() - 1;
        if total != vocab {
            return Err(alloc::format!(
                "tokenizer: 256 + {} imbinari + {} speciale = {} simboluri, dar modelul are vocabularul {}",
                merges,
                specials.len(),
                total,
                vocab
            ));
        }
        // Longest first, so a special text that starts another one never hides it; equal lengths keep file order.
        specials.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Ok(Tokenizer { rules, ranks, data, offs, specials, merges })
    }

    fn rank(&self, a: u32, b: u32) -> Option<u32> {
        let key = (a as u64) << 32 | b as u64;
        self.ranks.binary_search_by_key(&key, |r| r.0).ok().map(|i| self.ranks[i].1)
    }

    pub fn vocab(&self) -> usize {
        self.offs.len() - 1
    }

    /// The bytes an id stands for (empty for an id outside the vocabulary).
    pub fn token_bytes(&self, id: u32) -> &[u8] {
        match (self.offs.get(id as usize), self.offs.get(id as usize + 1)) {
            (Some(&s), Some(&e)) => &self.data[s as usize..e as usize],
            _ => &[],
        }
    }

    pub fn is_special(&self, id: u32) -> bool {
        self.specials.iter().any(|s| s.1 == id)
    }

    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let (mut start, mut i) = (0, 0);
        while i < text.len() {
            if let Some((s, id)) = self.specials.iter().find(|(s, _)| text[i..].starts_with(s.as_str())) {
                self.encode_ordinary(&text[start..i], &mut out);
                out.push(*id);
                i += s.len();
                start = i;
            } else {
                i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
        self.encode_ordinary(&text[start..], &mut out);
        out
    }

    fn encode_ordinary(&self, text: &str, out: &mut Vec<u32>) {
        for piece in pre_tokenize(text, self.rules) {
            let b = piece.as_bytes();
            if self.rules == Rules::Ws && (1..=3).contains(&b.len()) && b.iter().all(u8::is_ascii_digit) {
                let d = |i: usize| (b[i] - b'0') as u32;
                out.push(match b.len() {
                    1 => b[0] as u32,
                    2 => 256 + 10 * d(0) + d(1),
                    _ => 356 + 100 * d(0) + 10 * d(1) + d(2),
                });
            } else {
                self.bpe(b, out);
            }
        }
    }

    /// Joins the bytes of one piece: always the neighbour pair with the lowest merge position, every
    /// non-overlapping occurrence from left to right, until no pair is a merge.
    fn bpe(&self, piece: &[u8], out: &mut Vec<u32>) {
        let mut syms: Vec<u32> = piece.iter().map(|&b| b as u32).collect();
        let mut next = Vec::with_capacity(syms.len());
        while syms.len() >= 2 {
            let best = syms.windows(2).filter_map(|w| self.rank(w[0], w[1]).map(|r| (r, w[0], w[1]))).min();
            let Some((rank, a, b)) = best else { break };
            next.clear();
            let mut i = 0;
            while i < syms.len() {
                if i + 1 < syms.len() && syms[i] == a && syms[i + 1] == b {
                    next.push(256 + rank);
                    i += 2;
                } else {
                    next.push(syms[i]);
                    i += 1;
                }
            }
            core::mem::swap(&mut syms, &mut next);
        }
        out.extend_from_slice(&syms);
    }

    pub fn decode(&self, ids: &[u32]) -> String {
        let mut b = Vec::new();
        for &id in ids {
            b.extend_from_slice(self.token_bytes(id));
        }
        String::from_utf8_lossy(&b).into_owned()
    }
}

/// The six alternatives of the specification, tried in order at every position (the first that matches, with its
/// longest match): `'(?:[sdmt]|ll|ve|re)`, ` ?LETTER+`, `\d{1,3}` (gpt2: ` ?\d+`), ` ?SIGN+`, `\s+(?!\S)`, `\s+`.
pub fn pre_tokenize(text: &str, rules: Rules) -> Vec<&str> {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    let n = cs.len();
    let at = |i: usize| cs[i].1;
    let is_sign = |c: char| !is_space(c) && !is_letter(c) && !is_decimal(c);
    let run = |from: usize, f: &dyn Fn(char) -> bool| {
        let mut e = from;
        while e < n && f(at(e)) {
            e += 1;
        }
        e
    };
    let mut pieces = Vec::new();
    let mut p = 0;
    while p < n {
        let c = at(p);
        let after_space = if c == ' ' { p + 1 } else { p };
        let end = 'm: {
            // 1. 's 'd 'm 't 'll 've 're (lower case only)
            if c == '\'' && p + 1 < n {
                if matches!(at(p + 1), 's' | 'd' | 'm' | 't') {
                    break 'm p + 2;
                }
                if p + 2 < n && matches!((at(p + 1), at(p + 2)), ('l', 'l') | ('v', 'e') | ('r', 'e')) {
                    break 'm p + 3;
                }
            }
            // 2. optional space + letters
            if after_space < n && is_letter(at(after_space)) {
                break 'm run(after_space, &is_letter);
            }
            // 3. digits
            match rules {
                Rules::Ws if is_decimal(c) => break 'm run(p, &is_decimal).min(p + 3),
                Rules::Gpt2 if after_space < n && is_decimal(at(after_space)) => break 'm run(after_space, &is_decimal),
                _ => {}
            }
            // 4. optional space + signs
            if after_space < n && is_sign(at(after_space)) {
                break 'm run(after_space, &is_sign);
            }
            if is_space(c) {
                let e = run(p, &is_space);
                // 5. spaces not followed by a non-space: at the end all of them, otherwise all but the last
                // (that one goes with the next piece) — when that leaves at least one.
                if e == n || e - p >= 2 {
                    break 'm if e == n { e } else { e - 1 };
                }
                // 6. any spaces
                break 'm e;
            }
            p + 1 // unreachable: every character is a letter, a digit, a sign or a space
        };
        let (s, e) = (cs[p].0, if end < n { cs[end].0 } else { text.len() });
        pieces.push(&text[s..e]);
        p = end;
    }
    pieces
}

/// Decoded text arriving one id at a time: keeps the bytes of an unfinished letter until it completes.
#[derive(Default)]
pub struct TextStream {
    pending: Vec<u8>,
}

impl TextStream {
    /// Appends bytes, returns the text that is complete so far (invalid sequences as U+FFFD).
    pub fn push(&mut self, b: &[u8]) -> String {
        self.pending.extend_from_slice(b);
        let mut out = String::new();
        loop {
            match core::str::from_utf8(&self.pending) {
                Ok(s) => {
                    out.push_str(s);
                    self.pending.clear();
                    break;
                }
                Err(e) => {
                    let ok = e.valid_up_to();
                    out.push_str(core::str::from_utf8(&self.pending[..ok]).unwrap_or(""));
                    match e.error_len() {
                        Some(bad) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..ok + bad);
                        }
                        None => {
                            self.pending.drain(..ok);
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    /// The end of the text: an unfinished letter becomes U+FFFD.
    pub fn finish(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        s
    }
}
