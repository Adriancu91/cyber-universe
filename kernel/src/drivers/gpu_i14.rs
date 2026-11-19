//! P6-62 (command C-83): I14 of the window chain in the OS — whole tokens of a Kronos model on the work channel,
//! after `i13-strat` (Cyber-GPU's notes `C-83_i14.md`, `C-83_i14_vectori.md`).
//!
//! `i14-simbol` runs **Kronos's tiny control model first** (`mic_q8_0.kgr`, 840 KB, 2 layers, E 32, vocabulary 16 384,
//! RANDOM weights; hub `integrare/kronos/vectori/`, copied in `tests/kronos/`), as Cyber-GPU proposed: the answer is
//! known exactly (`vectori.json`, made by Kronos's own engine), so if it passes here and #513 does not later, the
//! problem is size (VRAM, pushbuffer, time), not computation. #513 (P6-60) runs through `kronos --gpu`, which picks it by default from the stick.
//!
//! The step: the KGR1 file read from the stick (`/usb/kronos/`, on the test disk `/disk/kronos/` in SIMULATION), its
//! SHA-256 checked against `vectori.json`; all weights + a NaN scratch + the logits packed into one blob, uploaded in
//! pieces of at most 8 MiB through the DMA arena of I11 (one `copy_b128` launch each); then for each of the three
//! control texts: every prompt position and 4 more greedy positions, each = `kronos_model::token_launches`
//! (`16·layers + 3` = 35 launches) + one copy of the logits down into the arena, ONE doorbell per token. Each token's
//! logits are judged against our CPU oracle (`gpu_compute::model::step_cpu`, same weights) with `kronos_model::judge`
//! (greedy token identical, every logit within 1e-3), the argmax of every prompt position and the 5 greedy tokens
//! against Kronos's vectors. The next token is the board's greedy one.
//!
//! No new register is written: only the VRAM window and the work-channel doorbell, as in I10–I13.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU with `gpu-sim`'s command processor.
//! The KGR1 reader here is ours, from the hub's specification (`integrare/kronos/format_greutati.md`).

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_compute::layer::Shape;
use gpu_compute::model::{self as cm, Caches};
use gpu_core::diag::{Event, Journal};
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::gsp::golden::Mapping;
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::kronos_layer::{self as kl, LayerVas, OwnedLayer, Prog, WeightVas, PROGRAMS};
use nvidia_ga104::kronos_diag as kd;
use nvidia_ga104::kronos_fast as kf;
use nvidia_ga104::kronos_gen::{GenPlan, Generation, Stop, TokenCheck, TokenOutput, TokenRunner, VramBudget};
use nvidia_ga104::kronos_model::{self as km, ModelVas, OwnedModel};
use nvidia_ga104::kronos_perf as kp;
use nvidia_ga104::kronos_speed as ks;
use nvidia_ga104::mmu::SysMapping;
use nvidia_ga104::push::PushBuf;
use nvidia_ga104::transfer;

use super::gpu_golden::GpuState;
use super::gpu_i10::{put_bytes, Launched, WAIT_US};
use super::gpu_i11::{arena_pages, f32_bytes, map_vram, ram_read, ram_write, run_records, Bell, I11_BOARD_BYTES, PAGE};

/// Where I14 looks for the control model on the board's stick, and on the test disk in SIMULATION.
pub(super) const MIC_BOARD: &str = "/usb/kronos/mic_q8_0.kgr";
#[cfg(feature = "test-gpu-sim")]
pub(super) const MIC_SIM: &str = "/disk/kronos/mic_q8_0.kgr";
/// `vectori.json` → `fisiere.q8_0.sha256`.
const MIC_SHA256: &str = "b08f4c2976b95d9e1cc3058766b41e13d1ab1a8466b5eb06fa59ff665dd692a0";

/// One control text of `vectori.json` (part `q8_0`): its token ids, the argmax on every position, the 5 greedy tokens.
struct Control {
    text: &'static str,
    prompt: &'static [usize],
    argmax: &'static [usize],
    greedy: [usize; 5],
}

const MIC_CONTROLS: [Control; 3] = [
    Control {
        text: "Ana are mere.",
        prompt: &[65, 1988, 1585, 2717, 46],
        argmax: &[10337, 606, 3286, 3604, 4978],
        greedy: [4978, 3604, 4649, 13843, 3604],
    },
    Control {
        text: "Capitala Romaniei este",
        prompt: &[7132, 1373, 2710, 5734, 1744],
        argmax: &[3604, 3604, 3286, 3604, 3604],
        greedy: [3604, 3604, 3604, 13843, 3604],
    },
    Control {
        text: "def suma(a, b):",
        prompt: &[2082, 15746, 40, 97, 44, 1376, 1754],
        argmax: &[6961, 6961, 10450, 3604, 3604, 10793, 8117],
        greedy: [8117, 8117, 3604, 3604, 3604],
    },
];

/// VRAM kept free after a model's upload (C-87 step 4: the screen and the steps after I14).
const VRAM_RESERVE: u64 = 4 << 20;

/// P6-95 (C-141): the `kronos --gpu` result rows in the gpu journal (they go up to the hub with it); the speed rows are
/// cyber-gpu's `kv …` (`kronos_speed::rows`), plus the CPU engine's own two.
const KG_MODEL: &str = "kg kronos --gpu model";
const KG_ASKED: &str = "kg simboluri cerute";
const KG_MADE: &str = "kg simboluri generate pe placa (identice cu procesorul)";
const KG_FIRST_DIFF: &str = "kg primul simbol diferit de procesor (pozitia; ffffffff = niciunul)";
const KG_WORST: &str = "kg abaterea maxima a logit-urilor fata de procesor (x1e6)";
const KV_CPU_MED: &str = "kv motorul procesorului median pe pozitie (us, step_cpu)";
const KV_CPU_X100: &str = "kv simboluri pe secunda x100 (motorul procesorului)";

/// P6-97 (C-146): whether the board rewrote any word of the launch slots (read back after the text; 0 = the shadow held).
const KT_SHADOW: &str = "kt umbra lansarilor: cuvinte schimbate de placa in sloturi (0 = umbra corecta)";
/// P6-97 (C-145): the notes the OS repeats with the routine writes of a generation (the permit's reason, the doorbell's
/// note) — exactly the texts of `gpu_tur.rs`.
const GEN_NOTES: [&str; 2] = ["C-55 gpu lant: fereastra cu Adrian (P6-42)", "soneria canalului de lucru (I10): scriere fara recitire"];

const UP_FIRST: u32 = 0xc000;
const TOKEN_FIRST: u32 = 0x1_0000;
/// Payload room per doorbell (a token has 36 launches; #513 will have 196).
const STRIDE: u32 = 0x100;

fn bad(why: &'static str) -> GpuError {
    GpuError::InvalidArgument(why)
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// SHA-256 of a whole file, as lowercase hex.
fn sha256_hex(b: &[u8]) -> String {
    use cyber_crypto::{sha2::Sha256, Digest};
    let mut h = Sha256::new();
    h.update(b);
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    hex(&out)
}

// ---------------------------------------------------------------------------------------------------------------
// The KGR1 file (hub `integrare/kronos/format_greutati.md`): header, key-values, tensor list, zeros to 32, data zone.
// ---------------------------------------------------------------------------------------------------------------

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> GpuResult<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(bad("KGR1: fisier trunchiat"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> GpuResult<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> GpuResult<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }
    fn text(&mut self) -> GpuResult<&'a [u8]> {
        let n = usize::try_from(self.u64()?).map_err(|_| bad("KGR1: text prea lung"))?;
        self.take(n)
    }
}

struct Tensor<'a> {
    name: &'a [u8],
    dims: Vec<usize>,
    kind: u32,
    off: usize,
}

