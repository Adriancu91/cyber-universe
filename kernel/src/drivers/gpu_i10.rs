//! P6-56 step 3 (command C-72, Adrian's "da i7-i10"): I10 of the window chain in the OS — the first computation on the
//! work channel opened at I9 (`i10-add64`) and the cost of one launch (`i10-cost-lansare`).
//!
//! `i10-add64`: the program `add64` (64 f32 additions, one block of 64 threads, compiled by Cyber-GPU with NAK for SM 8.6;
//! our own program, a copy of their `tools/nak/out/add64.sm86.sass` at the pinned commit, checked by
//! `tools/after-gpu.sh`), its constant buffer (the addresses of A, B, C at 0x28/0x30/0x38), the inputs, C filled with NaN,
//! the QMD and a zeroed semaphore are written into VRAM (zone of golden/I9) and mapped in our page tables; the compute
//! setup and one pushbuffer (invalidate caches, launch, semaphore release) go into the work channel's ring
//! (`WorkQueue`). Then, in this order (Cyber-GPU's note `C-72_soneria-i10.md`):
//! 1. every collected VRAM word through the VRAM window, each read back;
//! 2. the TLB invalidate of our page directory, once, after the tables are written and before the first doorbell;
//! 3. **the work-channel doorbell** `push::DOORBELL` (0xBB0090, from NVIDIA's MIT 570.144 headers; documented, NOT YET
//!    OBSERVED on the board) = the token GSP-RM gave at I9 — the only new register write of I10, through its own fence
//!    (`gpu_i6::hw::DoorbellFence`: that one offset, that one value);
//! 4. the semaphore read through the VRAM window until it is the 64-bit value we asked for (≤ 2 s), then C, compared
//!    **bit for bit** with the CPU (`gpu_compute::reference::add_ftz`, the FTZ addition NAK compiled). Anything else is
//!    a failure, never a guess. The window register is put back on every path.
//!
//! `i10-cost-lansare`: Cyber-GPU's measurement 1 (`launch_cost`): a start record, then 16 launches of the same QMD, each
//! followed by a timestamp release (the GPU's own clock); one more doorbell; the 17 records read back and analysed
//! (every payload present and in order) → median / min / max ns per launch.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU with the `gpu-sim` command processor
//! running behind the doorbell on the VRAM model. On the board it is a first try.
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml): `gsp::work_queue`, `qmd`, `launch_cost`,
//! `vram_window`, `push::DOORBELL`; their simulation test of the first launch (`gsp::user_space`, I10b) for the order of
//! the buffers. Nothing copied from other projects.

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{poll_until, Clock, Mmio};
use nvidia_ga104::gsp::golden::{GoldenMemory, Mapping};
use nvidia_ga104::gsp::work_queue::{
    compute_setup, read_semaphore, ChannelMemory, WorkQueue, LOCAL_MEM_WINDOW, SHARED_MEM_WINDOW,
};
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::launch_cost;
use nvidia_ga104::mmu::SysMapping;
use nvidia_ga104::push::{PushBuf, DOORBELL};
use nvidia_ga104::qmd::ComputeQmd;
use nvidia_ga104::vram_window::VramWindow;

use super::gpu_golden::{flush, GpuState, Pending, VramZone};

/// `add64` for SM 8.6 (NAK), 240 bytes: a copy of Cyber-GPU's `tools/nak/out/add64.sm86.sass` (commit pinned in
/// kernel/Cargo.toml); `tools/after-gpu.sh` checks it is the same file. Registers per thread from their dump: 19.
pub(super) const ADD64: &[u8] = include_bytes!("gpu_prog/add64.sm86.sass");
const ADD64_REGISTERS: u32 = 19;
/// The completion value: both 32-bit halves matter (a GPU that wrote only one half would not match).
pub(super) const DONE: u64 = 0x1_0000_0007;
/// How long the driver waits for the semaphore / the last timestamp record.
pub(super) const WAIT_US: u64 = 2_000_000;
/// The launch-cost measurement: 16 launches, payloads 0x500..=0x510.
const COST_LAUNCHES: u32 = 16;
const COST_FIRST: u32 = 0x500;

