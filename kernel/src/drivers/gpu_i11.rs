//! P6-62 (command C-83, Adrian's "cât mai mulți pași în SIMULARE, și DUPĂ I10"): I11 and I12 of the window chain in
//! the OS, after `i10-cost-lansare` (Cyber-GPU's note `C-83_i11-i12.md`).
//!
//! `i11-copiere`: a large copy host RAM → VRAM → host RAM on the work channel. Two runs of the OS's DMA pages (source
//! filled with Cyber-GPU's pattern, destination with 0xEE) are mapped into our GPU VA with **host-RAM PTEs**
//! (`mmu::alloc_sys_mapped`: aperture SYSTEM_COHERENT, volatile — documented by NVIDIA's MIT headers, not yet observed
//! on the board); a VRAM buffer of the same size; the NAK program `copy_b128` twice (up, down) and three timestamp
//! records in one pushbuffer (`transfer::encode_round_trip`). After the doorbell: the last record, then the
//! destination compared **bit for bit** with the pattern (read from RAM), three spot words of the VRAM buffer through
//! the window, the speed of both directions from the board's own clock.
//!
//! `i12-q8-matvec`: y = W·x with W in Q8_0 (3072 × 768, Cyber-GPU's `test_matrix`): W|x copied up, `q8_matvec`, y
//! copied down (`transfer::encode_sequence`, 3 launches, 4 records); y read from RAM and judged by
//! `q8_matvec::judge` (identical to the program's order / inside the margin written beforehand / does not pass).
//!
//! No new register is written: only the VRAM window and the work-channel doorbell, as in I10 (`chain_log` checks it).
//! New on the board: the GPU writes by DMA into the OS's pages (the destination of I11, y of I12). Those pages are a
//! pinned arena used only by these steps (`frame::pin`, never given to anyone else), Bus Master is on from the chain.
//! A wrong host-RAM PTE shows as a missing record → clean stop; the chain puts Bus Master back.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU with `gpu-sim`'s command processor
//! behind the doorbell (host RAM = the OS's real DMA pages, copied into the model and back). On the board: a first try.
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml): `transfer`, `q8_matvec`, `mmu::alloc_sys_mapped`,
//! their simulation tests `sim_i11_copy.rs` / `sim_i12_q8.rs` for the order of the buffers. Nothing copied from others.

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{poll_until, Clock, Mmio};
use nvidia_ga104::gsp::golden::{GoldenMemory, Mapping};
use nvidia_ga104::gsp::work_queue::ChannelMemory;
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::launch_cost::RECORD;
use nvidia_ga104::mmu::SysMapping;
use nvidia_ga104::push::{PushBuf, DOORBELL};
use nvidia_ga104::q8_matvec::{self as mv, Q8_MATVEC};
use nvidia_ga104::transfer::{self, COPY_B128, ROUND_TRIP_WORDS};

use super::gpu_golden::GpuState;
use super::gpu_i10::{kick_and_wait, put_bytes, Launched, Ring, WAIT_US};

/// The copy size of I11 on the board (Cyber-GPU's default, 8 MiB).
pub(super) const I11_BOARD_BYTES: u64 = transfer::I11_DEFAULT_BYTES;
/// SIMULATION: the model moves every word through its own page walker; 1 MiB keeps the QEMU test short. Same code.
#[cfg(feature = "test-gpu-sim")]
pub(super) const I11_SIM_BYTES: u64 = 1 << 20;
const I11_SEED: u32 = 0x83;
const I11_FIRST: u32 = 0x600;
const I12_SEED: u32 = 0x12;
const I12_FIRST: u32 = 0x700;
const PAGE: u64 = 0x1000;

/// The DMA arena of I11/I12: 2 × 8 MiB (I11) + W|x and y of I12, contiguous, between 16 MiB and 4 GiB, pinned once
/// (the card may write there; nothing else ever gets these frames).
const ARENA_PAGES: u64 = 2 * (I11_BOARD_BYTES / PAGE) + 704;
static ARENA: crate::sched::Mutex<Option<u64>> = crate::sched::Mutex::new(None);

