//! The long-job manager (task P6-140, command C-158 point 8, tool 8): a job of hours runs on its own.
//!
//! A job is a card `<stick>/lucrari/<name>.txt`: when to start (`porneste HH:MM` or `porneste +N` minutes), when to stop
//! (`opreste HH:MM`, `durata N` minutes worked, or a stop file), how often to report to the hub (`raport N` minutes), how
//! many rounds of its steps (`repeta N`, 0 = until stopped) and the steps (`pas <command>`, read-only commands from the
//! automatic mode's allow-list only). The state `<name>.stare` is written to the stick after every step, so after a power
//! cut the OS takes the job up again at boot from the next step; a step cut off twice in a row is not tried a third time.
//! `lucrare opreste <name>` (or the file `<name>.opreste`) stops it between two steps.
//! P6-225 (C-166 step 4, "reluarea unui pas"): `lucrare reia <name>` takes up again, from the very step it gave up on, a
//! job the OS stopped because that step was cut off twice in a row — at most `MAX_ASKED` times per job, so a step that
//! keeps bringing the machine down cannot be retried forever. It only marks the job; the job goes on at the next run
//! (`lucrare <name>`, its line in the automatic list, or the boot). The on-board agent may ask for it (`repara`).
//! Idea: job schedulers with checkpoints (cron, systemd timers, batch queues); our own code.

use super::auto::{Report, date_stamp, free_name, now_text, say};
use crate::fs::{self, FsError, Kind};
use alloc::{format, string::String, vec::Vec};

/// Where the job cards and their states live: the stick; in the `auto` test builds the test disk.
#[cfg(not(feature = "test-auto"))]
const DIR: &str = "/usb/lucrari";
#[cfg(feature = "test-auto")]
const DIR: &str = "/disk/lucrari";

/// Where the reports go (sent to the hub by `trimite`); in the test builds next to the jobs, never sent.
#[cfg(not(feature = "test-auto"))]
fn reports_dir() -> &'static str {
    super::raport::hub_paths().0
}
#[cfg(feature = "test-auto")]
fn reports_dir() -> &'static str {
    "/disk/raport"
}

const MAX_STEPS: usize = 50;
const DEFAULT_REPORT_MIN: u64 = 30;
const DEFAULT_STEP_MIN: u64 = 30;
/// A step cut off this many times in a row (power cut, hang + safety clock) stops the job.
const MAX_CUTS: u64 = 2;
/// P6-225: how many times `lucrare reia` may take up one job the OS gave up on.
const MAX_ASKED: u64 = 2;
/// P6-225: the state a job asked to be taken up again is in (counts as running: the boot takes it up too).
const ASKED: &str = "de reluat";

/// Who runs the job: Adrian at the shell, a line of the automatic list, or the boot that finds a job cut off.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Shell,
    List,
    Boot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Start {
    Now,
    At(u8, u8),
    /// In units (minutes; seconds in the test builds with `unitate secunde`).
    After(u64),
}

/// The job card, parsed.
#[derive(Debug)]
struct Card {
    start: Start,
    stop_at: Option<(u8, u8)>,
    /// Seconds of work, over all resumptions.
    duration_s: Option<u64>,
    report_s: u64,
    /// Rounds of the step list; 0 = until stopped.
    rounds: u64,
    step_s: u64,
    /// Seconds per unit (60; 1 with `unitate secunde` in the test builds).
    unit_s: u64,
    steps: Vec<String>,
}

