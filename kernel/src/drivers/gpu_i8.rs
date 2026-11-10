//! P6-56 step 2 (command C-72, Adrian's "da i7-i10"): step I8 of the window chain in the OS.
//!
//! I8 = the first conversation with GSP-RM after I7 started it: `GSP_SET_SYSTEM_INFO` (where the BARs are, which PCI
//! device) and `SET_REGISTRY` (the two boot entries) go into the command queue prepared at I7, one doorbell
//! (`NV_PGSP_QUEUE_HEAD(0)`, inside the GSP window), then the status queue is read until `GSP_INIT_DONE`, running every
//! CPU-sequencer request on the way (`sequencer::wait_init_done`, through the same guarded window: GSP/SEC2 only).
//! Every piece is Cyber-GPU's (`nvidia_ga104::gsp::{sysinfo, registry, msgq, sequencer, falcon}`); the OS only gives
//! the memory, the window, the clock and the PCI facts.
//!
//! Written and tested by Cyber-GPU only in SIMULATION; in the OS tested in QEMU against a small stand-in for GSP-RM
//! ([`SimPeer`], test builds). On the board it is a first try.
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml), note C-71 (`integration/cyber-universe/`).

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::gsp::falcon::{self, GuardedBar0};
use nvidia_ga104::gsp::msgq::SharedMem;
use nvidia_ga104::gsp::rpc::function;
use nvidia_ga104::gsp::sequencer::{self, SeqContext};
use nvidia_ga104::gsp::sysinfo::{self, SystemInfoInput};
use nvidia_ga104::gsp::{registry, HardwareWritePermit};

use super::gpu_i7::I7Prep;

/// GSP-RM 570.144 (the only firmware `firmware::load_gsp_set` accepts): CORE_RESUME writes it back.
pub(super) const APP_VERSION: u32 = 570_144;
/// How long GSP-RM may take to write the status-queue header after the doorbell. P6-70 (C-99, Adrian's idea): as
/// long as the other waits after the doorbell (`sequencer::LONG_WAIT_US`, 60 s; was 2 s), and Esc stops it.
const STAT_HEADER_US: u64 = sequencer::LONG_WAIT_US;

/// The PCI facts `GSP_SET_SYSTEM_INFO` carries, read from the configuration space (reads only).
pub(super) fn system_info(d: &crate::pci::Device) -> GpuResult<SystemInfoInput> {
    let id = d.read32(0);
    let sub = d.read32(0x2C);
    let (Some(bar0), Some(bar1), Some(bar3)) = (d.bar(0), d.bar(1), d.bar(3)) else {
        return Err(GpuError::InvalidArgument("I8: BAR0/BAR1/BAR3 lipsesc din configuratia PCI"));
    };
    Ok(SystemInfoInput {
        bar0,
        bar1,
        bar3,
        domain: 0,
        bus: d.bus,
        slot: d.dev,
        vendor: id as u16,
        device: (id >> 16) as u16,
        subsys_vendor: sub as u16,
        subsys_device: (sub >> 16) as u16,
        revision: d.read32(8) as u8,
        // The workstation boots its screen on this card (UEFI GOP), like Cyber-GPU's workstation model.
        is_primary: true,
    })
}

