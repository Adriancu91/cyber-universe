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
use nvidia_ga104::gsp::golden::{self, GoldenImage, QueueTransportBellKeep, RmTransport, RM_REPLY_US};
use nvidia_ga104::gsp::msgq::SharedMem;
use nvidia_ga104::gsp::user_space::{self, C133Report, C135Report, I9Options, I9Report, UserSpace, WorkChannel};
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
    /// P6-161 (C-157, cyber-gpu `9359f21`): the copy engine object on the work channel (`AMPERE_DMA_COPY_B`), allocated
    /// only when the cycle asks for the RAM ↔ VRAM probe (`copy_object_step`).
    pub copy_obj: Option<u32>,
}

impl GpuState {
    pub(super) fn new(zone: VramZone) -> GpuResult<Self> {
        let pending: Pending = Rc::new(RefCell::new(Vec::new()));
        // The root table is allocated and cleared here (collected, written before the first RM message).
        let mem = GpuMemory::new(zone.start, zone.end, PendingVram(pending.clone()))?;
        Ok(GpuState { mem, pending, zone, golden: None, space: None, rm_calls: 0, vram_words: 0, copy_obj: None })
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
        // P6-85 (cyber-gpu note os-295_i9-motiv-in-jurnal.md, item 2): `QueueTransportBellKeep` keeps the GSP-RM
        // messages skipped while waiting for the reply, so they can be journaled after the call (the journal is no
        // longer borrowed by `GuardedBar0` then). Same doorbell, same queue protocol, no new write.
        let (r, rung, skipped) = {
            let mut b = GuardedBar0::new(&mut *self.bar, &mut *self.journal, self.permit)?;
            let poll = &mut self.poll;
            let prep = &mut *self.prep;
            let mut t = QueueTransportBellKeep {
                q: &mut prep.queues,
                mem: &mut prep.shared,
                clock: &mut *self.clock,
                timeout_us: RM_REPLY_US,
                poll: |m: &mut dyn SharedMem| poll(m),
                events: Vec::new(),
                bell: || falcon::ring_doorbell(&mut b),
                rung: 0,
                skipped: Vec::new(),
            };
            let r = t.call(function, payload);
            (r, t.rung, t.skipped)
        };
        *self.calls += rung;
        golden::journal_skipped(&mut *self.journal, "apel RM", &skipped);
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
    i9: I9Choice,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<()> {
    let calls0 = st.rm_calls;
    let GpuState { mem, pending, zone, rm_calls, vram_words, golden: gi, .. } = st;
    let Some(gi) = gi.as_ref() else { return Err(GpuError::InvalidArgument("I9: lipseste imaginea golden")) };
    let mut rep = I9Report::default();
    let mut rep133 = C133Report::default();
    let mut rep135 = C135Report::default();
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        (|| {
            let u = user_space::open_user_space(&mut t, gi.next_handle(), mem.root())?;
            let w = match i9 {
                I9Choice::Opts(o) => {
                    let cg = user_space::open_compute_group(&mut t, &u)?;
                    // P6-91 (C-125): `I9Options::OS_302` sends exactly what os-302 sent (cyber-gpu test
                    // `c125_off_sends_exactly_what_os302_sent`); `C125` = V2 (privileged ctx buffers) + V3 (RM's buffer list).
                    user_space::open_work_channel_with(&mut t, &mut *mem, &u, &cg, gi, o, &mut rep)?
                }
                // P6-92 (C-133): one PROMOTE_CTX (PA + VA) before the compute object (try A, then B in a new group);
                // it makes its own channel group, replacing open_compute_group + open_work_channel_with.
                I9Choice::C133 => user_space::open_work_channel_c133(&mut t, &mut *mem, &u, gi, &mut rep133)?.1,
                // P6-93 (C-135): straight to form B; try N = channel without error notifier, then F = fixed ChID +
                // computed token (only if GSP-RM refuses N's token). Makes its own channel group, like C133.
                I9Choice::C135 => user_space::open_work_channel_c135(&mut t, &mut *mem, &u, gi, &mut rep135)?.1,
            };
            user_space::query_gr_info(&mut t, &u)?.check_sm86()?;
            Ok::<_, GpuError>((u, w))
        })()
    }
    .and_then(|x| flush(vbar, journal, permit, zone, pending, vram_words).map(|_| x));
    // The journal is free again (`t` ended): the report reaches the stick also when I9 fails (the os-302 case).
    match i9 {
        I9Choice::Opts(o) if o == I9Options::OS_302 => {}
        I9Choice::Opts(_) => {
            rep.journal(journal);
            lines.push(format!("{}: {}", p, i9_report_line(&rep)));
        }
        I9Choice::C133 => {
            rep133.journal(journal);
            lines.push(format!("{}: {}", p, c133_report_line(&rep133)));
        }
        I9Choice::C135 => {
            rep135.journal(journal);
            lines.push(format!("{}: {}", p, c135_report_line(&rep135)));
        }
    }
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

/// P6-98 (C-147 stage 0 (c), C-148 point 3): at the end of the chain — after `i14-simbol` and `kronos --gpu`, with their
/// rows already on the stick — the board's current performance state is READ once on the work channel's client and
/// subdevice (`PERF_GET_CURRENT_PSTATE`, nothing set), one `ks …` row. A refusal by GSP-RM is a row (`8000xxxx`), not a
/// failure of the chain; only a transport error or a reply for another command is an error. The levels and the
/// temperature have no documented way in 570.144 (cyber-gpu `docs/C-147_ETAPA0.md` §2): a row says so.
#[allow(clippy::too_many_arguments)]
pub(super) fn pstate_step<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
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
    use nvidia_ga104::gsp::perf::{self as gp, Pstate, When};
    let Some((u, _)) = st.space.as_ref() else { return Err(GpuError::InvalidArgument("ks: lipseste canalul de lucru")) };
    let sent = gp::encode_get_pstate(u.client, u.subdevice);
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let reply = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        t.call(nvidia_ga104::gsp::rpc::function::GSP_RM_CONTROL, &sent)
    }?;
    let ps = gp::read_pstate(&sent, &reply)?;
    gp::journal_pstate(journal, When::End, ps);
    journal.push(Event::Note(gp::rows::NO_LEVELS));
    lines.push(format!(
        "{}: ks: starea de performanta a placii la final: {} (doar citire; nivelurile si temperatura nu au cale documentata)",
        p,
        match ps {
            Pstate::P(n) => format!("P{}", n),
            Pstate::Odd(m) => format!("masca neobisnuita {:#x}", m),
            Pstate::Refused(st) => format!("refuzata de GSP-RM (NV_STATUS {:#x})", st),
        }
    ));
    Ok(())
}