fn parse_card(text: &str) -> Result<Card, String> {
    let mut c = Card {
        start: Start::Now,
        stop_at: None,
        duration_s: None,
        report_s: DEFAULT_REPORT_MIN * 60,
        rounds: 1,
        step_s: DEFAULT_STEP_MIN * 60,
        unit_s: 60,
        steps: Vec::new(),
    };
    // Numbers first in units, converted at the end (so `unitate` may come anywhere).
    let (mut report_u, mut step_u, mut duration_u, mut after_u) = (None, None, None, None);
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (line, ""),
        };
        let bad = |why: &str| format!("randul {}: `{}`: {}", n + 1, line, why);
        match key {
            "pas" => {
                if rest.is_empty() {
                    return Err(bad("pas fara comanda"));
                }
                if c.steps.len() >= MAX_STEPS {
                    return Err(bad("prea multi pasi (cel mult 50)"));
                }
                step_allowed(rest).map_err(|why| bad(&format!("pasul nu e permis: {}", why)))?;
                c.steps.push(String::from(rest));
            }
            "porneste" => {
                if let Some(m) = rest.strip_prefix('+') {
                    after_u = Some(number(m, 0, 24 * 60).ok_or_else(|| bad("porneste +N: N minute intre 0 si 1440"))?);
                } else {
                    let (h, m) = clock(rest).ok_or_else(|| bad("porneste HH:MM sau porneste +N"))?;
                    c.start = Start::At(h, m);
                }
            }
            "opreste" => c.stop_at = Some(clock(rest).ok_or_else(|| bad("opreste HH:MM"))?),
            "durata" => duration_u = Some(number(rest, 1, 7 * 24 * 60).ok_or_else(|| bad("durata N: minute intre 1 si 10080"))?),
            "raport" => report_u = Some(number(rest, 1, 24 * 60).ok_or_else(|| bad("raport N: minute intre 1 si 1440"))?),
            "repeta" => c.rounds = number(rest, 0, 1_000_000).ok_or_else(|| bad("repeta N: 0 (pana la oprire) sau un numar"))?,
            "ceas-pas" => step_u = Some(number(rest, 1, 24 * 60).ok_or_else(|| bad("ceas-pas N: minute intre 1 si 1440"))?),
            #[cfg(feature = "test-auto")]
            "unitate" if rest == "secunde" => c.unit_s = 1,
            _ => return Err(bad("cuvant necunoscut (porneste, opreste, durata, raport, repeta, ceas-pas, pas)")),
        }
    }
    if c.steps.is_empty() {
        return Err(String::from("fisa nu are niciun `pas`"));
    }
    if c.rounds == 0 && c.stop_at.is_none() && duration_u.is_none() {
        return Err(String::from("`repeta 0` (pana la oprire) cere `opreste HH:MM` sau `durata N`"));
    }
    if let Some(a) = after_u {
        c.start = Start::After(a);
    }
    if let Some(r) = report_u {
        c.report_s = r * c.unit_s;
    }
    if let Some(s) = step_u {
        c.step_s = s * c.unit_s;
    }
    c.duration_s = duration_u.map(|d| d * c.unit_s);
    Ok(c)
}

/// A step: a command of the automatic mode's allow-list (reads only); in the test builds also `test-asteapta N`.
fn step_allowed(cmd: &str) -> Result<(), &'static str> {
    #[cfg(feature = "test-auto")]
    if let Some(n) = cmd.strip_prefix("test-asteapta ") {
        return number(n, 0, 600).map(|_| ()).ok_or("test-asteapta N (secunde)");
    }
    match cmd.split_whitespace().next() {
        Some("lucrare" | "live" | "test-placa") => Err("o lucrare nu porneste alta lucrare, canalul viu sau teste pe placa"),
        _ => super::auto::allowed(cmd),
    }
}

fn number(s: &str, lo: u64, hi: u64) -> Option<u64> {
    s.trim().parse::<u64>().ok().filter(|n| (lo..=hi).contains(n))
}

fn clock(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60 && s.trim().len() <= 5).then_some((h, m))
}

/// Seconds from `now` (h, m, s) until the next `target` (h, m); a target equal to now is a day away (never "already").
fn secs_until(now: (u8, u8, u8), target: (u8, u8)) -> u64 {
    let n = now.0 as u64 * 3600 + now.1 as u64 * 60 + now.2 as u64;
    let t = target.0 as u64 * 3600 + target.1 as u64 * 60;
    if t > n { t - n } else { t + 86_400 - n }
}

fn rtc_hms() -> Option<(u8, u8, u8)> {
    crate::drivers::rtc::now().map(|t| (t.hour, t.minute, t.second))
}

/// The state on the stick: everything needed to go on after a power cut.
#[derive(Default, Debug, PartialEq)]
struct State {
    /// `asteapta` / `in lucru` / `terminata` / `oprita` (empty = never run).
    stare: String,
    /// Rounds finished.
    tur: u64,
    /// The next step in the current round (0-based).
    pas: usize,
    facuti: u64,
    lucrat_s: u64,
    pornit: String,
    reluari: u64,
    /// `tur.pas` where the last cut happened, and how many cuts in a row there.
    taiat_la: String,
    taieri: u64,
    motiv: String,
    /// P6-225: how many times `lucrare reia` took it up.
    cerute: u64,
}

