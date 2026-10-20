//! XHCI USB host controller: start-up, command and event rings, and device enumeration
//! (port reset, Enable Slot, Address Device, device and configuration descriptors), with polling.
//! Written from the eXtensible Host Controller Interface for USB (xHCI) specification 1.2 and the
//! USB 2.0 specification, chapter 9 (standard requests and descriptors).

use crate::mem::{frame, map::{PAGE_SIZE, phys_to_virt}, paging};
use alloc::vec::Vec;

const TRBS: usize = (PAGE_SIZE / 16) as usize; // one 4 KiB frame per ring segment
const SPIN_LIMIT: u32 = 20_000_000;

// TRB types
const TRB_SETUP: u32 = 2;
const TRB_DATA: u32 = 3;
const TRB_STATUS: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_EVALUATE_CONTEXT: u32 = 13;
const EVT_TRANSFER: u32 = 32;
const EVT_COMMAND_DONE: u32 = 33;
const CC_SUCCESS: u32 = 1;
const CC_SHORT_PACKET: u32 = 13;

fn rd(a: u64) -> u32 {
    // SAFETY: `a` is a register of the mapped (uncached) XHCI register block.
    unsafe { (a as *const u32).read_volatile() }
}
fn wr(a: u64, v: u32) {
    // SAFETY: as in `rd`.
    unsafe { (a as *mut u32).write_volatile(v) }
}
fn wr64(a: u64, v: u64) {
    wr(a, v as u32);
    wr(a + 4, (v >> 32) as u32);
}

