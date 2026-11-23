//! P6-157 (C-161 points 3 + 5): the first whole training run inside the OS, on the processor. `antrenare mini` joins
//! the pieces that until now ran apart: Kronos's training data (P6-150, checked as Kronos checks it at start-up), the
//! batches drawn exactly as Kronos draws them (P6-153: batch k of a run = `antrenare date --lot` batch k), Cyber-GPU's
//! CPU oracle of a whole training step (`gpu_compute::train`: forward → loss → backward → clipping at 1.0 → AdamW) and
//! the checkpoint with the safe rewrite (P6-151). The model is the golden step's small shape (2 layers, E 64, 4 heads)
//! on Kronos's vocabulary (16 384 symbols), initialised from our own generator (not PyTorch's: nothing to compare with
//! bit for bit — the verdict is whether the loss goes down). A run can continue from its checkpoint (weights + AdamW
//! state), with the next hour's batch seed as Kronos computes it (`samanta * 1_000_003 + pasi_total`).
//! Nothing here touches the board; the board's training programs replace the oracle step by step (C-161 point 4).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use gpu_compute::train::{self, AdamW, Arch, Batch};

use super::date_bucati::{self, Mt19937};
use super::punct_control::{self, Checkpoint};

const USAGE: &str = "folosire: antrenare mini [dosar] [--pasi <1..1000>] [--lot <numar 1..16> <context 1..256>] [--vocabular <64..16384>] [--samanta <s>] [--lr <x>] [--salveaza <cale>] [--din <cale>]";

/// Kronos's vocabulary (#513 and #651: 16 384 symbols) — the default; a smaller one only for data whose symbols fit
/// (the QEMU test: on the processor without KVM one step with 16 384 takes ~50 s).
pub const VOCAB: usize = train::M513.vocab;
/// The width of the uniform start values: standard deviation 0.02, as GPT-2's initialisation (0.02 · √3).
const INIT_HALF_WIDTH: f64 = 0.034_641_016;
/// Over how many steps the start and the end of the loss curve are averaged.
const WINDOW: usize = 5;
/// The checkpoint's kind (meta `fel`), so `--din` refuses the golden step's checkpoints.
const KIND: &str = "antrenare-mini";

fn arch(vocab: usize, context: usize) -> Arch {
    Arch { vocab, ctx: context, n_emb: 64, heads: 4, layers: 2 }
}

/// The start weights: weights uniform in ±0.0346 (std 0.02), biases 0, LayerNorm weights 1. The generator is seeded with
/// `samanta + 1`, apart from the batches' generator (seeded with `samanta`, as Kronos), so the batches stay those of
/// `antrenare date --lot`.
pub fn initial(a: &Arch, samanta: u64) -> Vec<Vec<f32>> {
    let mut g = Mt19937::new(samanta.wrapping_add(1));
    a.tensors()
        .iter()
        .map(|t| {
            let n = t.elements();
            let leaf = t.name.rsplit('.').nth(1).unwrap_or("");
            if t.name.ends_with("bias") {
                alloc::vec![0.0f32; n]
            } else if leaf.starts_with("ln") {
                alloc::vec![1.0f32; n]
            } else {
                (0..n).map(|_| ((g.next_u32() as f64 / 4_294_967_296.0 * 2.0 - 1.0) * INIT_HALF_WIDTH) as f32).collect()
            }
        })
        .collect()
}

/// Natural logarithm for the start-loss reference (no libm): ln x = 2·atanh((x−1)/(x+1)), after halving to [1, 2).
fn ln(mut x: f64) -> f64 {
    let mut k = 0.0;
    while x >= 2.0 {
        x /= 2.0;
        k += 1.0;
    }
    let z = (x - 1.0) / (x + 1.0);
    let (mut term, mut sum) = (z, 0.0);
    for n in 0..30 {
        sum += term / (2 * n + 1) as f64;
        term *= z * z;
    }
    2.0 * sum + k * core::f64::consts::LN_2
}

fn mean(v: &[f32]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

/// What the run starts from: fresh weights, or a checkpoint of an earlier run.
struct Start {
    params: Vec<Vec<f32>>,
    adam: AdamW,
    done: u64,
    samanta: u64,
    numar: usize,
    context: usize,
    vocab: usize,
    lr: f64,
}

fn meta<'a>(c: &'a Checkpoint, k: &str) -> Option<&'a str> {
    c.meta.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
}

