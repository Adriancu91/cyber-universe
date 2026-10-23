//! Long-running self-tests that are not part of every test build.

#[cfg(feature = "test-stress")]
pub mod stress;
#[cfg(feature = "test-fpu")]
pub mod fpu;
