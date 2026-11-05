//! P6-56 (command C-72, Adrian's "da i7-i10"): step I7 of the window chain in the OS.
//!
//! I7 = Booter (the NVIDIA-signed program `booter_load-570.144.bin`) runs on the SEC2 falcon, reads `GspFwWprMeta` from
//! system memory, copies GSP-RM into the protected region WPR2 that I6 raised and starts the GSP RISC-V core. What the
//! OS does here:
//! - [`prepare`] — nothing written to the card: the 4 GSP 570.144 files from the stick (verified with SHA-256 by
//!   `firmware::load_gsp_set`, as in I4), one physically contiguous DMA area between 16 MiB and 4 GiB holding the
//!   `.fwimage` pages, the radix3 table describing them, the bootloader blob, the `.fwsignature_ga10x` copy, the
//!   `GspFwWprMeta` page (`wpr_meta::encode` over the same `FbLayout` I6 used: checked to give the same FRTS region),
//!   and the Booter image with the signature chosen by the SEC2 fuse (one read). The area is pinned and never freed:
//!   GSP-RM keeps reading it after I7;
//! - [`booter`] — `falcon::booter_load_dma` through the caller's guarded window (GSP + SEC2 only), then, whatever the
//!   result, `read_after_i7` (3 reads, no permit) so the journal shows how I7 ended.
//!
//! Every step was written and tested by Cyber-GPU only in SIMULATION; on the board each one is a first try.
//! The NVIDIA files are never in the repo, the tests or the stick archive (the tests use `gpu_sim::fw_stick`).
//!
//! Sources: Cyber-GPU (our library, pinned in kernel/Cargo.toml), notes C-69 and C-71.

#![cfg_attr(not(any(feature = "gpu-scrie", feature = "test-gpu-sim")), allow(dead_code))]

use alloc::{format, string::String, vec::Vec};
use gpu_core::diag::Journal;
use gpu_core::error::{GpuError, GpuResult};
use gpu_core::hal::{Clock, Mmio};
use nvidia_ga104::fb_layout::{FbLayout, FbParams};
use nvidia_ga104::firmware::{GspFirmware, GspRole, GSP_570_144};
use nvidia_ga104::fw_hdr::{HsV2, RiscvBootloader};
use nvidia_ga104::gsp::falcon::{self, GuardedBar0, I7Status};
use nvidia_ga104::gsp::HardwareWritePermit;
use nvidia_ga104::gsp::hs_load::{self, BooterMeta, DmaImage, HsUcode};
use nvidia_ga104::gsp::libos;
use nvidia_ga104::gsp::msgq::{CpuQueues, QueueLayout, SharedMem};
use nvidia_ga104::gsp::{elf, wpr_meta};
use nvidia_ga104::i6_check::I6Check;
use nvidia_ga104::radix3::{Radix3, Radix3Shape, PAGE};

const PAGE_US: usize = PAGE as usize;

/// What I7 needs, all in the pinned DMA area.
pub(super) struct I7Prep {
    pub ucode: HsUcode,
    pub image: DmaImage,
    pub wpr_meta: u64,
    /// P6-56 step 2: the LibOS argument page (LOGINIT, LOGINTR, LOGRM, RMARGS) GSP-RM looks for at boot.
    pub libos: u64,
    /// P6-56 step 2: the shared area with the command/status queues (inside the pinned area) and its driver side.
    pub shared: DmaShared,
    pub queues: CpuQueues,
}

/// The queue area inside the pinned DMA area, seen by `msgq` (volatile accesses through the HHDM).
pub(super) struct DmaShared {
    phys: u64,
    len: usize,
}

impl DmaShared {
    fn mem(&self) -> *mut u8 {
        crate::mem::map::phys_to_virt(self.phys)
    }
}