/// A KGR1 Q8_0 `kronos_gpt` file → the model Cyber-GPU's launches and CPU oracle take (Q8_0 blocks = f32 scale +
/// 32 × i8, the same 36 bytes in both).
pub(super) fn read_kgr1(f: &[u8]) -> GpuResult<OwnedModel> {
    let mut c = Cursor { b: f, at: 0 };
    if c.take(4)? != b"KGR1" || c.u32()? != 1 {
        return Err(bad("KGR1: antet necunoscut"));
    }
    let (nkv, nt) = (c.u64()?, c.u64()?);
    if nkv > 4096 || nt > 4096 {
        return Err(bad("KGR1: prea multe chei sau tensori"));
    }
    let mut ints: Vec<(&[u8], i64)> = Vec::new();
    let (mut eps, mut arch_ok, mut q8_ok) = (None, false, false);
    for _ in 0..nkv {
        let k = c.text()?;
        match c.u32()? {
            1 => ints.push((k, c.u64()? as i64)),
            2 => {
                let v = f64::from_bits(c.u64()?);
                if k == b"ln_eps" {
                    eps = Some(v);
                }
            }
            3 => {
                let v = c.text()?;
                if k == b"arhitectura" {
                    arch_ok = v == b"kronos_gpt";
                }
                if k == b"fel" {
                    q8_ok = v == b"q8_0";
                }
            }
            _ => return Err(bad("KGR1: fel de valoare necunoscut")),
        }
    }
    if !arch_ok || !q8_ok {
        return Err(bad("KGR1: nu e kronos_gpt Q8_0"));
    }
    let int = |k: &[u8]| -> GpuResult<usize> {
        ints.iter()
            .find(|(n, _)| *n == k)
            .and_then(|(_, v)| usize::try_from(*v).ok())
            .ok_or(bad("KGR1: lipseste o cheie (n_straturi, n_capete, n_emb, context, vocabular, cap_legat)"))
    };
    let mut ts: Vec<Tensor<'_>> = Vec::new();
    for _ in 0..nt {
        let name = c.text()?;
        let nd = c.u32()? as usize;
        if nd == 0 || nd > 4 {
            return Err(bad("KGR1: numar de dimensiuni nesustinut"));
        }
        let mut dims = Vec::with_capacity(nd);
        for _ in 0..nd {
            dims.push(usize::try_from(c.u64()?).map_err(|_| bad("KGR1: dimensiune prea mare"))?);
        }
        let kind = c.u32()?;
        let off = usize::try_from(c.u64()?).map_err(|_| bad("KGR1: loc prea mare"))?;
        ts.push(Tensor { name, dims, kind, off });
    }
    let data = c.at.next_multiple_of(32);
    let bytes = |name: &str, kind: u32, want: &[usize]| -> GpuResult<&[u8]> {
        let t = ts.iter().find(|t| t.name == name.as_bytes()).ok_or(bad("KGR1: lipseste un tensor"))?;
        if t.kind != kind || t.dims != want {
            return Err(bad("KGR1: tensor cu fel sau forma neasteptata"));
        }
        let n = match kind {
            0 => 4 * t.dims.iter().product::<usize>(),
            _ => {
                if t.dims.len() != 2 || t.dims[1] % 32 != 0 {
                    return Err(bad("KGR1: Q8_0 cere 2 dimensiuni, coloane multiplu de 32"));
                }
                t.dims[0] * t.dims[1] / 32 * 36
            }
        };
        let start = data.checked_add(t.off).ok_or(bad("KGR1: loc gresit"))?;
        f.get(start..start + n).ok_or(bad("KGR1: tensor in afara fisierului"))
    };
    let f32s = |name: &str, n: usize| -> GpuResult<Vec<f32>> {
        Ok(bytes(name, 0, &[n])?.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
    };
    let q8 = |name: &str, r: usize, cols: usize| -> GpuResult<Vec<u8>> { Ok(bytes(name, 1, &[r, cols])?.to_vec()) };
    if int(b"cap_legat")? != 1 {
        return Err(bad("KGR1: capul trebuie sa fie legat de emb_token"));
    }
    let (layers, heads, e, context, vocab) =
        (int(b"n_straturi")?, int(b"n_capete")?, int(b"n_emb")?, int(b"context")?, int(b"vocabular")?);
    if layers == 0 || layers > 64 || heads == 0 || e == 0 || e % heads != 0 || e % 32 != 0 {
        return Err(bad("KGR1: forma modelului nesustinuta"));
    }
    let shape = Shape { e, heads, eps: eps.ok_or(bad("KGR1: lipseste ln_eps"))? as f32 };
    let mut owned = Vec::with_capacity(layers);
    for i in 0..layers {
        let p = |s: &str| format!("straturi.{}.{}", i, s);
        owned.push(OwnedLayer {
            ln1_g: f32s(&p("ln1.weight"), e)?,
            ln1_b: f32s(&p("ln1.bias"), e)?,
            wqkv: q8(&p("atentie.in_proj_weight"), 3 * e, e)?,
            bqkv: f32s(&p("atentie.in_proj_bias"), 3 * e)?,
            wout: q8(&p("atentie.out_proj.weight"), e, e)?,
            bout: f32s(&p("atentie.out_proj.bias"), e)?,
            ln2_g: f32s(&p("ln2.weight"), e)?,
            ln2_b: f32s(&p("ln2.bias"), e)?,
            wfc: q8(&p("mlp.0.weight"), 4 * e, e)?,
            bfc: f32s(&p("mlp.0.bias"), 4 * e)?,
            wproj: q8(&p("mlp.2.weight"), e, 4 * e)?,
            bproj: f32s(&p("mlp.2.bias"), e)?,
        });
    }
    let m = OwnedModel {
        shape,
        vocab,
        context,
        emb_tok: q8("emb_token.weight", vocab, e)?,
        emb_pos: q8("emb_pozitie.weight", context, e)?,
        layers: owned,
        ln_g: f32s("ln.weight", e)?,
        ln_b: f32s("ln.bias", e)?,
    };
    m.weights().check()?;
    Ok(m)
}

// ---------------------------------------------------------------------------------------------------------------
// The step.
// ---------------------------------------------------------------------------------------------------------------

/// The blob in VRAM: offsets of every part (all 16-aligned).
struct Blob {
    bytes: Vec<u8>,
    emb_tok: u64,
    emb_pos: u64,
    ln_g: u64,
    ln_b: u64,
    layers: Vec<[u64; 12]>,
    scratch: u64,
    fin: u64,
}

fn pack(m: &OwnedModel) -> Blob {
    let mut b: Vec<u8> = Vec::new();
    let put = |b: &mut Vec<u8>, part: &[u8]| {
        let o = b.len() as u64;
        b.extend_from_slice(part);
        b.resize(b.len().next_multiple_of(16), 0);
        o
    };
    let emb_tok = put(&mut b, &m.emb_tok);
    let emb_pos = put(&mut b, &m.emb_pos);
    let ln_g = put(&mut b, &f32_bytes(&m.ln_g));
    let ln_b = put(&mut b, &f32_bytes(&m.ln_b));
    let layers = m
        .layers
        .iter()
        .map(|l| {
            [
                put(&mut b, &f32_bytes(&l.ln1_g)),
                put(&mut b, &f32_bytes(&l.ln1_b)),
                put(&mut b, &l.wqkv),
                put(&mut b, &f32_bytes(&l.bqkv)),
                put(&mut b, &l.wout),
                put(&mut b, &f32_bytes(&l.bout)),
                put(&mut b, &f32_bytes(&l.ln2_g)),
                put(&mut b, &f32_bytes(&l.ln2_b)),
                put(&mut b, &l.wfc),
                put(&mut b, &f32_bytes(&l.bfc)),
                put(&mut b, &l.wproj),
                put(&mut b, &f32_bytes(&l.bproj)),
            ]
        })
        .collect();
    // The scratch and the final LayerNorm + logits start as NaN: anything not computed cannot pass.
    let scratch = b.len() as u64;
    b.resize((scratch + kl::scratch_bytes(m.shape.e)) as usize, 0xff);
    let fin = b.len() as u64;
    b.resize((fin + 4 * (m.shape.e + m.vocab) as u64).next_multiple_of(16) as usize, 0xff);
    Blob { bytes: b, emb_tok, emb_pos, ln_g, ln_b, layers, scratch, fin }
}

/// P6-62 step 3: a `kronos --gpu` request. `/bin/kronos` gives the model file, the prompt's token ids and how many
/// tokens to generate; the chain's `i14-simbol` step runs it on the board right after the control model passed (the
/// board is set up only inside the chain, behind Adrian's `DA`). Every token is judged against the CPU oracle like the
/// control texts; any error ends the request and `/bin/kronos` continues on the processor.
pub struct KronosJob {
    pub path: String,
    pub prompt: Vec<usize>,
    pub n: usize,
}

/// The request waiting for the next chain, and its answer (the greedy tokens of the board, or why not).
pub static KRONOS_JOB: crate::sched::Mutex<Option<KronosJob>> = crate::sched::Mutex::new(None);
pub static KRONOS_RESULT: crate::sched::Mutex<Option<Result<Vec<usize>, String>>> = crate::sched::Mutex::new(None);

/// One text run on the board: its prompt, how many greedy tokens, and (control texts only) Kronos's expected answer.
struct Run<'a> {
    name: String,
    prompt: Vec<usize>,
    steps: usize,
    argmax: Option<&'a [usize]>,
    greedy: Option<&'a [usize]>,
}

