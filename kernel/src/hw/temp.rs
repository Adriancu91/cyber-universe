//! P6-104 (C-152 point 4): the processor's temperature, by READS ONLY of the thermal MSRs Intel documents
//! (SDM vol. 3B §15.8 "Thermal Monitoring", vol. 4 "Model-Specific Registers"):
//! - IA32_THERM_STATUS (0x19C): the digital readout of the core this code runs on, in degrees below TjMax;
//! - IA32_PACKAGE_THERM_STATUS (0x1B1): the same for the whole package (the hottest core), when CPUID reports it;
//! - MSR_TEMPERATURE_TARGET (0x1A2) bits 23:16: TjMax, the temperature where the processor starts slowing down.
//!
//! An MSR that does not exist raises #GP, which stops the kernel, so every read is gated by CPUID first: Intel only,
//! leaf 6 EAX bit 0 (digital thermal sensor) for 0x19C, bit 6 (package thermal management) for 0x1B1, and family 6
//! model >= 0x1A (Nehalem and later, where 0x1A2 exists) for TjMax. Virtual machines (QEMU, KVM) report neither bit,
//! so nothing is read there and the row says why. No MSR is ever written.

use alloc::{format, string::String};
use core::arch::x86_64::__cpuid;

const IA32_THERM_STATUS: u32 = 0x19C;
const MSR_TEMPERATURE_TARGET: u32 = 0x1A2;
const IA32_PACKAGE_THERM_STATUS: u32 = 0x1B1;
/// Used when TjMax cannot be read (the common value of Intel desktop processors).
const TJMAX_DEFAULT: u32 = 100;
/// Long work stops this many degrees below TjMax.
pub const MARGIN: u32 = 10;

/// What could be read, in degrees Celsius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub tjmax: u32,
    /// Whether `tjmax` was read from the processor (false: `TJMAX_DEFAULT`).
    pub tjmax_read: bool,
    /// The hottest core of the package, if the package sensor exists and its readout is valid.
    pub package: Option<u32>,
    /// The core this ran on (its number in the scheduler) and its temperature, if the readout is valid.
    pub core: Option<(usize, u32)>,
}

impl Reading {
    /// The highest temperature read.
    pub fn hottest(&self) -> Option<u32> {
        match (self.package, self.core.map(|c| c.1)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// The temperature at which long work stops: TjMax − `MARGIN`.
    pub fn limit(&self) -> u32 {
        self.tjmax.saturating_sub(MARGIN)
    }
}

/// The temperature from a THERM_STATUS value (bit 31 = readout valid, bits 22:16 = degrees below TjMax).
pub fn decode(status: u64, tjmax: u32) -> Option<u32> {
    if status & (1 << 31) == 0 {
        return None;
    }
    let below = ((status >> 16) & 0x7F) as u32;
    Some(tjmax.saturating_sub(below))
}

/// TjMax from MSR_TEMPERATURE_TARGET (bits 23:16); 0 is not a real value.
pub fn decode_tjmax(target: u64) -> Option<u32> {
    let t = ((target >> 16) & 0xFF) as u32;
    (t != 0).then_some(t)
}

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: `read` calls this only for the thermal MSRs that CPUID says exist on this processor (see the module
    // comment); rdmsr has no side effects.
    unsafe { core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) }
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Reads the sensors, or says in Romanian why nothing can be read here.
pub fn read() -> Result<Reading, &'static str> {
    let v = __cpuid(0);
    let intel = (v.ebx, v.edx, v.ecx) == (0x756e_6547, 0x4965_6e69, 0x6c65_746e); // "GenuineIntel"
    if !intel {
        return Err("doar procesoarele Intel au senzorii documentati folositi aici");
    }
    if v.eax < 6 {
        return Err("procesorul nu are frunza CPUID 6 (senzori termici)");
    }
    let leaf6 = __cpuid(6).eax;
    let (dts, ptm) = (leaf6 & 1 != 0, leaf6 & (1 << 6) != 0);
    if !dts && !ptm {
        return Err("CPUID nu raporteaza senzorul termic digital (normal intr-o masina virtuala)");
    }
    let sig = __cpuid(1).eax;
    let family = (sig >> 8) & 0xF;
    let model = ((sig >> 4) & 0xF) | (((sig >> 16) & 0xF) << 4);
    let target = if family == 6 && model >= 0x1A { decode_tjmax(rdmsr(MSR_TEMPERATURE_TARGET)) } else { None };
    let tjmax = target.unwrap_or(TJMAX_DEFAULT);
    let core = if dts { decode(rdmsr(IA32_THERM_STATUS), tjmax).map(|t| (crate::sched::current_cpu(), t)) } else { None };
    let package = if ptm { decode(rdmsr(IA32_PACKAGE_THERM_STATUS), tjmax) } else { None };
    Ok(Reading { tjmax, tjmax_read: target.is_some(), package, core })
}

/// One row for `hw` and the cycle report.
pub fn line() -> String {
    match read() {
        Err(why) => format!("temperatura procesorului: necitita ({})", why),
        Ok(r) => {
            let mut s = String::from("temperatura procesorului:");
            match r.package {
                Some(t) => s.push_str(&format!(" pachetul {} C", t)),
                None => s.push_str(" pachetul necitit"),
            }
            match r.core {
                Some((cpu, t)) => s.push_str(&format!(", nucleul {} {} C", cpu, t)),
                None => s.push_str(", nucleul necitit"),
            }
            s.push_str(&format!(
                "; TjMax {} C{}, pragul de oprire a lucrului lung {} C",
                r.tjmax,
                if r.tjmax_read { "" } else { " (presupus, necitit)" },
                r.limit()
            ));
            s
        }
    }
}

/// `Some(reason)` when the processor is at or above the limit (long work must not start or go on); `None` when it is
/// below it or nothing can be read (a machine without sensors is not stopped).
pub fn too_hot() -> Option<String> {
    let r = read().ok()?;
    let t = r.hottest()?;
    (t >= r.limit()).then(|| format!("procesorul are {} C, peste pragul de {} C (TjMax {} C - {})", t, r.limit(), r.tjmax, MARGIN))
}

/// Checks the decoding with known register values (scenario `log` runs every self-test).
pub fn self_test() {
    assert_eq!(decode(0x8840_0000, 100), Some(100 - 0x40), "temperatura: decodarea citirii valide");
    assert_eq!(decode(0x0840_0000, 100), None, "temperatura: citirea invalida trebuie ignorata");
    assert_eq!(decode_tjmax(0x0064_0000), Some(100), "temperatura: TjMax din MSR_TEMPERATURE_TARGET");
    assert_eq!(decode_tjmax(0), None, "temperatura: TjMax 0 nu e o valoare");
    let hot = Reading { tjmax: 100, tjmax_read: true, package: Some(91), core: Some((0, 80)) };
    assert!(hot.hottest() == Some(91) && hot.limit() == 90, "temperatura: pragul");
    crate::info!("test: temperatura procesorului: decodarea si pragul OK; {}", line());
}