impl SharedMem for DmaShared {
    fn size(&self) -> usize {
        self.len
    }
    fn read(&self, off: usize, buf: &mut [u8]) {
        assert!(off + buf.len() <= self.len, "DmaShared: citire in afara zonei");
        for (i, b) in buf.iter_mut().enumerate() {
            // SAFETY: inside the pinned area (checked above); volatile because GSP-RM writes it by DMA.
            *b = unsafe { self.mem().add(off + i).read_volatile() };
        }
    }
    fn write(&mut self, off: usize, data: &[u8]) {
        assert!(off + data.len() <= self.len, "DmaShared: scriere in afara zonei");
        for (i, b) in data.iter().enumerate() {
            // SAFETY: inside the pinned area (checked above); volatile because GSP-RM reads it by DMA.
            unsafe { self.mem().add(off + i).write_volatile(*b) };
        }
    }
    fn fence(&mut self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}

/// The pinned area: (physical start, pages). Reused when a later run fits in it (never freed).
static AREA: crate::sched::Mutex<Option<(u64, u64)>> = crate::sched::Mutex::new(None);

fn area(pages: u64) -> Result<u64, String> {
    let mut slot = AREA.lock();
    if let Some((p, n)) = *slot {
        if n >= pages {
            return Ok(p);
        }
    }
    let p = crate::mem::frame::alloc_contiguous_between(pages, 16 << 20, 1 << 32)
        .ok_or_else(|| format!("nu exista {} MiB contigui intre 16 MiB si 4 GiB", (pages * PAGE).div_ceil(1 << 20)))?;
    crate::mem::frame::pin(p, pages, "gpu-i7"); // GSP-RM reads here by DMA after I7: never given to anyone else
    *slot = Some((p, pages));
    Ok(p)
}

/// The bytes of the area (zeroed by [`prepare`] first).
fn bytes(phys: u64, len: usize) -> &'static mut [u8] {
    let va = crate::mem::map::phys_to_virt(phys);
    // SAFETY: [phys, phys + len) lies inside the pinned area returned by `area` (frames allocated for it only, never
    // freed), reached through the HHDM; callers stay inside the page count they asked for.
    unsafe { core::slice::from_raw_parts_mut(va, len) }
}

/// Where each piece goes, in pages from the start of the area.
struct Plan {
    data: usize,
    l0: usize,
    l1: usize,
    l2: usize,
    bootloader: usize,
    signature: usize,
    meta: usize,
    booter: usize,
    libos: usize,
    logs: [usize; 3],
    rmargs: usize,
    queues: usize,
    total: usize,
}

/// One LibOS log buffer (64 KiB, `libos::standard_regions`).
const LOG_PAGES: usize = 0x10000 / PAGE_US;

fn pages_of(n: usize) -> usize {
    n.div_ceil(PAGE_US).max(1)
}

fn plan(fwimage: usize, bl: usize, sig: usize, booter: usize) -> GpuResult<Plan> {
    let shape = Radix3Shape::for_size(fwimage as u64)?;
    let data = 0;
    let l0 = data + shape.data_pages;
    let l1 = l0 + 1;
    let l2 = l1 + 1;
    let bootloader = l2 + shape.level2_pages;
    let signature = bootloader + pages_of(bl);
    let meta = signature + pages_of(sig);
    let booter_at = meta + 1;
    let libos = booter_at + pages_of(booter);
    let logs = [libos + 1, libos + 1 + LOG_PAGES, libos + 1 + 2 * LOG_PAGES];
    let rmargs = libos + 1 + 3 * LOG_PAGES;
    let queues = rmargs + 1;
    let total = queues + pages_of(QueueLayout::standard().total_size);
    Ok(Plan { data, l0, l1, l2, bootloader, signature, meta, booter: booter_at, libos, logs, rmargs, queues, total })
}

fn fail(what: &'static str) -> GpuError {
    GpuError::InvalidArgument(what)
}

/// The I6 layout again, with the parameters `i6_check` used (the sizes of the official files from the manifest):
/// `GspFwWprMeta` must describe the same regions FWSEC-FRTS was asked for.
fn layout(c: &I6Check) -> GpuResult<FbLayout> {
    let size = |r: GspRole| GSP_570_144.iter().find(|f| f.role == r).map(|f| f.size).unwrap_or(0);
    let l = FbLayout::compute(&FbParams {
        fb_size: c.fb_size,
        vga_workspace_reg: None,
        bootloader_size: size(GspRole::Bootloader),
        gsp_elf_size: size(GspRole::GspRm),
    })?;
    if (l.frts.start, l.frts.end) != c.frts {
        return Err(fail("I7: harta memoriei placii difera de cea de la I6"));
    }
    Ok(l)
}