/// P6-161 (C-157, Adrian's point of 23 Nov a; cyber-gpu note `C-157_RAM_VRAM_proba-motor-copiere.md`, `9359f21`): ONLY
/// when the cycle asks for the RAM ↔ VRAM probe (`test-placa … --ram-vram`), once, right after the work channel is open:
/// one RM message allocating the copy engine object (`AMPERE_DMA_COPY_B`, no parameters) on the work channel, with its
/// first free handle (the same class the golden image allocates on its own channel). The channel's `next_handle` moves
/// past it. Nothing is written to the board's registers. A refusal is an `Err` (the probe then does not run).
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_object_step<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
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
    let Some((u, w)) = st.space.clone() else { return Err(GpuError::InvalidArgument("kc: lipseste canalul de lucru")) };
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        user_space::alloc_copy_object(&mut t, &u, &w)
    };
    let (h, next) = r?;
    if let Some((_, w)) = st.space.as_mut() {
        w.next_handle = next;
    }
    st.copy_obj = Some(h);
    lines.push(format!("{}: motorul de copiere (AMPERE_DMA_COPY_B) alocat pe canalul de lucru: obiectul {:#x} (un mesaj RM)", p, h));
    Ok(())
}

fn pstate_text(ps: nvidia_ga104::gsp::perf::Pstate) -> String {
    use nvidia_ga104::gsp::perf::Pstate;
    match ps {
        Pstate::P(n) => format!("P{}", n),
        Pstate::Odd(m) => format!("masca neobisnuita {:#x}", m),
        Pstate::Refused(st) => format!("refuzata de GSP-RM (NV_STATUS {:#x})", st),
    }
}

