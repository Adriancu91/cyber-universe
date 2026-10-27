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

/// Starts every USB controller, enumerates the connected devices and starts the keyboards (with a thread that
/// collects their key presses) and the sticks.
pub fn init() {
    let mut controllers = xhci::init_all();
    let mut keyboards = Vec::new();
    let mut disks = Vec::new();
    for (c, x) in controllers.iter_mut().enumerate() {
        let devices = x.devices.clone();
        for d in devices.iter().filter(|d| d.class == (0x03, 0x01, 0x01) || d.class == (0x03, 0x01, 0x02)) {
            match hid::Keyboard::start(x, d) {
                Some(k) => {
                    crate::info!("usb: {} de pe portul {} pornit(a)", d.kind(), d.location());
                    keyboards.push((c, k));
                }
                None => crate::warn!("usb: {} de pe portul {} nu porneste", d.kind(), d.location()),
            }
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
    *CONTROLLERS.lock() = controllers;
    if !keyboards.is_empty() {
        *KEYBOARDS.lock() = keyboards;
        crate::sched::spawn_system(poll_loop);
    }
}

/// Runs forever: hands finished transfers to the keyboard drivers.
fn poll_loop() {
    loop {
        {
            let mut ctrl = CONTROLLERS.lock();
            let mut keyboards = KEYBOARDS.lock();
            for (c, k) in keyboards.iter_mut() {
                k.poll(&mut ctrl[*c]);
            }
        }
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