fn from_checkpoint(path: &str) -> Result<Start, String> {
    let c = punct_control::load(path)?;
    if meta(&c, "fel") != Some(KIND) {
        return Err(format!("{}: nu e un punct de control al lui `antrenare mini` (fel = {:?})", path, meta(&c, "fel")));
    }
    let num = |k: &str| meta(&c, k).and_then(|v| v.parse::<u64>().ok()).ok_or_else(|| format!("{}: lipseste {}", path, k));
    let (done, samanta, numar, context, t) = (num("pas")?, num("samanta")?, num("lot")? as usize, num("context")? as usize, num("adam_t")?);
    let vocab = num("vocabular")? as usize;
    let lr = meta(&c, "lr").and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| format!("{}: lipseste lr", path))?;
    let a = arch(vocab, context);
    let specs = a.tensors();
    let take = |prefix: &str| -> Result<Vec<Vec<f32>>, String> {
        specs
            .iter()
            .map(|s| {
                let name = format!("{}{}", prefix, s.name);
                match c.tensors.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) if v.len() == s.elements() => Ok(v.clone()),
                    Some(_) => Err(format!("{}: {} are alta marime", path, name)),
                    None => Err(format!("{}: lipseste tensorul {}", path, name)),
                }
            })
            .collect()
    };
    let params = take("")?;
    let mut adam = AdamW::new(&params, lr, 0.9, 0.999, 1e-8, 0.1);
    adam.m = take("adam.m.")?;
    adam.v = take("adam.v.")?;
    adam.t = t as u32;
    Ok(Start { params, adam, done, samanta, numar, context, vocab, lr })
}

fn to_checkpoint(a: &Arch, s: &Start, losses: &[f32], data: &str) -> Checkpoint {
    let specs = a.tensors();
    let mut tensors = Vec::with_capacity(3 * specs.len());
    for (prefix, set) in [("", &s.params), ("adam.m.", &s.adam.m), ("adam.v.", &s.adam.v)] {
        for (spec, t) in specs.iter().zip(set.iter()) {
            tensors.push((format!("{}{}", prefix, spec.name), t.clone()));
        }
    }
    let m = |k: &str, v: String| (k.to_string(), v);
    Checkpoint {
        meta: alloc::vec![
            m("fel", KIND.to_string()),
            m("arhitectura", format!("vocabular {}, context {}, E {}, {} capete, {} straturi", a.vocab, a.ctx, a.n_emb, a.heads, a.layers)),
            m("pas", format!("{}", s.done)),
            m("adam_t", format!("{}", s.adam.t)),
            m("lr", format!("{}", s.lr)),
            m("samanta", format!("{}", s.samanta)),
            m("lot", format!("{}", s.numar)),
            m("context", format!("{}", s.context)),
            m("vocabular", format!("{}", s.vocab)),
            m("date", data.to_string()),
            m("pierdere_ultima", losses.last().map_or(String::from("-"), |l| format!("{:.6}", l))),
        ],
        tensors,
    }
}

