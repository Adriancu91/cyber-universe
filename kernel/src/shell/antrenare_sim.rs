//! P6-158 (C-161 point 5, C-158 point 10): `antrenare mini --placa-sim` — the whole training step of `antrenare mini`
//! done by the board's own list of launches (`nvidia_ga104::train_step::Plan::step`, the 13 training programs of the
//! small golden step that already ran on the RTX 3070 Ti in os-351) instead of Cyber-GPU's CPU oracle, on the
//! SIMULATED board (`gpu_sim::cmdproc`: the programs' CPU stand-ins behind the real control zone, QMDs and pushbuffer).
//! Every step is one submission, as on the board: the CPU writes the arena (weights, AdamW's m and v, the batch, the
//! segment lists), the list runs, the CPU reads back the per-row losses, the gradient norm, the weights and m, v.
//! The oracle runs beside it on the same batches from the same start (`--fara-oracol` drops it), so every step says
//! how far the board's list is from the oracle. Test builds only (`test-gpu-sim`): nothing here touches a real board.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu_compute::train::{AdamW, Arch, Recipe};
use gpu_sim::cmdproc as c;
use nvidia_ga104::push::{self, PushBuf};
use nvidia_ga104::train_step::{self as ts, Plan, Prog};
use nvidia_ga104::transfer;

const VA: u64 = nvidia_ga104::mmu::VA_BASE;
/// The control zone of one step (cbuf + QMD per launch).
const ZONE: u64 = VA + 0x20_0000;
/// The timestamp records of one submission (16 bytes each).
const REC: u64 = VA + 0x30_0000;
const PUSH: u64 = VA + 0x32_0000;
/// First program; one 64 KiB slot each.
const PROGS: u64 = VA + 0x40_0000;
/// The arena (`Plan::new`).
const HEAP: u64 = VA + 0x100_0000;
/// Compute class C7C0 (`nvidia_rm570::regs::clc7c0`): `SET_OBJECT` is method 0, `AMPERE_COMPUTE_B` = 0xc7c0.
const SET_OBJECT: u32 = 0x0;
const AMPERE_COMPUTE_B: u32 = 0xc7c0;

fn prog_va(p: Prog) -> u64 {
    PROGS + 0x1_0000 * p.index() as u64
}

/// Which arena / attention programs the step's list uses (P6-159, cyber-gpu `1b9f79d`, note C-160_P7_placi-in-pas).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `Plan::new`: P kept per layer, the simple attention programs (13 programs, os-351 on the board).
    Simple,
    /// `Plan::new_p_only`: one shared P, rebuilt on the way back with `attn_p` (14 programs).
    POnly,
    /// `Plan::new_tiles`: P kept per layer, attention on the tiled `attn_*_2` programs (14 programs, os-364).
    Tiles,
    /// `Plan::new_p_only_tiles`: one shared P, the tiled attention, P rebuilt with `attn_p_tile` (15 programs).
    TilesPOnly,
}

impl Layout {
    /// The word after `--atentie` (`simpla`, `p`, `placi`, `placi-p`).
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "simpla" => Some(Self::Simple),
            "p" => Some(Self::POnly),
            "placi" => Some(Self::Tiles),
            "placi-p" => Some(Self::TilesPOnly),
            _ => None,
        }
    }

    /// For the report.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Simple => "atentia simpla, P pastrat pe strat (13 programe, Plan::new)",
            Self::POnly => "atentia simpla, un singur P refacut cu attn_p (14 programe, Plan::new_p_only)",
            Self::Tiles => "atentia pe placi attn_*_2, P pastrat pe strat (14 programe, Plan::new_tiles)",
            Self::TilesPOnly => "atentia pe placi attn_*_2, un singur P refacut cu attn_p_tile (15 programe, Plan::new_p_only_tiles)",
        }
    }

    fn plan(self, a: Arch, batch: usize, len: usize, inputs: &[u32]) -> gpu_core::error::GpuResult<Plan> {
        match self {
            Self::Simple => Plan::new(a, batch, len, inputs, HEAP),
            Self::POnly => Plan::new_p_only(a, batch, len, inputs, HEAP),
            Self::Tiles => Plan::new_tiles(a, batch, len, inputs, HEAP),
            Self::TilesPOnly => Plan::new_p_only_tiles(a, batch, len, inputs, HEAP),
        }
    }
}

/// The simulated board with every program of `train_step::PROGRAMS_ALL` in its memory.
pub struct SimBoard {
    cp: c::CommandProcessor,
    layout: Layout,
    first: u32,
    /// Launches sent so far.
    pub launches: usize,
}

/// What one step gave back.
pub struct StepOut {
    /// The distinct programs this step's list launched.
    pub programs: usize,
    pub loss: f32,
    pub norm: f32,
    pub launches: usize,
    /// Bytes of board memory the step's arena takes (`Plan::bytes`: weights, gradients, m, v, activations, logits).
    pub arena: u64,
}

