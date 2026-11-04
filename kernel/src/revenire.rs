//! Self-update with automatic return to the old version (task P6-50, command C-68 F3).
//!
//! `/bin/actualizeaza` keeps a copy of the running version in `boot/vechi/` (kernel, initrd.tar, VERSIUNE) and, after
//! putting the new files in place, writes `boot/incercare.txt` = "de-incercat" + the new version's name. At boot, after
//! the stick is mounted, `at_boot` reads that marker:
//! - "de-incercat": the first boot of the new version. The marker becomes "in-incercare"; a system thread confirms the
//!   version (removes the marker) once the system reached the desktop / shell and ran 60 s; if that does not happen in
//!   5 minutes it restarts the machine;
//! - "in-incercare": the previous boot of the new version never confirmed it (it crashed, hung or was cut off). The old
//!   files from `boot/vechi/` go back in place, the story is kept in `boot/revenit.txt`, and the machine restarts into
//!   the old version.
//! With `/usb/actualizare-automata` on the stick, `auto_update` runs `actualizeaza` at boot and restarts into the new
//! version when there is one.
//! Limit: a new kernel that fails before it mounts the stick cannot return by itself (it never reads the marker); the
//! old files are still in `boot/vechi/` for a manual return.
//! Idea: A/B updates with a trial boot and confirmation (as in phones and embedded systems); our own code.

use crate::fs::{self, Kind};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The files that are kept and put back: (place on the stick, its copy in boot/vechi/).
const KEPT: [(&str, &str); 3] =
    [("boot/kernel", "boot/vechi/kernel"), ("boot/initrd.tar", "boot/vechi/initrd.tar"), ("VERSIUNE", "boot/vechi/VERSIUNE")];
const MARKER: &str = "boot/incercare.txt";
const STORY: &str = "boot/revenit.txt";
const CONFIRM_AFTER_MS: u64 = 60_000;
const TRIAL_LIMIT_MS: u64 = 5 * 60_000;

/// What `at_boot` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No marker: a normal boot.
    Normal,
    /// First boot of a new version: on trial until confirmed.
    Trial,
    /// The new version did not confirm: the old files are back (the machine must restart into them).
    Returned,
    /// The marker said to return but there is nothing to return to (or a file failed): marker removed.
    Failed(String),
}

static READY: AtomicBool = AtomicBool::new(false);
static TRIAL_DIR: crate::sched::Mutex<Option<String>> = crate::sched::Mutex::new(None);
/// Test builds: shorter confirmation delay.
static CONFIRM_MS: AtomicU64 = AtomicU64::new(CONFIRM_AFTER_MS);

fn path(dir: &str, rel: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), rel)
}

fn write_file(p: &str, data: &[u8]) -> Result<(), String> {
    let _ = fs::remove(p);
    fs::create(p, Kind::File).map_err(|e| format!("{}: {}", p, e.message()))?;
    let mut at = 0;
    while at < data.len() {
        let n = fs::write(p, at as u64, &data[at..]).map_err(|e| format!("{}: {}", p, e.message()))?;
        if n == 0 {
            return Err(format!("{}: scriere incompleta", p));
        }
        at += n;
    }
    fs::flush(p).map_err(|e| format!("{}: {}", p, e.message()))
}

/// The marker's first line (the state) and second line (the version).
fn read_marker(dir: &str) -> Option<(String, String)> {
    let bytes = fs::read_all(&path(dir, MARKER)).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut lines = text.lines().map(str::trim);
    let state = String::from(lines.next().unwrap_or(""));
    let version = String::from(lines.next().unwrap_or("?"));
    Some((state, version))
}