/// P6-106 (C-148 completion 1, C-147 point 4): ONLY in the cycle, once, right after the work channel is open: the board
/// is asked for its maximum speed (`gp::boost_start`, NVIDIA's form, 600 s). The `ks` rows go to the journal right after
/// the GSP-RM calls they come from. A refusal is a row; only a transport error is `Err`.
pub(super) fn boost_begin<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
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
) -> GpuResult<nvidia_ga104::gsp::perf::BoostStart> {
    use nvidia_ga104::gsp::perf::{self as gp, BoostPlan};
    let Some((u, _)) = st.space.as_ref() else { return Err(GpuError::InvalidArgument("ks: lipseste canalul de lucru")) };
    let (client, sub) = (u.client, u.subdevice);
    let plan = BoostPlan { enabled: true, ..BoostPlan::default() };
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let mut rows = Journal::new(16);
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        let mut rm = |sent: &[u8]| t.call(nvidia_ga104::gsp::rpc::function::GSP_RM_CONTROL, sent);
        gp::boost_start(&plan, client, sub, &mut rm, &mut rows)
    };
    for e in rows.events() {
        journal.push(*e);
    }
    let start = r?;
    lines.push(format!(
        "{}: ks: viteza maxima ceruta (doar in ciclu, {} s): inainte {}, cererea {}, dupa {}; verdict {} ({:?})",
        p,
        plan.duration_s,
        pstate_text(start.before),
        match start.answer {
            gp::BoostAnswer::Off => String::from("netrimisa"),
            gp::BoostAnswer::Accepted => String::from("acceptata"),
            gp::BoostAnswer::Refused(v) => format!("refuzata (NV_STATUS {:#x})", v),
        },
        start.after.map_or_else(|| String::from("-"), pstate_text),
        start.verdict() as u32,
        start.verdict()
    ));
    Ok(start)
}

/// P6-106: the end of the cycle's test, in place of `pstate_step`: the boost cleared when it holds, then `ks la final`.
pub(super) fn boost_end<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    bar: &mut B,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    st: &mut GpuState,
    start: &nvidia_ga104::gsp::perf::BoostStart,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<()> {
    use nvidia_ga104::gsp::perf::{self as gp, BoostPlan};
    let Some((u, _)) = st.space.as_ref() else { return Err(GpuError::InvalidArgument("ks: lipseste canalul de lucru")) };
    let (client, sub) = (u.client, u.subdevice);
    let plan = BoostPlan { enabled: true, ..BoostPlan::default() };
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let mut rows = Journal::new(16);
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        let mut rm = |sent: &[u8]| t.call(nvidia_ga104::gsp::rpc::function::GSP_RM_CONTROL, sent);
        gp::boost_finish(start, &plan, client, sub, &mut rm, &mut rows)
    };
    for e in rows.events() {
        journal.push(*e);
    }
    let end = r?;
    journal.push(Event::Note(gp::rows::NO_LEVELS));
    lines.push(format!(
        "{}: ks: inainte de stergere {}, stergerea {}, starea de performanta la final: {}",
        p,
        end.before_clear.map_or_else(|| String::from("-"), pstate_text),
        end.clear.map_or_else(|| String::from("nu era nevoie"), |c| if c == 0 { String::from("acceptata") } else { format!("refuzata ({:#x})", c) }),
        pstate_text(end.end)
    ));
    Ok(())
}

