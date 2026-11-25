//! The on-board agent's tools (P6-203, C-166 step 2): read-only. Claude may list folders, read files, read the log,
//! see the state of the machine and run shell commands from a white list — nothing that writes, deletes, touches the
//! video card or waits for a "DA". Everything is decided here, in the OS, not by the model: a command outside the list
//! is refused whatever the question said. Secrets (keys, tokens) are hidden by the kernel from every program that did
//! not ask for them (P6-23), and the agent gives up its key right after reading it.
//!
//! P6-219 (C-166 step 4): one tool that changes something, `repara`, limited to a white list decided here: the safe
//! stop of a long job (`lucrare opreste <nume>`: after the step in progress) and a few values of `agent.setari` within
//! fixed bounds. Every other action or setting is refused; every repair, done or refused, goes into
//! `<root>/agent/reparatii.txt`.
//!
//! P6-220 (C-166 step 5): `citeste_cod` reads the OS's code on GitHub (`main`, read only) and `propune` writes a change
//! as a proposal on a new branch `agent/<day>-<subject>` of the code repository — never `main`, never over an existing
//! branch, never the CI workflows or the project's rules — with the explanation and the proof in
//! `docs/agent/propuneri/<branch>.md`. The OS session checks it, tests it in QEMU and decides. Every proposal, made or
//! refused, goes into `<root>/agent/propuneri.txt`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_json::Value;

use crate::claude_tools;
use crate::github;

/// P6-220: GitHub for the code tools: the settings and the token (read once at the start of a question).
pub struct Gh {
    pub conf: github::Config,
    pub token: Option<String>,
}

/// The largest piece of a file Claude gets at once.
const READ_MAX: usize = 32 * 1024;

pub const DEFINITIONS: &str = r#"[
 {"name":"fisiere","description":"Listeaza un dosar din Cyber Universe OS (stick-ul /usb, discul /disk, /tmp si dosarul agentului). Dosarele se termina cu /.","input_schema":{"type":"object","properties":{"cale":{"type":"string","description":"de ex. /usb/raport"}},"required":["cale"]}},
 {"name":"citeste","description":"Citeste un fisier text (aceleasi dosare ca fisiere; cel mult 32 KiB de la `de_la`, in octeti). Cheile si tokenurile nu se vad.","input_schema":{"type":"object","properties":{"cale":{"type":"string"},"de_la":{"type":"integer","description":"octetul de inceput (implicit 0)"}},"required":["cale"]}},
 {"name":"citeste_jurnal","description":"Ultimele randuri din jurnalul OS-ului, optional doar cele cu un cuvant (de ex. gpu, usb, kt).","input_schema":{"type":"object","properties":{"cuvant":{"type":"string"},"n":{"type":"integer","description":"cate randuri (implicit 40, cel mult 400)"}}}},
 {"name":"stare","description":"Starea calculatorului: de cand merge, memoria, lucrarile lungi (pasi, viteza, pierderi), placa de retea.","input_schema":{"type":"object","properties":{}}},
 {"name":"tablou","description":"Ultimele citiri ale tabloului placii video pe care OS-ul le-a scris deja in jurnal: ceasul nucleului si al memoriei, starea P, incarcarea, puterea fata de limita, motivele incetinirii, temperatura. Nu trimite nicio comanda placii.","input_schema":{"type":"object","properties":{"n":{"type":"integer","description":"cate citiri (implicit 12, cel mult 100)"}}}},
 {"name":"ruleaza","description":"Ruleaza o comanda de citire din lista alba a shell-ului: ls, mem, uptime, lspci, lsusb, discuri, net, cpu, hw, hw lista [--detalii], log ..., usb timp, lucrare stare, porneste (fara argumente), hub stare|comenzi|decizii [proiect]. Orice altceva e refuzat de OS.","input_schema":{"type":"object","properties":{"comanda":{"type":"string"}},"required":["comanda"]}},
 {"name":"citeste_cod","description":"Citeste codul lui Cyber Universe OS de pe GitHub (ramura main, doar citire): un fisier (cel mult 32 KiB de la `de_la`) sau lista unui dosar (cale goala = radacina). De ex. user/src/agent_tools.rs, kernel/src/shell.","input_schema":{"type":"object","properties":{"cale":{"type":"string"},"de_la":{"type":"integer"}},"required":["cale"]}},
 {"name":"propune","description":"Scrie o modificare de cod ca PROPUNERE pe o ramura noua agent/<zi>-<subiect> a depozitului OS-ului (niciodata main; sesiunea OS o verifica, o testeaza si decide). Fisierele se dau intregi (continutul nou complet). Cel mult 8 fisiere de cel mult 128 KiB; refuzate: .github/, LICENSE, CLAUDE.md, docs/DECISIONS.md. Explicatia si dovada (de ce, ce ai vazut, cum se poate verifica) sunt obligatorii.","input_schema":{"type":"object","properties":{"subiect":{"type":"string","description":"litere mici, cifre si -, cel mult 40"},"explicatie":{"type":"string"},"dovada":{"type":"string"},"fisiere":{"type":"array","items":{"type":"object","properties":{"cale":{"type":"string"},"continut":{"type":"string"}},"required":["cale","continut"]}}},"required":["subiect","explicatie","dovada","fisiere"]}},
 {"name":"repara","description":"Reparatii din lista alba a OS-ului (singurele schimbari permise): actiune `opreste_lucrarea` = oprirea sigura a unei lucrari lungi dupa pasul in lucru (`nume` = lucrarea, ca in `lucrare stare`); actiune `setare` = o valoare din agent.setari, doar live_minute (1-60), analiza_minute (2-120) sau max_tokens (256-8192), in `valoare`. Orice alta actiune sau setare e refuzata de OS. Fiecare reparatie se scrie in agent/reparatii.txt.","input_schema":{"type":"object","properties":{"actiune":{"type":"string","enum":["opreste_lucrarea","setare"]},"nume":{"type":"string"},"valoare":{"type":"string"}},"required":["actiune","nume"]}}
]"#;