/// Builds everything I7 needs in RAM. `bar0` is read only (one fuse read). Nothing is written to the card.
pub(super) fn prepare<M: Mmio + ?Sized>(
    p: &str,
    bar0: &mut M,
    journal: &mut Journal,
    fw: &GspFirmware,
    c: &I6Check,
    lines: &mut Vec<String>,
) -> GpuResult<I7Prep> {
    // 1. The pieces, from the verified files.
    let fwimage = elf::section(&fw.gsp_rm, ".fwimage")?;
    let signature = elf::section(&fw.gsp_rm, ".fwsignature_ga10x")?;
    let bl = RiscvBootloader::parse(&fw.bootloader)?;
    let bl_blob = fw.bootloader.get(bl.hdr.data()).ok_or(fail("bootloader: datele in afara fisierului"))?;
    let h = HsV2::parse(&fw.booter_load)?;
    let meta = BooterMeta::parse(&fw.booter_load, &h)?;
    let fuse = nvidia_ga104::fuse::booter_fuse_version(bar0, journal, &meta)?;
    let booter_img = hs_load::booter_image(&h, &fw.booter_load, u32::from(fuse))?;
    let ucode = HsUcode::from_booter(&h, &fw.booter_load)?;
    lines.push(format!(
        "{}: I7 pregatire: GSP-RM .fwimage {} KiB, semnatura {} B, bootloader {} B, Booter {} B (siguranta SEC2 ucode {} = versiunea {})",
        p, fwimage.len() >> 10, signature.len(), bl_blob.len(), booter_img.len(), meta.ucode_id, fuse
    ));
    let fb = layout(c)?;

    // 2. The area.
    let pl = plan(fwimage.len(), bl_blob.len(), signature.len(), booter_img.len())?;
    let base = area(pl.total as u64).map_err(|why| {
        lines.push(format!("{}: I7 zona DMA: {}; nimic scris in placa", p, why));
        fail("I7: zona DMA")
    })?;
    let all = bytes(base, pl.total * PAGE_US);
    all.fill(0);
    let at = |page: usize| base + (page * PAGE_US) as u64;

    // 3. `.fwimage` page by page, and the radix3 table describing those pages.
    all[..fwimage.len()].copy_from_slice(fwimage);
    let shape = Radix3Shape::for_size(fwimage.len() as u64)?;
    let data_pages: Vec<u64> = (0..shape.data_pages).map(|k| at(pl.data + k)).collect();
    let l2_pages: Vec<u64> = (0..shape.level2_pages).map(|k| at(pl.l2 + k)).collect();
    let r = Radix3::build(&data_pages, at(pl.l1), &l2_pages)?;
    let mut put = |page: usize, b: &[u8]| {
        let o = page * PAGE_US;
        all[o..o + b.len()].copy_from_slice(b);
    };
    put(pl.l0, &Radix3::page_bytes(&r.level0));
    put(pl.l1, &Radix3::page_bytes(&r.level1));
    for (k, l2) in r.level2.iter().enumerate() {
        put(pl.l2 + k, &Radix3::page_bytes(l2));
    }
    put(pl.bootloader, bl_blob);
    put(pl.signature, signature);
    put(pl.booter, &booter_img);

    // 4. `GspFwWprMeta`.
    let inputs = wpr_meta::SysmemInputs {
        radix3_elf_addr: at(pl.l0),
        radix3_elf_size: fwimage.len() as u64,
        bootloader_addr: at(pl.bootloader),
        bootloader_size: bl_blob.len() as u64,
        bootloader_code_offset: u64::from(bl.code_offset),
        bootloader_data_offset: u64::from(bl.data_offset),
        bootloader_manifest_offset: u64::from(bl.manifest_offset),
        signature_addr: at(pl.signature),
        signature_size: signature.len() as u64,
    };
    let m = wpr_meta::encode(&fb, &inputs)?;
    put(pl.meta, &m);

    // 4b. P6-56 step 2: the message queues (page table + command-queue header) and the LibOS arguments that tell
    // GSP-RM where they are (RMARGS = GSP_ARGUMENTS_CACHED). RAM only.
    let ql = QueueLayout::standard();
    let qpages: Vec<u64> = (0..ql.page_table_entries).map(|k| at(pl.queues + k)).collect();
    let args = libos::encode_gsp_args(&ql, at(pl.queues))?;
    put(pl.rmargs, &args);
    let regions = libos::standard_regions(at(pl.logs[0]), at(pl.logs[1]), at(pl.logs[2]), at(pl.rmargs));
    let libos_page = libos::encode_region_args(&regions)?;
    put(pl.libos, &libos_page);
    // (the slice above is not used again; the queue area is written through its own volatile view)
    let mut shared = DmaShared { phys: at(pl.queues), len: ql.total_size };
    let queues = CpuQueues::init(&mut shared, ql, &qpages)?;

    // 5. Read back: the area must hold exactly what was put there (and the table must lead to the image).
    let all = bytes(base, pl.total * PAGE_US);
    let o = |page: usize| page * PAGE_US;
    let same = all[..fwimage.len()] == *fwimage
        && all[o(pl.bootloader)..o(pl.bootloader) + bl_blob.len()] == *bl_blob
        && all[o(pl.signature)..o(pl.signature) + signature.len()] == *signature
        && all[o(pl.meta)..o(pl.meta) + m.len()] == m[..]
        && all[o(pl.booter)..o(pl.booter) + booter_img.len()] == booter_img[..]
        && all[o(pl.libos)..o(pl.libos) + libos_page.len()] == libos_page[..]
        && all[o(pl.rmargs)..o(pl.rmargs) + args.len()] == args[..];
    let last = fwimage.len() as u64 - 1;
    let walks = r.translate(&l2_pages, last) == Some(at(pl.data) + last);
    if !same || !walks {
        lines.push(format!("{}: I7 pregatire: recitirea din RAM difera; nimic scris in placa", p));
        return Err(fail("I7: recitirea zonei DMA"));
    }
    let image = DmaImage { phys: at(pl.booter), len: booter_img.len() as u32 };
    lines.push(format!(
        "{}: I7 pregatire: zona DMA {} KiB la {:#x} (recitita); radix3 nivelul 0 la {:#x}; GspFwWprMeta la {:#x}; Booter la {:#x}; nimic scris in placa",
        p, pl.total * 4, base, inputs.radix3_elf_addr, at(pl.meta), image.phys
    ));
    lines.push(format!(
        "{}: I7 pregatire: cozile GSP ({} KiB) la {:#x}, argumentele LibOS la {:#x} (RMARGS {:#x}); nimic scris in placa",
        p, ql.total_size >> 10, at(pl.queues), at(pl.libos), at(pl.rmargs)
    ));
    Ok(I7Prep { ucode, image, wpr_meta: at(pl.meta), libos: at(pl.libos), shared, queues })
}

