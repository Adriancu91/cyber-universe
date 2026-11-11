//! Drivers that live inside the kernel (only what is needed to boot and debug).
//! Everything else becomes a module later (see docs/ARCHITECTURE.md).

pub mod ahci;
pub mod e1000;
pub mod gpu;
pub mod gpu_i6;
pub mod gpu_i7;
pub mod gpu_rec; // P6-74 (C-103): `gpu recupereaza`
pub mod gpu_i8;
pub mod gpu_golden;
pub mod gpu_i10;
pub mod gpu_i11;
pub mod gpu_i14;
pub mod gpu_tur;
pub mod keyboard;
pub mod mouse;
pub mod nic;
pub mod nvme;
pub mod rtc;
pub mod rtl8168;
pub mod serial;
pub mod usb;