/// P6-219: the settings `repara` may change, with their bounds (inclusive). Not the model, the daily cap, the prices,
/// the key or the address: those stay Adrian's.
pub const SETTINGS_ALLOWED: [(&str, u64, u64); 3] = [("live_minute", 1, 60), ("analiza_minute", 2, 120), ("max_tokens", 256, 8192)];

/// P6-219: a job name as `lucrare` accepts it (lowercase, digits, - _; at most 32).
fn job_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// P6-219: `setari` with `name = value` (the first non-comment line of that name replaced, else added at the end).
pub fn set_value(setari: &str, name: &str, value: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for line in setari.lines() {
        let t = line.trim();
        let same = !t.starts_with('#') && t.split_once('=').is_some_and(|(n, _)| n.trim() == name);
        if same && !done {
            out.push_str(&format!("{} = {}\n", name, value));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !done {
        out.push_str(&format!("{} = {}\n", name, value));
    }
    out
}

/// Writes `data` as the whole file `path` (FAT has no append here).
pub fn write_whole(path: &str, data: &[u8]) -> Result<(), String> {
    let r = cyber_user::remove(path);
    let r = if r < 0 && r != cyber_user::E_NOT_FOUND { r } else { cyber_user::create(path, false) };
    let fd = if r < 0 { r } else { cyber_user::open(path) };
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let mut done = 0;
    while done < data.len() {
        let n = cyber_user::write(fd as u64, &data[done..data.len().min(done + 16 * 1024)]);
        if n <= 0 {
            break;
        }
        done += n as usize;
    }
    cyber_user::close(fd);
    if done == data.len() { Ok(()) } else { Err(format!("{}: scriere incompleta", path)) }
}

/// P6-219: one line into `<root>/agent/reparatii.txt` (kept under 64 KiB: the oldest half dropped past it).
fn note_repair(root: &str, line: &str) {
    let dir = format!("{}/agent", root);
    let _ = cyber_user::create(&dir, true);
    let path = format!("{}/reparatii.txt", dir);
    let mut text = read(&path, 0).unwrap_or_default();
    if text.len() > 64 * 1024 {
        let cut = text[text.len() / 2..].find('\n').map_or(text.len(), |i| text.len() / 2 + i + 1);
        text = text[cut..].to_string();
    }
    text.push_str(line);
    text.push('\n');
    if write_whole(&path, text.as_bytes()).is_ok() {
        let _ = cyber_user::sync(&dir);
    }
}

/// P6-219: the repair itself; Ok = what was done, Err = refused (with why).
fn repair(action: &str, name: &str, value: Option<&str>, root: &str, term: &mut Option<i64>) -> Result<String, String> {
    match action {
        "opreste_lucrarea" => {
            if !job_name(name) {
                return Err(format!("numele lucrarii `{}` nu e permis (litere mici, cifre, - _)", name));
            }
            let out = claude_tools::run_command(&format!("lucrare opreste {}", name), term)?;
            if out.contains("oprirea ceruta") { Ok(out) } else { Err(out) }
        }
        "setare" => {
            let Some(&(_, lo, hi)) = SETTINGS_ALLOWED.iter().find(|(n, _, _)| *n == name) else {
                return Err(format!(
                    "setarea `{}` nu e in lista aprobata (doar {}); OS-ul a refuzat-o",
                    name,
                    SETTINGS_ALLOWED.iter().map(|(n, lo, hi)| format!("{} {}-{}", n, lo, hi)).collect::<Vec<_>>().join(", ")
                ));
            };
            let value = value.unwrap_or("").trim();
            match value.parse::<u64>() {
                Ok(v) if (lo..=hi).contains(&v) => {}
                _ => return Err(format!("valoarea `{}` pentru {} iese din limite ({}-{}); OS-ul a refuzat-o", value, name, lo, hi)),
            }
            let path = format!("{}/agent.setari", root);
            let old = read(&path, 0).unwrap_or_default();
            write_whole(&path, set_value(&old, name, value).as_bytes())?;
            let _ = cyber_user::sync(root);
            Ok(format!("{} = {} scris in {} (de la urmatoarea pornire a agentului)", name, value, path))
        }
        other => Err(format!("actiunea `{}` nu e in lista alba a reparatiilor (doar opreste_lucrarea, setare); OS-ul a refuzat-o", other)),
    }
}

/// P6-220: the most files of one proposal and the largest one.
pub const PROPOSAL_FILES: usize = 8;
pub const PROPOSAL_FILE_MAX: usize = 128 * 1024;

/// P6-220: a path a proposal may write: a valid repository path outside the CI workflows and the project's rules.
pub fn proposal_path(path: &str) -> bool {
    const KEPT: [&str; 3] = ["LICENSE", "CLAUDE.md", "docs/DECISIONS.md"];
    github::valid_path(path) && !path.starts_with(".github/") && path != ".github" && !KEPT.contains(&path) && !path.starts_with("docs/agent/propuneri/")
}

/// P6-220: a proposal's subject: lowercase, digits, '-', 1..=40, not starting or ending with '-'.
pub fn subject_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && !s.starts_with('-') && !s.ends_with('-') && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// P6-220: one line into `<root>/agent/propuneri.txt`.
fn note_proposal(root: &str, line: &str) {
    let dir = format!("{}/agent", root);
    let _ = cyber_user::create(&dir, true);
    let path = format!("{}/propuneri.txt", dir);
    let mut text = read(&path, 0).unwrap_or_default();
    if text.len() > 64 * 1024 {
        let cut = text[text.len() / 2..].find('\n').map_or(text.len(), |i| text.len() / 2 + i + 1);
        text = text[cut..].to_string();
    }
    text.push_str(line);
    text.push('\n');
    if write_whole(&path, text.as_bytes()).is_ok() {
        let _ = cyber_user::sync(&dir);
    }
}

/// P6-220: `citeste_cod`.
fn read_code(gh: Option<&Gh>, path: &str, from: usize) -> Result<String, String> {
    let gh = gh.ok_or("uneltele de cod nu sunt pornite aici")?;
    let token = gh.token.as_deref().ok_or_else(|| format!("lipseste tokenul GitHub ({})", gh.conf.token))?;
    let path = path.trim().trim_matches('/');
    match github::get_file(&gh.conf, "agent", token, &gh.conf.code, path, "main")? {
        github::Found::Dir(names) => Ok(format!("{}/ pe main ({}):\n{}\n", path, gh.conf.code, names.join("\n"))),
        github::Found::File(data) => {
            let total = data.len();
            let start = from.min(total);
            let mut end = (start + READ_MAX).min(total);
            while end < total && end > start && (data[end] & 0xc0) == 0x80 {
                end -= 1;
            }
            let mut text = String::from_utf8_lossy(&data[start..end]).into_owned();
            if end < total {
                text.push_str(&format!("\n(taiat: {} din {} octeti; continua cu de_la = {})", end - start, total, end));
            }
            Ok(text)
        }
    }
}

/// P6-220: `propune`; Ok = the branch and what was written, Err = refused (with why).
fn propose(gh: Option<&Gh>, input: &Value) -> Result<String, String> {
    let gh = gh.ok_or("uneltele de cod nu sunt pornite aici")?;
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let subject = text("subiect");
    if !subject_ok(&subject) {
        return Err(format!("subiectul `{}` nu e permis (litere mici, cifre, -, cel mult 40); OS-ul a refuzat propunerea", subject));
    }
    let (why, proof) = (text("explicatie"), text("dovada"));
    if why.is_empty() || proof.is_empty() {
        return Err("lipseste explicatia sau dovada; OS-ul a refuzat propunerea".to_string());
    }
    let files = input.get("fisiere").and_then(Value::as_array).unwrap_or(&[]);
    if files.is_empty() || files.len() > PROPOSAL_FILES {
        return Err(format!("o propunere are intre 1 si {} fisiere (aici {}); OS-ul a refuzat-o", PROPOSAL_FILES, files.len()));
    }
    let mut list: Vec<(String, &str)> = Vec::new();
    for f in files {
        let path = f.get("cale").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let data = f.get("continut").and_then(Value::as_str).ok_or_else(|| format!("{}: lipseste continutul", path))?;
        if !proposal_path(&path) {
            return Err(format!("calea `{}` nu e permisa intr-o propunere (fara .github/, LICENSE, CLAUDE.md, docs/DECISIONS.md, fara ..); OS-ul a refuzat-o", path));
        }
        if data.len() > PROPOSAL_FILE_MAX {
            return Err(format!("{}: {} octeti, peste {} KiB; OS-ul a refuzat-o", path, data.len(), PROPOSAL_FILE_MAX / 1024));
        }
        if list.iter().any(|(p, _)| *p == path) {
            return Err(format!("{} apare de doua ori", path));
        }
        list.push((path, data));
    }
    let token = gh.token.as_deref().ok_or_else(|| format!("lipseste tokenul GitHub ({})", gh.conf.token))?;
    let stamp = crate::now();
    let branch = format!("agent/{}-{}", stamp[..10].replace('-', ""), subject);
    let repo = &gh.conf.code;
    github::create_branch(&gh.conf, "agent", token, repo, &branch)?;
    let note_path = format!("docs/agent/propuneri/{}.md", branch.trim_start_matches("agent/"));
    let note = format!(
        "# Propunerea agentului de bord: {}\n\n**Ramura:** `{}` · **scrisa:** {} (ceasul OS-ului) · **stare:** de verificat de sesiunea OS (C-166 pasul 5)\n\n## Explicatia\n{}\n\n## Dovada\n{}\n\n## Fisierele\n{}",
        subject,
        branch,
        stamp,
        why,
        proof,
        list.iter().map(|(p, d)| format!("- `{}` ({} octeti)\n", p, d.len())).collect::<String>()
    );
    let mut out = format!("ramura {} facuta din main in {}\n", branch, repo);
    let msg = format!("agent (propunere): {}", subject);
    let r = github::put_contents_on(&gh.conf, "agent", token, repo, &note_path, note.as_bytes(), &msg, Some(&branch))?;
    out.push_str(&format!("{} {}\n", note_path, r));
    for (path, data) in &list {
        match github::put_contents_on(&gh.conf, "agent", token, repo, path, data.as_bytes(), &msg, Some(&branch)) {
            Ok(r) => out.push_str(&format!("{} {}\n", path, r)),
            Err(e) => return Err(format!("{}{}: NU s-a scris: {} (propunerea e incompleta pe {})", out, path, e, branch)),
        }
    }
    out.push_str("propunerea asteapta verificarea sesiunii OS; nimic nu ajunge pe main fara ea\n");
    Ok(out)
}

/// Is `path` one the agent may read? /usb, /disk, /tmp and the agent's own root (the tests keep it in /etc).
pub fn allowed(path: &str, root: &str) -> bool {
    let inside = ["/usb", "/disk", "/tmp", root].iter().any(|r| path == *r || path.starts_with(&format!("{}/", r)));
    inside && !path.split('/').any(|p| p == "..")
}

/// Is `command` on the white list? Only commands that read; no `cat`/`compara` (they run in the kernel, where the
/// secret files are not hidden): files go through `citeste`, which runs in this program.
pub fn white_listed(command: &str) -> bool {
    if command.contains([';', '|', '&', '<', '>', '\n', '\r', '`', '$']) {
        return false;
    }
    let w: Vec<&str> = command.split_whitespace().collect();
    let plain_word = |s: &str| s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    match w.as_slice() {
        ["ls"] => true,
        ["ls", p] => !p.split('/').any(|x| x == ".."),
        ["mem"] | ["uptime"] | ["lspci"] | ["lsusb"] | ["discuri"] | ["net"] | ["cpu"] | ["hw"] | ["porneste"] => true,
        ["hw", "lista"] | ["hw", "lista", "--detalii"] | ["usb", "timp"] | ["lucrare", "stare"] => true,
        ["log", rest @ ..] => rest.len() <= 2 && rest.iter().all(|a| plain_word(a)),
        ["hub", what] => matches!(*what, "stare" | "comenzi" | "decizii"),
        ["hub", what, p] => matches!(*what, "stare" | "comenzi" | "decizii") && plain_word(p),
        _ => false,
    }
}

fn list(path: &str) -> Result<String, String> {
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let n = cyber_user::list(path, &mut buf);
    if n < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(n)));
    }
    Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