/// What the GPU needs to see for one doorbell, given to the board's doorbell (unused there) and to the simulated GPU
/// (which fetches exactly these through the page tables).
#[cfg_attr(not(feature = "test-gpu-sim"), allow(dead_code))]
pub(super) struct Ring<'a> {
    pub root: u64,
    pub ring_va: u64,
    pub entries: u32,
    pub userd_va: u64,
    pub maps: &'a [Mapping],
    /// P6-62 (C-83, I11+): host-RAM buffers (the OS's DMA pages) the GPU reads or writes through its PTEs.
    pub sys: &'a [SysMapping],
}

/// Buffers of the `add64` launch, all mapped in our VA.
pub(super) struct Add64 {
    pub prog: Mapping,
    pub cbuf: Mapping,
    pub a: Mapping,
    pub b: Mapping,
    pub c: Mapping,
    pub qmd: Mapping,
    pub sem: Mapping,
    want: Vec<u32>,
}

/// What I10 keeps for the launch cost.
pub(super) struct Launched {
    pub(super) k: Add64,
    pub(super) push: Mapping,
    pub(super) wq: WorkQueue,
}

/// The inputs (the same as Cyber-GPU's simulation test, so the results compare): x = i/4 − 3, y = i²/7.
fn inputs() -> (Vec<f32>, Vec<f32>) {
    ((0..64).map(|i| i as f32 * 0.25 - 3.0).collect(), (0..64).map(|i| (i * i) as f32 / 7.0).collect())
}