/// The `I7: ...` line from the three status reads.
fn status_line(p: &str, s: &GpuResult<I7Status>) -> String {
    match s {
        Ok(s) => format!(
            "{}: I7: cutia SEC2 0 = {:#010x}, cutia 1 = {:#010x}, GSP RISC-V {} (CPUCTL {:#010x}) -> {}",
            p,
            s.sec2_mailbox0,
            s.sec2_mailbox1,
            if s.passed() || s.riscv_cpuctl & (1 << 7) != 0 { "ACTIV" } else { "NU" },
            s.riscv_cpuctl,
            if s.passed() { "TRECE" } else { "NU TRECE" }
        ),
        Err(e) => format!("{}: I7: citirea starii: {}", p, e),
    }
}

/// Step `i7-booter-gsp`: Booter on SEC2 through `rw` (the caller's fence: GSP + SEC2 windows only), then always the
/// three status reads. Returns whether Booter returned 0 and the RISC-V core runs.
pub(super) fn booter<W: Mmio + ?Sized, K: Clock + ?Sized>(
    p: &str,
    rw: &mut W,
    clock: &mut K,
    journal: &mut Journal,
    prep: &I7Prep,
    permit: Option<&HardwareWritePermit>,
    lines: &mut Vec<String>,
) -> GpuResult<()> {
    let r = GuardedBar0::new(rw, journal, permit)
        .and_then(|mut b| falcon::booter_load_dma(&mut b, clock, &prep.ucode, &prep.image, prep.wpr_meta));
    match &r {
        Ok(()) => lines.push(format!("{}: pas I7 Booter + GSP RISC-V: ok (GspFwWprMeta {:#x})", p, prep.wpr_meta)),
        Err(e) => lines.push(format!("{}: pas I7 Booter + GSP RISC-V: ESEC: {} (oprit la prima eroare)", p, e)),
    }
    lines.push(status_line(p, &falcon::read_after_i7(rw, journal)));
    r
}

