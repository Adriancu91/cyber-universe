//! USB: the XHCI host controller and the devices connected to it.

pub mod xhci;

use alloc::vec::Vec;

static CONTROLLER: crate::sched::Mutex<Option<xhci::Xhci>> = crate::sched::Mutex::new(None);

/// Starts the USB controller (if there is one) and enumerates the connected devices.
pub fn init() {
    if let Some(x) = xhci::init() {
        *CONTROLLER.lock() = Some(x);
    }
}

/// The USB devices found at start-up.
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // used by the shell (`lsusb`)
pub fn devices() -> Vec<xhci::UsbDevice> {
    CONTROLLER.lock().as_ref().map(|x| x.devices.clone()).unwrap_or_default()
}
