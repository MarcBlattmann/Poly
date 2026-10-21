//! Time and system information (ADR 0076): the services our `kernelbase`
//! needs for `GetSystemTimeAsFileTime`, `GetTickCount(64)`,
//! `QueryPerformanceCounter`/`Frequency` and `GetSystemInfo`.
//!
//! - `NtQuerySystemTime(SystemTime*)` (Learn, `winternl.h`): the system
//!   time as a `FILETIME` value, 100 ns units since 1601-01-01 UTC.
//! - `NtQueryPerformanceCounter(PerformanceCounter*, PerformanceFrequency*)`
//!   (Learn): the counter and, if asked, its frequency. Poly's counter is
//!   the time since boot in 100 ns units, so the frequency is 10 MHz, the
//!   value Learn's "Acquiring high-resolution time stamps" describes for
//!   current Windows; it never changes while the system runs.
//! - `NtQuerySystemInformation(SystemInformationClass, SystemInformation,
//!   Length, ReturnLength*)` (Learn, `winternl.h`): only
//!   `SystemBasicInformation` (0), whose one documented field is
//!   `NumberOfProcessors` (a `CCHAR` after `Reserved1[24]` and
//!   `Reserved2[4]` pointers: offset 56; the structure is 64 bytes on x64).
//!   The reserved fields stay zero. Every other class is
//!   `STATUS_INVALID_INFO_CLASS` (to be confirmed by the spec agent).

use crate::abi;
use crate::kernel::NtKernel;
use poly_abi::SyscallRegs;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_INFO_LENGTH_MISMATCH, STATUS_INVALID_INFO_CLASS, STATUS_SUCCESS};

/// The performance counter's frequency: 100 ns ticks.
pub const PERFORMANCE_FREQUENCY: u64 = 10_000_000;
/// `SystemBasicInformation`.
pub const SYSTEM_BASIC_INFORMATION: u32 = 0;
/// `sizeof(SYSTEM_BASIC_INFORMATION)` on x64 (`winternl.h`).
pub const SYSTEM_BASIC_INFORMATION_SIZE: usize = 64;
/// `SYSTEM_BASIC_INFORMATION.NumberOfProcessors`.
pub const NUMBER_OF_PROCESSORS_OFFSET: usize = 56;

type Result<T> = core::result::Result<T, NtStatus>;

/// `NtQuerySystemTime`.
pub fn query_system_time<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let out = abi::arg(k, regs, 0)?;
    let now = k.system_time();
    abi::write_u64(k, out, now)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryPerformanceCounter`.
pub fn query_performance_counter<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [counter, frequency] = abi::args::<K, 2>(k, regs)?;
    let now = k.monotonic_100ns();
    abi::write_u64(k, counter, now)?;
    if frequency != 0 {
        abi::write_u64(k, frequency, PERFORMANCE_FREQUENCY)?;
    }
    Ok(STATUS_SUCCESS)
}

/// `NtQuerySystemInformation`.
pub fn query_system_information<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [class, buf, length, ret] = abi::args::<K, 4>(k, regs)?;
    if class as u32 != SYSTEM_BASIC_INFORMATION {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let mut info = [0u8; SYSTEM_BASIC_INFORMATION_SIZE];
    info[NUMBER_OF_PROCESSORS_OFFSET] = k.processor_count().clamp(1, 64) as u8;
    abi::write_opt_u32(k, ret, info.len() as u32)?;
    if (length as u32 as usize) != info.len() {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, &info)?;
    Ok(STATUS_SUCCESS)
}

#[cfg(test)]
mod tests {
    use crate::services::Service;
    use crate::tests::Fake;
    use poly_ntobj::status::*;

    #[test]
    fn time_and_counter() {
        let mut f = Fake::new();
        let t = f.alloc(8);
        assert_eq!(f.call(Service::QuerySystemTime, &[t]), STATUS_SUCCESS);
        let (c, q) = (f.alloc(8), f.alloc(8));
        assert_eq!(f.call(Service::QueryPerformanceCounter, &[c, q]), STATUS_SUCCESS);
        assert_eq!(f.u64_at(q), super::PERFORMANCE_FREQUENCY);
        // The frequency is optional.
        assert_eq!(f.call(Service::QueryPerformanceCounter, &[c, 0]), STATUS_SUCCESS);
        assert_eq!(f.call(Service::QuerySystemTime, &[0x10]), STATUS_ACCESS_VIOLATION);
    }

    #[test]
    fn basic_information() {
        let mut f = Fake::new();
        let (buf, ret) = (f.alloc(64), f.alloc(4));
        assert_eq!(f.call(Service::QuerySystemInformation, &[0, buf, 64, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(ret), 64);
        assert_eq!(f.bytes_at(buf + 56, 1)[0], 1);
        assert_eq!(f.call(Service::QuerySystemInformation, &[0, buf, 32, ret]), STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(f.u32_at(ret), 64);
        assert_eq!(f.call(Service::QuerySystemInformation, &[5, buf, 64, ret]), STATUS_INVALID_INFO_CLASS);
    }
}
