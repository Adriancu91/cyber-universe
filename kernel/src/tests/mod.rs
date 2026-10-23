//! Long-running self-tests that are not part of every test build.

#[cfg(feature = "test-stress")]
pub mod stress;
#[cfg(feature = "test-fpu")]
pub mod fpu;
#[cfg(feature = "test-smp")]
pub mod smp;
#[cfg(feature = "test-mem")]
pub mod memory;
#[cfg(feature = "test-threads")]
pub mod threads;
#[cfg(feature = "test-kronos-calc")]
pub mod kronos;