impl State {
    fn text(&self) -> String {
        format!(
            "stare={}\ntur={}\npas={}\nfacuti={}\nlucrat_s={}\npornit={}\nreluari={}\ntaiat_la={}\ntaieri={}\nmotiv={}\ncerute={}\n",
            self.stare, self.tur, self.pas, self.facuti, self.lucrat_s, self.pornit, self.reluari, self.taiat_la, self.taieri, self.motiv, self.cerute
        )
    }

    fn parse(text: &str) -> State {
        let mut s = State::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "stare" => s.stare = String::from(v),
                "tur" => s.tur = v.parse().unwrap_or(0),
                "pas" => s.pas = v.parse().unwrap_or(0),
                "facuti" => s.facuti = v.parse().unwrap_or(0),
                "lucrat_s" => s.lucrat_s = v.parse().unwrap_or(0),
                "pornit" => s.pornit = String::from(v),
                "reluari" => s.reluari = v.parse().unwrap_or(0),
                "taiat_la" => s.taiat_la = String::from(v),
                "taieri" => s.taieri = v.parse().unwrap_or(0),
                "motiv" => s.motiv = String::from(v),
                "cerute" => s.cerute = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        s
    }

    fn running(&self) -> bool {
        matches!(self.stare.as_str(), "asteapta" | "in lucru" | ASKED)
    }

    /// P6-225: whether the OS gave up on it because a step was cut off too many times in a row.
    fn gave_up(&self) -> bool {
        self.stare == "oprita" && self.taieri >= MAX_CUTS
    }

    /// P6-225: marks a job the OS gave up on to be taken up again from the same step; Err = refused, with why.
    fn ask_resume(&mut self) -> Result<(), String> {
        if !self.gave_up() {
            let now = if self.stare.is_empty() { String::from("nepornita") } else { self.stare.clone() };
            return Err(format!(
                "se reia doar o lucrare oprita de OS dupa {} intreruperi la rand ale aceluiasi pas; starea ei: {}{}",
                MAX_CUTS,
                now,
                if self.motiv.is_empty() { String::new() } else { format!(" ({})", self.motiv) }
            ));
        }
        if self.cerute >= MAX_ASKED {
            return Err(format!("a fost reluata la cerere de {} ori deja (cel mult {}); de vazut de Adrian", self.cerute, MAX_ASKED));
        }
        self.cerute += 1;
        self.stare = String::from(ASKED);
        self.taieri = 0;
        self.taiat_la = String::new();
        self.motiv = format!("reluare ceruta ({} din {})", self.cerute, MAX_ASKED);
        Ok(())
    }
}

fn path(name: &str, ext: &str) -> String {
    format!("{}/{}{}", DIR, name, ext)
}

/// The state as last written: `.stare`, or `.stare.nou` when the power went between writing it and putting it in place.
fn load_state(name: &str) -> Option<State> {
    let main = fs::read_all(&path(name, ".stare")).ok();
    let bytes = main.or_else(|| fs::read_all(&path(name, ".stare.nou")).ok())?;
    Some(State::parse(&String::from_utf8_lossy(&bytes)))
}