/// What a run left behind: the greedy tokens of every text, the worst logit deviation, the median token time, and
/// whether the logits had to be read from VRAM because only the copy into RAM was wrong (P6-94, C-139).
struct RunOut {
    all: Vec<Vec<usize>>,
    worst: f32,
    median: u64,
    from_vram: bool,
    /// P6-95 (C-141/C-142): the upload and every position (OS clock without the CPU check), and the CPU engine's time
    /// per position (`step_cpu`, the same weights) for the comparison.
    speed: ks::Speed,
    cpu_us: Vec<u64>,
    /// P6-97 (C-146): every position of the last text, split (board / journal / screen / the rest = driver / CPU check).
    split: Vec<kf::TokenSplit>,
    /// P6-97: words of the launch slots the board changed (read back after the texts); `None` = no shadow (control model).
    shadow_changed: Option<usize>,
    /// P6-98 (C-147 stage 0 (a), `kronos --gpu` only): the launch times split by kind, and why a token was left out.
    prof: kp::KindProfile,
    prof_err: Option<String>,
    /// P6-98 (C-147 stage 0 (d), `kronos --gpu` only): what the VRAM → VRAM copy needs after the texts.
    copy: Option<CopyCtx>,
}

/// P6-98 (C-147 stage 0 (d)): the pieces of a `kronos --gpu` run the VRAM → VRAM copy reuses (the weights as source,
/// the first launch slot, the records, the `copy_b128` program), kept until the `kg`/`kv`/`kt`/`kp` rows are on the stick.
struct CopyCtx {
    src_va: u64,
    /// The host's copy of the bytes at `src_va` (the weights part of the uploaded blob — the board never writes it), for
    /// the three words read back.
    src: Vec<u8>,
    launches: Mapping,
    rec: Mapping,
    maps: Vec<Mapping>,
    sys: SysMapping,
    copy_va: u64,
}

