//! Hardware sources of entropy for the kernel's random generator
//! (`crate::random`): the RDSEED and RDRAND instructions where the CPU has
//! them, and the jitter of the timestamp counter.
//!
//! RDSEED (CPUID.(EAX=07H,ECX=0):EBX[18]) returns output of the CPU's
//! entropy source, RDRAND (CPUID.01H:ECX[30]) output of a generator seeded
//! from it; both may fail (CF = 0) when the source is drained and are
//! retried a few times (Intel SDM vol. 1, "Random Number Generator
//! Instructions"; vol. 2, RDRAND / RDSEED).

use super::cpu;
use core::arch::x86_64::{__cpuid, __cpuid_count};

/// Whether the CPU has RDRAND.
pub fn has_rdrand() -> bool {
    __cpuid(1).ecx & (1 << 30) != 0
}

/// Whether the CPU has RDSEED.
pub fn has_rdseed() -> bool {
    __cpuid(0).eax >= 7 && __cpuid_count(7, 0).ebx & (1 << 18) != 0
}

/// One RDRAND result, if the CPU has the instruction and it delivers
/// within ten tries.
pub fn rdrand64() -> Option<u64> {
    if !has_rdrand() {
        return None;
    }
    for _ in 0..10 {
        let (value, ok): (u64, u8);
        // SAFETY: CPUID says the CPU has RDRAND; it writes only the
        // destination register and the flags.
        unsafe {
            core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) value, ok = out(reg_byte) ok, options(nomem, nostack));
        }
        if ok != 0 {
            return Some(value);
        }
    }
    None
}

/// One RDSEED result, like [`rdrand64`]. RDSEED fails more often under
/// load, so it is tried more times.
pub fn rdseed64() -> Option<u64> {
    if !has_rdseed() {
        return None;
    }
    for _ in 0..100 {
        let (value, ok): (u64, u8);
        // SAFETY: CPUID says the CPU has RDSEED; it writes only the
        // destination register and the flags.
        unsafe {
            core::arch::asm!("rdseed {v}", "setc {ok}", v = out(reg) value, ok = out(reg_byte) ok, options(nomem, nostack));
        }
        if ok != 0 {
            return Some(value);
        }
        core::hint::spin_loop();
    }
    None
}

/// The timestamp counter.
pub fn tsc() -> u64 {
    cpu::rdtsc()
}

/// Timing jitter: `out.len()` samples of the low byte of the time a
/// small, data-dependent loop over `scratch` takes, as measured by the
/// TSC. Interrupts, caches and (in a virtual machine) the host's
/// scheduling make the low bits unpredictable; how much entropy they hold
/// is not known, so callers credit it conservatively.
pub fn jitter(out: &mut [u8], scratch: &mut [u64; 64]) {
    let mut prev = tsc();
    let mut acc = prev;
    for (i, b) in out.iter_mut().enumerate() {
        // Work whose duration depends on what came before.
        let rounds = 1 + (acc & 7) as usize;
        for r in 0..rounds {
            let j = (acc as usize).wrapping_add(r * 13) % scratch.len();
            scratch[j] = scratch[j].rotate_left(7) ^ acc ^ i as u64;
            acc = acc.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(scratch[j]);
        }
        let now = tsc();
        let delta = now.wrapping_sub(prev);
        prev = now;
        acc ^= delta;
        *b = (delta ^ (delta >> 8)) as u8;
    }
}