pub(super) fn put_bytes<W: ChannelMemory + ?Sized>(w: &mut W, pa: u64, bytes: &[u8]) -> GpuResult<()> {
    for (i, ch) in bytes.chunks(4).enumerate() {
        let mut b = [0u8; 4];
        b[..ch.len()].copy_from_slice(ch);
        w.write_u32(pa + 4 * i as u64, u32::from_le_bytes(b))?;
    }
    Ok(())
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Allocates, maps and fills the buffers (collected, written at the next flush).
fn buffers(st: &mut GpuState) -> GpuResult<Add64> {
    let mut m = || st.mem.alloc_mapped(0x1000, 0x1000);
    let (prog, cbuf, a, b, c, qmd, sem) = (m()?, m()?, m()?, m()?, m()?, m()?, m()?);
    let (x, y) = inputs();
    let mut want = alloc::vec![0.0f32; 64];
    gpu_compute::reference::add_ftz(&x, &y, &mut want)?;
    let w = st.mem.writer_mut();
    put_bytes(w, prog.pa, ADD64)?;
    let mut cb = alloc::vec![0u8; 0x100];
    for (off, va) in [(0x28usize, a.va), (0x30, b.va), (0x38, c.va)] {
        cb[off..off + 8].copy_from_slice(&va.to_le_bytes());
    }
    put_bytes(w, cbuf.pa, &cb)?;
    put_bytes(w, a.pa, &f32_bytes(&x))?;
    put_bytes(w, b.pa, &f32_bytes(&y))?;
    put_bytes(w, c.pa, &f32_bytes(&[f32::NAN; 64]))?;
    let q = ComputeQmd {
        program_va: prog.va,
        registers: ADD64_REGISTERS,
        cbuf0_va: cbuf.va,
        cbuf0_size: 0x100,
        grid: [1, 1, 1],
        block: [64, 1, 1],
        release: None,
    };
    put_bytes(w, qmd.pa, &q.to_bytes()?)?;
    put_bytes(w, sem.pa, &[0; 16])?;
    Ok(Add64 { prog, cbuf, a, b, c, qmd, sem, want: want.iter().map(|v| v.to_bits()).collect() })
}

/// Reads through the VRAM window (writes refused: everything is written by `flush`, read back there).
pub(super) struct Reader<'w, 'a, M: Mmio + ?Sized>(&'w mut VramWindow<'a, M>);

impl<M: Mmio + ?Sized> ChannelMemory for Reader<'_, '_, M> {
    fn write_u32(&mut self, _pa: u64, _v: u32) -> GpuResult<()> {
        Err(GpuError::NotPermitted("I10: scrierile VRAM doar prin colector"))
    }
    fn read_u32(&mut self, pa: u64) -> GpuResult<u32> {
        self.0.read_u32(pa)
    }
}

/// Flush + TLB invalidate (window put back), the doorbell, then `wait` on the window (window put back on every path).
#[allow(clippy::too_many_arguments)]
pub(super) fn kick_and_wait<V: Mmio + ?Sized, K: Clock + ?Sized, T>(
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    permit: Option<&HardwareWritePermit>,
    zone: &VramZone,
    pending: &Pending,
    words: &mut u64,
    root: u64,
    bell: (u64, u32),
    ring: &Ring<'_>,
    doorbell: &mut dyn FnMut(&mut V, &mut Journal, u64, u32, &Ring<'_>) -> GpuResult<()>,
    wait: impl FnOnce(&mut Reader<'_, '_, V>, &mut K) -> GpuResult<T>,
) -> GpuResult<T> {
    flush(&mut *vbar, &mut *journal, permit, zone, pending, words)?;
    {
        let mut vw = VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
        let r = vw.invalidate_tlb(&mut *clock, root);
        let back = vw.restore();
        r.and(back)?;
    }
    doorbell(&mut *vbar, &mut *journal, bell.0, bell.1, ring)?;
    let mut vw = VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
    let r = wait(&mut Reader(&mut vw), clock);
    let back = vw.restore();
    r.and_then(|t| back.map(|_| t))
}

/// Step `i10-add64`. `vbar` = the VRAM-window fence; `doorbell` writes the work-channel doorbell (board: its own fence;
/// SIMULATION: the GPU model runs the channel).
#[allow(clippy::too_many_arguments)]
pub(super) fn add64_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut dyn FnMut(&mut V, &mut Journal, u64, u32, &Ring<'_>) -> GpuResult<()>,
) -> GpuResult<Launched> {
    let words0 = st.vram_words;
    let r = (|| {
        let w = st.space.as_ref().map(|(_, w)| w.clone()).ok_or(GpuError::InvalidArgument("I10: lipseste canalul de lucru"))?;
        let k = buffers(st)?;
        let push = st.mem.alloc_mapped(0x1000, 0x1000)?;
        let mut wq = WorkQueue::new(&w, push)?;
        let mut buf = [0u32; 16];
        let mut pb = PushBuf::new(&mut buf);
        compute_setup(&mut pb, LOCAL_MEM_WINDOW, SHARED_MEM_WINDOW)?;
        let n = pb.len();
        wq.submit(st.mem.writer_mut(), &buf[..n])?;
        let bell = wq.launch_and_signal(st.mem.writer_mut(), k.qmd.va, k.sem.va, DONE)?;
        if bell.bar0_offset != DOORBELL || bell.value != w.doorbell_token {
            return Err(GpuError::InvalidArgument("I10: soneria nu e cea documentata sau jetonul difera"));
        }
        let root = st.mem.root();
        let maps = [w.ring, push, k.prog, k.cbuf, k.a, k.b, k.c, k.qmd, k.sem];
        let ring = Ring { root, ring_va: w.ring.va, entries: w.entries, userd_va: w.ring.va + w.userd_offset, maps: &maps, sys: &[] };
        let GpuState { pending, zone, vram_words, .. } = &mut *st;
        let (sem, c) = kick_and_wait(vbar, clock, journal, permit, zone, pending, vram_words, root, (bell.bar0_offset, bell.value), &ring, doorbell, |rd, clock| {
            let mut last = 0;
            let got = poll_until(clock, WAIT_US, 100, || {
                last = read_semaphore(rd, k.sem.pa).ok()?;
                (last == DONE).then_some(())
            });
            if got.is_err() {
                return Ok((last, Vec::new()));
            }
            let c = (0..64).map(|i| rd.read_u32(k.c.pa + 4 * i)).collect::<GpuResult<Vec<u32>>>()?;
            wq.update_get(rd)?;
            Ok((last, c))
        })?;
        if sem != DONE {
            lines.push(format!("{}: I10: semaforul = {:#x} dupa {} ms (asteptat {:#x}): placa nu a terminat", p, sem, WAIT_US / 1000, DONE));
            return Err(GpuError::Timeout { waited_us: WAIT_US });
        }
        let diff = c.iter().zip(&k.want).filter(|(a, b)| a != b).count();
        if diff != 0 {
            let first = c.iter().zip(&k.want).position(|(a, b)| a != b).unwrap_or(0);
            lines.push(format!(
                "{}: I10: add64: {} din 64 rezultate diferite de procesor (primul la {}: {:#010x}, asteptat {:#010x})",
                p, diff, first, c[first], k.want[first]
            ));
            return Err(GpuError::VerifyFailed { offset: first as u32 * 4, expected: k.want[first], actual: c[first] });
        }
        wq.retire(k.sem.va, DONE)?;
        Ok(Launched { k, push, wq })
    })();
    match r {
        Ok(l) => {
            lines.push(format!(
                "{}: pas I10 add64: ok (64 rezultate identice bit cu bit cu procesorul; semafor {:#x}; soneria canalului {:#x} o data; {} cuvinte VRAM scrise si recitite)",
                p,
                DONE,
                DOORBELL,
                st.vram_words - words0
            ));
            Ok(l)
        }
        Err(e) => {
            lines.push(format!("{}: pas I10 add64: ESEC: {} (oprit la prima eroare)", p, e));
            Err(e)
        }
    }
}

/// Step `i10-cost-lansare`: 16 launches with a timestamp release after each, one doorbell, the records analysed.
#[allow(clippy::too_many_arguments)]
pub(super) fn cost_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut dyn FnMut(&mut V, &mut Journal, u64, u32, &Ring<'_>) -> GpuResult<()>,
) -> GpuResult<u64> {
    let r = (|| {
        let w = st.space.as_ref().map(|(_, w)| w.clone()).ok_or(GpuError::InvalidArgument("I10: lipseste canalul de lucru"))?;
        let bytes = (u64::from(COST_LAUNCHES) + 1) * launch_cost::RECORD;
        let rec = st.mem.alloc_mapped(0x1000, 0x1000)?;
        put_bytes(st.mem.writer_mut(), rec.pa, &alloc::vec![0u8; bytes as usize])?;
        let mut words = alloc::vec![0u32; 256];
        let mut pb = PushBuf::new(&mut words);
        launch_cost::encode(&mut pb, l.k.qmd.va, rec.va, COST_LAUNCHES, COST_FIRST)?;
        let n = pb.len();
        let bell = l.wq.submit(st.mem.writer_mut(), &words[..n])?;
        if bell.bar0_offset != DOORBELL || bell.value != w.doorbell_token {
            return Err(GpuError::InvalidArgument("I10: soneria nu e cea documentata sau jetonul difera"));
        }
        let root = st.mem.root();
        let k = &l.k;
        let maps = [w.ring, l.push, k.prog, k.cbuf, k.a, k.b, k.c, k.qmd, k.sem, rec];
        let ring = Ring { root, ring_va: w.ring.va, entries: w.entries, userd_va: w.ring.va + w.userd_offset, maps: &maps, sys: &[] };
        let last_pa = rec.pa + launch_cost::RECORD * u64::from(COST_LAUNCHES);
        let want_last = COST_FIRST + COST_LAUNCHES;
        let wq = &mut l.wq;
        let GpuState { pending, zone, vram_words, .. } = &mut *st;
        let rec_bytes = kick_and_wait(vbar, clock, journal, permit, zone, pending, vram_words, root, (bell.bar0_offset, bell.value), &ring, doorbell, |rd, clock| {
            let mut last = 0;
            if poll_until(clock, WAIT_US, 100, || {
                last = rd.read_u32(last_pa).ok()?;
                (last == want_last).then_some(())
            })
            .is_err()
            {
                return Ok(Err(last));
            }
            let mut b = Vec::with_capacity(bytes as usize);
            for o in (0..bytes).step_by(4) {
                b.extend_from_slice(&rd.read_u32(rec.pa + o)?.to_le_bytes());
            }
            wq.update_get(rd)?;
            Ok(Ok(b))
        })?;
        let b = match rec_bytes {
            Ok(b) => b,
            Err(last) => {
                lines.push(format!(
                    "{}: I10 cost: ultima inregistrare = {:#x} dupa {} ms (asteptat {:#x}): placa nu a terminat",
                    p, last, WAIT_US / 1000, want_last
                ));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        let cost = launch_cost::analyze(&b, COST_LAUNCHES, COST_FIRST)?;
        lines.push(format!("{}: I10 {}", p, cost.line()));
        Ok(cost.median_ns)
    })();
    match r {
        Ok(m) => {
            lines.push(format!("{}: pas I10 cost lansare: ok (mediana {} ns, masurata de ceasul placii)", p, m));
            Ok(m)
        }
        Err(e) => {
            lines.push(format!("{}: pas I10 cost lansare: ESEC: {} (oprit la prima eroare)", p, e));
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SIMULARE (test builds): the GPU behind the doorbell = Cyber-GPU's command processor on the VRAM model.
// ---------------------------------------------------------------------------------------------------------------

/// The simulated GPU: on the doorbell it checks the offset, the token and that the TLB was invalidated after the last
/// table write, fetches the ring, the pushbuffer and the buffers through the page tables, runs the channel and writes
/// back what it changed. `sim_launch_ns` = 4000 (the numbers of the cost step are the model's, only the method counts).
#[cfg(feature = "test-gpu-sim")]
pub(super) struct SimGpu {
    cp: gpu_sim::cmdproc::CommandProcessor,
    pub token: u32,
    pub rung: u32,
    pub launches: usize,
    pub invalidates_seen: usize,
    /// P6-62: the model of host RAM (the OS's DMA pages, indexed by bus address), as in Cyber-GPU's tests.
    sys: gpu_sim::vram::SimVram,
}

#[cfg(feature = "test-gpu-sim")]
impl SimGpu {
    pub(super) fn new(token: u32) -> Self {
        use gpu_sim::cmdproc as c;
        let mut cp = c::CommandProcessor::new();
        cp.register_program(ADD64, c::add64_on_cpu);
        // P6-62 (C-83): the programs of I11/I12 (Cyber-GPU's NAK builds) and their CPU stand-ins.
        cp.register_program(nvidia_ga104::transfer::COPY_B128, c::copy_b128_on_cpu);
        cp.register_program_with_needs(nvidia_ga104::q8_matvec::Q8_MATVEC, c::q8_matvec_on_cpu, c::Q8_MATVEC_NEEDS);
        {
            use nvidia_ga104::kronos_layer as kl;
            cp.register_program(kl::VADD, c::vadd_on_cpu);
            cp.register_program_with_needs(kl::LAYERNORM, c::layernorm_on_cpu, c::LAYERNORM_NEEDS);
            cp.register_program_with_needs(kl::ATTENTION, c::attention_on_cpu, c::ATTENTION_NEEDS);
            cp.register_program(kl::GELU, c::gelu_on_cpu);
        }
        cp.sim_launch_ns = 4000;
        let sys = gpu_sim::vram::SimVram::new(crate::mem::frame::top_of_ram().next_multiple_of(0x1000));
        SimGpu { cp, token, rung: 0, launches: 0, invalidates_seen: 0, sys }
    }

    /// The OS's DMA pages of `m` (real RAM, the arena of `gpu_i11`) copied into the model's host memory, or back.
    fn sys_sync(&mut self, m: &SysMapping, to_model: bool) -> GpuResult<()> {
        for (page, &bus) in super::gpu_i11::ram(m).zip(&m.pages) {
            for (i, w) in page.chunks_exact_mut(8).enumerate() {
                let a = bus + 8 * i as u64;
                if to_model {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(w);
                    self.sys.write_u64(a, u64::from_le_bytes(b)).map_err(GpuError::InvalidArgument)?;
                } else {
                    w.copy_from_slice(&self.sys.read_u64(a).map_err(GpuError::InvalidArgument)?.to_le_bytes());
                }
            }
        }
        Ok(())
    }

    pub(super) fn ring(&mut self, v: &mut gpu_sim::pramin::SimBar0Vram, off: u64, value: u32, r: &Ring<'_>) -> GpuResult<()> {
        if off != DOORBELL || value != self.token {
            return Err(GpuError::NotPermitted("SIM: alta sonerie sau alt jeton"));
        }
        if v.invalidates.len() <= self.invalidates_seen {
            return Err(GpuError::InvalidArgument("SIM: soneria fara invalidarea TLB inainte"));
        }
        self.invalidates_seen = v.invalidates.len();
        self.rung += 1;
        for m in r.sys {
            self.sys_sync(m, true)?;
        }
        let spans: Vec<(u64, u64)> =
            r.maps.iter().map(|m| (m.va, m.size.next_multiple_of(0x1000))).chain(r.sys.iter().map(|m| (m.va, m.size))).collect();
        for &(va, len) in &spans {
            self.cp.mem.load_from(&v.vram, &self.sys, r.root, va, len)?;
        }
        self.cp.run_channel(r.ring_va, r.entries, r.userd_va)?;
        for &(va, len) in &spans {
            self.cp.mem.store_to(&mut v.vram, &mut self.sys, r.root, va, len)?;
        }
        for m in r.sys {
            self.sys_sync(m, false)?;
        }
        self.launches = self.cp.events.iter().filter(|e| matches!(e, gpu_sim::cmdproc::Event::Launch { .. })).count();
        Ok(())
    }
}