/// The board's pieces a run needs (P6-62 step 3: shared by the control model and a `kronos --gpu` request).
///
/// P6-94 (C-139, cyber-gpu `kronos_diag`): when `way` is `None` (the control model), the ladder runs first on the first
/// token of the first text (the os-305 way as a control, then the repair, then the fallbacks), writes its `i14d` rows
/// in the journal and picks how the texts are sent; `kronos --gpu` reuses that way. A position that does not pass is
/// explained in the journal (`kd::explain`, `kd::locate`).
#[allow(clippy::too_many_arguments)]
fn run_model<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    m: &OwnedModel,
    runs: &[Run<'_>],
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    tokens_done: &mut usize,
    subs: &mut u32,
    way: &mut Option<kd::Way>,
) -> GpuResult<RunOut> {
    let s = m.shape;
    let e = s.e;
    let n_tok = km::launches_per_token(m.layers.len());
    let n = n_tok + 1; // + the copy of the logits down
    // 0. C-87 step 4 + P6-95 (C-142 `kronos_speed::ModelLimits`): the VRAM, the pushbuffer area (two tokens in the way that
    //    needs the most words) and the records' stride, from the model's shapes, checked before anything is uploaded.
    let budget = VramBudget::of_model(m)?;
    let lim = ks::ModelLimits::of(&s, m.layers.len(), m.vocab, m.context, I11_BOARD_BYTES)?;
    let (free, largest) = st.mem.vram_free();
    lines.push(format!("{}: i14: {}; VRAM liber {} MiB (cea mai lunga bucata {} MiB)", p, budget.line(m.layers.len()), free >> 20, largest >> 20));
    let vram_line = budget.line(m.layers.len());
    for x in lim.lines().into_iter().filter(|x| *x != vram_line) {
        let have = if x.contains("zona de comenzi: cel putin") { format!(" (are {} KiB)", l.push.size >> 10) } else { String::new() };
        lines.push(format!("{}: i14: {}{}", p, x, have));
    }
    let refused = lim.check(l.push.size, STRIDE, free, largest, VRAM_RESERVE);
    if !refused.is_empty() {
        for r in &refused {
            lines.push(format!("{}: I14: {}", p, r.line()));
        }
        lines.push(format!("{}: I14: modelul nu incape (motivul e mai sus; pastrez {} MiB rezerva de VRAM): nimic urcat", p, VRAM_RESERVE >> 20));
        return Err(bad("I14: modelul nu incape (zona de comenzi, pasul inregistrarilor sau VRAM-ul): nimic urcat"));
    }
    // 1. The blob, uploaded through the arena in pieces of at most 8 MiB.
    let blob = pack(m);
    let total = blob.bytes.len() as u64;
    let vblob = map_vram(st, total)?;
    let piece = total.min(I11_BOARD_BYTES);
    let host = st.mem.alloc_sys_mapped(&arena_pages(0, piece.div_ceil(PAGE))?)?;
    let logits_bytes = (4 * m.vocab as u64).next_multiple_of(16);
    let host_logits = st.mem.alloc_sys_mapped(&arena_pages(I11_BOARD_BYTES / PAGE, logits_bytes.div_ceil(PAGE))?)?;
    let mut progs: Vec<(Prog, Mapping)> = Vec::new();
    for prog in PROGRAMS {
        let pm = map_vram(st, prog.code().len() as u64)?;
        put_bytes(st.mem.writer_mut(), pm.pa, prog.code())?;
        progs.push((prog, pm));
    }
    let prog_va = |want: Prog| progs.iter().find(|(q, _)| *q == want).map(|(_, pm)| pm.va);
    let copy_va = prog_va(Prog::Copy).ok_or(bad("I14: lipseste programul de copiere"))?;
    let mut caches = Vec::with_capacity(m.layers.len());
    for _ in 0..m.layers.len() {
        caches.push((map_vram(st, kl::cache_bytes(e))?, map_vram(st, kl::cache_bytes(e))?));
    }
    let launches = map_vram(st, 512 * n as u64)?;
    let (cb_up, q_up, rec_up) = (map_vram(st, PAGE)?, map_vram(st, PAGE)?, map_vram(st, PAGE)?);
    let rec = map_vram(st, (16 * (n as u64 + 1)).next_multiple_of(PAGE))?;
    let mut maps: Vec<Mapping> = alloc::vec![vblob, launches, cb_up, q_up, rec_up];
    maps.extend(caches.iter().flat_map(|(k, v)| [*k, *v]));
    maps.extend(progs.iter().map(|(_, pm)| *pm));
    let mut up_ns = 0u64;
    let mut k = 0u32;
    let mut speed = ks::Speed::new(if permit.is_some() { ks::Source::Placa } else { ks::Source::Simulare });
    for (i, chunk) in blob.bytes.chunks(piece as usize).enumerate() {
        let host_t0 = crate::time::since_boot_us();
        let len = (chunk.len() as u64).next_multiple_of(16);
        ram_write(&host, chunk);
        let w = st.mem.writer_mut();
        put_bytes(w, cb_up.pa, &transfer::copy_cbuf(host.va, vblob.va + i as u64 * piece, len)?)?;
        put_bytes(w, q_up.pa, &transfer::copy_qmd(copy_va, cb_up.va, len)?.to_bytes()?)?;
        put_bytes(w, rec_up.pa, &[0; 32])?;
        let first = UP_FIRST + STRIDE * k;
        k += 1;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[q_up.va], rec_up.va, first)?;
        let nw = pb.len();
        let sys = [host.clone()];
        match run_records(vbar, clock, journal, st, l, permit, &words[..nw], rec_up, 2, first + 1, &maps, &sys, doorbell)? {
            Ok(b) => {
                let ns = transfer::analyze_sequence(&b, 1, first)?[0];
                up_ns += ns;
                let host = host_t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a));
                speed.add_piece(chunk.len() as u64, ns, host);
            }
            Err(last) => {
                lines.push(format!("{}: I14: urcarea bucatii {} nu s-a terminat (inregistrarea 1 = {:#x})", p, i, last));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        }
    }
    lines.push(format!(
        "{}: i14: {} octeti (greutati + spatiu de lucru + logit-uri) urcati in {} bucati in {} ns ({} MB/s)",
        p, total, k, up_ns, transfer::CopySpeed::mb_per_s(total, up_ns)
    ));
    // 2. The VAs. The copy of the logits down is now the last launch of a token's list (`kl::copy_launch`, C-139).
    let at = |o: u64| vblob.va + o;
    let mut lvas = Vec::with_capacity(m.layers.len());
    for (o, (kc, vc)) in blob.layers.iter().zip(&caches) {
        let wv = WeightVas {
            ln1_g: at(o[0]),
            ln1_b: at(o[1]),
            wqkv: at(o[2]),
            bqkv: at(o[3]),
            wout: at(o[4]),
            bout: at(o[5]),
            ln2_g: at(o[6]),
            ln2_b: at(o[7]),
            wfc: at(o[8]),
            bfc: at(o[9]),
            wproj: at(o[10]),
            bproj: at(o[11]),
        };
        lvas.push(LayerVas::with_scratch(e, at(blob.scratch), kc.va, vc.va, wv)?);
    }
    let vas = ModelVas {
        emb_tok: at(blob.emb_tok),
        emb_pos: at(blob.emb_pos),
        ln_g: at(blob.ln_g),
        ln_b: at(blob.ln_b),
        layers: lvas,
        h_final: at(blob.fin),
        logits: at(blob.fin) + 4 * e as u64,
    };
    let mut tmaps = maps.clone();
    tmaps.push(rec);
    let weights = m.weights();
    let mut worst = 0.0f32;
    let mut medians = Vec::new();
    let mut cpu_us: Vec<u64> = Vec::new();
    let mut all = Vec::with_capacity(runs.len());
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let fast = runs.iter().all(|r| r.argmax.is_none());
    let mut split_all: Vec<kf::TokenSplit> = Vec::new();
    let mut runner = BoardRunner {
        p,
        vbar: &mut *vbar,
        clock: &mut *clock,
        journal: &mut *journal,
        st: &mut *st,
        l: &mut *l,
        permit,
        lines: &mut *lines,
        doorbell: &mut *doorbell,
        tokens_done: &mut *tokens_done,
        subs: &mut *subs,
        m,
        vas: &vas,
        launches,
        rec,
        host_logits: host_logits.clone(),
        logits_bytes,
        progs: &progs,
        tmaps: &tmaps,
        n,
        name: String::new(),
        way: way.unwrap_or(kd::Way::REPAIR),
        from_vram: false,
        pos: 0,
        timing: None,
        last_logits: Vec::new(),
        argmaxes: Vec::new(),
        speed,
        // P6-97 (C-145/C-146): only for `kronos --gpu` (no control answers): the control model and the ladder keep
        // writing every slot whole, as in os-306/os-307.
        shadow: if fast { Some(kf::WriteShadow::new(launches.pa, launches.size)?) } else { None },
        shadow_ranges: alloc::collections::BTreeMap::new(),
        fast,
        split: Vec::new(),
        launch_ns: Vec::new(),
        prof: kp::KindProfile::new(),
        prof_err: None,
    };
    // 3. C-139: the ladder, once, on the first token of the first control text, before the texts.
    if way.is_none() {
        let first = runs.first().ok_or(bad("I14: niciun text"))?;
        let tok0 = *first.prompt.first().ok_or(bad("I14: text fara simboluri"))?;
        let cpu0 = cm::step_cpu(&weights, &mut Caches::new(m.layers.len(), e), tok0, 0)?;
        let prev = Caches::new(m.layers.len(), e);
        let ctx = kd::ProbeCtx {
            w: &weights,
            vas: &vas,
            prev: &prev,
            token: tok0,
            pos: 0,
            cpu_logits: &cpu0,
            want: first.argmax.and_then(|a| a.first().copied()),
        };
        runner.name = String::from("scara");
        let ladder = kd::ladder(&mut runner, &ctx); // writes its own rows in the journal
        runner.lines.extend(ladder.lines().into_iter().map(|x| format!("{}: {}", p, x)));
        let Some(chosen) = ladder.chosen else {
            return Err(bad("I14: niciun fel de a trimite simbolul nu trece (diagnosticul e in jurnal)"));
        };
        *way = Some(chosen);
        runner.way = chosen;
    }
    // 4. The texts: cyber-gpu's continuous generation (C-87 step 1, `kronos_gen::Generation`), every position judged
    //    against the CPU engine first; stop at the first error, explained in the journal (C-139).
    for run in runs {
        runner.name = run.name.clone();
        let (generation, positions) = loop {
            let plan = GenPlan::new(&run.prompt, run.steps, m.vocab, m.context)?;
            let positions = plan.positions();
            let mut generation = Generation::new(plan);
            runner.argmaxes.clear();
            runner.split.clear();
            let mut check = CpuCheck { w: &weights, caches: Caches::new(m.layers.len(), e), us: Vec::new() };
            let stop = generation.run(&mut runner, Some(&mut check)).clone();
            // P6-97: the CPU check of each position next to its token (Generation checks every position once, in order).
            let mut split = core::mem::take(&mut runner.split);
            if split.len() == check.us.len() {
                for (t, &c) in split.iter_mut().zip(&check.us) {
                    t.check_us = c;
                }
            }
            split_all = split;
            cpu_us.append(&mut check.us);
            if stop.is_clean_end() {
                break (generation, positions);
            }
            runner.lines.push(format!("{}: I14: \"{}\": {}", p, run.name, stop.line()));
            let pos = match stop {
                Stop::NuTrece { pos, .. } | Stop::LogitiNefiniti { pos } => pos,
                Stop::Eroare { err, .. } => return Err(err),
                _ => return Err(bad("I14: generarea s-a oprit (vezi randul de mai sus)")),
            };
            // P6-95 (C-141): for `kronos --gpu` the first position that differs from the processor, in the journal.
            if run.argmax.is_none() {
                let made = pos.saturating_sub(run.prompt.len());
                runner.journal.push(Event::Value { what: KG_FIRST_DIFF, value: pos as u32 });
                runner.journal.push(Event::Value { what: KG_MADE, value: made as u32 });
                runner.lines.push(format!(
                    "{}: kronos --gpu: primul simbol diferit de procesor la pozitia {} (dupa {} simboluri generate identice)",
                    p, pos, made
                ));
            }
            // What that token left in memory (nothing sent). The CPU engine's caches hold the earlier rows.
            let tok = generation.seq[pos];
            let cpu = check.cpu_logits(tok, pos)?;
            let ctx = kd::ProbeCtx {
                w: &weights,
                vas: &vas,
                prev: &check.caches,
                token: tok,
                pos,
                cpu_logits: &cpu,
                want: run.argmax.and_then(|a| a.get(pos).copied()),
            };
            let read = core::mem::take(&mut runner.last_logits);
            let ex = kd::explain(&mut runner, &ctx, &read);
            runner.lines.extend(ex.probe.lines().into_iter().map(|x| format!("{}: {}", p, x)));
            if ex.vram_pass && !runner.from_vram {
                runner.lines.push(format!(
                    "{}: I14: \"{}\": doar copierea logit-urilor in RAM e gresita; textul o ia de la capat cu logit-urile citite din VRAM",
                    p, run.name
                ));
                runner.from_vram = true;
                continue;
            }
            if matches!(ex.probe.verdict, kd::Verdict::Mostenit(_)) {
                let way_now = runner.way;
                for pr in kd::locate(&mut runner, &ctx, way_now.embed) {
                    runner.lines.extend(pr.lines().into_iter().map(|x| format!("{}: {}", p, x)));
                }
            }
            return Err(bad("I14: simbolul nu trece fata de procesor (diagnosticul i14d e in jurnal)"));
        };
        let stats = generation.stats();
        worst = worst.max(generation.worst_abs);
        // C-139: the argmax against Kronos's vectors only after the text ended clean against the CPU engine.
        if let Some(want) = run.argmax {
            for (pos, (&g, &w)) in runner.argmaxes.iter().zip(want).enumerate() {
                if g != w {
                    runner.lines.push(format!("{}: I14: \"{}\" pozitia {}: argmax {} pe placa, vectori.json {}", p, run.name, pos, g, w));
                    return Err(bad("I14: argmax diferit de vectorii lui Kronos"));
                }
            }
        }
        let greedy = generation.generated().to_vec();
        if let Some(want) = run.greedy {
            if greedy[..] != want[..] {
                runner.lines.push(format!("{}: I14: \"{}\": 5 simboluri greedy {:?}, vectori.json {:?}", p, run.name, greedy, want));
                return Err(bad("I14: simbolurile greedy difera de vectorii lui Kronos"));
            }
            runner.lines.push(format!(
                "{}: i14: \"{}\": TRECE - {} pozitii, argmax ca vectori.json pe fiecare pozitie, 5 simboluri greedy {:?} identice",
                p, run.name, positions, greedy
            ));
        } else {
            runner.lines.push(format!("{}: {}: {} ({})", p, run.name, stats.line(label), generation.stop.as_ref().map(|s| s.line()).unwrap_or_default()));
        }
        medians.push(stats.median_ns);
        all.push(greedy);
    }
    let from_vram = runner.from_vram;
    let speed = core::mem::replace(&mut runner.speed, ks::Speed::new(ks::Source::Simulare));
    let shadow_changed = runner.shadow_check()?;
    let prof = core::mem::take(&mut runner.prof);
    let prof_err = runner.prof_err.take();
    drop(runner);
    medians.sort_unstable();
    let median = medians.get(medians.len() / 2).copied().unwrap_or(0);
    // P6-98 (C-147 stage 0 (d)): `kronos --gpu` keeps what the VRAM → VRAM copy needs (it runs after the result rows).
    // Only the weights (the bytes before the scratch): the scratch, the final vector and the logits are written by the board.
    let mut src = blob.bytes;
    src.truncate(blob.scratch as usize);
    let copy = fast.then(|| CopyCtx { src_va: vblob.va, src, launches, rec, maps: tmaps, sys: host_logits, copy_va });
    Ok(RunOut { all, worst, median, from_vram, speed, cpu_us, split: split_all, shadow_changed, prof, prof_err, copy })
}