fn wait(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..SPIN_LIMIT {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// A zeroed frame the controller will use (DMA).
fn dma_frame() -> Option<u64> {
    let f = frame::alloc_frame()?;
    // SAFETY: fresh frame, reachable through the HHDM.
    unsafe { core::ptr::write_bytes(phys_to_virt(f), 0, PAGE_SIZE as usize) };
    Some(f)
}

fn mem32(phys: u64, index: usize) -> *mut u32 {
    // SAFETY (for users): `phys` is a DMA frame owned by the driver; `index` < 1024.
    unsafe { (phys_to_virt(phys) as *mut u32).add(index) }
}

/// A producer ring (command ring or transfer ring): one segment that loops through a Link TRB.
pub struct Ring {
    pub base: u64,
    index: usize,
    cycle: bool,
}

impl Ring {
    fn new() -> Option<Ring> {
        let base = dma_frame()?;
        // Last TRB: link back to the start, toggling the cycle bit.
        let link = TRBS - 1;
        // SAFETY: inside the ring's own frame.
        unsafe {
            mem32(base, link * 4).write_volatile(base as u32);
            mem32(base, link * 4 + 1).write_volatile((base >> 32) as u32);
            mem32(base, link * 4 + 3).write_volatile(TRB_LINK << 10 | 1 << 1); // toggle cycle
        }
        Some(Ring { base, index: 0, cycle: true })
    }

    /// Writes one TRB (the cycle bit is added here); returns its physical address.
    fn push(&mut self, trb: [u32; 4]) -> u64 {
        let addr = self.base + self.index as u64 * 16;
        // SAFETY: inside the ring's own frame; the controller does not own this slot (cycle bit not set yet).
        unsafe {
            for i in 0..3 {
                mem32(self.base, self.index * 4 + i).write_volatile(trb[i]);
            }
            mem32(self.base, self.index * 4 + 3).write_volatile(trb[3] | self.cycle as u32);
        }
        self.index += 1;
        if self.index == TRBS - 1 {
            // Hand the link TRB to the controller with the current cycle bit, then wrap.
            // SAFETY: the link TRB is inside the ring's frame.
            unsafe {
                let ctrl = mem32(self.base, (TRBS - 1) * 4 + 3);
                ctrl.write_volatile((ctrl.read_volatile() & !1) | self.cycle as u32);
            }
            self.index = 0;
            self.cycle = !self.cycle;
        }
        addr
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UsbDevice {
    #[allow(dead_code)] // used by the keyboard and storage drivers (P3-06, P3-07)
    pub slot: u8,
    pub port: u8,
    pub speed: u8,
    pub vendor: u16,
    pub product: u16,
    /// Class, subclass and protocol of the first interface.
    pub class: (u8, u8, u8),
}

impl UsbDevice {
    pub fn kind(&self) -> &'static str {
        match self.class {
            (0x03, 0x01, 0x01) => "tastatura (HID)",
            (0x03, 0x01, 0x02) => "mouse (HID)",
            (0x03, _, _) => "dispozitiv HID",
            (0x08, 0x06, 0x50) => "stocare (Mass Storage)",
            (0x08, _, _) => "stocare",
            (0x09, _, _) => "hub",
            (0x0E, _, _) => "camera video",
            (0xE0, _, _) => "bluetooth/wireless",
            _ => "alt dispozitiv",
        }
    }
}

pub struct Xhci {
    op: u64,
    runtime: u64,
    doorbells: u64,
    ports: u8,
    context_size: usize,
    dcbaa: u64,
    commands: Ring,
    events: u64,
    event_index: usize,
    event_cycle: bool,
    /// Transfer ring of endpoint 0 for each slot.
    ep0: Vec<(u8, Ring)>,
    pub devices: Vec<UsbDevice>,
}

impl Xhci {
    fn port_reg(&self, port: u8) -> u64 {
        self.op + 0x400 + 0x10 * (port as u64 - 1)
    }

    /// Takes the next event from the event ring, if the controller has written one.
    fn next_event(&mut self) -> Option<[u32; 4]> {
        let i = self.event_index;
        // SAFETY: the event ring frame belongs to the driver; slot i < TRBS.
        let ctrl = unsafe { mem32(self.events, i * 4 + 3).read_volatile() };
        if (ctrl & 1 != 0) != self.event_cycle {
            return None;
        }
        // SAFETY: as above.
        let trb = unsafe { [0, 1, 2].map(|k| mem32(self.events, i * 4 + k).read_volatile()) };
        self.event_index += 1;
        if self.event_index == TRBS {
            self.event_index = 0;
            self.event_cycle = !self.event_cycle;
        }
        // Tell the controller how far we read (and clear the "event handler busy" bit).
        wr64(self.runtime + 0x20 + 0x18, (self.events + self.event_index as u64 * 16) | 1 << 3);
        Some([trb[0], trb[1], trb[2], ctrl])
    }

    /// Waits for an event of type `kind` (other events, like port changes, are skipped).
    fn wait_event(&mut self, kind: u32) -> Option<[u32; 4]> {
        for _ in 0..SPIN_LIMIT {
            if let Some(e) = self.next_event() {
                if (e[3] >> 10) & 0x3F == kind {
                    return Some(e);
                }
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Runs one command; returns the completion event.
    fn command(&mut self, trb: [u32; 4]) -> Result<[u32; 4], u32> {
        self.commands.push(trb);
        wr(self.doorbells, 0);
        let e = self.wait_event(EVT_COMMAND_DONE).ok_or(0u32)?;
        let code = e[2] >> 24;
        if code == CC_SUCCESS { Ok(e) } else { Err(code) }
    }

    /// Control transfer on endpoint 0 of `slot` with an IN data stage into `buf` (physical, <= 4 KiB).
    fn control_in(&mut self, slot: u8, request: [u8; 8], buf: u64, len: u16) -> Result<(), u32> {
        let ring = &mut self.ep0.iter_mut().find(|(s, _)| *s == slot).ok_or(0u32)?.1;
        let setup = u64::from_le_bytes(request);
        ring.push([setup as u32, (setup >> 32) as u32, 8, TRB_SETUP << 10 | 1 << 6 | 3 << 16]); // IDT, IN data
        ring.push([buf as u32, (buf >> 32) as u32, len as u32, TRB_DATA << 10 | 1 << 16]); // IN
        ring.push([0, 0, 0, TRB_STATUS << 10 | 1 << 5]); // OUT status, interrupt on completion
        wr(self.doorbells + 4 * slot as u64, 1); // endpoint 0 = DCI 1
        let e = self.wait_event(EVT_TRANSFER).ok_or(0u32)?;
        let code = e[2] >> 24;
        if code == CC_SUCCESS || code == CC_SHORT_PACKET { Ok(()) } else { Err(code) }
    }

    /// Context `index` (0 = slot / input control, 1 = slot in an input context, ...) inside a context frame.
    fn ctx(&self, frame: u64, index: usize) -> *mut u32 {
        mem32(frame, index * self.context_size / 4)
    }

    /// Resets a connected port and gives the device an address. Returns its slot.
    fn attach(&mut self, port: u8) -> Option<UsbDevice> {
        let reg = self.port_reg(port);
        let sc = rd(reg);
        if sc & 1 == 0 {
            return None; // nothing connected
        }
        if sc & (1 << 1) == 0 {
            // Not enabled yet (USB 2 ports need a reset): PP | PR, then wait for "reset change".
            wr(reg, 1 << 9 | 1 << 4);
            if !wait(|| rd(reg) & (1 << 21) != 0) {
                crate::warn!("usb: portul {} nu se reseteaza", port);
                return None;
            }
            wr(reg, 1 << 9 | 1 << 21 | 1 << 17); // clear the change bits
            if !wait(|| rd(reg) & (1 << 1) != 0) {
                return None;
            }
        }
        let speed = ((rd(reg) >> 10) & 0xF) as u8;
        let slot = (self.command([0, 0, 0, TRB_ENABLE_SLOT << 10]).ok()?[3] >> 24) as u8;

        // Device context (output) and input context for Address Device.
        let output = dma_frame()?;
        let input = dma_frame()?;
        let ring = Ring::new()?;
        // SAFETY: DCBAA has 256 entries of 8 bytes in its frame; slot < 256.
        unsafe { (phys_to_virt(self.dcbaa) as *mut u64).add(slot as usize).write_volatile(output) };
        let max_packet: u32 = match speed {
            4 => 512, // SuperSpeed
            3 => 64,  // High Speed
            _ => 8,   // Full/Low Speed: 8 to start with, corrected after the first descriptor
        };
        // SAFETY: input context frame owned by the driver; all offsets are inside it.
        unsafe {
            self.ctx(input, 0).add(1).write_volatile(0b11); // add slot context and endpoint 0
            let slot_ctx = self.ctx(input, 1);
            slot_ctx.write_volatile(1 << 27 | (speed as u32) << 20); // one context entry, speed
            slot_ctx.add(1).write_volatile((port as u32) << 16); // root hub port
            let ep0 = self.ctx(input, 2);
            ep0.add(1).write_volatile(max_packet << 16 | 4 << 3 | 3 << 1); // control endpoint, 3 retries
            ep0.add(2).write_volatile(ring.base as u32 | 1); // dequeue pointer, cycle state 1
            ep0.add(3).write_volatile((ring.base >> 32) as u32);
            ep0.add(4).write_volatile(8); // average TRB length
        }
        self.ep0.push((slot, ring));
        if let Err(code) = self.command([input as u32, (input >> 32) as u32, 0, TRB_ADDRESS_DEVICE << 10 | (slot as u32) << 24]) {
            crate::warn!("usb: portul {}: Address Device a esuat (cod {})", port, code);
            return None;
        }

        // Device descriptor: first 8 bytes (to learn the real max packet size), then all 18.
        let buf = dma_frame()?;
        let get = |kind: u16, len: u16| [0x80u8, 6, 0, (kind >> 8) as u8, 0, 0, len as u8, (len >> 8) as u8];
        self.control_in(slot, get(0x0100, 8), buf, 8).ok()?;
        // SAFETY: the buffer frame belongs to the driver; the transfer finished.
        let mps0 = unsafe { *phys_to_virt(buf).add(7) } as u32;
        if speed < 3 && mps0 != max_packet && mps0 != 0 {
            // SAFETY: input context frame owned by the driver.
            unsafe {
                self.ctx(input, 0).add(1).write_volatile(0b10); // evaluate endpoint 0 only
                let ep0 = self.ctx(input, 2).add(1);
                ep0.write_volatile((ep0.read_volatile() & 0xFFFF) | mps0 << 16);
            }
            self.command([input as u32, (input >> 32) as u32, 0, TRB_EVALUATE_CONTEXT << 10 | (slot as u32) << 24]).ok()?;
        }
        self.control_in(slot, get(0x0100, 18), buf, 18).ok()?;
        // SAFETY: as above; 18 bytes were written.
        let d = unsafe { core::slice::from_raw_parts(phys_to_virt(buf), 18) };
        let (vendor, product) = (u16::from_le_bytes([d[8], d[9]]), u16::from_le_bytes([d[10], d[11]]));
        let mut class = (d[4], d[5], d[6]);

        // Configuration descriptor: the device class is often in the first interface.
        self.control_in(slot, get(0x0200, 9), buf, 9).ok()?;
        // SAFETY: as above.
        let total = u16::from_le_bytes(unsafe { [*phys_to_virt(buf).add(2), *phys_to_virt(buf).add(3)] }).min(512);
        self.control_in(slot, get(0x0200, total), buf, total).ok()?;
        // SAFETY: as above; `total` bytes were requested (fewer may arrive: the rest stays zero).
        let cfg = unsafe { core::slice::from_raw_parts(phys_to_virt(buf), total as usize) };
        let mut i = 0;
        while i + 2 <= cfg.len() && cfg[i] >= 2 {
            if cfg[i + 1] == 4 && i + 9 <= cfg.len() && class.0 == 0 {
                class = (cfg[i + 5], cfg[i + 6], cfg[i + 7]); // interface: class, subclass, protocol
            }
            i += cfg[i] as usize;
        }
        frame::free_frame(buf);
        frame::free_frame(input);
        Some(UsbDevice { slot, port, speed, vendor, product, class })
    }
}

/// Starts the first XHCI controller and finds the devices connected to its ports.
pub fn init() -> Option<Xhci> {
    let dev = crate::pci::find(0x0C, 0x03, 0x30)?;
    dev.enable();
    let base = paging::map_mmio(dev.bar(0)?, 0x10000).ok()?;
    let caplen = rd(base) & 0xFF;
    let hcs1 = rd(base + 0x04);
    let hcs2 = rd(base + 0x08);
    let hcc1 = rd(base + 0x10);
    let op = base + caplen as u64;
    let runtime = base + (rd(base + 0x18) & !0x1F) as u64;
    let doorbells = base + (rd(base + 0x14) & !0x3) as u64;

    // Take the controller from the firmware (USB Legacy Support capability).
    let mut cap = base + (((hcc1 >> 16) & 0xFFFF) as u64) * 4;
    if cap != base {
        loop {
            let v = rd(cap);
            if v & 0xFF == 1 {
                wr(cap, v | 1 << 24);
                wait(|| rd(cap) & (1 << 16) == 0);
                break;
            }
            let next = (v >> 8) & 0xFF;
            if next == 0 {
                break;
            }
            cap += next as u64 * 4;
        }
    }

    // Stop and reset.
    wr(op, rd(op) & !1);
    if !wait(|| rd(op + 0x04) & 1 != 0) {
        crate::warn!("usb: controlerul nu se opreste");
        return None;
    }
    wr(op, 1 << 1);
    if !wait(|| rd(op) & (1 << 1) == 0 && rd(op + 0x04) & (1 << 11) == 0) {
        crate::warn!("usb: controlerul nu termina resetarea");
        return None;
    }

    let slots = (hcs1 & 0xFF).min(32);
    wr(op + 0x38, slots);
    let dcbaa = dma_frame()?;
    let scratch = ((hcs2 >> 27) & 0x1F) | ((hcs2 >> 21) & 0x1F) << 5;
    if scratch > 0 {
        let array = dma_frame()?;
        for i in 0..scratch.min(512) as usize {
            let page = dma_frame()?;
            // SAFETY: the scratchpad array frame holds 512 pointers; i < 512.
            unsafe { (phys_to_virt(array) as *mut u64).add(i).write_volatile(page) };
        }
        // SAFETY: entry 0 of the DCBAA points to the scratchpad array.
        unsafe { (phys_to_virt(dcbaa) as *mut u64).write_volatile(array) };
    }
    wr64(op + 0x30, dcbaa);
    let commands = Ring::new()?;
    wr64(op + 0x18, commands.base | 1);

    // Event ring: one segment, described by a one-entry segment table.
    let events = dma_frame()?;
    let erst = dma_frame()?;
    // SAFETY: the ERST frame belongs to the driver: entry 0 = segment address and size.
    unsafe {
        (phys_to_virt(erst) as *mut u64).write_volatile(events);
        (phys_to_virt(erst) as *mut u32).add(2).write_volatile(TRBS as u32);
    }
    let ir0 = runtime + 0x20;
    wr(ir0 + 0x08, 1);
    wr64(ir0 + 0x18, events);
    wr64(ir0 + 0x10, erst);

    wr(op, rd(op) | 1); // run
    if !wait(|| rd(op + 0x04) & 1 == 0) {
        crate::warn!("usb: controlerul nu porneste");
        return None;
    }
    let mut x = Xhci {
        op,
        runtime,
        doorbells,
        ports: (hcs1 >> 24) as u8,
        context_size: if hcc1 & (1 << 2) != 0 { 64 } else { 32 },
        dcbaa,
        commands,
        events,
        event_index: 0,
        event_cycle: true,
        ep0: Vec::new(),
        devices: Vec::new(),
    };
    // USB 3 ports finish link training on their own; give them a moment after the start.
    crate::sched::sleep_ms(50);
    for port in 1..=x.ports {
        if let Some(d) = x.attach(port) {
            crate::info!(
                "usb: port {}: {:04x}:{:04x} {} (clasa {:02x}:{:02x}:{:02x}, viteza {})",
                d.port, d.vendor, d.product, d.kind(), d.class.0, d.class.1, d.class.2, d.speed
            );
            x.devices.push(d);
        }
    }
    crate::info!("usb: controler XHCI pornit, {} porturi, {} dispozitive", x.ports, x.devices.len());
    Some(x)
}
