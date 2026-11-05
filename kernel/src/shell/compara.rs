//! `compara <fisier1> <fisier2>` (task P6-52, command C-68 F9): the lines that differ between two reports or logs.
//!
//! Walks both files line by line; where they differ, it looks up to 64 lines ahead in each for the next common line
//! (the shortest jump wins) and prints what is only in the first file (`-`) and only in the second (`+`), with line
//! numbers. Little memory (no table of all line pairs), enough for reports that mostly differ in numbers.
//! Secret files (key, token) are refused like in `cat`. At most 200 lines of differences are printed.
//! Idea: line comparison as in `diff`, with a bounded look-ahead instead of a full LCS; our own code.

use alloc::{format, string::String, vec::Vec};

const LOOKAHEAD: usize = 64;
const MAX_SHOWN: usize = 200;

/// The comparison as text (the shell prints it).
pub fn compare_files(a: &str, b: &str) -> String {
    for p in [a, b] {
        if crate::proc::rights::canonical(p).is_some_and(|c| crate::proc::rights::is_secret(&c)) {
            return format!("compara: {}: fisier secret\n", p);
        }
    }
    let read = |p: &str| crate::fs::read_all(p).map(|d| String::from_utf8_lossy(&d).into_owned()).map_err(|e| format!("compara: {}: {}\n", p, e.message()));
    let (ta, tb) = match (read(a), read(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let la: Vec<&str> = ta.lines().map(|l| l.trim_end()).collect();
    let lb: Vec<&str> = tb.lines().map(|l| l.trim_end()).collect();
    let mut out = format!("compara: {} ({} randuri) cu {} ({} randuri)\n", a, la.len(), b, lb.len());
    out.push_str(&diff(&la, &lb));
    out
}

/// The differences between two lists of lines (`- n: text` only in the first, `+ n: text` only in the second).
pub fn diff(la: &[&str], lb: &[&str]) -> String {
    let (mut i, mut j) = (0, 0);
    let (mut shown, mut removed, mut added, mut same) = (0usize, 0usize, 0usize, 0usize);
    let mut out = String::new();
    let emit = |out: &mut String, sign: char, n: usize, text: &str, shown: &mut usize| {
        if *shown < MAX_SHOWN {
            out.push_str(&format!("{} {}: {}\n", sign, n + 1, text));
        }
        *shown += 1;
    };
    while i < la.len() || j < lb.len() {
        if i < la.len() && j < lb.len() && la[i] == lb[j] {
            same += 1;
            i += 1;
            j += 1;
            continue;
        }
        // The nearest common line ahead: skip `di` lines of the first and `dj` of the second, smallest di + dj.
        let mut best: Option<(usize, usize)> = None;
        'search: for total in 1..=2 * LOOKAHEAD {
            for di in 0..=total.min(LOOKAHEAD) {
                let dj = total - di;
                if dj > LOOKAHEAD {
                    continue;
                }
                if i + di < la.len() && j + dj < lb.len() && la[i + di] == lb[j + dj] {
                    best = Some((di, dj));
                    break 'search;
                }
            }
        }
        // No common line close by: everything left is different.
        let (di, dj) = best.unwrap_or((la.len() - i, lb.len() - j));
        for k in 0..di {
            emit(&mut out, '-', i + k, la[i + k], &mut shown);
        }
        for k in 0..dj {
            emit(&mut out, '+', j + k, lb[j + k], &mut shown);
        }
        removed += di;
        added += dj;
        i += di;
        j += dj;
    }
    if shown > MAX_SHOWN {
        out.push_str(&format!("... inca {} randuri diferite (nu se mai arata)\n", shown - MAX_SHOWN));
    }
    if removed == 0 && added == 0 {
        out.push_str(&format!("la fel: cele {} randuri sunt identice\n", same));
    } else {
        out.push_str(&format!("rezultat: {} la fel, {} doar in primul (-), {} doar in al doilea (+)\n", same, removed, added));
    }
    out
}

/// Boot-time self test (test builds).
#[cfg(feature = "qemu-test")]
pub fn self_test() {
    let a = ["viteza: 206 simb/s", "control: TRECE", "fire: 35", "gata"];
    let b = ["viteza: 350 simb/s", "control: TRECE", "fire: 35", "rand nou", "gata"];
    let d = diff(&a, &b);
    assert!(d.contains("- 1: viteza: 206 simb/s") && d.contains("+ 1: viteza: 350 simb/s"), "compara: {}", d);
    assert!(d.contains("+ 4: rand nou") && d.contains("3 la fel, 1 doar in primul (-), 2 doar in al doilea (+)"), "compara: {}", d);
    assert!(diff(&a, &a).contains("cele 4 randuri sunt identice"), "compara: identice");
    assert!(diff(&a, &[]).contains("0 la fel, 4 doar in primul"), "compara: gol");
    crate::info!("test: compara OK (diferente, identice, fisier gol)");
}