/// One position on the board for `kronos_gen::Generation`, and the 4 operations of cyber-gpu's diagnosis
/// (`kronos_diag::TokenIo`, C-139): the launches written, the submissions of the chosen way, the records and the
/// logits read back.
struct BoardRunner<'r, 'b, V: Mmio + ?Sized, K: Clock + ?Sized> {
    p: &'r str,
    vbar: &'r mut V,
    clock: &'r mut K,
    journal: &'r mut Journal,
    st: &'r mut GpuState,
    l: &'r mut Launched,
    permit: Option<&'r HardwareWritePermit>,
    lines: &'r mut Vec<String>,
    doorbell: &'r mut Bell<'b, V>,
    tokens_done: &'r mut usize,
    /// Submissions made in this step (the records' first payload grows per submission: a token may have several).
    subs: &'r mut u32,
    m: &'r OwnedModel,
    vas: &'r ModelVas,
    launches: Mapping,
    rec: Mapping,
    host_logits: SysMapping,
    logits_bytes: u64,
    progs: &'r [(Prog, Mapping)],
    tmaps: &'r [Mapping],
    /// Launch slots: the launches of a token + the copy of the logits down.
    n: usize,
    name: String,
    /// How tokens are sent (chosen by the ladder).
    way: kd::Way,
    /// The logits are read from VRAM (only the copy into RAM was wrong); the copy launch is not sent.
    from_vram: bool,
    /// The position being sent (for the error lines).
    pos: usize,
    /// Board clock of the current token: start of its first record, end of its last, busy time summed.
    timing: Option<(u64, u64, u64)>,
    /// The logits the last token returned (for `kd::explain`).
    last_logits: Vec<f32>,
    /// The board's argmax at every position of the current text.
    argmaxes: Vec<usize>,
    /// P6-95 (C-142): the speed counters (upload + every position).
    speed: ks::Speed,
    /// P6-97 (C-145 point 2): what was last written in the launch slots (`kronos --gpu` only): a token writes (and reads
    /// back) only the words that changed. Forgotten after any error and after any other write through `TokenIo::write`.
    shadow: Option<kf::WriteShadow>,
    /// The slot pieces written under the shadow (VRAM address → bytes), read back once after the texts.
    shadow_ranges: alloc::collections::BTreeMap<u64, usize>,
    /// `kronos --gpu`: the routine journal rows go to the stick once per token (C-145 point 1).
    fast: bool,
    /// P6-97 (C-146): every position of the current text, split (the CPU check is added after).
    split: Vec<kf::TokenSplit>,
    /// P6-98 (C-147 stage 0 (a), `kronos --gpu` only): every launch time of the current token, in order (all its
    /// submissions), then added to `prof` by kind.
    launch_ns: Vec<u64>,
    prof: kp::KindProfile,
    prof_err: Option<String>,
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> BoardRunner<'_, '_, V, K> {
    /// The VRAM address of `bytes` at GPU VA `va` (one of the step's own mappings).
    fn pa_of(&self, va: u64, bytes: u64) -> GpuResult<u64> {
        self.tmaps
            .iter()
            .find(|m| va >= m.va && va.checked_add(bytes).is_some_and(|end| end <= m.va + m.size))
            .map(|m| m.pa + (va - m.va))
            .ok_or(bad("I14: adresa GPU in afara zonelor pasului"))
    }

    /// P6-97: after the texts, the slot pieces written under the shadow are read back once and compared with it: a word
    /// the board changed would mean the shadow is wrong (C-145 §6). Returns how many words differ (`None`: no shadow).
    /// The shadow is dropped either way (the next run starts whole).
    fn shadow_check(&mut self) -> GpuResult<Option<usize>> {
        let Some(sh) = self.shadow.take() else { return Ok(None) };
        let ranges = core::mem::take(&mut self.shadow_ranges);
        let GpuState { zone, .. } = &*self.st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, zone.vram_size, &zone.protected)?;
        let mut changed = 0usize;
        let mut r: GpuResult<()> = Ok(());
        'all: for (&pa, &len) in &ranges {
            let mut got = Vec::with_capacity(len);
            for i in 0..(len / 4) as u64 {
                match vw.read_u32(pa + 4 * i) {
                    Ok(v) => got.extend_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        r = Err(e);
                        break 'all;
                    }
                }
            }
            match sh.changes(pa, &got) {
                Ok(c) => changed += c.iter().map(|(_, b)| b.len() / 4).sum::<usize>(),
                Err(e) => {
                    r = Err(e);
                    break;
                }
            }
        }
        let back = vw.restore();
        r?;
        back?;
        Ok(Some(changed))
    }
}

