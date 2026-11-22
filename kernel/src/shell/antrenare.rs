//! P6-121 (C-161 point 3, first piece): the training engine in the OS — Kronos's "golden step" checked inside our own
//! system. `antrenare proba [dosar]` reads the golden step (hub `integrare/kronos/pas_de_aur_mic/`: `reteta.json`, the
//! batch, the initial weights and PyTorch's results) from a disk, read only, runs Cyber-GPU's CPU oracle of a whole
//! training step (`gpu_compute::train::golden_run`, forward → loss → backward → clipping → SGD / AdamW) for the two
//! recipes, 10 steps each, and compares with the tolerances written in the hub's `PAS_DE_AUR.md` (`train::TOL_*`).
//! Verdict: `ACELEASI NUMERE` or `DIFERITE`, with the worst deviations. Nothing here touches the board; the GPU programs
//! of training (C-160 point 4) join this check one by one when Cyber-GPU hands them over.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu_compute::train::{self, Batch, Recipe, MIC};

/// Where the golden step is looked for, in this order (the first with a `reteta.json`).
pub const DIRS: [&str; 2] = ["/disk/antrenare/pas_de_aur_mic", "/usb/antrenare/pas_de_aur_mic"];

/// SHA-256 of `reteta.json` of the S132 golden step (hub `PAS_DE_AUR.md`, cyber-gpu `tests/date/pas_de_aur_mic/CITESTE.md`).
pub const RETETA_SHA256: &str = "5e3c5824e61b9974aca527ee3c04237868f28cc28a2c0d04b742c81dce84e3a8";

const STEPS: usize = 10;

