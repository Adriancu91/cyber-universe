//! P6-56 step 2b/3 (command C-72, Adrian's "da i7-i10"): the golden image (`i8e-golden`) and the work channel
//! (`i9-canal-lucru`) of the window chain, in the OS.
//!
//! Both steps are conversations with GSP-RM over the queues prepared at I7 (`gsp::golden::QueueTransportBell`: the GSP
//! doorbell after every message, as C-71 asks) plus VRAM the driver allocates: page tables, the GR context buffers,
//! the ring, USERD. The VRAM writes go through the BAR0 VRAM window (`vram_window::VramWindow`, Cyber-GPU's write
//! policy: the window register, the PRAMIN range, never a protected range). They are collected while the library
//! builds its plan and written **before the next RM message** (and at the end of the step), so GSP-RM never hears of a
//! table before it is in VRAM.
//!
//! The VRAM zone the driver may use (DEDUS, conservative): above the whole BAR1 aperture (256 MiB, everything the
//! UEFI screen could show; the screen's own range from `vram_window::screen_vram_range` must be inside it) and below
//! GSP-RM's regions (`FbLayout::free_after_i7`: the I6/I7 layout, checked against WPR2 read back after I7, P6-58).
//! Everything else is protected.
//!
//! I10 (`add64`, the launch cost) is in `gpu_i10.rs`: it adds the VRAM reads, the TLB invalidate and the work-channel
//! doorbell on top of the memory kept here.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU against `gpu_sim::fake_rm::FakeRm`
//! behind a small queue peer (test builds). On the board it is a first try.
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml): `gsp::golden`, `gsp::user_space`, `mmu`,
//! `vram_window`; notes C-71 (`integration/cyber-universe/`). Nothing copied from other projects.

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;
use gpu_core::diag::{Event, Journal};
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{checked_read32, Clock, Mmio};
use nvidia_ga104::fb_layout::FbLayout;
use nvidia_ga104::gsp::falcon::{self, GuardedBar0};
use nvidia_ga104::gsp::golden::{self, GoldenImage, QueueTransportBell, RmTransport, RM_REPLY_US};
use nvidia_ga104::gsp::msgq::SharedMem;
use nvidia_ga104::gsp::user_space::{self, UserSpace, WorkChannel};
use nvidia_ga104::gsp::work_queue::ChannelMemory;
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::mmu::{GpuMemory, VramWrite};
use nvidia_ga104::regs::is_priv_error;
use nvidia_ga104::vram_window::VramWindow;

use super::gpu_i7::I7Prep;

/// BAR1 of the workstation board (FAPT, K0 inventory and os-195 `hw`: 256 MiB at 0xe0000000, the screen at its start).
pub(super) const BAR1_LEN: u64 = 256 << 20;

/// 32-bit VRAM words waiting to be written (address, value), shared by the memory manager and the transport.
pub(super) type Pending = Rc<RefCell<Vec<(u64, u32)>>>;

/// The memory manager's sink: it only collects the words; [`flush`] writes them through the VRAM window.
pub(super) struct PendingVram(Pending);

impl VramWrite for PendingVram {
    fn write_u64(&mut self, pa: u64, value: u64) -> GpuResult<()> {
        if !pa.is_multiple_of(8) {
            return Err(GpuError::InvalidArgument("VRAM: scriere de 8 octeti nealiniata"));
        }
        let mut w = self.0.borrow_mut();
        w.push((pa, value as u32));
        w.push((pa + 4, (value >> 32) as u32));
        Ok(())
    }
}

impl ChannelMemory for PendingVram {
    fn write_u32(&mut self, pa: u64, v: u32) -> GpuResult<()> {
        self.0.borrow_mut().push((pa, v));
        Ok(())
    }
    /// The collector never reads: I10 reads VRAM back through the window itself (`gpu_i10.rs`).
    fn read_u32(&mut self, _pa: u64) -> GpuResult<u32> {
        Err(GpuError::NotPermitted("citirea VRAM doar prin fereastra VRAM (I10)"))
    }
}

/// The VRAM the driver may use and the ranges it must never write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VramZone {
    pub start: u64,
    pub end: u64,
    pub vram_size: u64,
    pub protected: Vec<(u64, u64)>,
}

