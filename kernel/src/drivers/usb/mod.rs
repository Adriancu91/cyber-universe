//! USB: the XHCI host controller and the devices connected to it.

pub mod hid;
pub mod storage;
pub mod xhci;

use alloc::vec::Vec;

static CONTROLLER: crate::sched::Mutex<Option<xhci::Xhci>> = crate::sched::Mutex::new(None);
static KEYBOARDS: crate::sched::Mutex<Vec<hid::Keyboard>> = crate::sched::Mutex::new(Vec::new());
/// Sticks found at start-up, until `take_disk` hands them to the file system.
static DISKS: crate::sched::Mutex<Vec<storage::UsbDisk>> = crate::sched::Mutex::new(Vec::new());

/// How often the USB keyboards are checked for new reports.
const POLL_MS: u64 = 8;

/// Starts the USB controller (if there is one), enumerates the connected devices and starts the
/// keyboards (with a thread that collects their key presses).
pub fn init() {
    let Some(mut x) = xhci::init() else { return };
    let devices = x.devices.clone();
    let mut keyboards = Vec::new();
    for d in devices.iter().filter(|d| d.class == (0x03, 0x01, 0x01) || d.class == (0x03, 0x01, 0x02)) {
        match hid::Keyboard::start(&mut x, d) {
            Some(k) => {
                crate::info!("usb: {} de pe portul {} pornit(a)", d.kind(), d.port);
                keyboards.push(k);
            }
            None => crate::warn!("usb: {} de pe portul {} nu porneste", d.kind(), d.port),
        }
    }
    let mut disks = Vec::new();
    for d in devices.iter().filter(|d| d.class == (0x08, 0x06, 0x50)) {
        match storage::UsbDisk::start(&mut x, d) {
            Some(disk) => disks.push(disk),
            None => {
                crate::warn!("usb: stick-ul de pe portul {} nu porneste", d.port);
                if crate::fs::failure("/usb").is_none() {
                    crate::fs::note_failure("/usb", "stick-ul nu porneste (configurare USB)");
                }
            }
        }
    }
    *DISKS.lock() = disks;
    *CONTROLLER.lock() = Some(x);
    if !keyboards.is_empty() {
        *KEYBOARDS.lock() = keyboards;
        crate::sched::spawn_system(poll_loop);
    }
}

/// Runs forever: hands finished transfers to the keyboard drivers.
fn poll_loop() {
    loop {
        {
            let mut ctrl = CONTROLLER.lock();
            let x = ctrl.as_mut().expect("usb: controlerul lipseste");
            let mut keyboards = KEYBOARDS.lock();
            for k in keyboards.iter_mut() {
                k.poll(x);
            }
        }
        crate::sched::sleep_ms(POLL_MS);
    }
}

/// The USB devices found at start-up.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell (`lsusb`)
pub fn devices() -> Vec<xhci::UsbDevice> {
    CONTROLLER.lock().as_ref().map(|x| x.devices.clone()).unwrap_or_default()
}

/// Runs `f` with the controller (for the device drivers). `None` if there is no controller.
pub fn with_controller<R>(f: impl FnOnce(&mut xhci::Xhci) -> R) -> Option<R> {
    CONTROLLER.lock().as_mut().map(f)
}

/// The first USB stick found at start-up (each one is handed out once).
pub fn take_disk() -> Option<storage::UsbDisk> {
    DISKS.lock().pop()
}