/// Written whole as `.stare.nou` (flushed), then put in place of `.stare`: a cut at any moment leaves one whole copy.
fn save_state(name: &str, st: &State) -> Result<(), String> {
    let (main, new) = (path(name, ".stare"), path(name, ".stare.nou"));
    crate::revenire::write_file(&new, st.text().as_bytes())?;
    let _ = fs::remove(&main);
    fs::rename(&new, &main).map_err(|e| format!("{}: {}", main, e.message()))?;
    let _ = fs::flush(DIR);
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// The shell command.
pub(super) fn command(args: &[&str]) {
    match args {
        [] | ["stare"] => {
            super::emit(&format!("lucrarile din {}:\n", DIR));
            let names = names();
            if names.is_empty() {
                super::emit("  niciuna (fisele: <nume>.txt; vezi docs/LUCRARI.md)\n");
            }
            for n in names {
                super::emit(&format!("  {}\n", summary(&n)));
            }
            if args.is_empty() {
                super::emit("folosire: lucrare <nume> | lucrare stare [nume] | lucrare opreste <nume> | lucrare reia <nume>\n");
            }
        }
        ["stare", name] => super::emit(&format!("{}\n", summary(name))),
        ["opreste", name] => super::emit(&stop_request(name)),
        ["reia", name] => super::emit(&resume_request(name)),
        [name] => {
            let text = run(name, Mode::Shell);
            super::emit(&text);
        }
        _ => super::emit("folosire: lucrare <nume> | lucrare stare [nume] | lucrare opreste <nume> | lucrare reia <nume>\n"),
    }
}

/// A `lucrare …` line of the automatic list: the job runs (pushing the list's safety clock while it works); the text
/// goes into the list's report.
pub(super) fn auto_line(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["lucrare", "stare"] => names().iter().map(|n| format!("{}\n", summary(n))).collect(),
        ["lucrare", "stare", n] => format!("{}\n", summary(n)),
        ["lucrare", "opreste", n] => stop_request(n),
        ["lucrare", "reia", n] => resume_request(n),
        ["lucrare", n] => run(n, Mode::List),
        _ => String::from("REFUZAT: folosire: lucrare <nume> | lucrare stare [nume] | lucrare opreste <nume> | lucrare reia <nume>\n"),
    }
}

/// At boot, before the automatic list: every job whose state says it was running (power cut, hang) goes on from its
/// next step. Returns how many were taken up.
pub fn resume_if_present() -> usize {
    let mut n = 0;
    for name in names() {
        if load_state(&name).is_some_and(|s| s.running()) {
            say(&format!("lucrare {}: gasita neterminata la pornire: o reiau\n", name));
            let text = run(&name, Mode::Boot);
            say(&text);
            crate::info!("lucrare {}: reluarea de la pornire s-a incheiat", name);
            n += 1;
        }
    }
    if n > 0 {
        super::auto::disarm();
    }
    n
}

/// The job names (cards `<name>.txt`), sorted.
fn names() -> Vec<String> {
    let mut v: Vec<String> = fs::list(DIR)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, k)| *k == Kind::File)
        .filter_map(|(n, _)| {
            let low = n.to_ascii_lowercase();
            let stem = low.strip_suffix(".txt")?;
            (valid_name(stem) && !stem.ends_with("-jurnal")).then(|| String::from(stem))
        })
        .collect();
    v.sort();
    v
}

fn summary(name: &str) -> String {
    if !valid_name(name) {
        return format!("lucrare {}: nume nepermis (litere mici, cifre, - si _)", name);
    }
    match load_state(name) {
        None if fs::kind(&path(name, ".txt")).is_ok() => format!("{}: nepornita inca", name),
        None => format!("{}: nu exista ({} lipseste)", name, path(name, ".txt")),
        Some(s) => format!(
            "{}: {}{} · tururi terminate {}, pasul urmator {}, pasi facuti {}, lucrat {} s, reluari {}",
            name,
            s.stare,
            if s.motiv.is_empty() { String::new() } else { format!(" ({})", s.motiv) },
            s.tur,
            s.pas + 1,
            s.facuti,
            s.lucrat_s,
            s.reluari
        ),
    }
}

fn stop_request(name: &str) -> String {
    if !valid_name(name) {
        return format!("lucrare {}: nume nepermis\n", name);
    }
    let p = path(name, ".opreste");
    match crate::revenire::write_file(&p, now_text().as_bytes()) {
        Ok(()) => format!("lucrare {}: oprirea ceruta ({}); se opreste dupa pasul in lucru\n", name, p),
        Err(e) => format!("lucrare {}: oprirea NU s-a putut cere: {}\n", name, e),
    }
}