/// Bus addresses (no IOMMU = physical) of `n` arena pages from page `first`; the arena is allocated on first use.
fn arena_pages(first: u64, n: u64) -> GpuResult<Vec<u64>> {
    if first + n > ARENA_PAGES {
        return Err(GpuError::OutOfMemory { requested: n * PAGE });
    }
    let mut slot = ARENA.lock();
    let base = match *slot {
        Some(b) => b,
        None => {
            let b = crate::mem::frame::alloc_contiguous_between(ARENA_PAGES, 16 << 20, 1 << 32)
                .ok_or(GpuError::OutOfMemory { requested: ARENA_PAGES * PAGE })?;
            crate::mem::frame::pin(b, ARENA_PAGES, "gpu-i11");
            *slot = Some(b);
            b
        }
    };
    Ok((first..first + n).map(|i| base + i * PAGE).collect())
}

/// The bytes of a host-RAM mapping (CPU side, through the direct map).
pub(super) fn ram(m: &SysMapping) -> impl Iterator<Item = &'static mut [u8]> + '_ {
    m.pages.iter().map(|&bus| {
        // SAFETY: `bus` is a page of the pinned arena (`arena_pages`), mapped in the direct map; only this step uses it.
        unsafe { core::slice::from_raw_parts_mut(crate::mem::map::phys_to_virt(bus), PAGE as usize) }
    })
}

fn ram_write(m: &SysMapping, data: &[u8]) {
    let mut rest = data;
    for page in ram(m) {
        let n = rest.len().min(page.len());
        page[..n].copy_from_slice(&rest[..n]);
        page[n..].fill(0);
        rest = &rest[n..];
    }
}

fn ram_read(m: &SysMapping, len: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(len as usize);
    for page in ram(m) {
        let n = (len as usize - v.len()).min(page.len());
        v.extend_from_slice(&page[..n]);
        if v.len() == len as usize {
            break;
        }
    }
    v
}

fn map_vram(st: &mut GpuState, bytes: u64) -> GpuResult<Mapping> {
    st.mem.alloc_mapped(bytes.next_multiple_of(PAGE), PAGE)
}

/// What the doorbell closures get (the same as I10's).
type Bell<'d, V> = dyn FnMut(&mut V, &mut Journal, u64, u32, &Ring<'_>) -> GpuResult<()> + 'd;

