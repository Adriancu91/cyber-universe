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
