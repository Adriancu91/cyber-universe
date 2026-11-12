//! USB: the XHCI host controller and the devices connected to it.

pub mod hid;
pub mod storage;
pub mod xhci;

use alloc::vec::Vec;

/// Every XHCI controller started, in PCI order (P6-25); `Xhci::index` is the position here.
static CONTROLLERS: crate::sched::Mutex<Vec<xhci::Xhci>> = crate::sched::Mutex::new(Vec::new());
/// Keyboards and mice, with the index of their controller.
static KEYBOARDS: crate::sched::Mutex<Vec<(usize, hid::Keyboard)>> = crate::sched::Mutex::new(Vec::new());
/// Sticks found at start-up, until `take_disk` hands them to the file system.
static DISKS: crate::sched::Mutex<Vec<storage::UsbDisk>> = crate::sched::Mutex::new(Vec::new());

/// How often the USB keyboards are checked for new reports.
const POLL_MS: u64 = 8;
/// How often the root ports are checked for devices plugged in or out (P6-80): every this many polls (~250 ms).
const PORT_EVERY: u32 = 32;

/// Starts every USB controller, enumerates the connected devices and starts the keyboards (with a thread that
/// collects their key presses) and the sticks.
pub fn init() {
    let mut controllers = xhci::init_all();
    let mut keyboards = Vec::new();
    let mut disks = Vec::new();
    for (c, x) in controllers.iter_mut().enumerate() {
        let devices = x.devices.clone();
        for d in devices.iter().filter(|d| d.hid[0].is_some()) {
            start_hid(c, x, d, &mut keyboards);
        }
        for d in devices.iter().filter(|d| d.class == (0x08, 0x06, 0x50)) {
            match storage::UsbDisk::start(x, d) {
                Some(disk) => disks.push(disk),
                None => {
                    crate::warn!("usb: stick-ul de pe portul {} nu porneste", d.location());
                    if crate::fs::failure("/usb").is_none() {
                        crate::fs::note_failure("/usb", "stick-ul nu porneste (configurare USB; vezi log usb)");
                    }
                }
            }
        }
    }
    *DISKS.lock() = disks;
    let any = !controllers.is_empty();
    *CONTROLLERS.lock() = controllers;
    *KEYBOARDS.lock() = keyboards;
    // Always, when there is a controller: a keyboard or mouse may be plugged in later (P6-80).
    if any {
        crate::sched::spawn_system(poll_loop);
    }
}

/// Starts the keyboard and/or mouse interfaces of `d` (on controller `c`) and adds them to `keyboards`.
fn start_hid(c: usize, x: &mut xhci::Xhci, d: &xhci::UsbDevice, keyboards: &mut Vec<(usize, hid::Keyboard)>) {
    match hid::Keyboard::start_all(x, d) {
        Some(ks) => {
            for k in ks {
                crate::info!("usb: {} de pe portul {} pornit(a)", if k.is_mouse() { "mouse (HID)" } else { "tastatura (HID)" }, d.location());
                keyboards.push((c, k));
            }
        }
        None => crate::warn!("usb: {} de pe portul {} nu porneste", d.kind(), d.location()),
    }
}

/// Runs forever: hands finished transfers to the keyboard drivers and, every ~250 ms, starts the keyboards and mice
/// plugged in since the last look and stops the ones unplugged (P6-80).
fn poll_loop() {
    let mut n: u32 = 0;
    loop {
        {
            let mut ctrl = CONTROLLERS.lock();
            let mut keyboards = KEYBOARDS.lock();
            for (c, k) in keyboards.iter_mut() {
                k.poll(&mut ctrl[*c]);
            }
            if n % PORT_EVERY == 0 {
                for (c, x) in ctrl.iter_mut().enumerate() {
                    for change in x.port_changes() {
                        match change {
                            xhci::PortChange::Connected(found) => {
                                for d in found.iter().filter(|d| d.hid[0].is_some()) {
                                    start_hid(c, x, d, &mut keyboards);
                                }
                            }
                            xhci::PortChange::Disconnected(slots) => {
                                let mut i = 0;
                                while i < keyboards.len() {
                                    if keyboards[i].0 == c && slots.contains(&keyboards[i].1.slot()) {
                                        keyboards.swap_remove(i).1.release();
                                    } else {
                                        i += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        n = n.wrapping_add(1);
        crate::sched::sleep_ms(POLL_MS);
    }
}

/// The USB devices found at start-up, on every controller.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell (`lsusb`)
pub fn devices() -> Vec<xhci::UsbDevice> {
    CONTROLLERS.lock().iter().flat_map(|x| x.devices.iter().copied()).collect()
}

/// Runs `f` with controller `index` (for the device drivers). `None` if there is no such controller.
pub fn with_controller<R>(index: u8, f: impl FnOnce(&mut xhci::Xhci) -> R) -> Option<R> {
    CONTROLLERS.lock().get_mut(index as usize).map(f)
}

/// The USB sticks found at start-up, in the order they were found (handed out once).
pub fn take_disks() -> Vec<storage::UsbDisk> {
    core::mem::take(&mut *DISKS.lock())
}