/// Submits `words` fenced on the last record, rings, waits for the last record's payload and returns all records.
#[allow(clippy::too_many_arguments)]
fn run_records<V: Mmio + ?Sized, K: Clock + ?Sized>(
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    words: &[u32],
    rec: Mapping,
    n_rec: u64,
    last_payload: u32,
    maps: &[Mapping],
    sys: &[SysMapping],
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<Result<Vec<u8>, u32>> {
    let w = st.space.as_ref().map(|(_, w)| w.clone()).ok_or(GpuError::InvalidArgument("I11: lipseste canalul de lucru"))?;
    let last_va = rec.va + RECORD * (n_rec - 1);
    let last_pa = rec.pa + RECORD * (n_rec - 1);
    let bell = l.wq.submit_fenced(st.mem.writer_mut(), words, last_va, u64::from(last_payload))?;
    if bell.bar0_offset != DOORBELL || bell.value != w.doorbell_token {
        return Err(GpuError::InvalidArgument("I11: soneria nu e cea documentata sau jetonul difera"));
    }
    let root = st.mem.root();
    let mut all: Vec<Mapping> = alloc::vec![w.ring, l.push];
    all.extend_from_slice(maps);
    all.push(rec);
    let ring = Ring { root, ring_va: w.ring.va, entries: w.entries, userd_va: w.ring.va + w.userd_offset, maps: &all, sys };
    let wq = &mut l.wq;
    let GpuState { pending, zone, vram_words, .. } = &mut *st;
    kick_and_wait(vbar, clock, journal, permit, zone, pending, vram_words, root, (bell.bar0_offset, bell.value), &ring, doorbell, |rd, clock| {
        let mut last = 0;
        if poll_until(clock, WAIT_US, 100, || {
            last = rd.read_u32(last_pa).ok()?;
            (last == last_payload).then_some(())
        })
        .is_err()
        {
            return Ok(Err(last));
        }
        let mut b = Vec::with_capacity((RECORD * n_rec) as usize);
        for o in (0..RECORD * n_rec).step_by(4) {
            b.extend_from_slice(&rd.read_u32(rec.pa + o)?.to_le_bytes());
        }
        wq.update_get(rd)?;
        wq.retire(last_va, u64::from(last_payload))?;
        Ok(Ok(b))
    })
}

/// Step `i11-copiere` (`bytes`: [`I11_BOARD_BYTES`] on the board).
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    bytes: u64,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<()> {
    let words0 = st.vram_words;
    let r = (|| {
        let pages = bytes.div_ceil(PAGE);
        let src = st.mem.alloc_sys_mapped(&arena_pages(0, pages)?)?;
        let dst = st.mem.alloc_sys_mapped(&arena_pages(I11_BOARD_BYTES / PAGE, pages)?)?;
        let mut data = alloc::vec![0u8; bytes as usize];
        transfer::fill_pattern(&mut data, I11_SEED)?;
        ram_write(&src, &data);
        drop(data);
        ram_write(&dst, &alloc::vec![0xee; bytes as usize]);
        let vbuf = map_vram(st, bytes)?;
        let prog = map_vram(st, COPY_B128.len() as u64)?;
        let mut m = || map_vram(st, PAGE);
        let (cb0, cb1, q0, q1, rec) = (m()?, m()?, m()?, m()?, m()?);
        let w = st.mem.writer_mut();
        put_bytes(w, prog.pa, COPY_B128)?;
        put_bytes(w, cb0.pa, &transfer::copy_cbuf(src.va, vbuf.va, bytes)?)?;
        put_bytes(w, cb1.pa, &transfer::copy_cbuf(vbuf.va, dst.va, bytes)?)?;
        put_bytes(w, q0.pa, &transfer::copy_qmd(prog.va, cb0.va, bytes)?.to_bytes()?)?;
        put_bytes(w, q1.pa, &transfer::copy_qmd(prog.va, cb1.va, bytes)?.to_bytes()?)?;
        put_bytes(w, rec.pa, &[0; 48])?;
        let mut words = [0u32; ROUND_TRIP_WORDS];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_round_trip(&mut pb, q0.va, q1.va, rec.va, I11_FIRST)?;
        let n = pb.len();
        let maps = [vbuf, prog, cb0, cb1, q0, q1];
        let sys = [src.clone(), dst.clone()];
        let b = run_records(vbar, clock, journal, st, l, permit, &words[..n], rec, 3, I11_FIRST + 2, &maps, &sys, doorbell)?;
        let b = match b {
            Ok(b) => b,
            Err(last) => {
                lines.push(format!(
                    "{}: I11: inregistrarea 2 = {:#x} dupa {} ms (asteptat {:#x}): placa nu a terminat",
                    p, last, WAIT_US / 1000, I11_FIRST + 2
                ));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        let speed = transfer::analyze_round_trip(&b, bytes, I11_FIRST)?;
        if let Some((off, got, want)) = transfer::first_mismatch(&ram_read(&dst, bytes), I11_SEED) {
            lines.push(format!("{}: I11: intoarcerea in RAM difera la octetul {:#x}: {:#010x}, asteptat {:#010x}", p, off, got, want));
            return Err(GpuError::VerifyFailed { offset: off as u32, expected: want, actual: got });
        }
        if let Some((off, ..)) = transfer::first_mismatch(&ram_read(&src, bytes), I11_SEED) {
            lines.push(format!("{}: I11: sursa din RAM s-a schimbat la octetul {:#x}", p, off));
            return Err(GpuError::InvalidArgument("I11: sursa schimbata"));
        }
        let mut spots = Vec::new();
        {
            let GpuState { zone, .. } = &*st;
            let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
            let r = transfer::spot_offsets(bytes).into_iter().try_for_each(|off| {
                spots.push((off, vw.read_u32(vbuf.pa + off)?));
                Ok(())
            });
            let back = vw.restore();
            r.and(back)?;
        }
        for (off, got) in spots {
            let want = transfer::pattern_word(I11_SEED, off / 4);
            if got != want {
                lines.push(format!("{}: I11: VRAM la {:#x} = {:#010x}, asteptat {:#010x}", p, off, got, want));
                return Err(GpuError::VerifyFailed { offset: off as u32, expected: want, actual: got });
            }
        }
        lines.push(format!("{}: {}", p, speed.line()));
        Ok(())
    })();
    match r {
        Ok(()) => {
            lines.push(format!(
                "{}: pas I11 copiere: ok ({} octeti RAM -> VRAM -> RAM identici bit cu bit; 3 cuvinte VRAM recitite; {} cuvinte VRAM scrise si recitite)",
                p,
                bytes,
                st.vram_words - words0
            ));
            Ok(())
        }
        Err(e) => {
            lines.push(format!("{}: pas I11 copiere: ESEC: {} (oprit la prima eroare)", p, e));
            Err(e)
        }
    }
}

/// Step `i12-q8-matvec`.
#[allow(clippy::too_many_arguments)]
pub(super) fn matvec_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<()> {
    let (rows, cols) = (mv::TEST_ROWS, mv::TEST_COLS);
    let r = (|| {
        let (w, x) = mv::test_matrix(rows, cols, I12_SEED)?;
        let x_off = (w.len() as u64).next_multiple_of(16);
        let up_bytes = (x_off + 4 * cols as u64).next_multiple_of(16);
        let y_bytes = (4 * rows as u64).next_multiple_of(16);
        let in_pages = up_bytes.div_ceil(PAGE);
        let base = 2 * (I11_BOARD_BYTES / PAGE);
        let host_in = st.mem.alloc_sys_mapped(&arena_pages(base, in_pages)?)?;
        let host_y = st.mem.alloc_sys_mapped(&arena_pages(base + in_pages, y_bytes.div_ceil(PAGE))?)?;
        let mut data = w.clone();
        data.resize(x_off as usize, 0);
        data.extend(x.iter().flat_map(|v| v.to_le_bytes()));
        data.resize(up_bytes as usize, 0);
        ram_write(&host_in, &data);
        drop(data);
        ram_write(&host_y, &alloc::vec![0xee; y_bytes as usize]);
        let vin = map_vram(st, up_bytes)?;
        let vy = map_vram(st, y_bytes)?;
        let pc = map_vram(st, COPY_B128.len() as u64)?;
        let pm = map_vram(st, Q8_MATVEC.len() as u64)?; // 8 048 B: two pages
        let mut m = || map_vram(st, PAGE);
        let (cb0, cb1, cb2, q0, q1, q2, rec) = (m()?, m()?, m()?, m()?, m()?, m()?, m()?);
        let wr = st.mem.writer_mut();
        put_bytes(wr, pc.pa, COPY_B128)?;
        put_bytes(wr, pm.pa, Q8_MATVEC)?;
        put_bytes(wr, cb0.pa, &transfer::copy_cbuf(host_in.va, vin.va, up_bytes)?)?;
        put_bytes(wr, cb1.pa, &mv::q8_cbuf(vin.va, vin.va + x_off, vy.va, rows, cols)?)?;
        put_bytes(wr, cb2.pa, &transfer::copy_cbuf(vy.va, host_y.va, y_bytes)?)?;
        put_bytes(wr, q0.pa, &transfer::copy_qmd(pc.va, cb0.va, up_bytes)?.to_bytes()?)?;
        put_bytes(wr, q1.pa, &mv::q8_qmd(pm.va, cb1.va, rows)?)?;
        put_bytes(wr, q2.pa, &transfer::copy_qmd(pc.va, cb2.va, y_bytes)?.to_bytes()?)?;
        // y in VRAM starts as NaN: a row nobody wrote cannot pass.
        put_bytes(wr, vy.pa, &alloc::vec![0xff; y_bytes as usize])?;
        put_bytes(wr, rec.pa, &[0; 64])?;
        let mut words = alloc::vec![0u32; transfer::sequence_words(3)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[q0.va, q1.va, q2.va], rec.va, I12_FIRST)?;
        let n = pb.len();
        let maps = [vin, vy, pc, pm, cb0, cb1, cb2, q0, q1, q2];
        let sys = [host_in, host_y.clone()];
        let b = run_records(vbar, clock, journal, st, l, permit, &words[..n], rec, 4, I12_FIRST + 3, &maps, &sys, doorbell)?;
        let b = match b {
            Ok(b) => b,
            Err(last) => {
                lines.push(format!(
                    "{}: I12: inregistrarea 3 = {:#x} dupa {} ms (asteptat {:#x}): placa nu a terminat",
                    p, last, WAIT_US / 1000, I12_FIRST + 3
                ));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        let t = transfer::analyze_sequence(&b, 3, I12_FIRST)?;
        let y: Vec<f32> = ram_read(&host_y, 4 * rows as u64).chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let v = mv::judge(&w, rows, cols, &x, &y)?;
        lines.push(format!("{}: {}", p, v.line()));
        lines.push(format!(
            "{}: i12: durate (ceasul placii): copiere sus {} ns, inmultire {} ns ({} MB/s din W), copiere jos {} ns",
            p,
            t[0],
            t[1],
            transfer::CopySpeed::mb_per_s(w.len() as u64, t[1]),
            t[2]
        ));
        if !v.passes() {
            return Err(GpuError::InvalidArgument("I12: rezultatul nu trece (vezi randul de mai sus)"));
        }
        Ok(())
    })();
    match r {
        Ok(()) => {
            lines.push(format!("{}: pas I12 q8 matvec: ok ({}x{} Q8_0, y comparat cu procesorul)", p, rows, cols));
            Ok(())
        }
        Err(e) => {
            lines.push(format!("{}: pas I12 q8 matvec: ESEC: {} (oprit la prima eroare)", p, e));
            Err(e)
        }
    }
}

/// I13 on the board: one Kronos-sized layer (E 768, 12 heads), 4 positions.
pub(super) const I13_BOARD: (usize, usize) = (768, 12);
/// SIMULATION: the model moves every word of the ~7.6 MB of weights through its page walker at each doorbell, so the
/// QEMU test uses a small layer (E 128, 4 heads); the same code and the same launches.
#[cfg(feature = "test-gpu-sim")]
pub(super) const I13_SIM: (usize, usize) = (128, 4);
const I13_POSITIONS: usize = 4;
const I13_SEED: u32 = 0x13;
const I13_FIRST: u32 = 0x900;

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Step `i13-strat`: the layer's weights and a NaN scratch packed into one blob in the DMA arena, copied up with one
/// `copy_b128` launch (I11), then for each position: x written, the 16 launches of `kronos_layer::token_launches`
/// (one record after each), x read back through the window and judged against `gpu_compute::layer::step_cpu`.
#[allow(clippy::too_many_arguments)]
pub(super) fn layer_step<V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    st: &mut GpuState,
    l: &mut Launched,
    shape: (usize, usize),
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    doorbell: &mut Bell<'_, V>,
) -> GpuResult<()> {
    use gpu_compute::layer::{self as cl, Shape};
    use nvidia_ga104::kronos_layer::{self as kl, LayerVas, Prog, WeightVas, LAUNCHES_PER_TOKEN};
    let s = Shape { e: shape.0, heads: shape.1, eps: 1e-5 };
    let e = s.e;
    let r = (|| {
        let lw = kl::test_layer(e, I13_SEED)?;
        // The blob: the 12 weight parts then the scratch (NaN: anything not computed cannot pass), each 16-aligned.
        let parts: [Vec<u8>; 12] = [
            f32_bytes(&lw.ln1_g),
            f32_bytes(&lw.ln1_b),
            lw.wqkv.clone(),
            f32_bytes(&lw.bqkv),
            lw.wout.clone(),
            f32_bytes(&lw.bout),
            f32_bytes(&lw.ln2_g),
            f32_bytes(&lw.ln2_b),
            lw.wfc.clone(),
            f32_bytes(&lw.bfc),
            lw.wproj.clone(),
            f32_bytes(&lw.bproj),
        ];
        let mut blob = Vec::new();
        let mut offs = [0u64; 12];
        for (o, part) in offs.iter_mut().zip(&parts) {
            *o = blob.len() as u64;
            blob.extend_from_slice(part);
            blob.resize(blob.len().next_multiple_of(16), 0);
        }
        drop(parts);
        let scratch_off = blob.len() as u64;
        blob.resize((scratch_off + kl::scratch_bytes(e)) as usize, 0xff);
        let bytes = blob.len() as u64;
        // The arena pages of I11's source (I11 is done with them; the chain stops before I13 if I11 failed).
        if bytes > I11_BOARD_BYTES {
            return Err(GpuError::OutOfMemory { requested: bytes });
        }
        let host = st.mem.alloc_sys_mapped(&arena_pages(0, bytes.div_ceil(PAGE))?)?;
        ram_write(&host, &blob);
        drop(blob);
        let vblob = map_vram(st, bytes)?;
        let kc = map_vram(st, kl::cache_bytes(e))?;
        let vc = map_vram(st, kl::cache_bytes(e))?;
        let mut progs: Vec<(Prog, Mapping)> = Vec::new();
        for prog in [Prog::Copy, Prog::Q8Matvec, Prog::Vadd, Prog::LayerNorm, Prog::Attention, Prog::Gelu] {
            let m = map_vram(st, prog.code().len() as u64)?;
            put_bytes(st.mem.writer_mut(), m.pa, prog.code())?;
            progs.push((prog, m));
        }
        let prog_va = |want: Prog| progs.iter().find(|(q, _)| *q == want).map(|(_, m)| m.va);
        let launches = map_vram(st, 512 * LAUNCHES_PER_TOKEN as u64)?;
        let cb_up = map_vram(st, PAGE)?;
        let q_up = map_vram(st, PAGE)?;
        let rec = map_vram(st, PAGE)?;
        let copy_va = prog_va(Prog::Copy).ok_or(GpuError::InvalidArgument("I13: lipseste programul de copiere"))?;
        put_bytes(st.mem.writer_mut(), cb_up.pa, &transfer::copy_cbuf(host.va, vblob.va, bytes)?)?;
        put_bytes(st.mem.writer_mut(), q_up.pa, &transfer::copy_qmd(copy_va, cb_up.va, bytes)?.to_bytes()?)?;
        let mut maps: Vec<Mapping> = alloc::vec![vblob, kc, vc, launches, cb_up, q_up];
        maps.extend(progs.iter().map(|(_, m)| *m));
        // 1. The upload: one launch, two records.
        put_bytes(st.mem.writer_mut(), rec.pa, &[0; 32])?;
        let mut words = alloc::vec![0u32; transfer::sequence_words(1)];
        let mut pb = PushBuf::new(&mut words);
        transfer::encode_sequence(&mut pb, &[q_up.va], rec.va, I13_FIRST)?;
        let n = pb.len();
        let sys = [host];
        let b = run_records(vbar, clock, journal, st, l, permit, &words[..n], rec, 2, I13_FIRST + 1, &maps, &sys, doorbell)?;
        let up_ns = match b {
            Ok(b) => transfer::analyze_sequence(&b, 1, I13_FIRST)?[0],
            Err(last) => {
                lines.push(format!("{}: I13: urcarea greutatilor nu s-a terminat (inregistrarea 1 = {:#x})", p, last));
                return Err(GpuError::Timeout { waited_us: WAIT_US });
            }
        };
        lines.push(format!(
            "{}: i13: strat E {} / {} capete: {} octeti (greutati + spatiu de lucru) urcati cu o copiere in {} ns ({} MB/s)",
            p, e, s.heads, bytes, up_ns, transfer::CopySpeed::mb_per_s(bytes, up_ns)
        ));
        let at = |o: u64| vblob.va + o;
        let wv = WeightVas {
            ln1_g: at(offs[0]),
            ln1_b: at(offs[1]),
            wqkv: at(offs[2]),
            bqkv: at(offs[3]),
            wout: at(offs[4]),
            bout: at(offs[5]),
            ln2_g: at(offs[6]),
            ln2_b: at(offs[7]),
            wfc: at(offs[8]),
            bfc: at(offs[9]),
            wproj: at(offs[10]),
            bproj: at(offs[11]),
        };
        let vas = LayerVas::with_scratch(e, at(scratch_off), kc.va, vc.va, wv)?;
        let x_pa = vblob.pa + (vas.x - vblob.va);
        let (mut kcpu, mut vcpu) = (alloc::vec![0.0f32; cl::MAX_POS * e], alloc::vec![0.0f32; cl::MAX_POS * e]);
        // 2. The positions: 16 launches each.
        for pos in 0..I13_POSITIONS {
            let x = kl::test_input(e, I13_SEED, pos);
            let list = kl::token_launches(&s, &vas, pos)?;
            let w = st.mem.writer_mut();
            put_bytes(w, x_pa, &f32_bytes(&x))?;
            let mut qmds = Vec::with_capacity(list.len());
            for (k, la) in list.iter().enumerate() {
                let (cb_va, cb_pa) = (launches.va + 512 * k as u64, launches.pa + 512 * k as u64);
                let pv = prog_va(la.prog).ok_or(GpuError::InvalidArgument("I13: program necunoscut"))?;
                put_bytes(w, cb_pa, &la.cbuf)?;
                put_bytes(w, cb_pa + 256, &la.qmd(pv, cb_va)?)?;
                qmds.push(cb_va + 256);
            }
            let first = I13_FIRST + 0x20 * (pos as u32 + 1);
            put_bytes(w, rec.pa, &alloc::vec![0u8; 16 * (list.len() + 1)])?;
            let mut words = alloc::vec![0u32; transfer::sequence_words(qmds.len())];
            let mut pb = PushBuf::new(&mut words);
            transfer::encode_sequence(&mut pb, &qmds, rec.va, first)?;
            let n = pb.len();
            let last = first + list.len() as u32;
            let b = run_records(vbar, clock, journal, st, l, permit, &words[..n], rec, list.len() as u64 + 1, last, &maps, &[], doorbell)?;
            let b = match b {
                Ok(b) => b,
                Err(got) => {
                    lines.push(format!("{}: I13: pozitia {}: ultima inregistrare = {:#x} (asteptat {:#x}): placa nu a terminat", p, pos, got, last));
                    return Err(GpuError::Timeout { waited_us: WAIT_US });
                }
            };
            let t = transfer::analyze_sequence(&b, list.len(), first)?;
            let got = {
                let GpuState { zone, .. } = &*st;
                let mut vw = nvidia_ga104::vram_window::VramWindow::new(&mut *vbar, &mut *journal, permit, zone.vram_size, &zone.protected)?;
                let r = (0..e as u64).map(|i| vw.read_u32(x_pa + 4 * i).map(f32::from_bits)).collect::<GpuResult<Vec<f32>>>();
                let back = vw.restore();
                r.and_then(|v| back.map(|_| v))?
            };
            let mut want = x.clone();
            cl::step_cpu(&s, &lw.weights(), &mut want, &mut kcpu, &mut vcpu, pos)?;
            let v = kl::judge(&got, &want);
            lines.push(format!("{}: {}; {} lansari, {} ns in total (ceasul placii)", p, v.line(pos), t.len(), t.iter().sum::<u64>()));
            if !v.passes() {
                return Err(GpuError::InvalidArgument("I13: stratul nu trece (vezi randul de mai sus)"));
            }
        }
        Ok(())
    })();
    match r {
        Ok(()) => {
            lines.push(format!(
                "{}: pas I13 strat: ok (E {}, {} capete, {} pozitii x {} lansari, comparat cu procesorul)",
                p, e, s.heads, I13_POSITIONS, LAUNCHES_PER_TOKEN
            ));
            Ok(())
        }
        Err(err) => {
            lines.push(format!("{}: pas I13 strat: ESEC: {} (oprit la prima eroare)", p, err));
            Err(err)
        }
    }
}