/// P6-97 (C-145 point 2): writes `bytes` at `pa` through the shadow: only the words that changed since the last token
/// (each written and read back as before), then remembered. Any error forgets the shadow. `bytes` is padded to whole
/// words with zeros, exactly as `put_bytes` writes it.
fn shadow_put<W: nvidia_ga104::gsp::work_queue::ChannelMemory + ?Sized>(
    w: &mut W,
    sh: &mut kf::WriteShadow,
    ranges: &mut alloc::collections::BTreeMap<u64, usize>,
    pa: u64,
    bytes: &[u8],
) -> GpuResult<()> {
    let mut b = bytes.to_vec();
    b.resize(bytes.len().next_multiple_of(4), 0);
    let r = (|| {
        for (at, range) in sh.changes(pa, &b)? {
            put_bytes(w, at, &b[range])?;
        }
        sh.remember(pa, &b)
    })();
    if r.is_err() {
        sh.forget();
    } else {
        let e = ranges.entry(pa).or_insert(0);
        *e = (*e).max(b.len());
    }
    r
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> kd::TokenIo for BoardRunner<'_, '_, V, K> {
    fn submit(&mut self, list: &[kl::Launch], first_slot: usize, mode: kd::SeqMode) -> GpuResult<()> {
        let n = list.len();
        if n == 0 {
            return Ok(());
        }
        if first_slot + n > self.n {
            return Err(bad("I14: mai multe lansari decat locurile lor"));
        }
        let progs = self.progs;
        let prog_va = |want: Prog| progs.iter().find(|(q, _)| *q == want).map(|(_, pm)| pm.va);
        let w = self.st.mem.writer_mut();
        let mut qmds = Vec::with_capacity(n);
        for (i, la) in list.iter().enumerate() {
            let j = (first_slot + i) as u64;
            let (cb_va, cb_pa) = (self.launches.va + 512 * j, self.launches.pa + 512 * j);
            let pv = prog_va(la.prog).ok_or(bad("I14: program necunoscut"))?;
            if let Some(sh) = self.shadow.as_mut() {
                shadow_put(w, sh, &mut self.shadow_ranges, cb_pa, &la.cbuf)?;
                shadow_put(w, sh, &mut self.shadow_ranges, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
            } else {
                put_bytes(w, cb_pa, &la.cbuf)?;
                put_bytes(w, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
            }
            qmds.push(cb_va + 256);
        }
        put_bytes(w, self.rec.pa, &alloc::vec![0u8; 16 * (n + 1)])?;
        let first = TOKEN_FIRST + STRIDE * *self.subs;
        let mut words = alloc::vec![0u32; kd::words(mode, n)];
        let mut pb = PushBuf::new(&mut words);
        kd::encode(mode, &mut pb, &qmds, self.rec.va, first)?;
        let nw = pb.len();
        let last = first + n as u32;
        let sys = [self.host_logits.clone()];
        let b = match run_records(
            self.vbar, self.clock, self.journal, self.st, self.l, self.permit, &words[..nw], self.rec, n as u64 + 1, last, self.tmaps, &sys,
            self.doorbell,
        )? {
            Ok(b) => b,
            Err(got) => {
                self.lines.push(format!(
                    "{}: I14: \"{}\" pozitia {}: ultima inregistrare = {:#x} (asteptat {:#x}): placa nu a terminat",
                    self.p, self.name, self.pos, got, last
                ));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        *self.subs += 1;
        let t = transfer::analyze_sequence(&b, n, first)?;
        if self.fast {
            self.launch_ns.extend_from_slice(&t);
        }
        let time = |k: usize| u64::from_le_bytes(b[16 * k + 8..16 * k + 16].try_into().unwrap_or([0; 8]));
        let busy: u64 = t.iter().sum();
        self.timing = Some(match self.timing {
            None => (time(0), time(n), busy),
            Some((s, _, sum)) => (s, time(n), sum + busy),
        });
        Ok(())
    }

    fn read(&mut self, ranges: &[(u64, usize)]) -> GpuResult<Vec<Vec<u32>>> {
        let pas = ranges.iter().map(|&(va, n)| self.pa_of(va, 4 * n as u64)).collect::<GpuResult<Vec<u64>>>()?;
        let GpuState { zone, .. } = &*self.st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *self.vbar, &mut *self.journal, self.permit, zone.vram_size, &zone.protected)?;
        let r = ranges
            .iter()
            .zip(&pas)
            .map(|(&(_, n), &pa)| (0..n as u64).map(|i| vw.read_u32(pa + 4 * i)).collect::<GpuResult<Vec<u32>>>())
            .collect::<GpuResult<Vec<Vec<u32>>>>();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))
    }

    fn write(&mut self, va: u64, words: &[u32]) -> GpuResult<()> {
        let pa = self.pa_of(va, 4 * words.len() as u64)?;
        // P6-97: a write from the diagnosis may land in the launch slots: the shadow no longer knows them.
        if let Some(sh) = self.shadow.as_mut() {
            sh.forget();
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        put_bytes(self.st.mem.writer_mut(), pa, &bytes)
    }

    fn journal(&mut self) -> &mut Journal {
        &mut *self.journal
    }
}

impl<V: Mmio + ?Sized, K: Clock + ?Sized> TokenRunner for BoardRunner<'_, '_, V, K> {
    fn run_token(&mut self, tok: usize, pos: usize) -> GpuResult<TokenOutput> {
        let (m, vas, way) = (self.m, self.vas, self.way);
        let weights = m.weights();
        self.pos = pos;
        self.timing = None;
        self.launch_ns.clear();
        let extra = if self.from_vram {
            Vec::new()
        } else {
            ram_write(&self.host_logits, &alloc::vec![0xee; self.logits_bytes as usize]);
            alloc::vec![kl::copy_launch(vas.logits, self.host_logits.va, self.logits_bytes)?]
        };
        let host_t0 = crate::time::since_boot_us();
        let journal_t0 = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed);
        kd::send_token(self, &weights, vas, tok, pos, way, &extra)?;
        *self.tokens_done += 1;
        let logits: Vec<f32> = if self.from_vram {
            let got = kd::TokenIo::read(self, &[(vas.logits, m.vocab)])?;
            got.into_iter().next().unwrap_or_default().into_iter().map(f32::from_bits).collect()
        } else {
            ram_read(&self.host_logits, 4 * m.vocab as u64)
                .chunks(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        self.argmaxes.truncate(pos);
        self.argmaxes.push(cm::argmax(&logits).unwrap_or(usize::MAX));
        self.last_logits = logits.clone();
        let (start_ns, end_ns, busy_ns) = self.timing.unwrap_or((0, 0, 0));
        let out = TokenOutput { logits, start_ns, end_ns, busy_ns };
        // P6-97 (C-145 point 1): the routine rows of this token are handed to the stick (P6-98: written every 16 tokens
        // or every second, `gpu_i6::live_flush`), inside the token's time.
        if self.fast {
            self.journal.flush_live();
            // P6-98 (C-147 stage 0 (a)): this token's launch times by kind; a token that does not fit is left out (once
            // noted), never guessed.
            let kinds = kp::token_kinds(m.layers.len(), way.embed == kd::EmbedBy::Program, !self.from_vram);
            if let Err(e) = self.prof.add_token(&kinds, &self.launch_ns) {
                self.prof_err.get_or_insert_with(|| format!("pozitia {}: {} ({} lansari, {} timpi)", pos, e, kinds.len(), self.launch_ns.len()));
            }
        }
        // P6-95: the OS clock from the first launch written to the logits read (without the CPU check, C-142 point 3).
        let host = host_t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a));
        let _ = self.speed.add_token(pos, &out, host); // a board clock that went back is counted by Generation itself
        if self.fast {
            // P6-97 (C-146): the split. The board's span is on the board's clock: never more than the OS's own span
            // of the same token (the two clocks are not the same clock).
            let host_us = host.unwrap_or(0);
            let journal_us = super::gpu_i6::JOURNAL_US.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(journal_t0);
            self.split.push(kf::TokenSplit {
                pos,
                host_us,
                board_us: (end_ns.saturating_sub(start_ns) / 1000).min(host_us),
                journal_us: journal_us.min(host_us),
                screen_us: 0, // nothing is printed while a token runs (the text is shown at the end)
                check_us: 0,
            });
        }
        Ok(out)
    }
}

/// The CPU engine on the same weights, for `Generation`'s judgement of every position.
struct CpuCheck<'a, 'w> {
    w: &'a cm::ModelWeights<'w>,
    caches: Caches,
    /// P6-95: the CPU engine's time of every position (us, OS clock).
    us: Vec<u64>,
}

impl TokenCheck for CpuCheck<'_, '_> {
    fn cpu_logits(&mut self, token: usize, pos: usize) -> GpuResult<Vec<f32>> {
        let t0 = crate::time::since_boot_us();
        let r = cm::step_cpu(self.w, &mut self.caches, token, pos);
        if let Some(d) = t0.zip(crate::time::since_boot_us()).map(|(a, b)| b.saturating_sub(a)) {
            self.us.push(d);
        }
        r
    }
}

/// Step `i14-simbol` with the model file at `path` (the control model `mic_q8_0.kgr`); then a waiting `kronos --gpu`
/// request, if any (P6-62 step 3).
#[allow(clippy::too_many_arguments)]
pub(super) fn token_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    path: &str,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<()> {
    let mut tokens_done = 0usize;
    let mut subs = 0u32;
    let mut way: Option<kd::Way> = None;
    let r = (|| {
        let file = crate::fs::read_all(path).map_err(|_| {
            lines.push(format!("{}: I14: nu gasesc modelul de control {} (stick-ul de test il are in /kronos/)", p, path));
            bad("I14: lipseste modelul de control")
        })?;
        let sum = sha256_hex(&file);
        if sum != MIC_SHA256 {
            lines.push(format!("{}: I14: {}: SHA-256 {} difera de vectori.json ({})", p, path, sum, MIC_SHA256));
            return Err(bad("I14: modelul de control nu e cel din vectori.json"));
        }
        let m = read_kgr1(&file)?;
        drop(file);
        let n = km::launches_per_token(m.layers.len()) + 1;
        lines.push(format!(
            "{}: i14: modelul de control {} (SHA-256 ca vectori.json; greutati la intamplare, NU #513): {} straturi, E {}, {} capete, vocabular {}, context {}; {} lansari pe simbol + 1 copiere a logit-urilor",
            p, path, m.layers.len(), m.shape.e, m.shape.heads, m.vocab, m.context, n - 1
        ));
        let runs: Vec<Run<'_>> = MIC_CONTROLS
            .iter()
            .map(|c| Run {
                name: String::from(c.text),
                prompt: c.prompt.to_vec(),
                steps: c.greedy.len(),
                argmax: Some(c.argmax),
                greedy: Some(&c.greedy),
            })
            .collect();
        let out = run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, &mut tokens_done, &mut subs, &mut way)?;
        let chosen = way.ok_or(bad("I14: scara nu a ales un fel"))?;
        lines.push(format!(
            "{}: i14: {} simboluri pe placa in {} trimiteri ({} lansari pe simbol), trimise cu: {}; abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii)",
            p, tokens_done, subs, n, chosen.name(), out.worst, out.median
        ));
        // C-139, the honesty rule: a way of SENDING the same launches is a repair; going AROUND a launch is not.
        if chosen.embed == kd::EmbedBy::Driver {
            return Err(bad("I14: programul embed e gresit pe placa; cu embedding-ul scris de driver textele trec"));
        }
        if out.from_vram {
            return Err(bad("I14: copierea logit-urilor in RAM e gresita; cu logit-urile din VRAM textele trec"));
        }
        Ok(chosen)
    })();
    match &r {
        Ok(chosen) => lines.push(format!(
            "{}: pas I14 simbol: ok (modelul de control, {} simboluri comparate cu procesorul si cu vectorii lui Kronos, trimise cu: {}; #513 prin kronos --gpu)",
            p, tokens_done, chosen.name()
        )),
        Err(err) => {
            lines.push(format!("{}: pas I14 simbol: ESEC: {} (oprit la prima eroare, dupa {} simboluri)", p, err, tokens_done))
        }
    }
    // P6-62 step 3: the `kronos --gpu` request runs only after the control model passed. Its own failure does not fail
    // I14 (the chain still cleans up the same way); `/bin/kronos` then continues on the processor.
    let job = KRONOS_JOB.lock().take();
    if let Some(job) = job {
        let res = match &r {
            Ok(chosen) => kronos_job(p, vbar, clock, journal, st, l, &job, permit, lines, doorbell, &mut tokens_done, &mut subs, *chosen),
            Err(err) => Err(format!("modelul de control nu a trecut pe placa ({})", err)),
        };
        match &res {
            Ok(t) => lines.push(format!("{}: kronos --gpu: {} simboluri generate pe placa (fiecare comparat cu procesorul)", p, t.len())),
            Err(why) => lines.push(format!("{}: kronos --gpu: ESEC: {} -> kronos continua pe procesor", p, why)),
        }
        *KRONOS_RESULT.lock() = Some(res);
    }
    r.map(|_| ())
}

/// The `kronos --gpu` request: the model from its path, the prompt + `n` greedy tokens on the board.
#[allow(clippy::too_many_arguments)]
fn kronos_job<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    job: &KronosJob,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    tokens_done: &mut usize,
    subs: &mut u32,
    way: kd::Way,
) -> Result<Vec<usize>, String> {
    let file = crate::fs::read_all(&job.path).map_err(|e| format!("nu pot citi {}: {}", job.path, e.message()))?;
    let m = read_kgr1(&file).map_err(|e| format!("{}: {}", job.path, e))?;
    drop(file);
    if job.prompt.is_empty() || job.n == 0 {
        return Err(String::from("cererea nu are simboluri sau n = 0"));
    }
    if let Some(bad_id) = job.prompt.iter().find(|&&t| t >= m.vocab) {
        return Err(format!("simbolul {} e in afara vocabularului ({})", bad_id, m.vocab));
    }
    if job.prompt.len() >= m.context {
        return Err(format!("textul are {} simboluri, contextul modelului {}", job.prompt.len(), m.context));
    }
    lines.push(format!(
        "{}: kronos --gpu: {} ({} straturi, E {}, vocabular {}, context {}): {} simboluri date, {} de generat",
        p, job.path, m.layers.len(), m.shape.e, m.vocab, m.context, job.prompt.len(), job.n
    ));
    let name = job.path.rsplit('/').next().unwrap_or(&job.path);
    journal.push(Event::text(KG_MODEL, name.as_bytes()));
    journal.push(Event::Value { what: KG_ASKED, value: job.n as u32 });
    let runs = [Run { name: String::from("kronos --gpu"), prompt: job.prompt.clone(), steps: job.n, argmax: None, greedy: None }];
    let writes0 = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed);
    // P6-97 (C-145 point 1): the routine rows (window moves, TLB invalidates, doorbell and their notes) are held in
    // memory during the upload and the text and go to the stick once per token; every other row as before.
    journal.set_routine(Some(kf::gen_routine(&GEN_NOTES)));
    let run = run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, tokens_done, subs, &mut Some(way));
    let live = journal.live_stats();
    journal.set_routine(None);
    let mut out = run.map_err(|e| format!("{}", e))?;
    let writes = super::gpu_i6::HELD_WRITES.load(core::sync::atomic::Ordering::Relaxed).saturating_sub(writes0);
    if out.from_vram {
        return Err(String::from("copierea logit-urilor in RAM e gresita (logit-urile citite din VRAM)"));
    }
    let (mut all, worst, median) = (out.all, out.worst, out.median);
    lines.push(format!(
        "{}: kronos --gpu: abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii)",
        p, worst, median
    ));
    let made = all.last().map_or(0, |g| g.len());
    // P6-95 (C-141): the result and the speed in the gpu journal (it goes up to the hub by itself) and on the screen.
    journal.push(Event::Value { what: KG_MADE, value: made as u32 });
    journal.push(Event::Value { what: KG_FIRST_DIFF, value: u32::MAX });
    journal.push(Event::Value { what: KG_WORST, value: (worst as f64 * 1e6).min(u32::MAX as f64) as u32 });
    out.speed.journal(journal, false);
    let (cpu_med, cpu_x100) = cpu_speed(&out.cpu_us);
    journal.push(Event::Value { what: KV_CPU_MED, value: cpu_med.map_or(u32::MAX, |v| v.min(u32::MAX as u64) as u32) });
    journal.push(Event::Value { what: KV_CPU_X100, value: cpu_x100.map_or(u32::MAX, |v| v.min(u32::MAX as u64) as u32) });
    lines.extend(out.speed.lines().into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    let x100 = |v: Option<u64>| v.map_or(String::from("?"), |x| format!("{}.{:02}", x / 100, x % 100));
    lines.push(format!(
        "{}: kronos --gpu: viteza: placa {} simboluri/s (ceasul placii), cu driverul {} simboluri/s; motorul procesorului (nucleul, 1 nucleu) median {} us pe pozitie = {} simboluri/s",
        p,
        x100(out.speed.summary().board_per_s_x100),
        x100(out.speed.summary().host_per_s_x100),
        cpu_med.map_or(String::from("?"), |v| format!("{}", v)),
        x100(cpu_x100)
    ));
    // P6-97 (C-146): the time per token, split (board / driver / journal / screen + the CPU check), in the gpu journal
    // (`kt …`) and in the output (so also in the cycle's report).
    let label = if permit.is_some() { "placa" } else { "SIMULARE" };
    let mut split = kf::TimeSplit::new();
    for t in &out.split {
        let _ = split.add(*t); // parts are clamped to the whole in `run_token`
    }
    split.journal(journal);
    if let Some(c) = out.shadow_changed {
        journal.push(Event::Value { what: KT_SHADOW, value: c.min(u32::MAX as usize) as u32 });
        lines.push(format!(
            "{}: kronos --gpu: umbra lansarilor: {}",
            p,
            if c == 0 { String::from("recitite la final, identice (placa nu rescrie sloturile)") } else { format!("{} cuvinte schimbate de placa in sloturi (umbra NU e sigura)", c) }
        ));
    }
    lines.extend(split.lines(label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    // P6-98 (C-147 stage 0 (a)): the time per kind of launch (`kp …` rows + the table in the output / cycle report).
    out.prof.journal(journal);
    lines.extend(kind_lines(&out.prof, label).into_iter().map(|x| format!("{}: kronos --gpu: {}", p, x)));
    if let Some(why) = &out.prof_err {
        lines.push(format!("{}: kronos --gpu: timpul pe fel de lansare: un simbol lasat deoparte: {}", p, why));
    }
    lines.push(format!(
        "{}: kronos --gpu: jurnalul pe stick: {} randuri de rutina tinute in memorie (nu pe ecran), scrise pe stick in {} scrieri (la {} simboluri sau la o secunda; {} simboluri)",
        p, live.held, writes, super::gpu_i6::FLUSH_TOKENS, live.flushes
    ));
    lines.push(format!("{}: kronos --gpu: {} simboluri generate pe placa, toate identice cu procesorul (niciun simbol diferit)", p, made));
    // P6-98 (C-147 stage 0 (d)): the VRAM → VRAM copy, after the result rows are on the stick; its failure is a line,
    // not a failure of `kronos --gpu` (the text is already generated and checked).
    if let Some(ctx) = out.copy.take() {
        if let Err(e) = vram_copy(p, vbar, clock, journal, st, l, permit, lines, doorbell, subs, &ctx) {
            lines.push(format!("{}: kronos --gpu: copierea VRAM -> VRAM: nemasurata: {}", p, e));
        }
    }
    Ok(all.pop().unwrap_or_default())
}

/// P6-98 (C-147 stage 0 (a)): the table "time per kind of launch", longest first (median per token and per launch).
fn kind_lines(prof: &kp::KindProfile, label: &str) -> Vec<String> {
    if prof.tokens() == 0 {
        return Vec::new();
    }
    let mut kinds: Vec<kp::KindTime> = prof.summary().into_iter().filter(|k| k.kind.is_some()).collect();
    kinds.sort_by(|a, b| b.token_median_ns.cmp(&a.token_median_ns));
    let total: u64 = kinds.iter().map(|k| k.token_median_ns).sum();
    let mut v = alloc::vec![format!(
        "timpul pe fel de lansare ({}, median pe simbol, {} simboluri; toate ~{} us pe simbol):",
        label,
        prof.tokens(),
        total / 1000
    )];
    for k in kinds {
        let name = k.kind.map_or("?", |x| x.name());
        v.push(format!(
            "  {:<12} {:>7} us pe simbol ({} lansari; una {} ns)",
            name,
            k.token_median_ns / 1000,
            k.per_token,
            k.launch_median_ns
        ));
    }
    v
}

/// P6-98 (C-147 stage 0 (d)): one VRAM → VRAM copy of (at most) 64 MiB of the weights into a fresh scratch piece,
/// repeated `kp::VRAM_COPY_REPEATS` times in one submission with a timestamp record after each (`copy_b128`, already on
/// the board since I11), three words of the destination read back and compared with the host's copy; `kc …` rows.
#[allow(clippy::too_many_arguments)]
fn vram_copy<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
    subs: &mut u32,
    ctx: &CopyCtx,
) -> GpuResult<()> {
    let bytes = kp::VramCopy::size_for(ctx.src.len() as u64);
    let scratch = map_vram(st, bytes)?;
    let c = kp::VramCopy::new(ctx.src_va, scratch.va, bytes, kp::VRAM_COPY_REPEATS)?;
    let (cb_va, cb_pa) = (ctx.launches.va, ctx.launches.pa);
    {
        let w = st.mem.writer_mut();
        put_bytes(w, cb_pa, &c.cbuf()?)?;
        put_bytes(w, cb_pa + 256, &c.qmd(ctx.copy_va, cb_va)?)?;
        put_bytes(w, ctx.rec.pa, &alloc::vec![0u8; c.record_bytes() as usize])?;
    }
    let first = TOKEN_FIRST + STRIDE * *subs;
    let mut words = alloc::vec![0u32; c.words()];
    let mut pb = PushBuf::new(&mut words);
    c.encode(&mut pb, cb_va + 256, ctx.rec.va, first)?;
    let nw = pb.len();
    let repeats = kp::VRAM_COPY_REPEATS;
    let last = first + repeats as u32;
    let mut maps = ctx.maps.clone();
    maps.push(scratch);
    let sys = [ctx.sys.clone()];
    let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], ctx.rec, repeats as u64 + 1, last, &maps, &sys, doorbell)? {
        Ok(b) => b,
        Err(got) => {
            lines.push(format!("{}: kronos --gpu: copierea VRAM -> VRAM: ultima inregistrare = {:#x} (asteptat {:#x})", p, got, last));
            return Err(GpuError::Timeout { waited_us: WAIT_US });
        }
    };
    *subs += 1;
    let s = c.analyze(&b, first)?;
    let got = {
        let GpuState { zone, .. } = &*st;
        let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
        let spots = c.spots();
        let r = (|| Ok::<[u32; 3], GpuError>([vw.read_u32(scratch.pa + spots[0])?, vw.read_u32(scratch.pa + spots[1])?, vw.read_u32(scratch.pa + spots[2])?]))();
        let back = vw.restore();
        r.and_then(|v| back.map(|_| v))?
    };
    let wrong = c.wrong_spots(&ctx.src, &got);
    c.journal(&s, Some(wrong), journal);
    lines.push(format!(
        "{}: kronos --gpu: copierea VRAM -> VRAM: {} KiB x {}, cuvinte gresite {} din 3; cea mai rapida {} us, mediana {} us = {} MB/s ({} MB/s trafic, {} promile din 608 GB/s)",
        p,
        bytes >> 10,
        repeats,
        wrong,
        s.min_ns / 1000,
        s.median_ns / 1000,
        s.copy_mb_s(),
        s.traffic_mb_s(),
        s.share_permille()
    ));
    Ok(())
}

/// The CPU engine's median time per position (us) and positions per second × 100 over all of them.
fn cpu_speed(us: &[u64]) -> (Option<u64>, Option<u64>) {
    if us.is_empty() {
        return (None, None);
    }
    let mut v = us.to_vec();
    v.sort_unstable();
    let total: u64 = us.iter().sum();
    let x100 = (total > 0).then(|| (us.len() as u128 * 100 * 1_000_000 / total as u128) as u64);
    (Some(v[v.len() / 2]), x100)
}