/// P6-105 (C-152 point 5, C-151 point 1): the board's dashboard (RM User Shared Data), ONLY when armed
/// (`gpu_tur::RUSD_ARMED`), at the end of the chain — after I14 and the `ks` rows, with everything before already on the
/// stick. Step 1: the page given to GSP-RM and the polling asked (`kd::rusd_start`). A refusal is a `kd` row (8000xxxx),
/// after which nothing else is sent; only a transport error is `Err`. The caller then waits (3 x the interval) and calls
/// [`dashboard_finish`].
#[allow(clippy::too_many_arguments)]
pub(super) fn dashboard_begin<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
    bar: &mut B,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    st: &mut GpuState,
    plan: &nvidia_ga104::gsp::rusd::RusdPlan,
    phys: u64,
    note: &'static str,
    permit: Option<&HardwareWritePermit>,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<nvidia_ga104::gsp::rusd::RusdStart> {
    use nvidia_ga104::gsp::rusd as kd;
    let Some((u, _)) = st.space.as_ref() else { return Err(GpuError::InvalidArgument("kd: lipseste canalul de lucru")) };
    let (client, sub) = (u.client, u.subdevice);
    // An important row: everything held so far (I14, kronos, ks) goes to the stick before the first new command.
    journal.push(Event::Note(note));
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let mut rows = Journal::new(16);
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        let mut rm = |sent: &[u8]| t.call(nvidia_ga104::gsp::rpc::function::GSP_RM_CONTROL, sent);
        kd::rusd_start(plan, phys, client, sub, &mut rm, &mut rows)
    };
    for e in rows.events() {
        journal.push(*e);
    }
    r
}

/// The note written right before the dashboard's first command (the rows before it are flushed to the stick).
pub(super) const DASHBOARD_BEGIN: &str = "kd tabloul de bord (C-151 punctul 1): rezultatele lantului sunt pe stick; pornesc zona comuna";