/// P6-225: `lucrare reia <name>` — only marks the job (see `State::ask_resume`); it goes on at its next run.
fn resume_request(name: &str) -> String {
    if !valid_name(name) {
        return format!("lucrare {}: nume nepermis\n", name);
    }
    if fs::kind(&path(name, ".txt")).is_err() {
        return format!("lucrare {}: reluarea REFUZATA: {} nu exista\n", name, path(name, ".txt"));
    }
    let Some(mut st) = load_state(name) else {
        return format!("lucrare {}: reluarea REFUZATA: nepornita inca (nimic de reluat)\n", name);
    };
    if let Err(why) = st.ask_resume() {
        return format!("lucrare {}: reluarea REFUZATA: {}\n", name, why);
    }
    match save_state(name, &st) {
        Ok(()) => format!(
            "lucrare {}: reluarea ceruta ({} din {}): turul {}, pasul {} se reia la urmatoarea rulare (lucrare {}, randul din lista sau pornirea)\n",
            name,
            st.cerute,
            MAX_ASKED,
            st.tur + 1,
            st.pas + 1,
            name
        ),
        Err(e) => format!("lucrare {}: reluarea NU s-a putut scrie: {}\n", name, e),
    }
}

fn stop_asked(name: &str) -> bool {
    fs::kind(&path(name, ".opreste")).is_ok()
}

/// Keeps the safety clock ahead of the job: the list's clock in the list, armed at boot, nothing at Adrian's shell.
fn clock_push(mode: Mode, ms: u64) {
    match mode {
        Mode::List => super::auto::keep_alive(ms),
        Mode::Boot => super::auto::arm_or_push(ms),
        Mode::Shell => {}
    }
}

fn now_ms() -> u64 {
    crate::time::uptime_ms().unwrap_or(0)
}

/// Runs (or takes up again) the job; the text = what the list's report keeps.
pub(super) fn run(name: &str, mode: Mode) -> String {
    Job::open(name, mode).go()
}

/// One run of a job: the card, the state, the logs.
struct Job<'a> {
    name: &'a str,
    mode: Mode,
    log: Option<Report>,
    /// The last step's output (for the reports).
    last: String,
    last_report_ms: u64,
    reports: u64,
}

