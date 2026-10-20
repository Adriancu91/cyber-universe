//! Drivers that live inside the kernel (only what is needed to boot and debug).
//! Everything else becomes a module later (see docs/ARCHITECTURE.md).

pub mod ahci;
pub mod e1000;
pub mod keyboard;
pub mod nvme;
pub mod serial;
pub mod usb;