pub(super) fn sha256_hex(b: &[u8]) -> String {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut h = Sha256::new();
    h.update(b);
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

fn read(dir: &str, name: &str) -> Result<Vec<u8>, String> {
    let path = format!("{}/{}", dir, name);
    crate::fs::read_all(&path).map_err(|e| format!("nu pot citi {}: {}", path, e.message()))
}

fn read_f32(dir: &str, name: &str, n: usize) -> Result<Vec<f32>, String> {
    let b = read(dir, name)?;
    if b.len() != 4 * n {
        return Err(format!("{}/{}: {} octeti, astept {}", dir, name, b.len(), 4 * n));
    }
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn read_i32(dir: &str, name: &str, n: usize) -> Result<Vec<u32>, String> {
    let b = read(dir, name)?;
    if b.len() != 4 * n {
        return Err(format!("{}/{}: {} octeti, astept {}", dir, name, b.len(), 4 * n));
    }
    b.chunks_exact(4)
        .map(|c| {
            let v = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            u32::try_from(v).map_err(|_| format!("{}/{}: simbol negativ {}", dir, name, v))
        })
        .collect()
}

fn tensors(dir: &str, sub: &str) -> Result<Vec<Vec<f32>>, String> {
    MIC.tensors().iter().map(|t| read_f32(dir, &format!("{}/{}.f32", sub, t.name), t.elements())).collect()
}

/// The losses of a recipe, read from `reteta.json`: the list after `"<recipe>": {` … `"pierderi": [` (the same reading as
/// Cyber-GPU's T151; the file's SHA-256 is checked first, so its shape is known).
pub fn losses(reteta: &str, recipe: &str) -> Option<Vec<f64>> {
    let r = reteta.find(&format!("\"{}\": {{", recipe))?;
    let key = "\"pierderi\": [";
    let p = r + reteta[r..].find(key)? + key.len();
    let end = p + reteta[p..].find(']')?;
    reteta[p..end].split(',').map(|x| x.trim().parse::<f64>().ok()).collect()
}

/// The worst deviation between two sets of tensors: the largest |a - b| (and in which tensor) and the largest
/// relative norm ‖a - b‖ / ‖b‖ of a tensor.
pub struct Dev {
    pub abs: f32,
    pub rel_norm: f64,
    pub at: String,
}

pub fn worst(ours: &[Vec<f32>], theirs: &[Vec<f32>]) -> Dev {
    let mut d = Dev { abs: 0.0, rel_norm: 0.0, at: String::from("-") };
    for ((o, t), spec) in ours.iter().zip(theirs).zip(MIC.tensors()) {
        let (mut diff2, mut norm2) = (0.0f64, 0.0f64);
        for (&a, &b) in o.iter().zip(t) {
            let e = (a - b).abs();
            // NaN must not pass as "no deviation".
            if e > d.abs || e.is_nan() {
                d.abs = if e.is_nan() { f32::INFINITY } else { e };
                d.at = spec.name.clone();
            }
            diff2 += (e as f64) * (e as f64);
            norm2 += (b as f64) * (b as f64);
        }
        if norm2 > 0.0 {
            let r = libm_sqrt(diff2 / norm2);
            if r > d.rel_norm || r.is_nan() {
                d.rel_norm = if r.is_nan() { f64::INFINITY } else { r };
            }
        }
    }
    d
}

fn libm_sqrt(x: f64) -> f64 {
    gpu_compute::math::sqrt(x as f32) as f64
}

/// One recipe: run, compare, rows. Returns whether it is within every tolerance.
fn recipe(dir: &str, reteta: &str, batch: &Batch, initial: &[Vec<f32>], r: Recipe, name: &str, w_tol: f32, rows: &mut Vec<String>) -> bool {
    let Some(want) = losses(reteta, name).filter(|w| w.len() == STEPS) else {
        rows.push(format!("antrenare: {}: reteta.json nu are cele {} pierderi", name, STEPS));
        return false;
    };
    let t0 = crate::time::uptime_ms();
    let run = match train::golden_run(&MIC, initial, batch, r, STEPS) {
        Ok(x) => x,
        Err(e) => {
            rows.push(format!("antrenare: {}: oracolul a refuzat: {:?}", name, e));
            return false;
        }
    };
    let ms = match (t0, crate::time::uptime_ms()) {
        (Some(a), Some(b)) => format!("{} ms", b.saturating_sub(a)),
        _ => String::from("? ms"),
    };
    let mut loss_rel = 0.0f64;
    for (&o, &w) in run.losses.iter().zip(&want) {
        let rel = ((o as f64) - w).abs() / w.abs();
        loss_rel = if rel.is_nan() { f64::INFINITY } else { loss_rel.max(rel) };
    }
    let load = |sub: &str| tensors(dir, &format!("{}/{}", name, sub));
    let (g, w1, w10) = match (load("grad_pas1"), load("dupa_pas1"), load("dupa_pas10")) {
        (Ok(g), Ok(w1), Ok(w10)) => (worst(&run.grad_step1, &g), worst(&run.after_step1, &w1), worst(&run.after_last, &w10)),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
            rows.push(format!("antrenare: {}: {}", name, e));
            return false;
        }
    };
    let ok = loss_rel <= train::TOL_LOSS_REL
        && g.abs <= train::TOL_GRAD_ABS
        && g.rel_norm <= train::TOL_GRAD_REL_NORM
        && w1.abs <= w_tol
        && w10.abs <= w_tol;
    rows.push(format!(
        "antrenare: {}: {} pasi in {}; pierderea pasului 1 {} -> pasul {} {}; abaterea relativa maxima a pierderii {:e} (prag {:e})",
        name,
        STEPS,
        ms,
        run.losses[0],
        STEPS,
        run.losses[STEPS - 1],
        loss_rel,
        train::TOL_LOSS_REL
    ));
    rows.push(format!(
        "antrenare: {}: gradientul pasului 1: max |d| {:e} in {} (prag {:e}), norma relativa {:e} (prag {:e})",
        name,
        g.abs,
        g.at,
        train::TOL_GRAD_ABS,
        g.rel_norm,
        train::TOL_GRAD_REL_NORM
    ));
    rows.push(format!(
        "antrenare: {}: greutatile dupa pasul 1: max |d| {:e} in {}; dupa pasul {}: max |d| {:e} in {} (prag {:e}) -> {}",
        name,
        w1.abs,
        w1.at,
        STEPS,
        w10.abs,
        w10.at,
        w_tol,
        if ok { "ACELEASI NUMERE" } else { "DIFERITE" }
    ));
    ok
}

/// `antrenare proba [dosar]`: the rows for the screen; the last one is the verdict.
pub fn proba(dir: Option<&str>) -> Vec<String> {
    let mut rows = Vec::new();
    let dir = match dir {
        Some(d) => String::from(d.trim_end_matches('/')),
        None => match DIRS.iter().find(|d| crate::fs::read_all(&format!("{}/reteta.json", d)).is_ok()) {
            Some(d) => String::from(*d),
            None => {
                rows.push(format!("antrenare proba: NU A RULAT: nu gasesc pasul de aur ({} sau {})", DIRS[0], DIRS[1]));
                return rows;
            }
        },
    };
    let verdict = |rows: &mut Vec<String>, why: String| rows.push(format!("antrenare proba: NU A RULAT: {}", why));
    let reteta_bytes = match read(&dir, "reteta.json") {
        Ok(b) => b,
        Err(e) => {
            verdict(&mut rows, e);
            return rows;
        }
    };
    let sum = sha256_hex(&reteta_bytes);
    if sum != RETETA_SHA256 {
        verdict(&mut rows, format!("{}/reteta.json are SHA-256 {}, nu cel al pasului de aur S132 ({})", dir, sum, RETETA_SHA256));
        return rows;
    }
    let reteta = String::from_utf8_lossy(&reteta_bytes).into_owned();
    rows.push(format!(
        "antrenare: pasul de aur din {} (reteta.json SHA-256 ca S132); modelul mic: vocabular {}, context {}, E {}, {} capete, {} straturi; lotul 2 x 32; oracolul Cyber-GPU pe procesor (fara placa)",
        dir, MIC.vocab, MIC.ctx, MIC.n_emb, MIC.heads, MIC.layers
    ));
    let n = 2 * 32;
    let (inputs, targets, initial) = match (read_i32(&dir, "intrari.i32", n), read_i32(&dir, "tinte.i32", n), tensors(&dir, "initiale")) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
            verdict(&mut rows, e);
            return rows;
        }
    };
    let batch = Batch { inputs: &inputs, targets: &targets, batch: 2, len: 32 };
    let sgd = recipe(&dir, &reteta, &batch, &initial, Recipe::Sgd, "sgd", train::TOL_SGD_W_ABS, &mut rows);
    let adamw = recipe(&dir, &reteta, &batch, &initial, Recipe::AdamW513, "adamw_513", train::TOL_ADAMW_W_ABS, &mut rows);
    rows.push(format!(
        "antrenare proba: {} (sgd: {}, adamw_513: {}; {} pasi fiecare, fata de PyTorch)",
        if sgd && adamw { "ACELEASI NUMERE" } else { "DIFERITE" },
        if sgd { "da" } else { "NU" },
        if adamw { "da" } else { "NU" },
        STEPS
    ));
    rows
}

/// Checks that need no disk: the reading of `reteta.json` and the comparison (a NaN or a far value is caught).
pub fn self_test() {
    let r = "{ \"retete\": { \"sgd\": { \"rata\": 0.01, \"pierderi\": [5.5, 5.25, 5.0] } } }";
    assert!(losses(r, "sgd") == Some(alloc::vec![5.5, 5.25, 5.0]), "test: antrenare: pierderile");
    assert!(losses(r, "adamw_513").is_none(), "test: antrenare: reteta lipsa");
    let a: Vec<Vec<f32>> = MIC.tensors().iter().map(|t| alloc::vec![1.0f32; t.elements()]).collect();
    let mut b = a.clone();
    assert!(worst(&a, &b).abs == 0.0 && worst(&a, &b).rel_norm == 0.0, "test: antrenare: identice");
    b[4][3] = 1.5;
    let d = worst(&a, &b);
    assert!(d.abs == 0.5 && d.at == MIC.tensors()[4].name && d.rel_norm > 0.0, "test: antrenare: o abatere");
    b[4][3] = f32::NAN;
    assert!(worst(&b, &a).abs == f32::INFINITY, "test: antrenare: NaN");
    crate::info!("test: antrenare: citirea retetei si comparatia OK");
}
