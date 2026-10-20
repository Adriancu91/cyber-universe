//! USB: the XHCI host controller and the devices connected to it.

pub mod hid;
pub mod xhci;

use alloc::vec::Vec;

static CONTROLLER: crate::sched::Mutex<Option<xhci::Xhci>> = crate::sched::Mutex::new(None);
static KEYBOARDS: crate::sched::Mutex<Vec<hid::Keyboard>> = crate::sched::Mutex::new(Vec::new());

/// How often the USB keyboards are checked for new reports.
const POLL_MS: u64 = 8;

/// Starts the USB controller (if there is one), enumerates the connected devices and starts the
/// keyboards (with a thread that collects their key presses).
pub fn init() {
    let Some(mut x) = xhci::init() else { return };
    let devices = x.devices.clone();
    let mut keyboards = Vec::new();
    for d in devices.iter().filter(|d| d.class == (0x03, 0x01, 0x01)) {
        match hid::Keyboard::start(&mut x, d) {
            Some(k) => {
                crate::info!("usb: tastatura de pe portul {} pornita", d.port);
                keyboards.push(k);
            }
            None => crate::warn!("usb: tastatura de pe portul {} nu porneste", d.port),
        }
    }
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
            let (transfers, n) = x.transfers();
            for &t in &transfers[..n] {
                for k in keyboards.iter_mut() {
                    k.on_transfer(x, t);
                }
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
