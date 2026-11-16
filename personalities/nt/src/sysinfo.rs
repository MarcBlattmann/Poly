//! Time and system information (ADR 0079): the services our `kernelbase`
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

/// `PolyRandomBytes(Buffer, Length)` (ADR 0206): random bytes from the kernel's generator into the
/// caller's buffer, in pieces of 4 KiB; `STATUS_NOT_IMPLEMENTED` from a kernel without one. The
/// buffer must be writable (`STATUS_ACCESS_VIOLATION`).
pub fn random_bytes<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [buffer, length] = abi::args::<K, 2>(k, regs)?;
    let mut done = 0u64;
    let mut piece = [0u8; 4096];
    while done < length {
        let n = (length - done).min(piece.len() as u64) as usize;
        if !k.random_bytes(&mut piece[..n]) {
            return Err(NOT_IMPLEMENTED);
        }
        abi::write(k, buffer + done, &piece[..n])?;
        done += n as u64;
    }
    Ok(STATUS_SUCCESS)
}

/// `PolyPhysicalMemory(Out*)` (ADR 0206): the physical memory and what is available, as two 64-bit
/// values in bytes; `STATUS_NOT_IMPLEMENTED` from a kernel that does not count it.
pub fn physical_memory<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let out = abi::arg(k, regs, 0)?;
    let (total, available) = k.physical_memory().ok_or(NOT_IMPLEMENTED)?;
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&total.to_le_bytes());
    b[8..].copy_from_slice(&available.to_le_bytes());
    abi::write(k, out, &b)?;
    Ok(STATUS_SUCCESS)
}

/// `STATUS_NOT_IMPLEMENTED` (`ntstatus.h`).
const NOT_IMPLEMENTED: NtStatus = NtStatus(0xC000_0002);

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
    fn locally_unique_ids_differ() {
        let mut f = Fake::new();
        let (a, b) = (f.alloc(8), f.alloc(8));
        assert_eq!(f.call(Service::AllocateLocallyUniqueId, &[a]), STATUS_SUCCESS);
        assert_eq!(f.call(Service::AllocateLocallyUniqueId, &[b]), STATUS_SUCCESS);
        let (a, b) = (f.u64_at(a), f.u64_at(b));
        assert!(a != 0 && b != 0 && a != b, "{a:#x} {b:#x}");
        // An address the caller cannot write.
        assert_eq!(f.call(Service::AllocateLocallyUniqueId, &[0x10]), STATUS_ACCESS_VIOLATION);
    }

    #[test]
    fn random_bytes_fill_the_buffer_in_pieces_and_refuse_an_unwritable_one() {
        let mut f = Fake::new();
        let buf = f.alloc(10_000);
        f.mem.write_bytes(buf, &[0xEE; 10_000]).unwrap();
        assert_eq!(f.call(Service::PolyRandomBytes, &[buf, 10_000]), STATUS_SUCCESS);
        // The fake writes 1, 2, 3 ... in each piece of 4096: every byte was written.
        assert_eq!(f.bytes_at(buf, 3), [1, 2, 3]);
        assert_eq!(f.bytes_at(buf + 4096, 2), [1, 2]);
        assert_eq!(f.bytes_at(buf + 9_999, 1)[0], ((10_000 - 8192 - 1) as u8).wrapping_add(1));
        // Length 0 touches nothing; an address the caller cannot write is refused.
        assert_eq!(f.call(Service::PolyRandomBytes, &[0x10, 0]), STATUS_SUCCESS);
        assert_eq!(f.call(Service::PolyRandomBytes, &[0x10, 8]), STATUS_ACCESS_VIOLATION);
    }

    #[test]
    fn physical_memory_is_two_sizes_and_needs_a_writable_buffer() {
        let mut f = Fake::new();
        let out = f.alloc(16);
        assert_eq!(f.call(Service::PolyPhysicalMemory, &[out]), STATUS_SUCCESS);
        assert_eq!((f.u64_at(out), f.u64_at(out + 8)), (8 << 30, 6 << 30));
        assert_eq!(f.call(Service::PolyPhysicalMemory, &[0x10]), STATUS_ACCESS_VIOLATION);
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

    /// `NtQueryInformationProcess(ProcessTimes)` (ADR 0087): creation, exit,
    /// kernel and user time, as the kernel reports them.
    #[test]
    fn process_times() {
        let mut f = Fake::new();
        let (buf, ret) = (f.alloc(32), f.alloc(4));
        let me = u64::MAX; // NtCurrentProcess()
        assert_eq!(f.call(Service::QueryInformationProcess, &[me, 4, buf, 32, ret]), STATUS_SUCCESS);
        assert_eq!(f.u32_at(ret), 32);
        assert_eq!(f.u64_at(buf), 0x01D9_0000_0000_0000);
        assert_eq!(f.u64_at(buf + 8), 0, "no exit time while it runs");
        assert_eq!((f.u64_at(buf + 16), f.u64_at(buf + 24)), (20_000, 30_000));
        assert_eq!(f.call(Service::QueryInformationProcess, &[me, 4, buf, 16, ret]), STATUS_INFO_LENGTH_MISMATCH);
        assert_eq!(f.call(Service::QueryInformationProcess, &[me, 9, buf, 32, ret]), STATUS_INVALID_INFO_CLASS);
    }
}