/// Reads the marker on the stick mounted at `dir` and acts on it (see the module comment). Never restarts by itself.
pub fn at_boot(dir: &str) -> Outcome {
    let Some((state, version)) = read_marker(dir) else { return Outcome::Normal };
    match state.as_str() {
        "de-incercat" => {
            if let Err(e) = write_file(&path(dir, MARKER), format!("in-incercare\n{}\n", version).as_bytes()) {
                crate::error!("actualizare: marcajul de incercare nu se poate scrie ({}); versiunea noua ramane fara revenire", e);
                return Outcome::Failed(e);
            }
            crate::info!(
                "actualizare: prima pornire a versiunii {}: o confirm dupa {} s de mers; altfel, la pornirea urmatoare, revin la cea veche",
                version,
                CONFIRM_MS.load(Ordering::Relaxed) / 1000
            );
            *TRIAL_DIR.lock() = Some(String::from(dir));
            Outcome::Trial
        }
        "in-incercare" => match put_back(dir) {
            Ok(()) => {
                let note = format!(
                    "versiunea {} nu a ajuns la confirmare (oprire, blocare sau eroare la prima pornire);\ns-a revenit la versiunea veche din boot/vechi/ la pornirea din {}\n",
                    version,
                    now_text()
                );
                let _ = write_file(&path(dir, STORY), note.as_bytes());
                let _ = fs::remove(&path(dir, MARKER));
                let _ = fs::flush(&path(dir, MARKER));
                crate::warn!("actualizare: versiunea {} nu s-a confirmat; am pus la loc versiunea veche (boot/vechi/), repornesc", version);
                Outcome::Returned
            }
            Err(e) => {
                crate::error!("actualizare: revenirea la versiunea veche a esuat: {}; raman pe versiunea {}", e, version);
                let _ = fs::remove(&path(dir, MARKER));
                Outcome::Failed(e)
            }
        },
        other => {
            crate::warn!("actualizare: {} are o stare necunoscuta ({}); il sterg", MARKER, other);
            let _ = fs::remove(&path(dir, MARKER));
            Outcome::Normal
        }
    }
}

/// The old files back in place: each written as `<name>.nou`, read back, then swapped in.
fn put_back(dir: &str) -> Result<(), String> {
    let mut olds: Vec<(String, Vec<u8>)> = Vec::new();
    for (dest, kept) in KEPT {
        let data = fs::read_all(&path(dir, kept)).map_err(|e| format!("{}: {}", kept, e.message()))?;
        if data.is_empty() {
            return Err(format!("{} e gol", kept));
        }
        olds.push((path(dir, dest), data));
    }
    for (dest, data) in &olds {
        let new = format!("{}.nou", dest);
        write_file(&new, data)?;
        if fs::read_all(&new).ok().as_deref() != Some(&data[..]) {
            return Err(format!("{}: ce am citit inapoi nu e ce am scris", new));
        }
    }
    for (dest, _) in &olds {
        let new = format!("{}.nou", dest);
        let _ = fs::remove(dest);
        fs::rename(&new, dest).map_err(|e| format!("{}: {}", dest, e.message()))?;
    }
    fs::flush(&path(dir, "boot")).map_err(|e| format!("golirea discului: {}", e.message()))
}

/// The system reached the desktop / shell (called by `main`): the trial thread may confirm the new version.
pub fn system_ready() {
    READY.store(true, Ordering::Release);
    if TRIAL_DIR.lock().is_some() {
        crate::sched::spawn_system(trial);
    }
}

fn trial() {
    let start = crate::time::uptime_ms().unwrap_or(0);
    loop {
        crate::sched::sleep_ms(500);
        let now = crate::time::uptime_ms().unwrap_or(0);
        if READY.load(Ordering::Acquire) && now >= CONFIRM_MS.load(Ordering::Relaxed) {
            break;
        }
        if now.saturating_sub(start) > TRIAL_LIMIT_MS {
            crate::error!("actualizare: versiunea noua nu s-a confirmat in 5 minute; repornesc (la pornire revin la cea veche)");
            crate::power::reboot(None);
        }
    }
    let Some(dir) = TRIAL_DIR.lock().take() else { return };
    let version = read_marker(&dir).map(|(_, v)| v).unwrap_or_default();
    match fs::remove(&path(&dir, MARKER)).and_then(|_| fs::flush(&path(&dir, "boot"))) {
        Ok(()) => crate::info!("actualizare: versiunea {} confirmata (merge de {} s)", version, crate::time::uptime_ms().unwrap_or(0) / 1000),
        Err(e) => crate::error!("actualizare: confirmarea nu s-a scris: {}", e.message()),
    }
}

