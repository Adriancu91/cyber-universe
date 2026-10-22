//! Module manager: installs, starts, stops and removes modules. A module is a package (`.cum` = a
//! ustar archive) with a `module.toml` description and its files; it is installed in
//! `/disk/modules/<name>/` (a FAT32 volume marked for writing) and listed in `/disk/modules/index`.
//! Format: docs/specs/MODULE.md.

use crate::fs::{self, FsError, Kind};
use crate::sched::Mutex;
use alloc::{format, string::String, vec::Vec};

/// Where modules are installed.
pub const ROOT: &str = "/disk/modules";

#[derive(Debug, PartialEq, Eq)]
pub enum ModError {
    Fs(FsError),
    BadPackage(&'static str),
    AlreadyInstalled,
    NotInstalled,
    /// A module this one needs is not installed.
    MissingDependency(String),
    /// Another installed module needs this one.
    NeededBy(String),
    Running,
    NotRunning,
    NoProgram,
    Spawn,
}

impl ModError {
    pub fn message(&self) -> String {
        match self {
            ModError::Fs(e) => String::from(e.message()),
            ModError::BadPackage(why) => format!("pachet stricat: {}", why),
            ModError::AlreadyInstalled => String::from("modulul e deja instalat"),
            ModError::NotInstalled => String::from("modulul nu e instalat"),
            ModError::MissingDependency(d) => format!("lipseste modulul necesar {}", d),
            ModError::NeededBy(d) => format!("modulul {} are nevoie de el", d),
            ModError::Running => String::from("modulul merge; opreste-l intai"),
            ModError::NotRunning => String::from("modulul nu merge"),
            ModError::NoProgram => String::from("modulul nu are program de pornire"),
            ModError::Spawn => String::from("programul modulului nu porneste"),
        }
    }
}

impl From<FsError> for ModError {
    fn from(e: FsError) -> Self {
        ModError::Fs(e)
    }
}

/// What `module.toml` says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Program to run on `start`, relative to the module's folder.
    pub start: Option<String>,
    pub depends: Vec<String>,
}

/// Reads the small part of TOML that manifests use: `key = "text"` and `key = ["a", "b"]` lines,
/// `#` comments, blank lines.
pub fn parse_manifest(text: &str) -> Result<Manifest, ModError> {
    let mut m = Manifest::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ModError::BadPackage("rand fara '=' in module.toml"))?;
        let value = value.trim();
        let string = |v: &str| -> Result<String, ModError> {
            let v = v.trim();
            (v.len() >= 2 && v.starts_with('"') && v.ends_with('"') && !v[1..v.len() - 1].contains('"'))
                .then(|| String::from(&v[1..v.len() - 1]))
                .ok_or(ModError::BadPackage("valoare fara ghilimele in module.toml"))
        };
        match key.trim() {
            "name" => m.name = string(value)?,
            "version" => m.version = string(value)?,
            "description" => m.description = string(value)?,
            "start" => m.start = Some(string(value)?),
            "depends" => {
                let inner = value
                    .strip_prefix('[')
                    .and_then(|v| v.strip_suffix(']'))
                    .ok_or(ModError::BadPackage("depends trebuie sa fie o lista [..]"))?;
                for item in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    m.depends.push(string(item)?);
                }
            }
            _ => {} // unknown keys are allowed (newer packages)
        }
    }
    let name_ok = !m.name.is_empty() && m.name.len() <= 32 && m.name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
    if !name_ok {
        return Err(ModError::BadPackage("nume de modul invalid (a-z, 0-9, -)"));
    }
    if m.version.is_empty() {
        return Err(ModError::BadPackage("lipseste version"));
    }
    if m.start.as_deref().is_some_and(|s| !safe_relative(s)) {
        return Err(ModError::BadPackage("start invalid"));
    }
    Ok(m)
}

/// A path inside the module's folder: relative, no `..`.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

fn dir(name: &str) -> String {
    format!("{}/{}", ROOT, name)
}

/// Modules started by `start` and still running: (name, program thread).
static RUNNING: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

fn running_id(name: &str) -> Option<u64> {
    let mut list = RUNNING.lock();
    list.retain(|&(_, id)| crate::proc::running(id));
    list.iter().find(|(n, _)| n == name).map(|&(_, id)| id)
}