/// Step `i7-citit`: the three reads again (no permit), as its own journalled step.
pub(super) fn after<M: Mmio + ?Sized>(p: &str, bar0: &mut M, journal: &mut Journal, lines: &mut Vec<String>) -> GpuResult<()> {
    let s = falcon::read_after_i7(bar0, journal);
    lines.push(status_line(p, &s));
    if s?.passed() {
        Ok(())
    } else {
        Err(fail("I7: Booter refuzat sau GSP RISC-V oprit"))
    }
}

/// The 4 files from the stick, verified (the official 570.144 set). Only the test-stick build reads them.
#[cfg(feature = "gpu-scrie")]
pub(super) fn stick_firmware(p: &str, journal: &mut Journal, lines: &mut Vec<String>) -> GpuResult<GspFirmware> {
    let r = nvidia_ga104::firmware::load_gsp_set(&mut super::gpu::StickFirmware, &GSP_570_144, journal);
    match &r {
        Ok(_) => lines.push(format!("{}: I7: cele 4 fisiere GSP 570.144 de pe stick verificate (SHA-256)", p)),
        Err(e) => lines.push(format!("{}: I7: fisierele GSP de pe stick: {} (vezi `gpu firmware`); nimic scris in placa", p, e)),
    }
    r
}

/// SIMULATION: the gpu-sim stand-ins (NOT NVIDIA files), as a verified set.
#[cfg(feature = "test-gpu-sim")]
pub(super) fn sim_firmware(journal: &mut Journal) -> GpuResult<GspFirmware> {
    use gpu_sim::fw_stick;
    use nvidia_ga104::{firmware::STICK_DIR, fw_check};
    let files = fw_stick::files();
    let man = core::array::from_fn(|i| fw_check::pin_of(fw_check::ROLE_ORDER[i], fw_stick::SIM_PATHS[i], &files[i]));
    let mut src = fw_check::MemSource::new();
    for (path, b) in fw_stick::SIM_PATHS.iter().zip(files) {
        src.insert(format!("{}{}", STICK_DIR, path), b);
    }
    nvidia_ga104::firmware::load_gsp_set(&mut src, &man, journal)
}

/// SIMULATION: what a correct loader puts in the SEC2 falcon from `prep` (for `FakeGsp::sim_expect`).
#[cfg(feature = "test-gpu-sim")]
pub(super) fn sim_expect(g: &mut gpu_sim::fake_gsp::FakeGsp, prep: &I7Prep) {
    use gpu_sim::fake_gsp::{SimHsImage, SimUcode, SEC2_BASE};
    let img = bytes(prep.image.phys, prep.image.len as usize);
    let u = &prep.ucode;
    let span = |o: u32, n: u32| img[o as usize..(o + n) as usize].to_vec();
    g.sim_expect(SimHsImage {
        falcon: SEC2_BASE,
        ucode: SimUcode::BooterLoad,
        imem_pa: u.imem_pa,
        code: span(u.code_offset, u.imem_size),
        dmem_pa: u.dmem_pa,
        data: span(u.data_offset, u.dmem_size),
        sig_dmem_addr: u.sig_dmem_addr,
        ucode_id: u32::from(u.ucode_id),
        engine_id_mask: u.engine_id_mask,
        bootvec: u.imem_va,
    });
    g.sysmem = Some((prep.image.phys, img.to_vec()));
}