fn read(path: &str, from: usize) -> Result<String, String> {
    let fd = cyber_user::open(path);
    if fd < 0 {
        return Err(format!("{}: {}", path, cyber_user::error_text(fd)));
    }
    let mut data = Vec::new();
    let mut seen = 0usize;
    let mut buf = [0u8; 4096];
    let mut more = false;
    loop {
        let n = cyber_user::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        let start = from.saturating_sub(seen).min(chunk.len());
        seen += chunk.len();
        data.extend_from_slice(&chunk[start..]);
        if data.len() > READ_MAX {
            data.truncate(READ_MAX);
            more = true;
            break;
        }
    }
    cyber_user::close(fd);
    let mut text = String::from_utf8_lossy(&data).into_owned();
    if more {
        text.push_str(&format!("\n(taiat la 32 KiB; continua cu de_la = {})", from + READ_MAX));
    }
    Ok(text)
}

/// Does one tool call. `log` = one line for the screen and the session file. Ok = result, Err = error for Claude.
pub fn run(name: &str, input: &Value, root: &str, gh: Option<&Gh>, term: &mut Option<i64>, log: &mut String) -> Result<String, String> {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    let number = |k: &str| input.get(k).and_then(Value::as_i64);
    match name {
        "fisiere" | "citeste" => {
            let path = text("cale").ok_or("lipseste campul cale")?;
            *log = format!("{} {}", name, path);
            if !allowed(&path, root) {
                return Err(format!("calea {} nu e permisa (doar /usb, /disk, /tmp, {}; fara ..)", path, root));
            }
            if name == "fisiere" { list(&path) } else { read(&path, number("de_la").unwrap_or(0).max(0) as usize) }
        }
        "citeste_jurnal" => {
            let n = number("n").unwrap_or(40).clamp(1, 400);
            let command = match text("cuvant") {
                Some(w) if !w.is_empty() => format!("log {} {}", w, n),
                _ => format!("log {}", n),
            };
            *log = format!("citeste_jurnal: {}", command);
            if !white_listed(&command) {
                return Err("cuvantul poate avea doar litere, cifre, - _ .".to_string());
            }
            claude_tools::run_command(&command, term)
        }
        "stare" => {
            *log = "stare".to_string();
            let mut out = String::new();
            for c in ["uptime", "mem", "lucrare stare", "net"] {
                out.push_str(&format!("$ {}\n", c));
                match claude_tools::run_command(c, term) {
                    Ok(t) | Err(t) => out.push_str(&t),
                }
                out.push('\n');
            }
            Ok(out)
        }
        // P6-211 (C-166 step 2): the dashboard as the OS already logged it (`kw tabloul …` rows); no new board command.
        "tablou" => {
            let n = number("n").unwrap_or(12).clamp(1, 100);
            let command = format!("log tabloul {}", n);
            *log = format!("tablou: {}", command);
            let rows = claude_tools::run_command(&command, term)?;
            let none = rows.trim().is_empty() || rows.contains("(niciun rand)");
            Ok(format!(
                "tablou: ultimele {} citiri ale tabloului placii video din jurnal (scrise de OS in timpul testelor; agentul nu trimite nicio comanda placii){}\n{}",
                n,
                if none { " — nicio citire inca de la pornire" } else { "" },
                rows
            ))
        }
        "ruleaza" => {
            let command = text("comanda").ok_or("lipseste campul comanda")?;
            let command = command.trim().to_string();
            if !white_listed(&command) {
                *log = format!("ruleaza: {} -> REFUZAT", command);
                return Err(format!("comanda `{}` nu e in lista alba (doar comenzi de citire); OS-ul a refuzat-o", command));
            }
            *log = format!("ruleaza: {}", command);
            claude_tools::run_command(&command, term)
        }
        // P6-220 (C-166 step 5): the code, read only, and the proposals on `agent/…` branches.
        "citeste_cod" => {
            let path = text("cale").unwrap_or_default();
            *log = format!("citeste_cod {}", path);
            read_code(gh, &path, number("de_la").unwrap_or(0).max(0) as usize)
        }
        "propune" => {
            let subject = text("subiect").unwrap_or_default();
            let n = input.get("fisiere").and_then(Value::as_array).map_or(0, |a| a.len());
            let r = propose(gh, input);
            *log = format!("propune: {} ({} fisiere) -> {}", subject, n, if r.is_ok() { "FACUT" } else { "REFUZAT" });
            let first = |t: &str| t.lines().next().unwrap_or("").to_string();
            note_proposal(root, &format!("{} propune {} ({} fisiere) -> {}", crate::now(), subject, n, match &r {
                Ok(t) => format!("FACUT: {}", first(t)),
                Err(t) => format!("REFUZAT: {}", first(t)),
            }));
            r
        }
        // P6-219 (C-166 step 4): the white-listed repairs.
        "repara" => {
            let action = text("actiune").unwrap_or_default();
            let name = text("nume").unwrap_or_default();
            let value = text("valoare");
            let r = repair(&action, &name, value.as_deref(), root, term);
            let what = format!("{} {}{}", action, name, value.as_deref().map(|v| format!(" = {}", v)).unwrap_or_default());
            *log = format!("repara: {} -> {}", what, if r.is_ok() { "FACUT" } else { "REFUZAT" });
            let first = |t: &str| t.lines().next().unwrap_or("").to_string();
            note_repair(root, &format!("{} {} -> {}", crate::now(), what, match &r {
                Ok(t) => format!("FACUT: {}", first(t)),
                Err(t) => format!("REFUZAT: {}", first(t)),
            }));
            r
        }
        other => {
            *log = format!("unealta necunoscuta: {}", other);
            Err(format!("unealta necunoscuta: {}", other))
        }
    }
}