/// P6-105 step 2: the page read (only when GSP-RM accepted all three commands), then the polling and the page stopped.
/// `Ok((dashboard, stop))`; `stop.page_free(&start)` false = the page stays reserved until the next power-on.
#[allow(clippy::too_many_arguments)]
pub(super) fn dashboard_finish<B: Mmio + ?Sized, V: Mmio + ?Sized, K: Clock + ?Sized>(
    bar: &mut B,
    vbar: &mut V,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    st: &mut GpuState,
    start: &nvidia_ga104::gsp::rusd::RusdStart,
    mem: &mut dyn nvidia_ga104::gsp::rusd::RusdMem,
    permit: Option<&HardwareWritePermit>,
    poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<(Option<nvidia_ga104::gsp::rusd::Dashboard>, nvidia_ga104::gsp::rusd::RusdStop)> {
    use nvidia_ga104::gsp::rusd as kd;
    let Some((u, _)) = st.space.as_ref() else { return Err(GpuError::InvalidArgument("kd: lipseste canalul de lucru")) };
    let (client, sub) = (u.client, u.subdevice);
    let dash = start.polling().then(|| kd::read_dashboard(mem, journal));
    let GpuState { pending, zone, rm_calls, vram_words, .. } = st;
    let mut rows = Journal::new(16);
    let r = {
        let mut t = OsRm { bar, vbar: &mut *vbar, journal: &mut *journal, permit, prep, clock, poll, zone, pending, calls: rm_calls, words: vram_words };
        let mut rm = |sent: &[u8]| t.call(nvidia_ga104::gsp::rpc::function::GSP_RM_CONTROL, sent);
        // P6-115 (cyber-gpu `fb22ffb`, after os-323): the stop without `INIT(0)` — the board refused it (0x8000001f + an
        // ASSERT) and NVIDIA's own driver never sends it. Polling off only; the page stays reserved until FLR / power-on.
        kd::rusd_stop_poll_only(start, client, sub, &mut rm, &mut rows)
    };
    for e in rows.events() {
        journal.push(*e);
    }
    Ok((dash, r?))
}

fn kd_answer(v: Option<u32>) -> String {
    match v {
        None => String::from("nu s-a trimis"),
        Some(0) => String::from("da"),
        Some(x) => format!("refuz {:#x}", x),
    }
}

/// P6-105: the dashboard's screen / report lines (also kept for `gpu stare`).
pub(super) fn dashboard_lines(
    p: &str,
    start: &nvidia_ga104::gsp::rusd::RusdStart,
    dash: Option<&nvidia_ga104::gsp::rusd::Dashboard>,
    stop: &nvidia_ga104::gsp::rusd::RusdStop,
) -> Vec<String> {
    let mut l = Vec::new();
    l.push(format!(
        "{}: kd tabloul de bord, raspunsurile placii: pornirea zonei = {}, intervalul = {}, cererea datelor = {}",
        p,
        kd_answer(start.init),
        kd_answer(start.interval),
        kd_answer(start.mask)
    ));
    match dash {
        Some(d) => {
            let opt = |v: Option<u32>, unit: &str| v.map_or_else(|| String::from("necitit"), |x| format!("{} {}", x, unit));
            l.push(format!(
                "{}: kd temperatura placii: {}; ceasul nucleului {}, al memoriei {}; stare {}",
                p,
                d.temp_c().map_or_else(|| String::from("necitita"), |c| format!("{} grade C", c)),
                opt(d.clk_graphics_mhz, "MHz"),
                opt(d.clk_memory_mhz, "MHz"),
                d.pstate().map_or_else(|| String::from("necitita"), pstate_text)
            ));
            l.push(format!(
                "{}: kd incarcare nucleu {} / memorie {} (valori brute); motive {}{}; puterea medie {}, limita {}",
                p,
                d.load_gpu.map_or_else(|| String::from("-"), |x| format!("{}", x)),
                d.load_mem.map_or_else(|| String::from("-"), |x| format!("{}", x)),
                d.reasons.map_or_else(|| String::from("necitite"), |x| format!("{:#x}", x)),
                if d.thermal_slowdown() { " (INCETINIRE TERMICA)" } else { "" },
                opt(d.avg_power_mw, "mW"),
                opt(d.power_limit_mw, "mW")
            ));
            l.push(format!("{}: kd sectiuni citite {:#x} (0x7f = toate), ocupate {:#x}", p, d.sections, d.busy));
        }
        None => l.push(format!("{}: kd tabloul nu s-a citit (placa nu a acceptat toate comenzile)", p)),
    }
    // P6-115: the zone is left on on purpose (no `INIT(0)`, as NVIDIA's driver); said as such, not as "not sent".
    let zone = if start.page_given() && stop.deinit.is_none() {
        String::from("nu se trimite (zona lasata pornita, ca driverul NVIDIA, fara INIT(0))")
    } else {
        kd_answer(stop.deinit)
    };
    l.push(format!(
        "{}: kd oprirea citirilor = {}, oprirea zonei = {}{}",
        p,
        kd_answer(stop.mask),
        zone,
        if stop.page_free(start) { "" } else { "; pagina ramane rezervata pana la FLR sau repornire" }
    ));
    l
}

/// P6-142 (C-157 point 5, cyber-gpu `1922eb2`): the dashboard kept on from before the max-speed request until the end
/// of the chain (`test-placa … --tablou-viteza`). Every read goes through the thermal watch (the proposed limits:
/// warn 80 °C, stop 85 °C, start again at 70 °C or below, silent dashboard > 10 s = stop).
pub(super) struct EarlyDashboard<M> {
    pub start: nvidia_ga104::gsp::rusd::RusdStart,
    pub plan: nvidia_ga104::gsp::rusd::RusdPlan,
    pub page: M,
    pub phys: u64,
    pub watch: nvidia_ga104::gsp::rusd_watch::Watch,
    /// The clock (µs) when the dashboard was started: the watch's times are ms from here.
    pub t0_us: u64,
    /// The "before" reading (for `compare_boost`).
    pub before: Option<nvidia_ga104::gsp::rusd::Dashboard>,
    /// The verdict of the last reading.
    pub last: Option<nvidia_ga104::gsp::rusd_watch::Verdict>,
}

/// The note written right before the early dashboard's first command.
pub(super) const DASHBOARD_EARLY: &str =
    "kd tabloul de bord pornit devreme (C-157 punctul 5): inainte de cererea de viteza maxima, tinut pornit pana la sfarsitul lantului";

fn verdict_text(v: nvidia_ga104::gsp::rusd_watch::Verdict) -> String {
    use nvidia_ga104::gsp::rusd_watch::{StopReason, Verdict};
    match v {
        Verdict::Continue => String::from("continua"),
        Verdict::Warn => String::from("AVERTIZARE (80 grade C sau peste)"),
        Verdict::Stop(StopReason::Hot(t)) => format!("OPRIRE: {} grade C (pragul 85)", t),
        Verdict::Stop(StopReason::Slowdown(b)) => format!("OPRIRE: incetinire termica / de urgenta (motive {:#x})", b),
        Verdict::Stop(StopReason::Silent(ms)) => format!("OPRIRE: tabloul neschimbat de {} ms", ms),
        Verdict::Stop(StopReason::NoTemperature) => String::from("OPRIRE: temperatura necitita"),
    }
}

impl<M: nvidia_ga104::gsp::rusd::RusdMem> EarlyDashboard<M> {
    /// The page after `rusd_start`: the watch with the proposed limits, read every 5 s.
    pub fn new(start: nvidia_ga104::gsp::rusd::RusdStart, plan: nvidia_ga104::gsp::rusd::RusdPlan, page: M, phys: u64, t0_us: u64) -> Option<Self> {
        use nvidia_ga104::gsp::rusd_watch::{ThermalLimits, Watch};
        let watch = Watch::new(ThermalLimits::PROPOSED, Watch::EVERY_MS_DEFAULT).ok()?;
        Some(EarlyDashboard { start, plan, page, phys, watch, t0_us, before: None, last: None })
    }

    /// One reading (only when GSP-RM accepted the three commands): the dashboard, its temperature mark and the
    /// thermal watch; one screen line. `None` = not polling.
    pub fn read<K: Clock + ?Sized>(
        &mut self,
        p: &str,
        what: &str,
        clock: &mut K,
        journal: &mut Journal,
        lines: &mut Vec<String>,
    ) -> Option<(nvidia_ga104::gsp::rusd::Dashboard, nvidia_ga104::gsp::rusd_watch::Verdict)> {
        use nvidia_ga104::gsp::{rusd as kd, rusd_watch as kw};
        if !self.start.polling() {
            return None;
        }
        let d = kd::read_dashboard(&mut self.page, journal);
        let stamp = kw::temp_stamp(&mut self.page);
        let t_ms = clock.now_us().saturating_sub(self.t0_us) / 1000;
        let v = self.watch.observe(t_ms, &d, stamp, journal);
        self.last = Some(v);
        lines.push(format!(
            "{}: kw tabloul {} (la {} ms): temperatura {}, ceasul nucleului {}, stare {}; ceasul termic: {}",
            p,
            what,
            t_ms,
            d.temp_c().map_or_else(|| String::from("necitita"), |c| format!("{} grade C", c)),
            d.clk_graphics_mhz.map_or_else(|| String::from("necitit"), |x| format!("{} MHz", x)),
            d.pstate().map_or_else(|| String::from("necitita"), pstate_text),
            verdict_text(v)
        ));
        Some((d, v))
    }

    /// True when the last reading said "stop" (the max-speed request is then not sent).
    pub fn says_stop(&self) -> bool {
        matches!(self.last, Some(nvidia_ga104::gsp::rusd_watch::Verdict::Stop(_)))
    }
}

/// P6-142: what the dashboard says the max-speed request did (`compare_boost`, `kw viteza maxima` rows + one line).
pub(super) fn boost_seen_line(
    p: &str,
    before: &nvidia_ga104::gsp::rusd::Dashboard,
    after: &nvidia_ga104::gsp::rusd::Dashboard,
    journal: &mut Journal,
) -> String {
    use nvidia_ga104::gsp::rusd_watch::{self as kw, BoostSeen};
    let seen = kw::compare_boost(before, after, journal);
    let ps = |d: &nvidia_ga104::gsp::rusd::Dashboard| d.pstate().map_or_else(|| String::from("-"), pstate_text);
    let clk = |d: &nvidia_ga104::gsp::rusd::Dashboard| d.clk_graphics_mhz.map_or_else(|| String::from("-"), |x| format!("{}", x));
    format!(
        "{}: kw viteza maxima vazuta pe tablou: {} (starea {} -> {}, ceasul nucleului {} -> {} MHz)",
        p,
        match seen {
            BoostSeen::Raised => "URCATA",
            BoostSeen::AlreadyUp => "era deja sus (P0 inainte)",
            BoostSeen::NoEffect => "FARA EFECT",
            BoostSeen::Unknown => "necunoscut (starea sau ceasul necitite)",
        },
        ps(before),
        ps(after),
        clk(before),
        clk(after)
    )
}

/// P6-142: the thermal watch's summary (`kw ceas termic rezumat` rows + one line).
pub(super) fn watch_summary_line(p: &str, w: &nvidia_ga104::gsp::rusd_watch::Watch, journal: &mut Journal) -> String {
    w.summary(journal);
    format!(
        "{}: kw ceas termic rezumat: {} citiri, temperatura {}..{} grade C, puterea maxima {}, citiri cu tabloul neschimbat {}{}",
        p,
        w.reads,
        w.temp_min.map_or_else(|| String::from("-"), |t| format!("{}", t)),
        w.temp_max.map_or_else(|| String::from("-"), |t| format!("{}", t)),
        w.power_max_mw.map_or_else(|| String::from("-"), |x| format!("{} mW", x)),
        w.silent_reads,
        w.stopped().map_or_else(String::new, |r| format!("; oprirea ceasului: {}", verdict_text(nvidia_ga104::gsp::rusd_watch::Verdict::Stop(r))))
    )
}

/// P6-91/P6-92: the I9 choice of `gpu lant --i9 ...`. `Default` = what os-302/os-303 sent without the option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I9Choice {
    /// The `open_compute_group` + `open_work_channel_with` path (os-302 default, C-125 V2/V3).
    Opts(I9Options),
    /// C-133: `open_work_channel_c133` (one PROMOTE_CTX PA+VA before the compute object).
    C133,
    /// C-135: `open_work_channel_c135` (form B directly; N without error notifier, then F with a fixed ChID).
    C135,
}