/// Step `i8-gsp-init`. `rw` = the caller's fence (GSP + SEC2 windows only); `poll` advances a simulated GSP-RM
/// between two empty reads (nothing on the board).
#[allow(clippy::too_many_arguments)]
pub(super) fn init<W: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    rw: &mut W,
    clock: &mut K,
    journal: &mut Journal,
    prep: &mut I7Prep,
    sys: &SystemInfoInput,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
    mut poll: impl FnMut(&mut dyn SharedMem),
) -> GpuResult<()> {
    let r = (|| {
        let info = sysinfo::encode(sys)?;
        let reg = registry::encode(&registry::boot_entries())?;
        prep.queues.send_rpc(&mut prep.shared, function::GSP_SET_SYSTEM_INFO, &info)?;
        prep.queues.send_rpc(&mut prep.shared, function::SET_REGISTRY, &reg)?;
        lines.push(format!(
            "{}: I8: in coada: SetSystemInfo (BAR0 {:#x}, BAR1 {:#x}, BAR3 {:#x}, {:04x}:{:04x} sub {:04x}:{:04x} rev {:#04x}) + SetRegistry (2 intrari)",
            p, sys.bar0, sys.bar1, sys.bar3, sys.vendor, sys.device, sys.subsys_vendor, sys.subsys_device, sys.revision
        ));
        let mut b = GuardedBar0::new(&mut *rw, journal, permit)?;
        falcon::ring_doorbell(&mut b)?;
        // GSP-RM writes the status-queue header when it starts; wait for a header of the 570.144 shape.
        let start = clock.now_us();
        loop {
            match prep.queues.link_rx(&prep.shared) {
                Ok(()) => {
                    lines.push(format!(
                        "{}: asteptat I8 antetul cozii de stare (de la sonerie): raspuns dupa {} us",
                        p,
                        clock.now_us().saturating_sub(start)
                    ));
                    break;
                }
                Err(e) if clock.now_us().saturating_sub(start) >= STAT_HEADER_US => return Err(e),
                Err(_) if clock.abort_requested() => {
                    return Err(GpuError::Aborted { waited_us: clock.now_us().saturating_sub(start) });
                }
                Err(_) => {
                    clock.delay_us(1000);
                    poll(&mut prep.shared);
                }
            }
        }
        let ctx = SeqContext { libos_addr: prep.libos, app_version: APP_VERSION };
        sequencer::wait_init_done(&mut prep.queues, &mut prep.shared, &mut b, clock, ctx, sequencer::INIT_DONE_US, poll)
    })();
    match &r {
        Ok(log) => lines.push(format!(
            "{}: pas I8 GSP init: ok (GSP_INIT_DONE; secventiator: {} rulari, {} comenzi; alte evenimente: {})",
            p,
            log.sequencer_runs,
            log.sequencer_commands,
            log.other_events.len()
        )),
        Err(e) => lines.push(format!("{}: pas I8 GSP init: ESEC: {} (oprit la prima eroare)", p, e)),
    }
    r.map(|_| ())
}

/// SIMULATION: a small stand-in for GSP-RM on the other side of the queues, enough for the OS plumbing (the real
/// DMA area, the doorbell through the fence, the wait). It checks that the command queue holds exactly
/// `SetSystemInfo` + `SetRegistry`, consumes them, writes the status-queue header and posts `GSP_INIT_DONE`. It proves
/// nothing about the real GSP-RM (Cyber-GPU's own tests cover the protocol with an independent peer).
#[cfg(feature = "test-gpu-sim")]
pub(super) struct SimPeer {
    cmd: usize,
    stat: usize,
    done: bool,
    pub seen: Vec<u32>,
    /// P6-56 step 2b: after GSP_INIT_DONE, the command-queue read slot, the status-queue write slot and sequence.
    cmd_read: u32,
    stat_write: u32,
    stat_seq: u32,
    /// RM messages answered by [`Self::serve`] (golden, I9).
    pub served: u32,
}

#[cfg(feature = "test-gpu-sim")]
impl SimPeer {
    pub(super) fn new(prep: &I7Prep) -> Self {
        let l = prep.queues.layout;
        SimPeer {
            cmd: l.cmd_queue_offset,
            stat: l.stat_queue_offset,
            done: false,
            seen: Vec::new(),
            cmd_read: 0,
            stat_write: 0,
            stat_seq: 0,
            served: 0,
        }
    }

