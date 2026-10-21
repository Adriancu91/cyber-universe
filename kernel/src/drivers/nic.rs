//! Network cards: which of our drivers handles a PCI network device (used by `hw` in the shell).

/// The name of the driver for a PCI network card, if we have one.
pub fn driver_for(vendor: u16, device: u16) -> Option<&'static str> {
    super::e1000::supports(vendor, device).then_some("e1000")
}