/// With `/usb/actualizare-automata` on the stick: runs `/bin/actualizeaza` at boot; true = a new version is in place
/// (the caller restarts into it).
pub fn auto_update(dir: &str) -> bool {
    if fs::kind(&path(dir, "actualizare-automata")).is_err() {
        return false;
    }
    if read_marker(dir).is_some() {
        return false; // a version on trial is not replaced before it is confirmed
    }
    if !crate::net::wait_ready(20_000) {
        crate::warn!("actualizare automata: reteaua nu are adresa; sar peste");
        return false;
    }
    crate::info!("actualizare automata: caut o versiune noua ({}actualizare-automata)", path(dir, ""));
    let Ok(id) = crate::proc::spawn_path("/bin/actualizeaza", &["actualizeaza"]) else {
        crate::warn!("actualizare automata: /bin/actualizeaza nu porneste");
        return false;
    };
    let code = crate::proc::wait_child(id);
    let updated = code == 0 && read_marker(dir).is_some_and(|(s, _)| s == "de-incercat");
    crate::info!("actualizare automata: {}", if updated { "versiune noua pusa, repornesc" } else { "nimic nou" });
    updated
}

fn now_text() -> String {
    crate::drivers::rtc::now()
        .map(|t| format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute))
        .unwrap_or_else(|| String::from("(ceas necitit)"))
}

/// The scenario `revenire` (test builds): the three cases on the writable test stick mounted at /usb.
#[cfg(feature = "test-revenire")]
pub fn self_test() {
    let d = "/usb";
    let _ = fs::create("/usb/boot", Kind::Dir);
    let _ = fs::create("/usb/boot/vechi", Kind::Dir);
    let put = |rel: &str, data: &[u8]| write_file(&path(d, rel), data).expect("test: revenire: scriere");
    let get = |rel: &str| fs::read_all(&path(d, rel)).unwrap_or_default();
    put("boot/kernel", b"nucleu NOU");
    put("boot/initrd.tar", b"initrd NOU");
    put("VERSIUNE", b"commit nou\n");
    put("boot/vechi/kernel", b"nucleu VECHI");
    put("boot/vechi/initrd.tar", b"initrd VECHI");
    put("boot/vechi/VERSIUNE", b"commit vechi\n");

    assert!(at_boot(d) == Outcome::Normal, "test: fara marcaj = pornire normala");
    // 1. First boot of the new version -> trial -> confirmed after the (short) delay.
    put(MARKER, b"de-incercat\nos-999\n");
    CONFIRM_MS.store(crate::time::uptime_ms().unwrap_or(0) + 1500, Ordering::Relaxed);
    assert!(at_boot(d) == Outcome::Trial, "test: de-incercat -> incercare");
    assert!(get(MARKER).starts_with(b"in-incercare"), "test: marcajul trebuia sa devina in-incercare");
    system_ready();
    let deadline = crate::time::uptime_ms().unwrap_or(0) + 10_000;
    while fs::kind(&path(d, MARKER)).is_ok() {
        assert!(crate::time::uptime_ms().unwrap_or(0) < deadline, "test: versiunea noua nu s-a confirmat");
        crate::sched::sleep_ms(100);
    }
    assert!(get("boot/kernel") == b"nucleu NOU", "test: dupa confirmare ramane versiunea noua");
    crate::info!("test: revenire: prima pornire confirmata");
    // 2. The trial boot never confirmed -> the old files go back.
    put(MARKER, b"in-incercare\nos-999\n");
    assert!(at_boot(d) == Outcome::Returned, "test: in-incercare -> revenire");
    assert!(get("boot/kernel") == b"nucleu VECHI" && get("boot/initrd.tar") == b"initrd VECHI" && get("VERSIUNE") == b"commit vechi\n", "test: fisierele vechi nu sunt la loc");
    assert!(fs::kind(&path(d, MARKER)).is_err() && get(STORY).windows(6).any(|w| w == b"os-999"), "test: marcaj sters, revenit.txt scris");
    assert!(fs::kind("/usb/boot/kernel.nou").is_err(), "test: a ramas kernel.nou");
    crate::info!("test: revenire: versiunea neconfirmata inlocuita cu cea veche");
    // 3. Return asked, nothing to return to -> marker removed, nothing touched.
    let _ = fs::remove("/usb/boot/vechi/initrd.tar");
    put("boot/kernel", b"nucleu NOU");
    put(MARKER, b"in-incercare\nos-1000\n");
    assert!(matches!(at_boot(d), Outcome::Failed(_)), "test: fara copie veche -> esec cinstit");
    assert!(get("boot/kernel") == b"nucleu NOU" && fs::kind(&path(d, MARKER)).is_err(), "test: fara copie: nimic atins, marcaj sters");
    crate::info!("test: revenire OK");
}
