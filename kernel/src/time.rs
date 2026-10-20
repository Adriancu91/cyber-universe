//! Time since boot. Returns `None` until the system timer exists (task P1-09).

/// Milliseconds since boot, if the timer is running.
pub fn uptime_ms() -> Option<u64> {
    None
}