impl<'a> Job<'a> {
    fn open(name: &'a str, mode: Mode) -> Job<'a> {
        Job { name, mode, log: None, last: String::new(), last_report_ms: 0, reports: 0 }
    }

    fn go(&mut self) -> String {
        let name = self.name;
        if !valid_name(name) {
            return format!("lucrare {}: REFUZATA: nume nepermis (litere mici, cifre, - si _, cel mult 32)\n", name);
        }
        let card = match fs::read_all(&path(name, ".txt")) {
            Ok(b) => parse_card(&String::from_utf8_lossy(&b)),
            Err(FsError::NotFound) => Err(format!("{} nu exista", path(name, ".txt"))),
            Err(e) => Err(format!("{}: {}", path(name, ".txt"), e.message())),
        };
        let card = match card {
            Ok(c) => c,
            Err(why) => return format!("lucrare {}: REFUZATA: {} (nimic nu s-a rulat)\n", name, why),
        };
        let mut st = load_state(name).unwrap_or_default();
        let resumed = st.running();
        if !resumed {
            st = State::default();
        }
        self.log = Some(Report::open(&path(name, "-jurnal.txt")));
        let mut head = String::new();
        if resumed && st.stare == ASKED {
            // P6-225: taken up on request (`lucrare reia`): not a cut; the cut counter starts again from this step.
            st.reluari += 1;
            // One more cut of this same step (counted at the next boot) and the OS gives up on it again.
            st.taiat_la = format!("{}.{}", st.tur, st.pas);
            st.taieri = 1;
            head.push_str(&format!(
                "lucrare {}: RELUATA la cerere ({}): turul {}, pasul {}; {} pasi facuti inainte\n",
                name,
                st.motiv,
                st.tur + 1,
                st.pas + 1,
                st.facuti
            ));
        } else if resumed {
            // The step it was on when it was cut (power, hang + safety clock, or Adrian restarting).
            let at = format!("{}.{}", st.tur, st.pas);
            st.taieri = if st.taiat_la == at { st.taieri + 1 } else { 1 };
            st.taiat_la = at;
            st.reluari += 1;
            head.push_str(&format!(
                "lucrare {}: RELUATA dupa o intrerupere (reluarea nr. {}): turul {}, pasul {}; {} pasi facuti inainte\n",
                name,
                st.reluari,
                st.tur + 1,
                st.pas + 1,
                st.facuti
            ));
            if st.taieri >= MAX_CUTS {
                st.stare = String::from("oprita");
                st.motiv = format!("pasul {} din turul {} a fost intrerupt de {} ori la rand; nu-l mai reiau", st.pas + 1, st.tur + 1, st.taieri);
                let _ = save_state(name, &st);
                let text = format!("{}lucrare {}: OPRITA: {}\n", head, name, st.motiv);
                self.note(&text);
                self.report(&card, &st, "oprita");
                return text;
            }
        }
        say(&head);
        self.note(&head);

        // Waiting for the start (only before it has started once).
        if st.pornit.is_empty() {
            st.stare = String::from("asteapta");
            let wait_s = match card.start {
                Start::Now => 0,
                Start::After(u) => u * card.unit_s,
                // The start minute itself counts as "now" (not a day later).
                Start::At(h, m) => rtc_hms().map(|n| secs_until(n, (h, m))).filter(|&w| w <= 86_400 - 60).unwrap_or(0),
            };
            let _ = save_state(name, &st);
            if wait_s > 0 {
                let line = format!("lucrare {}: astept pornirea ({} s, pana la {})\n", name, wait_s, describe_start(&card));
                say(&line);
                self.note(&line);
                let until = now_ms() + wait_s * 1000;
                while now_ms() < until {
                    if stop_asked(name) {
                        return self.end(&card, &mut st, "oprita", "la comanda, inainte de pornire");
                    }
                    clock_push(self.mode, 60_000);
                    crate::sched::sleep_ms(250);
                }
            }
            st.pornit = now_text();
        }
        st.stare = String::from("in lucru");
        let _ = save_state(name, &st);
        let line = format!(
            "lucrare {}: {} ({} pasi pe tur, {}; raport la {} s{}{})\n",
            name,
            if resumed { "continua" } else { "pornita" },
            card.steps.len(),
            if card.rounds == 0 { String::from("tururi pana la oprire") } else { format!("{} tururi", card.rounds) },
            card.report_s,
            card.stop_at.map(|(h, m)| format!(", opreste la {:02}:{:02}", h, m)).unwrap_or_default(),
            card.duration_s.map(|d| format!(", durata {} s", d)).unwrap_or_default()
        );
        say(&line);
        self.note(&line);
        self.report(&card, &st, if resumed { "reluata" } else { "pornita" });

        // When to stop: the hour (from the clock now) and the duration left.
        let t0 = now_ms();
        let worked0 = st.lucrat_s;
        let stop_hour_ms = card.stop_at.and_then(|t| rtc_hms().map(|n| t0 + secs_until(n, t) * 1000));
        let stop_dur_ms = card.duration_s.map(|d| t0 + d.saturating_sub(worked0) * 1000);
        loop {
            if card.rounds != 0 && st.tur >= card.rounds {
                return self.end(&card, &mut st, "terminata", "toate tururile");
            }
            while st.pas < card.steps.len() {
                st.lucrat_s = worked0 + (now_ms() - t0) / 1000;
                if stop_asked(name) {
                    return self.end(&card, &mut st, "oprita", "la comanda");
                }
                if stop_hour_ms.is_some_and(|d| now_ms() >= d) {
                    return self.end(&card, &mut st, "oprita", "la ora de oprire");
                }
                if stop_dur_ms.is_some_and(|d| now_ms() >= d) {
                    return self.end(&card, &mut st, "oprita", "durata s-a implinit");
                }
                let cmd = card.steps[st.pas].clone();
                clock_push(self.mode, card.step_s * 1000);
                let (text, ms) = self.step(&cmd);
                self.note(&format!("== turul {}, pasul {}/{}: {} ({}) ==\n{}-- gata in {} ms\n", st.tur + 1, st.pas + 1, card.steps.len(), cmd, now_text(), text, ms));
                self.last = format!("ultimul pas: {} (turul {}, pasul {}, gata in {} ms)\n{}", cmd, st.tur + 1, st.pas + 1, ms, tail(&text, 30));
                st.pas += 1;
                st.facuti += 1;
                st.lucrat_s = worked0 + (now_ms() - t0) / 1000;
                if let Err(e) = save_state(name, &st) {
                    say(&format!("lucrare {}: starea nu s-a scris ({}); o reluare ar reface pasi\n", name, e));
                }
                if now_ms().saturating_sub(self.last_report_ms) >= card.report_s * 1000 {
                    self.report(&card, &st, "in lucru");
                }
            }
            st.pas = 0;
            st.tur += 1;
            let _ = save_state(name, &st);
            say(&format!("lucrare {}: turul {} gata ({} pasi facuti)\n", name, st.tur, st.facuti));
        }
    }

    /// One step: run like a command of the automatic list, its output kept.
    fn step(&mut self, cmd: &str) -> (String, u64) {
        say(&format!("lucrare {}: {}\n", self.name, cmd));
        let t0 = now_ms();
        #[cfg(feature = "test-auto")]
        if let Some(n) = cmd.strip_prefix("test-asteapta ") {
            let s = n.trim().parse::<u64>().unwrap_or(0);
            crate::sched::sleep_ms(s * 1000);
            return (format!("test-asteapta: {} s\n", s), now_ms() - t0);
        }
        super::emit_capture_begin();
        super::program_capture_begin();
        super::dispatch(cmd);
        let mut text = super::emit_capture_end();
        text.push_str(&super::program_capture_end());
        (text, now_ms() - t0)
    }

    fn end(&mut self, card: &Card, st: &mut State, how: &str, why: &str) -> String {
        st.stare = String::from(how);
        st.motiv = String::from(why);
        let _ = save_state(self.name, st);
        if how == "oprita" && stop_asked(self.name) {
            let _ = fs::remove(&path(self.name, ".opreste"));
        }
        let text = format!(
            "lucrare {}: {} ({}): {} tururi terminate, {} pasi facuti, lucrat {} s, reluari {}, rapoarte {}\n",
            self.name,
            how.to_ascii_uppercase(),
            why,
            st.tur,
            st.facuti,
            st.lucrat_s,
            st.reluari,
            self.reports + 1
        );
        self.note(&text);
        self.report(card, st, how);
        text
    }

    fn note(&mut self, text: &str) {
        if let Some(l) = self.log.as_mut() {
            l.add(text);
        }
    }

    /// A report for the hub: where the job is, the last step's output; then `trimite` (not in the test builds).
    fn report(&mut self, card: &Card, st: &State, moment: &str) {
        self.last_report_ms = now_ms();
        self.reports += 1;
        let dir = reports_dir();
        if fs::kind(dir).is_err() {
            let _ = fs::create(dir, Kind::Dir);
        }
        let file = free_name(&format!("{}/lucrare-{}-{}", dir, self.name, date_stamp()));
        let mut r = Report::create(&file);
        r.add(&format!(
            "== lucrare lunga `{}`: raport Cyber Universe OS (P6-140) ==\nnucleu: cyber-universe {}\nora: {}\nmoment: {} (raportul nr. {} al acestei rulari)\n\
             stare: {}{}\npornita: {} · reluari: {}\nprogres: {} tururi terminate{}, pasul urmator {} din {}; pasi facuti {}; lucrat {} s\n\
             oprire: {}{}{}\n{}\n",
            self.name,
            env!("CARGO_PKG_VERSION"),
            now_text(),
            moment,
            self.reports,
            st.stare,
            if st.motiv.is_empty() { String::new() } else { format!(" ({})", st.motiv) },
            if st.pornit.is_empty() { "inca nu" } else { st.pornit.as_str() },
            st.reluari,
            st.tur,
            if card.rounds == 0 { String::from(" (pana la oprire)") } else { format!(" din {}", card.rounds) },
            st.pas + 1,
            card.steps.len(),
            st.facuti,
            st.lucrat_s,
            card.stop_at.map(|(h, m)| format!("la {:02}:{:02}; ", h, m)).unwrap_or_default(),
            card.duration_s.map(|d| format!("dupa {} s lucrate; ", d)).unwrap_or_default(),
            format!("la comanda: lucrare opreste {} (fisierul {})", self.name, path(self.name, ".opreste")),
            if self.last.is_empty() { "(niciun pas facut in aceasta rulare)" } else { self.last.as_str() }
        ));
        say(&format!("lucrare {}: raport ({}): {}\n", self.name, moment, file));
        #[cfg(not(feature = "test-auto"))]
        {
            super::emit_capture_begin();
            super::raport::send_to_hub();
            let sent = super::emit_capture_end();
            self.note(&sent);
        }
    }
}

fn describe_start(c: &Card) -> String {
    match c.start {
        Start::Now => String::from("acum"),
        Start::At(h, m) => format!("{:02}:{:02}", h, m),
        Start::After(u) => format!("+{} {}", u, if c.unit_s == 1 { "s" } else { "min" }),
    }
}

/// The last `n` lines of a text.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(n);
    let mut s = lines[from..].join("\n");
    s.push('\n');
    s
}