/// The installed modules, from the index.
pub fn installed() -> Result<Vec<Manifest>, ModError> {
    let index = match fs::read_all(&format!("{}/index", ROOT)) {
        Ok(d) => d,
        Err(FsError::NotFound) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for name in String::from_utf8_lossy(&index).lines().map(str::trim).filter(|l| !l.is_empty()) {
        let text = fs::read_all(&format!("{}/module.toml", dir(name)))?;
        out.push(parse_manifest(&String::from_utf8_lossy(&text))?);
    }
    Ok(out)
}

/// Rewrites the index with these module names.
fn write_index(names: &[String]) -> Result<(), ModError> {
    let path = format!("{}/index", ROOT);
    match fs::remove(&path) {
        Ok(()) | Err(FsError::NotFound) => {}
        Err(e) => return Err(e.into()),
    }
    fs::create(&path, Kind::File)?;
    let mut text = String::new();
    for n in names {
        text.push_str(n);
        text.push('\n');
    }
    fs::write(&path, 0, text.as_bytes())?;
    Ok(())
}

/// Installs the package at `package` (a `.cum` file). Returns its manifest.
pub fn install(package: &str) -> Result<Manifest, ModError> {
    let data = fs::read_all(package)?;
    let mut manifest = None;
    let mut files: Vec<(String, bool, &[u8])> = Vec::new();
    let mut bad = false;
    crate::fs::tar::for_each(&data, |e| {
        if !safe_relative(&e.name) {
            bad = true;
        } else if e.name == "module.toml" {
            manifest = Some(String::from_utf8_lossy(e.data).into_owned());
        } else {
            files.push((e.name, e.is_dir, e.data));
        }
    })
    .map_err(ModError::BadPackage)?;
    if bad {
        return Err(ModError::BadPackage("cale periculoasa in arhiva"));
    }
    let toml_text = manifest.ok_or(ModError::BadPackage("lipseste module.toml"))?;
    let m = parse_manifest(&toml_text)?;
    let present = installed()?;
    if present.iter().any(|p| p.name == m.name) {
        return Err(ModError::AlreadyInstalled);
    }
    if let Some(d) = m.depends.iter().find(|d| !present.iter().any(|p| &p.name == *d)) {
        return Err(ModError::MissingDependency(d.clone()));
    }
    match fs::create(ROOT, Kind::Dir) {
        Ok(()) | Err(FsError::AlreadyExists) => {}
        Err(e) => return Err(e.into()),
    }
    let base = dir(&m.name);
    let copy = || -> Result<(), ModError> {
        fs::create(&base, Kind::Dir)?;
        let toml = format!("{}/module.toml", base);
        fs::create(&toml, Kind::File)?;
        fs::write(&toml, 0, toml_text.as_bytes())?;
        for (name, is_dir, content) in &files {
            let path = format!("{}/{}", base, name);
            // Folders above the file (archives do not always list them).
            let mut at = base.clone();
            let parts: Vec<&str> = name.split('/').collect();
            for p in &parts[..parts.len() - 1] {
                at = format!("{}/{}", at, p);
                match fs::create(&at, Kind::Dir) {
                    Ok(()) | Err(FsError::AlreadyExists) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if *is_dir {
                match fs::create(&path, Kind::Dir) {
                    Ok(()) | Err(FsError::AlreadyExists) => {}
                    Err(e) => return Err(e.into()),
                }
            } else {
                fs::create(&path, Kind::File)?;
                fs::write(&path, 0, content)?;
            }
        }
        let mut names: Vec<String> = present.iter().map(|p| p.name.clone()).collect();
        names.push(m.name.clone());
        write_index(&names)
    };
    if let Err(e) = copy() {
        let _ = remove_tree(&base); // leave nothing half-installed
        return Err(e);
    }
    crate::info!("mod: instalat {} {}", m.name, m.version);
    Ok(m)
}

/// Deletes a folder and everything in it.
fn remove_tree(path: &str) -> Result<(), FsError> {
    if fs::kind(path)? == Kind::Dir {
        for (name, _) in fs::list(path)? {
            remove_tree(&format!("{}/{}", path, name))?;
        }
    }
    fs::remove(path)
}

/// Starts a module's program. Returns its thread id.
pub fn start(name: &str) -> Result<u64, ModError> {
    let m = installed()?.into_iter().find(|m| m.name == name).ok_or(ModError::NotInstalled)?;
    if running_id(name).is_some() {
        return Err(ModError::Running);
    }
    let program = format!("{}/{}", dir(name), m.start.ok_or(ModError::NoProgram)?);
    // Logged before the spawn: the new program may run (and print) before the line after it.
    crate::info!("mod: pornesc {} ({})", name, program);
    let id = crate::proc::spawn_path(&program, &[&program]).map_err(|_| ModError::Spawn)?;
    RUNNING.lock().push((String::from(name), id));
    crate::info!("mod: pornit {} (firul {})", name, id);
    Ok(id)
}

/// Stops a module's program and waits for it to end.
pub fn stop(name: &str) -> Result<(), ModError> {
    let id = running_id(name).ok_or(ModError::NotRunning)?;
    crate::proc::kill(id);
    let code = crate::proc::wait(id);
    RUNNING.lock().retain(|&(_, t)| t != id);
    crate::info!("mod: oprit {} (cod {})", name, code);
    Ok(())
}

/// Whether a module's program is running.
pub fn is_running(name: &str) -> bool {
    running_id(name).is_some()
}

/// Uninstalls a module (it must be stopped, and no other module may need it).
pub fn remove(name: &str) -> Result<(), ModError> {
    let present = installed()?;
    if !present.iter().any(|m| m.name == name) {
        return Err(ModError::NotInstalled);
    }
    if let Some(user) = present.iter().find(|m| m.depends.iter().any(|d| d == name)) {
        return Err(ModError::NeededBy(user.name.clone()));
    }
    if is_running(name) {
        return Err(ModError::Running);
    }
    remove_tree(&dir(name))?;
    let names: Vec<String> = present.iter().filter(|m| m.name != name).map(|m| m.name.clone()).collect();
    write_index(&names)?;
    crate::info!("mod: sters {}", name);
    Ok(())
}
