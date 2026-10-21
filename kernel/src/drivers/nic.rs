//! Network cards: one interface for all our drivers (Intel e1000/e1000e, Realtek RTL8168), used by the TCP/IP
//! stack (`net/`), the network system calls and the shell.

use alloc::{boxed::Box, vec::Vec};

/// A network card that sends and receives raw Ethernet frames (by polling).
pub trait Card: Send {
    /// Sends one frame (destination, source, type, payload; at most 1514 bytes).
    fn send(&mut self, frame: &[u8]) -> bool;
    /// The next received frame, if one has arrived.
    fn receive(&mut self) -> Option<Vec<u8>>;
    fn mac(&self) -> [u8; 6];
    fn name(&self) -> &'static str;
    /// Frames sent and received since start-up.
    fn counters(&self) -> (u64, u64);
    /// One line about the link (speed, duplex, chip status), if the driver knows it; for `net` in the shell.
    fn link_info(&mut self) -> Option<alloc::string::String> {
        None
    }
}

/// The network card found at start-up.
pub static NIC: crate::sched::Mutex<Option<Box<dyn Card>>> = crate::sched::Mutex::new(None);

/// Starts the first supported card (Intel first, then Realtek) and keeps it in `NIC`.
pub fn init_global() {
    let card: Option<Box<dyn Card>> = match super::e1000::init() {
        Some(n) => Some(Box::new(n)),
        None => super::rtl8168::init().map(|n| Box::new(n) as Box<dyn Card>),
    };
    *NIC.lock() = card;
}

/// The name of the driver for a PCI network card, if we have one.
pub fn driver_for(vendor: u16, device: u16) -> Option<&'static str> {
    if super::e1000::supports(vendor, device) {
        Some("e1000")
    } else if super::rtl8168::supports(vendor, device) {
        Some("rtl8168")
    } else {
        None
    }
}
