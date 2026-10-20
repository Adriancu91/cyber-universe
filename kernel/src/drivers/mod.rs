//! Drivers that live inside the kernel (only what is needed to boot and debug).
//! Everything else becomes a module later (see docs/ARCHITECTURE.md).

pub mod keyboard;
pub mod serial;