impl Default for I9Choice {
    fn default() -> Self {
        I9Choice::Opts(I9Options::OS_302)
    }
}

/// P6-91 (C-125) / P6-92 (C-133) / P6-93 (C-135): `--i9 c125|c133|c135` from the shell; anything else is refused before the `DA`.
pub fn i9_option(v: &str) -> Option<I9Choice> {
    match v {
        "c125" => Some(I9Choice::Opts(I9Options::C125)),
        "c133" => Some(I9Choice::C133),
        "c135" => Some(I9Choice::C135),
        "os302" => Some(I9Choice::Opts(I9Options::OS_302)),
        _ => None,
    }
}

/// P6-91 (C-125) / P6-92 (C-133): the I9 option on the screen (before `DA`) and in the result lines.
pub fn i9_line(c: I9Choice) -> String {
    let o = match c {
        I9Choice::C135 => {
            return String::from(
                "optiune I9: c135 (C-135, SIMULARE testata; pe placa doar cu Adrian) - direct forma B; N: canal fara notificator de erori; F (doar daca jetonul lui N e refuzat): canal cu id ales de noi + jeton calculat",
            )
        }
        I9Choice::C133 => {
            return String::from(
                "optiune I9: c133 (C-133, SIMULARE testata; pe placa doar cu Adrian) - PROMOTE_CTX intr-un singur mesaj PA+VA inaintea obiectului compute (A: MAIN+PM+PATCH; B doar daca A e refuzat)",
            )
        }
        I9Choice::Opts(o) => o,
    };
    if o == I9Options::OS_302 {
        String::from("optiune I9: implicita (ca os-302)")
    } else {
        format!(
            "optiune I9: c125 (C-125, SIMULARE testata; pe placa doar cu Adrian) - V2 buffere de context privilegiate: {}, V3 lista bufferelor de la RM in jurnal: {}",
            if o.privileged_ctx { "da" } else { "nu" },
            if o.ctx_buffer_info { "da" } else { "nu" }
        )
    }
}