/// P6-223: how long the agent waits for a signed board test (the longest board tests run tens of minutes).
const BOARD_MS: u64 = 120 * 60 * 1000;

/// P6-222 (C-166 step 6, part 1): one command of the centre's channel (`live/os-agent-comenzi.md`), decided here, by the
/// OS: `stare`, `tablou [n]`, `jurnal [n] [cuvant]`, `repara opreste_lucrarea <nume>`, `repara setare <nume> <valoare>`
/// and every read-only shell command of the white list. P6-223 (part 2): a command on the board (`test-placa …`) only
/// with the centre's signature `sig`, checked by the kernel (`agent-placa`); everything else is refused.
pub fn channel_command(nr: u32, text: &str, sig: Option<&str>, root: &str, term: &mut Option<i64>) -> Result<String, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let num = |w: &str| Value::Number(w.to_string());
    let mut note = String::new();
    match words.as_slice() {
        ["stare"] => run("stare", &Value::Null, root, None, term, &mut note),
        ["tablou"] => run("tablou", &Value::Null, root, None, term, &mut note),
        ["tablou", n] if n.parse::<u32>().is_ok() => run("tablou", &Value::object([("n", num(n))]), root, None, term, &mut note),
        ["jurnal"] => run("citeste_jurnal", &Value::Null, root, None, term, &mut note),
        ["jurnal", n] if n.parse::<u32>().is_ok() => run("citeste_jurnal", &Value::object([("n", num(n))]), root, None, term, &mut note),
        ["jurnal", n, w] if n.parse::<u32>().is_ok() => {
            run("citeste_jurnal", &Value::object([("n", num(n)), ("cuvant", Value::str(w))]), root, None, term, &mut note)
        }
        ["repara", "opreste_lucrarea", name] => {
            run("repara", &Value::object([("actiune", Value::str("opreste_lucrarea")), ("nume", Value::str(name))]), root, None, term, &mut note)
        }
        ["repara", "setare", name, value] => run(
            "repara",
            &Value::object([("actiune", Value::str("setare")), ("nume", Value::str(name)), ("valoare", Value::str(value))]),
            root,
            None,
            term,
            &mut note,
        ),
        // P6-223 (C-166 step 6, part 2): a board command only with the centre's signature; the kernel checks it
        // (the key is kernel-only) together with the number, the board key and the board test's own guards.
        [first, ..] if *first == "test-placa" => match sig {
            None => Err("comanda pe placa fara semnatura centrului (`· semn: <64 hex>`); OS-ul a refuzat-o".to_string()),
            Some(sig) if sig.len() != 64 || !sig.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Err("semnatura trebuie sa aiba 64 de cifre hex; OS-ul a refuzat comanda".to_string())
            }
            Some(sig) => {
                let out = claude_tools::run_command_for(&format!("agent-placa {} {} {}", nr, sig, text.trim()), term, BOARD_MS)?;
                if out.trim_start().starts_with("REFUZAT:") || out.contains("\nREFUZAT:") {
                    Err(out)
                } else {
                    Ok(out)
                }
            }
        },
        _ if white_listed(text.trim()) => claude_tools::run_command(text.trim(), term),
        _ => Err(format!(
            "`{}` nu e in lista canalului (stare, tablou, jurnal, repara ..., comenzile de citire din lista alba); OS-ul a refuzat-o",
            text.trim()
        )),
    }
}