/// Boot-time self test (every `auto` test build): the card, the state, the clock arithmetic.
#[cfg(feature = "test-auto")]
pub fn self_test() {
    assert_eq!(secs_until((22, 0, 0), (6, 0)), 8 * 3600, "peste miezul noptii");
    assert_eq!(secs_until((5, 59, 30), (6, 0)), 30);
    assert_eq!(secs_until((6, 0, 0), (6, 0)), 86_400, "aceeasi ora = maine");
    assert_eq!(clock("07:05"), Some((7, 5)));
    for bad in ["24:00", "7:60", "x", "07:05:00", ""] {
        assert_eq!(clock(bad), None, "{}", bad);
    }
    let c = parse_card("porneste 22:00\nopreste 06:00\nraport 15\nrepeta 0\npas uptime # comentariu\npas run kronos --n 8 Buna\n").unwrap();
    assert_eq!(c.start, Start::At(22, 0));
    assert_eq!((c.stop_at, c.report_s, c.rounds, c.steps.len(), c.step_s), (Some((6, 0)), 900, 0, 2, 1800));
    let c = parse_card("unitate secunde\nporneste +3\ndurata 10\nrepeta 0\npas test-asteapta 1\n").unwrap();
    assert_eq!((c.start, c.duration_s, c.unit_s), (Start::After(3), Some(10), 1));
    for bad in [
        "",
        "repeta 2\n",
        "pas gpu lant\n",
        "pas gpu i6 --scrie\n",
        "pas test-placa\n",
        "pas lucrare alta\n",
        "pas live 5\n",
        "pas run kronos --gpu Ana\n",
        "repeta 0\npas uptime\n",
        "porneste 25:00\npas uptime\n",
        "raport 0\npas uptime\n",
        "scrie ceva\npas uptime\n",
    ] {
        assert!(parse_card(bad).is_err(), "fisa trebuia refuzata: {:?}", bad);
    }
    let s = State { stare: String::from("in lucru"), tur: 2, pas: 1, facuti: 7, lucrat_s: 90, pornit: String::from("2026-11-22 22:00:00"), reluari: 1, taiat_la: String::from("2.1"), taieri: 1, motiv: String::new(), cerute: 1 };
    assert_eq!(State::parse(&s.text()), s);
    assert!(s.running());
    // P6-225: `lucrare reia` only for a job the OS gave up on, at most MAX_ASKED times.
    let mut r = State::default();
    assert!(r.ask_resume().is_err(), "nepornita");
    for stare in ["in lucru", "terminata", "asteapta"] {
        let mut r = State { stare: String::from(stare), taieri: MAX_CUTS, ..State::default() };
        assert!(r.ask_resume().is_err(), "{}", stare);
    }
    let mut r = State { stare: String::from("oprita"), motiv: String::from("la comanda"), ..State::default() };
    assert!(r.ask_resume().is_err(), "oprita la comanda");
    let mut r = State { stare: String::from("oprita"), tur: 1, pas: 2, taiat_la: String::from("1.2"), taieri: MAX_CUTS, ..State::default() };
    for k in 1..=MAX_ASKED {
        assert!(r.ask_resume().is_ok(), "ceruta {}", k);
        assert!(r.running() && r.stare == ASKED && r.taieri == 0 && (r.tur, r.pas) == (1, 2) && r.cerute == k);
        let back = State::parse(&r.text());
        assert_eq!(back, r);
        r.stare = String::from("oprita");
        r.taieri = MAX_CUTS;
    }
    assert!(r.ask_resume().unwrap_err().contains("de vazut de Adrian"), "a treia cerere");
    for (n, ok) in [("proba", true), ("noapte-1", true), ("a_b", true), ("Mare", false), ("../x", false), ("", false), ("a b", false)] {
        assert_eq!(valid_name(n), ok, "{}", n);
    }
    crate::info!("test: lucrarile lungi: fisa, starea si ora OK");
}
