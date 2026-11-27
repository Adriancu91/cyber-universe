#!/usr/bin/env python3
"""P6-256: checks `test-placa ...` rows on the host with the very code of the automatic list (kernel/src/shell/ciclu.rs).

Why: on 27 Nov 13:28 the command centre put a row in `auto\\teste.txt` that the OS refused (`--mm-tc-v5` needs
`--mm-tc-v4` before it); a board slot was lost. This tool answers before the row reaches the stick.

How: the parser of the list (`board_command`, `board_command_form`, `board_refusal_hint`, the option constants and the
I9 names of `gpu_golden::i9_option`) is copied out of the kernel sources into a small host program, built with rustc
and run on the rows. Nothing is rewritten by hand, so the answer is the kernel's answer; if the kernel's code changes
shape so that the copy fails, the tool stops with an error (CI shows it) instead of guessing.

Use:
  tools/verifica-rand.py "test-placa --tablou-viteza --mm-tc ..."   one or more rows
  tools/verifica-rand.py --din docs/STATE.md                        every `test-placa ...` row between backticks
  tools/verifica-rand.py --din docs/STATE.md --ultima-sesiune        only the newest session entry of STATE.md
Exit code: 0 = every row accepted, 1 = at least one row refused, 2 = the tool itself could not run.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CICLU = os.path.join(ROOT, "kernel/src/shell/ciclu.rs")
GOLDEN = os.path.join(ROOT, "kernel/src/drivers/gpu_golden.rs")


def die(msg):
    print("verifica-rand: EROARE: " + msg, file=sys.stderr)
    sys.exit(2)


def top_fn(lines, name):
    """The top-level function `name` (from its `fn` line to the first `}` in column 0)."""
    pat = re.compile(r"^(pub )?fn " + re.escape(name) + r"\(")
    for i, l in enumerate(lines):
        if pat.match(l):
            for j in range(i, len(lines)):
                if lines[j] == "}":
                    return "\n".join(lines[i : j + 1])
            die("functia %s nu se termina" % name)
    die("functia %s nu e in ciclu.rs (s-a schimbat codul? actualizeaza tools/verifica-rand.py)" % name)


def build_source():
    with open(CICLU, encoding="utf-8") as f:
        lines = f.read().split("\n")
    with open(GOLDEN, encoding="utf-8") as f:
        golden = f.read()
    consts = [l for l in lines if re.match(r'^pub const [A-Z0-9_]+: &str = ".*";$', l)]
    consts += [l for l in lines if re.match(r"^pub const TRAIN_513_BATCH_MAX: usize = \d+;$", l)]
    options = [l for l in lines if l.startswith("const BOARD_OPTIONS: &[&str] = &[")]
    if len(options) != 1 or not consts:
        die("lista optiunilor (BOARD_OPTIONS) sau constantele nu se gasesc in ciclu.rs")
    m = re.search(r"pub fn i9_option\(v: &str\)[^{]*\{(.*?)\n\}", golden, re.S)
    names = re.findall(r'"([a-z0-9]+)" => Some', m.group(1)) if m else []
    if not names:
        die("numele I9 (gpu_golden::i9_option) nu se gasesc")
    parts = [
        "#![allow(dead_code)]",
        "extern crate alloc;",
        "use alloc::{format, string::String, vec::Vec};",
        "\n".join(consts),
        options[0],
        "fn i9_ok(x: &str) -> bool { [%s].contains(&x) }" % ", ".join('"%s"' % n for n in names),
        top_fn(lines, "board_command"),
        top_fn(lines, "board_command_form"),
        top_fn(lines, "board_refusal_hint"),
        MAIN,
    ]
    return "\n\n".join(parts)


MAIN = r"""
fn main() {
    use std::io::BufRead;
    let mut refused = 0;
    for row in std::io::stdin().lock().lines() {
        let row = row.unwrap();
        match board_command(&row) {
            Some(Ok(i9)) => println!("PRIMIT\t{}\t{}", row, i9),
            Some(Err(_)) => {
                refused += 1;
                let why = board_refusal_hint(&row).unwrap_or_else(|| String::from("forma gresita"));
                println!("REFUZAT\t{}\t{}", row, why);
            }
            None => {
                refused += 1;
                println!("REFUZAT\t{}\tnu incepe cu `test-placa`", row);
            }
        }
    }
    std::process::exit(if refused == 0 { 0 } else { 1 });
}
"""


def rows_from(path, newest_only):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    if newest_only:
        # STATE.md: the newest session is the first `- ` item under `## Ultima sesiune`, up to the next one.
        h = text.find("\n## Ultima sesiune")
        start = text.find("\n- ", h) if h >= 0 else -1
        if start < 0:
            die("nu gasesc `## Ultima sesiune` in %s" % path)
        nxt = re.search(r"^(- |## )", text[start + 3 :], re.M)
        text = text[start : start + 3 + nxt.start()] if nxt else text[start:]
    rows = []
    for r in re.findall(r"`(test-placa[^`]*)`", text):
        # Abbreviated rows (`test-placa … --x`) are notes for people, not rows for the stick.
        if any(c in r for c in ("…", "...", "<", "[", "*")):
            continue
        r = " ".join(r.split())
        if r not in rows:
            rows.append(r)
    return rows


def main(argv):
    rows = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--din":
            if i + 1 >= len(argv):
                die("`--din` cere un fisier")
            newest = "--ultima-sesiune" in argv[i + 2 : i + 3]
            rows += rows_from(argv[i + 1], newest)
            i += 3 if newest else 2
        elif a in ("-h", "--help"):
            print(__doc__)
            return 0
        else:
            rows.append(" ".join(a.split()))
            i += 1
    if not rows:
        print("verifica-rand: niciun rand `test-placa` de verificat")
        return 0
    rustc = os.environ.get("RUSTC", "rustc")
    with tempfile.TemporaryDirectory(prefix="verifica-rand-") as d:
        src = os.path.join(d, "rand.rs")
        exe = os.path.join(d, "rand")
        with open(src, "w", encoding="utf-8") as f:
            f.write(build_source())
        b = subprocess.run([rustc, "--edition", "2021", "-O", "-o", exe, src], capture_output=True, text=True)
        if b.returncode != 0:
            die("codul copiat din ciclu.rs nu se compileaza pe gazda:\n" + b.stderr[-3000:])
        r = subprocess.run([exe], input="\n".join(rows) + "\n", capture_output=True, text=True)
    if r.returncode not in (0, 1):
        die("programul de verificare a cazut:\n" + r.stderr[-2000:])
    for line in r.stdout.splitlines():
        verdict, row, detail = line.split("\t", 2)
        if verdict == "PRIMIT":
            print("PRIMIT   " + row)
        else:
            print("REFUZAT  " + row)
            print("         motiv: " + detail)
    ok = r.stdout.count("PRIMIT\t")
    print("verifica-rand: %d primite, %d refuzate (codul lui ciclu.rs)" % (ok, len(rows) - ok))
    return r.returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
