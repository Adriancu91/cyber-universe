//! Random bytes for programs (P5-16: TLS keys). RDRAND when the processor has it; always mixed with the time-stamp
//! counter, the timer ticks and a running state through SHA-256, so the output never repeats even without RDRAND.
//! Without RDRAND the unpredictability rests on TSC timing only: good enough for tests, weak against a determined
//! attacker (noted in docs/STATE.md and tls/README.md).

use crate::sched::Mutex;
use cyber_crypto::{Digest, sha2::Sha256};

static STATE: Mutex<([u8; 32], u64)> = Mutex::new(([0; 32], 0));

fn has_rdrand() -> bool {
    let r = core::arch::x86_64::__cpuid(1);
    r.ecx & (1 << 30) != 0
}

fn rdrand() -> Option<u64> {
    if !has_rdrand() {
        return None;
    }
    // SAFETY: the processor reported RDRAND support just above.
    unsafe { rdrand_raw() }
}

#[target_feature(enable = "rdrand")]
fn rdrand_raw() -> Option<u64> {
    for _ in 0..10 {
        let mut v = 0u64;
        if core::arch::x86_64::_rdrand64_step(&mut v) == 1 {
            return Some(v);
        }
    }
    None
}

/// Fills `out` with random bytes.
pub fn fill(out: &mut [u8]) {
    let mut st = STATE.lock();
    for chunk in out.chunks_mut(32) {
        let mut h = Sha256::new();
        h.update(&st.0);
        st.1 += 1;
        h.update(&st.1.to_le_bytes());
        h.update(&crate::arch::x86_64::rdtsc().to_le_bytes());
        h.update(&crate::arch::x86_64::apic::ticks().to_le_bytes());
        if let Some(v) = rdrand() {
            h.update(&v.to_le_bytes());
        }
        let block = h.finish();
        // The next state comes from the block by a different path, so knowing an output says nothing of the next.
        let mut n = Sha256::new();
        n.update(&block);
        n.update(b"urmatoarea stare");
        n.update(&crate::arch::x86_64::rdtsc().to_le_bytes());
        st.0 = n.finish();
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
}

/// Whether the processor gives hardware random numbers (shown by the https test).
#[cfg_attr(not(feature = "test-https"), allow(dead_code))]
pub fn hardware() -> bool {
    has_rdrand()
}