/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` (Cyber-GPU's `tu102_dev_fb`; the same two registers the tour and `i6-wpr2-citit`
/// read).
const WPR2_LO: u64 = 0x1f_a824;
const WPR2_HI: u64 = 0x1f_a828;

/// P6-58: WPR2 read back after I7 with reads only (no permit, each read journaled), as `(start, end)`. HI = 0 (WPR2
/// down: Booter did not run, or the board was reset) stops golden before any VRAM write.
pub(super) fn read_wpr2_after_i7<M: Mmio + ?Sized>(mmio: &mut M, journal: &mut Journal) -> GpuResult<(u64, u64)> {
    let mut read = |off: u64| -> GpuResult<u32> {
        let v = checked_read32(mmio, off)?;
        journal.push(Event::MmioRead { bar: 0, offset: off, value: v });
        if is_priv_error(v) {
            return Err(GpuError::HardwareFault { code: v });
        }
        Ok(v)
    };
    let hi = read(WPR2_HI)?;
    if hi == 0 {
        return Err(GpuError::Blocked("WPR2 coborat dupa I7: zona VRAM necunoscuta, nimic scris"));
    }
    let lo = read(WPR2_LO)?;
    let w = falcon::decode_wpr2(lo, hi);
    Ok((w.start, w.end))
}

/// The zone from the I6/I7 layout, WPR2 as read after I7 and the screen. `wpr2` must be where `GspFwWprMeta` asked
/// (`FbLayout::free_after_i7`, P6-58), `screen` = the screen's VRAM range (`screen_vram_range`); otherwise (or
/// framebuffer not inside BAR1) the step stops before any write.
pub(super) fn zone(
    fb: &FbLayout,
    wpr2: (u64, u64),
    vram_size: u64,
    screen: Option<(u64, u64)>,
    bar1_len: u64,
) -> GpuResult<VramZone> {
    let free = fb.free_after_i7(wpr2)?;
    let Some((_, screen_end)) = screen else {
        return Err(GpuError::NotPermitted("ecranul nu e in BAR1: zona VRAM a ecranului necunoscuta, nimic scris"));
    };
    let start = screen_end.max(bar1_len).next_multiple_of(2 << 20);
    let end = free.end & !0xFFF;
    if end <= start || end > vram_size {
        return Err(GpuError::InvalidArgument("zona VRAM pentru golden: goala sau in afara memoriei"));
    }
    Ok(VramZone { start, end, vram_size, protected: alloc::vec![(0, start), (end, vram_size)] })
}

/// What golden + I9 keep between steps.
pub(super) struct GpuState {
    pub mem: GpuMemory<PendingVram>,
    pub(super) pending: Pending,
    pub zone: VramZone,
    pub golden: Option<GoldenImage>,
    pub space: Option<(UserSpace, WorkChannel)>,
    /// RM messages sent (one GSP doorbell each) and VRAM words written + read back.
    pub rm_calls: u32,
    pub vram_words: u64,
}

impl GpuState {
    pub(super) fn new(zone: VramZone) -> GpuResult<Self> {
        let pending: Pending = Rc::new(RefCell::new(Vec::new()));
        // The root table is allocated and cleared here (collected, written before the first RM message).
        let mem = GpuMemory::new(zone.start, zone.end, PendingVram(pending.clone()))?;
        Ok(GpuState { mem, pending, zone, golden: None, space: None, rm_calls: 0, vram_words: 0 })
    }
}

/// Writes every collected word through the VRAM window (each read back), then puts the window register back.
pub(super) fn flush<V: Mmio + ?Sized>(
    vbar: &mut V,
    journal: &mut Journal,
    permit: Option<&HardwareWritePermit>,
    zone: &VramZone,
    pending: &Pending,
    words: &mut u64,
) -> GpuResult<()> {
    let w = core::mem::take(&mut *pending.borrow_mut());
    if w.is_empty() {
        return Ok(());
    }
    let mut vw = VramWindow::new(vbar, journal, permit, zone.vram_size, &zone.protected)?;
    let r = w.iter().try_for_each(|&(pa, v)| vw.write_u32(pa, v));
    *words += vw.words;
    let back = vw.restore();
    r.and(back)
}

/// One RM conversation: flush the VRAM words, send, ring the GSP doorbell (GSP window only), wait for the reply.
struct OsRm<'a, B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized, P: FnMut(&mut dyn SharedMem)> {
    bar: &'a mut B,
    vbar: &'a mut V,
    journal: &'a mut Journal,
    permit: Option<&'a HardwareWritePermit>,
    prep: &'a mut I7Prep,
    clock: &'a mut K,
    poll: P,
    zone: &'a VramZone,
    pending: &'a Pending,
    calls: &'a mut u32,
    words: &'a mut u64,
}

impl<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized, P: FnMut(&mut dyn SharedMem)> RmTransport
    for OsRm<'_, B, V, K, P>
{
    fn call(&mut self, function: u32, payload: &[u8]) -> GpuResult<Vec<u8>> {
        flush(&mut *self.vbar, &mut *self.journal, self.permit, self.zone, self.pending, &mut *self.words)?;
        let mut b = GuardedBar0::new(&mut *self.bar, &mut *self.journal, self.permit)?;
        let poll = &mut self.poll;
        let prep = &mut *self.prep;
        let mut t = QueueTransportBell {
            q: &mut prep.queues,
            mem: &mut prep.shared,
            clock: &mut *self.clock,
            timeout_us: RM_REPLY_US,
            poll: |m: &mut dyn SharedMem| poll(m),
            events: Vec::new(),
            bell: || falcon::ring_doorbell(&mut b),
            rung: 0,
        };
        let r = t.call(function, payload);
        *self.calls += t.rung;
        r
    }
}

/// Step `i8e-golden`. `bar` = the GSP-only fence (the doorbell), `vbar` = the VRAM-window fence.
#[allow(clippy::too_many_arguments)]
pub(super) fn golden_step<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    bar: &mut B,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    st: &mut GpuState,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<()> {
    let (calls0, words0) = (st.rm_calls, st.vram_words);
    let GpuState { mem, pending, zone, rm_calls, vram_words, .. } = st;
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        golden::init_golden_image(&mut t, mem)
    }
    .and_then(|g| flush(vbar, journal, permit, zone, pending, vram_words).map(|_| g));
    match r {
        Ok(g) => {
            lines.push(format!(
                "{}: pas golden: ok (VRAM {:#x}-{:#x}; {} buffere GR; {} mesaje RM, fiecare cu soneria GSP; {} cuvinte VRAM scrise si recitite)",
                p,
                st.zone.start,
                st.zone.end,
                g.gr_bufs.len(),
                st.rm_calls - calls0,
                st.vram_words - words0
            ));
            st.golden = Some(g);
            Ok(())
        }
        Err(e) => {
            lines.push(format!("{}: pas golden: ESEC: {} (dupa {} mesaje RM; oprit la prima eroare)", p, e, st.rm_calls - calls0));
            Err(e)
        }
    }
}

/// Step `i9-canal-lucru`: the user client on our page directory, the compute group, the work channel (USERD cleared),
/// and the GR facts (SM 8.6).
#[allow(clippy::too_many_arguments)]
pub(super) fn channel_step<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    bar: &mut B,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    st: &mut GpuState,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<()> {
    let calls0 = st.rm_calls;
    let GpuState { mem, pending, zone, rm_calls, vram_words, golden: gi, .. } = st;
    let Some(gi) = gi.as_ref() else { return Err(GpuError::InvalidArgument("I9: lipseste imaginea golden")) };
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        (|| {
            let u = user_space::open_user_space(&mut t, gi.next_handle(), mem.root())?;
            let cg = user_space::open_compute_group(&mut t, &u)?;
            let w = user_space::open_work_channel_cleared(&mut t, &mut *mem, &u, &cg, gi)?;
            user_space::query_gr_info(&mut t, &u)?.check_sm86()?;
            Ok::<_, GpuError>((u, w))
        })()
    }
    .and_then(|x| flush(vbar, journal, permit, zone, pending, vram_words).map(|_| x));
    match r {
        Ok(x) => {
            lines.push(format!("{}: pas I9 canal de lucru: ok (SM 8.6; {} mesaje RM)", p, st.rm_calls - calls0));
            st.space = Some(x);
            Ok(())
        }
        Err(e) => {
            lines.push(format!("{}: pas I9 canal de lucru: ESEC: {} (oprit la prima eroare)", p, e));
            Err(e)
        }
    }
}