impl SimBoard {
    pub fn new(layout: Layout) -> Result<Self, String> {
        ts::check_programs().map_err(|e| format!("SHA-256 al programelor: {:?}", e))?;
        let mut cp = c::CommandProcessor::new();
        for prog in ts::PROGRAMS_ALL {
            let code = prog.code();
            let _ = match prog {
                Prog::MmTrain => cp.register_program_with_needs(code, c::mm_train_on_cpu, c::MM_TRAIN_NEEDS),
                Prog::Xent => cp.register_program_with_needs(code, c::xent_on_cpu, c::XENT_NEEDS),
                Prog::LnFwd => cp.register_program_with_needs(code, c::ln_fwd_on_cpu, c::LN_FWD_NEEDS),
                Prog::LnBack => cp.register_program_with_needs(code, c::ln_back_on_cpu, c::LN_BACK_NEEDS),
                Prog::GeluTr => cp.register_program(code, c::gelu_tr_on_cpu),
                Prog::AttnFwd => cp.register_program(code, c::attn_fwd_on_cpu),
                Prog::AttnBack => cp.register_program(code, c::attn_back_on_cpu),
                Prog::EmbFwd => cp.register_program(code, c::emb_fwd_on_cpu),
                Prog::EmbBack => cp.register_program(code, c::emb_back_on_cpu),
                Prog::Gnorm => cp.register_program_with_needs(code, c::gnorm_on_cpu, c::GNORM_NEEDS),
                Prog::OptStep => cp.register_program(code, c::opt_step_on_cpu),
                Prog::Colsum => cp.register_program(code, c::colsum_on_cpu),
                Prog::Vadd => cp.register_program(code, c::vadd_on_cpu),
                Prog::AttnP => cp.register_program(code, c::attn_p_on_cpu),
                // The `_2` programs do the same operations in the same order as the first tiled ones: the same CPU
                // models (as `gpu_i10.rs`, P6-149).
                Prog::AttnFwdTile2 => cp.register_program(code, c::attn_fwd_tile_on_cpu),
                Prog::AttnBackTile0_2 => cp.register_program(code, c::attn_back_tile0_on_cpu),
                Prog::AttnBackTile2 => cp.register_program(code, c::attn_back_tile_on_cpu),
                Prog::AttnPTile => cp.register_program(code, c::attn_p_tile_on_cpu),
                #[allow(unreachable_patterns)]
                _ => return Err(format!("programul {} nu are model pe procesor", prog.name())),
            };
            cp.mem.write(prog_va(prog), code);
        }
        Ok(Self { cp, layout, first: 1, launches: 0 })
    }

    /// One AdamW step (`adamw_513`'s list with `adam`'s own lr / betas / eps / decay, clipping at 1.0) on the batch
    /// `inputs` → `targets`; `params`, `adam.m`, `adam.v` and `adam.t` come back as the board left them.
    pub fn step(
        &mut self,
        a: &Arch,
        (batch, len): (usize, usize),
        inputs: &[u32],
        targets: &[u32],
        params: &mut [Vec<f32>],
        adam: &mut AdamW,
    ) -> Result<StepOut, String> {
        fn e(what: &'static str) -> impl Fn(gpu_core::error::GpuError) -> String {
            move |err| format!("{}: {:?}", what, err)
        }
        let plan = self.layout.plan(*a, batch, len, inputs).map_err(e("planul"))?;
        for (va, bytes) in plan.uploads(params, inputs, targets).map_err(e("urcarile"))? {
            self.cp.mem.write(va, &bytes);
        }
        // `uploads` starts m and v at zero (the golden step's first step); here they carry the run's AdamW state.
        let flat = |set: &[Vec<f32>]| set.iter().flatten().copied().collect::<Vec<f32>>();
        self.cp.mem.write_f32s(plan.m, &flat(&adam.m));
        self.cp.mem.write_f32s(plan.v, &flat(&adam.v));
        let t = adam.t + 1;
        let list = plan.step(Recipe::AdamW513, adam, t).map_err(e("lista pasului"))?;
        let (zone, qmds) = ts::pack(&list, prog_va, ZONE).map_err(e("zona de control"))?;
        if zone.len() as u64 > REC - ZONE {
            return Err(String::from("zona de control prea mare"));
        }
        self.cp.mem.write(ZONE, &zone);
        let n = list.len();
        self.cp.mem.write(REC, &alloc::vec![0u8; 16 * (n + 1)]);
        let mut w = alloc::vec![0u32; 2 + transfer::sequence_words_fresh(n)];
        let mut pb = PushBuf::new(&mut w);
        pb.inc(1, SET_OBJECT, &[AMPERE_COMPUTE_B]).map_err(e("pushbuffer"))?;
        transfer::encode_sequence_fresh(&mut pb, &qmds, REC, self.first).map_err(e("pushbuffer"))?;
        let nw = pb.len();
        let bytes: Vec<u8> = w[..nw].iter().flat_map(|x| x.to_le_bytes()).collect();
        self.cp.mem.write(PUSH, &bytes);
        let entry = push::gpfifo_entry(PUSH, nw as u32, false).map_err(e("gpfifo"))?;
        self.cp.run_gpfifo(&[(entry.entry0, entry.entry1)]).map_err(e("placa simulata"))?;
        self.first = self.first.wrapping_add(n as u32 + 1);
        self.launches += n;
        let read = |va: u64, k: usize| self.cp.mem.read_f32s(va, k).map_err(e("citirea"));
        let loss = ts::Outcome::mean_loss(&read(plan.loss_rows, batch * len)?);
        let norm = read(plan.res, 1)?[0];
        if !norm.is_finite() {
            return Err(format!("norma gradientului nu e un numar ({}): pasul se opreste", norm));
        }
        let new_w = plan.unflat(&read(plan.w, plan.n_params)?).map_err(e("greutatile"))?;
        let new_m = plan.unflat(&read(plan.m, plan.n_params)?).map_err(e("m"))?;
        let new_v = plan.unflat(&read(plan.v, plan.n_params)?).map_err(e("v"))?;
        for (dst, src) in params.iter_mut().zip(new_w) {
            *dst = src;
        }
        adam.m = new_m;
        adam.v = new_v;
        adam.t = t;
        Ok(StepOut { programs: plan.programs().len(), loss, norm, launches: n, arena: plan.bytes() })
    }
}
