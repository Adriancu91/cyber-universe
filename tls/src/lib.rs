//! Cyber Universe's own TLS 1.3 client (P5-06): one suite (TLS_CHACHA20_POLY1305_SHA256), X25519, certificates
//! checked with crypto/x509. `no_std`, no heap: the caller gives the buffer. Not audited (D-011).
#![cfg_attr(not(test), no_std)]

pub mod client;
pub mod record;

pub use client::{Error, MIN_BUFFER, Tls};
pub use record::Io;
