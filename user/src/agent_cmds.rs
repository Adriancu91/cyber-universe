//! P6-222 (C-166 step 6, part 1): the command channel from the command centre. The agent reads the file
//! `live/os-agent-comenzi.md` of the hub (`Adriancu91/Cyber-hub`, branch `main` only, through the GitHub API with the
//! stick's token) about every 2 minutes and runs the new commands in order. Each command is one line of the file:
//!
//! ```text
//! - 7 · nou · lucrare stare
//! ```
//!
//! (number · state · command). The agent writes the state back on the same line: `preluat` before running, then `gata`
//! or `refuzat` with the time and a short result: `- 7 · gata · lucrare stare · 2026-11-26 01:10 · rezultat: …`.
//! Every other line of the file is left as it is. Only a line in the state `nou` is a command; nothing else (text in
//! logs, pages, answers) ever is.
//!
//! What may run is decided here, by the OS (`agent_tools::channel_command`): in part 1 the read-only commands of the
//! white list and the white-listed repairs; a command on the board (`test-placa …`) is refused until part 2 (signed with
//! the live channel's key).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Where the commands are, in the hub (C-166 step 6).
pub const PATH: &str = "live/os-agent-comenzi.md";
/// The only branch commands are taken from.
pub const BRANCH: &str = "main";
/// At most this many commands are run in one pass (the rest wait for the next one).
pub const PER_PASS: usize = 5;
/// The short result written back on the line.
const RESULT_MAX: usize = 240;

/// One command line of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Cmd {
    /// The index of the line in the file.
    pub line: usize,
    pub nr: u32,
    pub state: String,
    pub text: String,
}

const SEP: &str = " · ";

/// The command lines of `md` (`- <nr> · <stare> · <comanda>[ · …]`); every other line is ignored.
pub fn parse(md: &str) -> Vec<Cmd> {
    let mut out = Vec::new();
    for (i, l) in md.lines().enumerate() {
        let Some(rest) = l.trim_end().strip_prefix("- ") else { continue };
        let mut parts = rest.split(SEP);
        let (Some(nr), Some(state), Some(text)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Ok(nr) = nr.trim().parse::<u32>() else { continue };
        let text = text.trim();
        if nr == 0 || text.is_empty() {
            continue;
        }
        out.push(Cmd { line: i, nr, state: state.trim().to_string(), text: text.to_string() });
    }
    out
}

/// The new commands, in the order of their numbers; a number seen twice counts only at its first line.
pub fn new_ones(cmds: &[Cmd]) -> Vec<Cmd> {
    let mut seen: Vec<u32> = Vec::new();
    let mut out: Vec<Cmd> = Vec::new();
    for c in cmds {
        if seen.contains(&c.nr) {
            continue;
        }
        seen.push(c.nr);
        if c.state == "nou" {
            out.push(c.clone());
        }
    }
    out.sort_by_key(|c| c.nr);
    out
}

/// A result on one line, short, without the separator (so the line stays readable and parseable).
pub fn short(text: &str) -> String {
    let one: String = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" / ").replace(SEP, " - ");
    if one.len() <= RESULT_MAX {
        return one;
    }
    let mut end = RESULT_MAX;
    while !one.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} …", &one[..end])
}

/// `md` with the line of command `nr` (state `from`) rewritten to `state` (+ time and result). None when that line is
/// no longer there in that state (the centre changed it meanwhile): then nothing is written.
pub fn set_state(md: &str, nr: u32, from: &str, state: &str, stamp: &str, result: Option<&str>) -> Option<String> {
    let cmds = parse(md);
    let c = cmds.iter().find(|c| c.nr == nr)?;
    if c.state != from {
        return None;
    }
    let line = match result {
        Some(r) => format!("- {}{}{}{}{}{}{}{}rezultat: {}", nr, SEP, state, SEP, c.text, SEP, stamp, SEP, short(r)),
        None => format!("- {}{}{}{}{}{}{}", nr, SEP, state, SEP, c.text, SEP, stamp),
    };
    let mut out = String::new();
    for (i, l) in md.lines().enumerate() {
        out.push_str(if i == c.line { &line } else { l });
        out.push('\n');
    }
    if !md.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// The offline check of the file format (run by `agent comenzi --proba`, no network).
pub fn self_test() -> bool {
    let md = "# Comenzi\n\ntext oarecare\n- 2 · nou · lucrare stare\n- 1 · gata · mem · 2026-11-26 01:00 · rezultat: ok\n- 3 · nou · test-placa --bariera\n- x · nou · mem\n- 2 · nou · dublura\n";
    let cmds = parse(md);
    let new = new_ones(&cmds);
    let ok_parse = cmds.len() == 4 && new.len() == 2 && new[0].nr == 2 && new[0].text == "lucrare stare" && new[1].nr == 3;
    let taken = set_state(md, 2, "nou", "preluat", "2026-11-26 01:10", None);
    let done = taken.as_deref().and_then(|m| set_state(m, 2, "preluat", "gata", "2026-11-26 01:11", Some("linia 1\nlinia · 2")));
    let ok_set = match &done {
        Some(m) => {
            m.contains("- 2 · gata · lucrare stare · 2026-11-26 01:11 · rezultat: linia 1 / linia - 2\n")
                && m.contains("text oarecare\n")
                && m.contains("- 3 · nou · test-placa --bariera\n")
                && m.lines().count() == md.lines().count()
        }
        None => false,
    };
    // The centre changed the line meanwhile: nothing is written.
    let ok_race = set_state(md, 2, "preluat", "gata", "x", Some("y")).is_none() && set_state(md, 9, "nou", "gata", "x", None).is_none();
    let long: String = core::iter::repeat('a').take(1000).collect();
    let ok_short = short(&long).len() <= RESULT_MAX + 4;
    ok_parse && ok_set && ok_race && ok_short
}