    pub(super) fn poll(&mut self, m: &mut dyn SharedMem) {
        use nvidia_ga104::gsp::msgq::{qoff, QueueLayout, ELEMENT_SIZE, MSGQ_FLAGS_SWAP_RX, QUEUE_SIZE};
        use nvidia_ga104::gsp::rpc::{self, event};
        if self.done {
            return;
        }
        self.done = true;
        // The command queue: every message the driver wrote (write pointer), decoded.
        let written = m.read_u32(self.cmd + qoff::WRITE_PTR);
        let mut slot = 0u32;
        while slot < written {
            let mut page = alloc::vec![0u8; ELEMENT_SIZE];
            m.read(self.cmd + qoff::ENTRIES + slot as usize * ELEMENT_SIZE, &mut page);
            let n = rpc::element_count(&page).max(1);
            for k in 1..n {
                let mut more = alloc::vec![0u8; ELEMENT_SIZE];
                m.read(self.cmd + qoff::ENTRIES + (slot + k) as usize * ELEMENT_SIZE, &mut more);
                page.extend_from_slice(&more);
            }
            match rpc::decode(&page) {
                Ok(msg) => self.seen.push(msg.function),
                Err(_) => self.seen.push(u32::MAX),
            }
            slot += n;
        }
        self.cmd_read = written;
        // GSP's read pointer of the command queue lives in the status queue's rx header (SWAP_RX).
        let count = QueueLayout::msg_count(QUEUE_SIZE);
        for (o, v) in [
            (qoff::VERSION, 0),
            (qoff::SIZE, QUEUE_SIZE as u32),
            (qoff::MSG_SIZE, ELEMENT_SIZE as u32),
            (qoff::MSG_COUNT, count),
            (qoff::WRITE_PTR, 0),
            (qoff::FLAGS, MSGQ_FLAGS_SWAP_RX),
            (qoff::RX_HDR_OFF, qoff::RX_HDR as u32),
            (qoff::ENTRY_OFF, qoff::ENTRIES as u32),
            (qoff::RX_HDR, written),
        ] {
            m.write_u32(self.stat + o, v);
        }
        if self.seen != [function::GSP_SET_SYSTEM_INFO, function::SET_REGISTRY] {
            return; // a wrong conversation: no INIT_DONE, the driver times out
        }
        let Ok(msg) = rpc::encode(event::GSP_INIT_DONE, 0, 0, &[]) else { return };
        let mut page = alloc::vec![0u8; ELEMENT_SIZE];
        page[..msg.len()].copy_from_slice(&msg);
        m.write(self.stat + qoff::ENTRIES, &page);
        m.fence();
        m.write_u32(self.stat + qoff::WRITE_PTR, 1);
        m.fence();
        self.stat_write = 1;
        self.stat_seq = 1;
    }

    /// P6-56 step 2b (golden, I9): consumes every new command and answers each with `rm` (`gpu_sim::fake_rm`, the
    /// RM API model). A refused call (non-zero RM status) gets no answer here: the driver times out. SIMULARE.
    pub(super) fn serve(&mut self, m: &mut dyn SharedMem, rm: &mut gpu_sim::fake_rm::FakeRm) {
        use nvidia_ga104::gsp::msgq::{qoff, QueueLayout, ELEMENT_SIZE, QUEUE_SIZE};
        use nvidia_ga104::gsp::rpc;
        let count = QueueLayout::msg_count(QUEUE_SIZE);
        let page_at = |base: usize, slot: u32| base + qoff::ENTRIES + (slot % count) as usize * ELEMENT_SIZE;
        loop {
            let w = m.read_u32(self.cmd + qoff::WRITE_PTR);
            if w == self.cmd_read {
                break;
            }
            let mut raw = alloc::vec![0u8; ELEMENT_SIZE];
            m.read(page_at(self.cmd, self.cmd_read), &mut raw);
            let n = rpc::element_count(&raw).max(1);
            for k in 1..n {
                let mut more = alloc::vec![0u8; ELEMENT_SIZE];
                m.read(page_at(self.cmd, self.cmd_read + k), &mut more);
                raw.extend_from_slice(&more);
            }
            self.cmd_read = (self.cmd_read + n) % count;
            m.write_u32(self.stat + qoff::RX_HDR, self.cmd_read);
            let Ok(msg) = rpc::decode(&raw) else { continue };
            self.served += 1;
            let Some((0, reply)) = rm.handle(msg.function, &msg.payload) else { continue };
            let Ok(out) = rpc::encode(msg.function, self.stat_seq, msg.sequence, &reply) else { continue };
            let pages = out.len().div_ceil(ELEMENT_SIZE);
            let mut padded = alloc::vec![0u8; pages * ELEMENT_SIZE];
            padded[..out.len()].copy_from_slice(&out);
            for (i, page) in padded.chunks(ELEMENT_SIZE).enumerate() {
                m.write(page_at(self.stat, self.stat_write + i as u32), page);
            }
            m.fence();
            self.stat_write = (self.stat_write + pages as u32) % count;
            self.stat_seq += 1;
            m.write_u32(self.stat + qoff::WRITE_PTR, self.stat_write);
            m.fence();
        }
    }
}
