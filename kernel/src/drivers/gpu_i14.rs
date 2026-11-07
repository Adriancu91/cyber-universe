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
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::gsp::golden::Mapping;
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::kronos_layer::{self as kl, LayerVas, OwnedLayer, Prog, WeightVas, PROGRAMS};
use nvidia_ga104::kronos_model::{self as km, ModelVas, OwnedModel};
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

/// The board's pieces a run needs (P6-62 step 3: shared by the control model and a `kronos --gpu` request).
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
) -> GpuResult<(Vec<Vec<usize>>, f32, u64)> {
    let s = m.shape;
    let e = s.e;
    let n_tok = km::launches_per_token(m.layers.len());
    let n = n_tok + 1; // + the copy of the logits down
    let words_per_token = transfer::sequence_words(n);
    if 4 * words_per_token as u64 > l.push.size / 2 {
        lines.push(format!(
            "{}: I14: {} lansari pe simbol = {} cuvinte, zona de comenzi are {} octeti (trebuie marita)",
            p, n, words_per_token, l.push.size
        ));
        return Err(bad("I14: zona de comenzi prea mica pentru un simbol"));
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
    for (i, chunk) in blob.bytes.chunks(piece as usize).enumerate() {
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
            Ok(b) => up_ns += transfer::analyze_sequence(&b, 1, first)?[0],
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
    // 2. The VAs, the copy of the logits down (the same every token).
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
    let (down_cb_va, down_cb_pa) = (launches.va + 512 * n_tok as u64, launches.pa + 512 * n_tok as u64);
    {
        let w = st.mem.writer_mut();
        put_bytes(w, down_cb_pa, &transfer::copy_cbuf(vas.logits, host_logits.va, logits_bytes)?)?;
        put_bytes(w, down_cb_pa + 256, &transfer::copy_qmd(copy_va, down_cb_va, logits_bytes)?.to_bytes()?)?;
    }
    let mut tmaps = maps.clone();
    tmaps.push(rec);
    let weights = m.weights();
    let mut worst = 0.0f32;
    let mut tok_ns = Vec::new();
    let mut all = Vec::with_capacity(runs.len());
    // 3. The texts.
    for run in runs {
        let mut cpu_caches = Caches::new(m.layers.len(), e);
        let mut seq: Vec<usize> = run.prompt.clone();
        let positions = (run.prompt.len() + run.steps - 1).min(m.context);
        let mut greedy = Vec::with_capacity(run.steps);
        for pos in 0..positions {
            let tok = seq[pos];
            let list = km::token_launches(&s, m.vocab, m.context, &vas, tok, pos)?;
            let w = st.mem.writer_mut();
            let mut qmds = Vec::with_capacity(n);
            for (j, la) in list.iter().enumerate() {
                let (cb_va, cb_pa) = (launches.va + 512 * j as u64, launches.pa + 512 * j as u64);
                let pv = prog_va(la.prog).ok_or(bad("I14: program necunoscut"))?;
                put_bytes(w, cb_pa, &la.cbuf)?;
                put_bytes(w, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
                qmds.push(cb_va + 256);
            }
            qmds.push(down_cb_va + 256);
            put_bytes(w, rec.pa, &alloc::vec![0u8; 16 * (n + 1)])?;
            ram_write(&host_logits, &alloc::vec![0xee; logits_bytes as usize]);
            let first = TOKEN_FIRST + STRIDE * *tokens_done as u32;
            let mut words = alloc::vec![0u32; transfer::sequence_words(n)];
            let mut pb = PushBuf::new(&mut words);
            transfer::encode_sequence(&mut pb, &qmds, rec.va, first)?;
            let nw = pb.len();
            let last = first + n as u32;
            let sys = [host_logits.clone()];
            let b = match run_records(vbar, clock, journal, st, l, permit, &words[..nw], rec, n as u64 + 1, last, &tmaps, &sys, doorbell)? {
                Ok(b) => b,
                Err(got) => {
                    lines.push(format!(
                        "{}: I14: \"{}\" pozitia {}: ultima inregistrare = {:#x} (asteptat {:#x}): placa nu a terminat",
                        p, run.name, pos, got, last
                    ));
                    return Err(GpuError::Timeout { waited_us: WAIT_US });
                }
            };
            let t = transfer::analyze_sequence(&b, n, first)?;
            tok_ns.push(t.iter().sum::<u64>());
            *tokens_done += 1;
            let got: Vec<f32> = ram_read(&host_logits, 4 * m.vocab as u64)
                .chunks(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            let cpu = cm::step_cpu(&weights, &mut cpu_caches, tok, pos)?;
            let v = km::judge(&got, &cpu);
            if let km::TokenVerdict::Trece { max_abs, .. } = v {
                worst = worst.max(max_abs);
            }
            if !v.passes() {
                lines.push(format!("{}: \"{}\": {}", p, run.name, v.line(pos)));
                return Err(bad("I14: simbolul nu trece (vezi randul de mai sus)"));
            }
            let g = cm::argmax(&got).ok_or(bad("I14: logit-uri fara maxim"))?;
            if let Some(want) = run.argmax
                && pos < run.prompt.len()
                && g != want[pos]
            {
                lines.push(format!(
                    "{}: I14: \"{}\" pozitia {}: argmax {} pe placa si procesor, vectori.json {}",
                    p, run.name, pos, g, want[pos]
                ));
                return Err(bad("I14: argmax diferit de vectorii lui Kronos"));
            }
            if pos + 1 >= run.prompt.len() {
                greedy.push(g);
                seq.push(g);
            }
        }
        if let Some(want) = run.greedy {
            if greedy[..] != want[..] {
                lines.push(format!("{}: I14: \"{}\": 5 simboluri greedy {:?}, vectori.json {:?}", p, run.name, greedy, want));
                return Err(bad("I14: simbolurile greedy difera de vectorii lui Kronos"));
            }
            lines.push(format!(
                "{}: i14: \"{}\": TRECE - {} pozitii, argmax ca vectori.json pe fiecare pozitie, 5 simboluri greedy {:?} identice",
                p, run.name, positions, greedy
            ));
        }
        all.push(greedy);
    }
    tok_ns.sort_unstable();
    let median = tok_ns.get(tok_ns.len() / 2).copied().unwrap_or(0);
    Ok((all, worst, median))
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
        let (_, worst, median) =
            run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, &mut tokens_done)?;
        lines.push(format!(
            "{}: i14: {} simboluri pe placa, fiecare = o sonerie ({} lansari); abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii)",
            p, tokens_done, n, worst, median
        ));
        Ok(())
    })();
    match &r {
        Ok(()) => lines.push(format!(
            "{}: pas I14 simbol: ok (modelul de control, {} simboluri comparate cu procesorul si cu vectorii lui Kronos; #513 prin kronos --gpu)",
            p, tokens_done
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
            Ok(()) => kronos_job(p, vbar, clock, journal, st, l, &job, permit, lines, doorbell, &mut tokens_done),
            Err(err) => Err(format!("modelul de control nu a trecut pe placa ({})", err)),
        };
        match &res {
            Ok(t) => lines.push(format!("{}: kronos --gpu: {} simboluri generate pe placa (fiecare comparat cu procesorul)", p, t.len())),
            Err(why) => lines.push(format!("{}: kronos --gpu: ESEC: {} -> kronos continua pe procesor", p, why)),
        }
        *KRONOS_RESULT.lock() = Some(res);
    }
    r
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
    let runs = [Run { name: String::from("kronos --gpu"), prompt: job.prompt.clone(), steps: job.n, argmax: None, greedy: None }];
    let (mut all, worst, median) = run_model(p, vbar, clock, journal, st, l, &m, &runs, permit, lines, doorbell, tokens_done)
        .map_err(|e| format!("{}", e))?;
    lines.push(format!(
        "{}: kronos --gpu: abaterea maxima a logit-urilor fata de procesor {:e}; durata mediana a unui simbol {} ns (ceasul placii)",
        p, worst, median
    ));
    Ok(all.pop().unwrap_or_default())
}