/// P6-91: one screen line from the I9 report (the details are in the journal rows `i9 ...`).
fn i9_report_line(r: &I9Report) -> String {
    use nvidia_ga104::gsp::user_space::CtxInfoAnswer;
    let v3 = match &r.ctx_info {
        CtxInfoAnswer::NotAsked => String::from("V3 neintrebat"),
        CtxInfoAnswer::Listed(l) => format!("V3 lista RM: {} buffere", l.len()),
        CtxInfoAnswer::Refused(c) => format!("V3 refuzat de RM: {:#x}", c),
    };
    format!(
        "I9 c125: optiuni {} (b0 V2, b1 V3); {}; {} buffere de context promovate (detalii: randurile i9 din jurnal)",
        r.options.bits(),
        v3,
        r.ctx_bufs.len()
    )
}

/// P6-92 (C-133): one screen line from the C-133 report (the details are in the journal rows `i9 c133 ...`).
fn c133_report_line(r: &C133Report) -> String {
    let passed = match r.passed {
        Some(v) => format!("reusita: {:?}", v),
        None => String::from("nicio varianta reusita"),
    };
    let refusals: Vec<String> = r
        .tries
        .iter()
        .filter_map(|t| t.refusal.map(|c| format!("{:?} refuzat dupa pasul {} cu {:#x}", t.variant, t.stage as u32, c)))
        .collect();
    format!(
        "I9 c133: {} incercari; {}{}{} (detalii: randurile i9 c133 din jurnal)",
        r.tries.len(),
        passed,
        if refusals.is_empty() { "" } else { "; " },
        refusals.join("; ")
    )
}

/// P6-93 (C-135): one screen line from the C-135 report (the details are in the journal rows `i9 c135 ...`).
fn c135_report_line(r: &C135Report) -> String {
    use nvidia_ga104::gsp::user_space::C135Token;
    let passed = match r.passed {
        Some(v) => format!("reusita: {:?}", v),
        None => String::from("nicio varianta reusita"),
    };
    let details: Vec<String> = r
        .tries
        .iter()
        .filter_map(|t| {
            let tok = match t.token {
                C135Token::Refused { status, .. } => Some(format!("{:?}: jeton refuzat cu {:#x}", t.variant, status)),
                _ => None,
            };
            let refusal = t.refusal.map(|c| format!("{:?} refuzat dupa pasul {} cu {:#x}", t.variant, t.stage as u32, c));
            match (tok, refusal) {
                (Some(a), Some(b)) => Some(format!("{}; {}", a, b)),
                (a, b) => a.or(b),
            }
        })
        .collect();
    format!(
        "I9 c135: {} incercari; {}{}{} (detalii: randurile i9 c135 din jurnal)",
        r.tries.len(),
        passed,
        if details.is_empty() { "" } else { "; " },
        details.join("; ")
    )
}