/// `antrenare mini …`: rows for the shell.
pub fn command(args: &[&str]) -> Vec<String> {
    let mut rows = Vec::new();
    let (mut dir, mut steps, mut lot, mut vocab, mut samanta, mut lr, mut save, mut from): (
        Option<&str>,
        usize,
        Option<(usize, usize)>,
        Option<usize>,
        Option<u64>,
        Option<f64>,
        Option<&str>,
        Option<&str>,
    ) = (None, 20, None, None, None, None, None, None);
    let mut i = 0;
    while i < args.len() {
        let next = |k: usize| args.get(i + k).copied();
        match args[i] {
            "--pasi" => match next(1).and_then(|v| v.parse::<usize>().ok()) {
                Some(n) if (1..=1000).contains(&n) => {
                    steps = n;
                    i += 2;
                }
                _ => return alloc::vec![String::from(USAGE)],
            },
            "--lot" => match (next(1).and_then(|v| v.parse::<usize>().ok()), next(2).and_then(|v| v.parse::<usize>().ok())) {
                (Some(b), Some(t)) if (1..=16).contains(&b) && (1..=256).contains(&t) => {
                    lot = Some((b, t));
                    i += 3;
                }
                _ => return alloc::vec![String::from(USAGE)],
            },
            "--vocabular" => match next(1).and_then(|v| v.parse::<usize>().ok()) {
                Some(v) if (64..=VOCAB).contains(&v) => {
                    vocab = Some(v);
                    i += 2;
                }
                _ => return alloc::vec![String::from(USAGE)],
            },
            "--samanta" => match next(1).and_then(|v| v.parse::<u64>().ok()) {
                Some(s) => {
                    samanta = Some(s);
                    i += 2;
                }
                None => return alloc::vec![String::from(USAGE)],
            },
            "--lr" => match next(1).and_then(|v| v.parse::<f64>().ok()) {
                Some(x) if x > 0.0 && x < 1.0 => {
                    lr = Some(x);
                    i += 2;
                }
                _ => return alloc::vec![String::from(USAGE)],
            },
            "--salveaza" if next(1).is_some() => {
                save = next(1);
                i += 2;
            }
            "--din" if next(1).is_some() => {
                from = next(1);
                i += 2;
            }
            x if !x.starts_with("--") && dir.is_none() => {
                dir = Some(x);
                i += 1;
            }
            _ => return alloc::vec![String::from(USAGE)],
        }
    }
    let fail = |mut rows: Vec<String>, why: String| {
        rows.push(format!("antrenare mini: NU A RULAT: {}", why));
        rows
    };

    // The data, checked as Kronos checks it at start-up.
    let dir = match dir {
        Some(d) => String::from(d.trim_end_matches('/')),
        None => match date_bucati::DIRS.iter().find(|d| crate::fs::size(&format!("{}/index.json", d)).is_ok()) {
            Some(d) => String::from(*d),
            None => return fail(rows, format!("niciun index.json in {} sau {}", date_bucati::DIRS[0], date_bucati::DIRS[1])),
        },
    };
    let ix = match date_bucati::read_index(&dir) {
        Ok(ix) => ix,
        Err(e) => return fail(rows, e),
    };
    let c = date_bucati::check(&ix, None);
    if !c.ok() {
        for (f, p) in &c.wrong {
            rows.push(format!("antrenare mini: GRESIT: {}: {}", f, p));
        }
        return fail(rows, format!("datele din {} nu trec verificarea lui Kronos ({} probleme)", dir, c.wrong.len()));
    }
    if !date_bucati::generator_matches_torch() {
        return fail(rows, String::from("generatorul nu da valorile PyTorch de referinta"));
    }
    let data = date_bucati::fingerprint(&ix);
    rows.push(format!("antrenare mini: datele {}: {} simboluri, verificate ca la Kronos, amprenta {}", dir, ix.total, data));

    // Fresh weights or a checkpoint.
    let mut st = match from {
        Some(path) => match from_checkpoint(path) {
            Ok(s) => {
                if lot.is_some_and(|(b, t)| (b, t) != (s.numar, s.context)) || vocab.is_some_and(|v| v != s.vocab) || samanta.is_some_and(|x| x != s.samanta) || lr.is_some_and(|x| x != s.lr) {
                    return fail(rows, format!("{}: --lot / --samanta / --lr / --vocabular altele decat in punctul de control (lot {} x {}, samanta {}, lr {}, vocabular {})", path, s.numar, s.context, s.samanta, s.lr, s.vocab));
                }
                rows.push(format!("antrenare mini: reluat din {}: dupa pasul {} (AdamW t = {}), lot {} x {}, samanta {}, lr {}", path, s.done, s.adam.t, s.numar, s.context, s.samanta, s.lr));
                s
            }
            Err(e) => return fail(rows, e),
        },
        None => {
            let (numar, context) = lot.unwrap_or((2, 32));
            let samanta = samanta.unwrap_or(0);
            let lr = lr.unwrap_or(1e-3);
            let vocab = vocab.unwrap_or(VOCAB);
            let a = arch(vocab, context);
            let params = initial(&a, samanta);
            let adam = AdamW::new(&params, lr, 0.9, 0.999, 1e-8, 0.1);
            Start { params, adam, done: 0, samanta, numar, context, vocab, lr }
        }
    };
    let a = arch(st.vocab, st.context);
    if let Err(e) = a.check_params(&st.params) {
        return fail(rows, format!("greutatile: {:?}", e));
    }
    let count: usize = st.params.iter().map(|p| p.len()).sum();
    // Kronos: one batch generator per training hour, seeded with `samanta` (first hour) or `samanta·1 000 003 + pasi_total`.
    let seed = date_bucati::batch_seed(st.samanta, st.done);
    rows.push(format!(
        "antrenare mini: modelul: vocabular {}, context {}, E {}, {} capete, {} straturi ({} greutati); lot {} x {}; AdamW lr {}, taiere la 1,0; loturile ca Kronos (generatorul cu {}); oracolul Cyber-GPU pe procesor (fara placa)",
        a.vocab, a.ctx, a.n_emb, a.heads, a.layers, count, st.numar, st.context, st.lr, seed
    ));

    let mut g = Mt19937::new(seed);
    let mut losses: Vec<f32> = Vec::with_capacity(steps);
    let t_all = crate::time::since_boot_us();
    for k in 0..steps {
        let t0 = crate::time::since_boot_us();
        let r = date_bucati::positions(&mut g, ix.total, st.numar, st.context as u64)
            .and_then(|p| date_bucati::batch(&ix, &p, st.context as u64));
        let (x, y) = match r {
            Ok(b) => b,
            Err(e) => return fail(rows, format!("lotul pasului {}: {}", st.done + 1, e)),
        };
        if let Some(bad) = x.iter().chain(y.iter()).find(|&&s| s as usize >= st.vocab) {
            return fail(rows, format!("lotul pasului {}: simbolul {} e in afara vocabularului ({})", st.done + 1, bad, st.vocab));
        }
        let b = Batch { inputs: &x, targets: &y, batch: st.numar, len: st.context };
        let (loss, mut grads) = match train::loss_and_grad(&a, &st.params, &b) {
            Ok(r) => r,
            Err(e) => return fail(rows, format!("pasul {}: {:?}", st.done + 1, e)),
        };
        if !loss.is_finite() {
            return fail(rows, format!("pasul {}: pierderea nu e un numar ({})", st.done + 1, loss));
        }
        let norm = train::clip_grad_norm(&mut grads, 1.0);
        if let Err(e) = st.adam.step(&mut st.params, &grads) {
            return fail(rows, format!("pasul {}: AdamW: {:?}", st.done + 1, e));
        }
        st.done += 1;
        losses.push(loss);
        let ms = t0.zip(crate::time::since_boot_us()).map_or(0, |(p, q)| q.saturating_sub(p) / 1000);
        if steps <= 20 || k % (steps / 10).max(1) == 0 || k + 1 == steps {
            rows.push(format!(
                "antrenare mini: pasul {}: pierderea {:.4}, norma gradientului {:.4}, lotul {}, {} ms",
                st.done,
                loss,
                norm,
                date_bucati::batch_fingerprint(&x, &y),
                ms
            ));
        }
    }
    let total_ms = t_all.zip(crate::time::since_boot_us()).map_or(0, |(p, q)| q.saturating_sub(p) / 1000);
    let w = WINDOW.min(losses.len() / 2).max(1);
    let (first, last) = (mean(&losses[..w]), mean(&losses[losses.len() - w..]));
    if from.is_none() {
        // A model that knows nothing guesses every symbol alike: loss ln(16 384) = 9.704.
        rows.push(format!("antrenare mini: pierderea la pornire {:.4} (ghicit la intamplare: ln {} = {:.4})", losses[0], st.vocab, ln(st.vocab as f64)));
    }
    if losses.len() >= 2 && last < first {
        rows.push(format!(
            "antrenare mini: INVATA: pierderea medie a scazut de la {:.4} (primii {} pasi) la {:.4} (ultimii {}); {} pasi in {} ms",
            first, w, last, w, steps, total_ms
        ));
    } else {
        rows.push(format!(
            "antrenare mini: NU SCADE: pierderea medie {:.4} (primii {} pasi) -> {:.4} (ultimii {}); {} pasi in {} ms",
            first, w, last, w, steps, total_ms
        ));
    }
    if let Some(path) = save {
        let ck = to_checkpoint(&a, &st, &losses, &data);
        match punct_control::save(path, &ck) {
            Ok((n, sha)) => rows.push(format!(
                "antrenare mini: punctul de control {} scris cu golirea sigura ({} octeti, sha256 {}; greutati + starea AdamW, dupa pasul {}), recitit identic bit cu bit",
                path,
                n,
                &sha[..16],
                st.done
            )),
            Err(e) => rows.push(format!("antrenare mini: punctul de control: EROARE: {}", e)),
        }
    }
    rows
}
