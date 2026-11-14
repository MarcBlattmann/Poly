//! What a process can ask and say about itself beyond its times (ADR 0155, step 9):
//! `GetProcessInformation` / `SetProcessInformation`, the shutdown parameters, the cycle time
//! and the device form of a mapped file's name. Chrome's browser reads its cycle time once a
//! second (a failure is fatal), asks `GetProcessInformation` in the browser and the GPU process
//! and `K32GetMappedFileNameW` for every module it finds (a failure ends a child).
//!
//! Everything here is from Microsoft Learn and the public SDK header `processthreadsapi.h`;
//! none of it was observed on Windows (the ADR lists it as doc-derived).

use crate::error::{ERROR_BAD_LENGTH, ERROR_INVALID_PARAMETER};
use alloc::vec::Vec;

// ----- GetProcessInformation / SetProcessInformation ---------------------------------------

/// `PROCESS_INFORMATION_CLASS` (`processthreadsapi.h`).
pub const PROCESS_MEMORY_PRIORITY: u32 = 0;
pub const PROCESS_APP_MEMORY_INFO: u32 = 2;
/// `ProcessInPrivateInfo`: one byte (the sheet: readable with a size of 1).
pub const PROCESS_IN_PRIVATE_INFO: u32 = 3;
pub const PROCESS_POWER_THROTTLING: u32 = 4;
pub const PROCESS_PROTECTION_LEVEL_INFO: u32 = 7;
pub const PROCESS_LEAP_SECOND_INFO: u32 = 8;
pub const PROCESS_MACHINE_TYPE_INFO: u32 = 9;
/// `ProcessMaxOverridePrefetchParameter`: four bytes, readable (the sheet), 0 for a process that
/// has no prefetch override.
pub const PROCESS_MAX_OVERRIDE_PREFETCH_PARAMETER: u32 = 11;

/// `MEMORY_PRIORITY_NORMAL` (`processthreadsapi.h`): "the default priority for all threads and
/// processes on the system" (Learn, `MEMORY_PRIORITY_INFORMATION`).
pub const MEMORY_PRIORITY_NORMAL: u32 = 5;
/// `MEMORY_PRIORITY_VERY_LOW`, the lowest value.
pub const MEMORY_PRIORITY_VERY_LOW: u32 = 1;
/// `PROCESS_POWER_THROTTLING_CURRENT_VERSION`.
pub const POWER_THROTTLING_VERSION: u32 = 1;
/// `PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION`:
/// the bits a `PROCESS_POWER_THROTTLING_STATE` may name.
pub const POWER_THROTTLING_VALID: u32 = 0x1 | 0x4;
/// `PROTECTION_LEVEL_NONE` (`processthreadsapi.h`): the process is not protected.
pub const PROTECTION_LEVEL_NONE: u32 = 0xFFFF_FFFE;
/// `PROCESS_LEAP_SECOND_INFO_FLAG_ENABLE_SIXTY_SECOND`.
pub const LEAP_SECOND_SIXTY: u32 = 1;
/// `IMAGE_FILE_MACHINE_AMD64` (`winnt.h`).
pub const MACHINE_AMD64: u16 = 0x8664;
/// `MACHINE_ATTRIBUTES`: `UserEnabled | KernelEnabled` (`processthreadsapi.h`).
pub const MACHINE_USER_AND_KERNEL: u32 = 0x1 | 0x2;

/// What `SetProcessInformation` changes and `GetProcessInformation` reads back, per process.
/// Poly's memory manager and scheduler do not act on any of it yet: it is kept and reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessState {
    /// `MEMORY_PRIORITY_INFORMATION.MemoryPriority`.
    pub memory_priority: u32,
    /// `PROCESS_POWER_THROTTLING_STATE.ControlMask`: the mechanisms the process controls itself.
    pub throttle_control: u32,
    /// `PROCESS_POWER_THROTTLING_STATE.StateMask`: of those, the ones switched on.
    pub throttle_state: u32,
    /// `PROCESS_LEAP_SECOND_INFO.Flags`.
    pub leap_flags: u32,
}

impl ProcessState {
    /// A new process: normal memory priority, the system decides about throttling, no leap seconds.
    pub const NEW: ProcessState =
        ProcessState { memory_priority: MEMORY_PRIORITY_NORMAL, throttle_control: 0, throttle_state: 0, leap_flags: 0 };
}

/// The memory a process uses, for `ProcessAppMemoryInfo` (`APP_MEMORY_INFORMATION`), in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppMemory {
    pub available_commit: u64,
    pub private_commit: u64,
    pub peak_private_commit: u64,
    pub total_commit: u64,
}

/// The size of the structure a class carries, or `None` for a class Poly does not know.
pub fn class_size(class: u32) -> Option<usize> {
    match class {
        PROCESS_MEMORY_PRIORITY | PROCESS_PROTECTION_LEVEL_INFO | PROCESS_MAX_OVERRIDE_PREFETCH_PARAMETER => Some(4),
        PROCESS_IN_PRIVATE_INFO => Some(1),
        PROCESS_POWER_THROTTLING => Some(12),
        PROCESS_APP_MEMORY_INFO => Some(32),
        PROCESS_LEAP_SECOND_INFO | PROCESS_MACHINE_TYPE_INFO => Some(8),
        _ => None,
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// `GetProcessInformation`: the bytes of the class's structure. A class Poly does not know is
/// `ERROR_INVALID_PARAMETER`; a size that is not the structure's is `ERROR_BAD_LENGTH` (Learn:
/// the size "must be" the structure's).
pub fn get(state: &ProcessState, memory: &AppMemory, class: u32, size: usize) -> Result<Vec<u8>, u32> {
    let want = class_size(class).ok_or(ERROR_INVALID_PARAMETER)?;
    if size != want {
        return Err(ERROR_BAD_LENGTH);
    }
    let mut out = Vec::with_capacity(want);
    match class {
        PROCESS_MEMORY_PRIORITY => out.extend(state.memory_priority.to_le_bytes()),
        PROCESS_POWER_THROTTLING => {
            for v in [POWER_THROTTLING_VERSION, state.throttle_control, state.throttle_state] {
                out.extend(v.to_le_bytes());
            }
        }
        PROCESS_APP_MEMORY_INFO => {
            for v in [memory.available_commit, memory.private_commit, memory.peak_private_commit, memory.total_commit] {
                out.extend(v.to_le_bytes());
            }
        }
        PROCESS_PROTECTION_LEVEL_INFO => out.extend(PROTECTION_LEVEL_NONE.to_le_bytes()),
        // Not an InPrivate (browsing) process.
        PROCESS_IN_PRIVATE_INFO => out.push(0),
        PROCESS_LEAP_SECOND_INFO => {
            out.extend(state.leap_flags.to_le_bytes());
            out.extend(0u32.to_le_bytes());
        }
        PROCESS_MAX_OVERRIDE_PREFETCH_PARAMETER => out.extend(0u32.to_le_bytes()),
        _ => {
            // PROCESS_MACHINE_INFORMATION: ProcessMachine, Res0, MachineAttributes.
            out.extend(MACHINE_AMD64.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(MACHINE_USER_AND_KERNEL.to_le_bytes());
        }
    }
    Ok(out)
}

/// `SetProcessInformation`: the classes Learn lets a process set (memory priority, power
/// throttling, leap seconds). The read-only classes and unknown ones are
/// `ERROR_INVALID_PARAMETER`, as is a value outside the header's range.
pub fn set(state: &mut ProcessState, class: u32, data: &[u8]) -> Result<(), u32> {
    let want = match class {
        PROCESS_MEMORY_PRIORITY | PROCESS_POWER_THROTTLING | PROCESS_LEAP_SECOND_INFO => class_size(class),
        _ => None,
    }
    .ok_or(ERROR_INVALID_PARAMETER)?;
    if data.len() != want {
        return Err(ERROR_BAD_LENGTH);
    }
    match class {
        PROCESS_MEMORY_PRIORITY => {
            let p = u32_at(data, 0);
            if !(MEMORY_PRIORITY_VERY_LOW..=MEMORY_PRIORITY_NORMAL).contains(&p) {
                return Err(ERROR_INVALID_PARAMETER);
            }
            state.memory_priority = p;
        }
        PROCESS_POWER_THROTTLING => {
            let (version, control, on) = (u32_at(data, 0), u32_at(data, 4), u32_at(data, 8));
            // Learn: StateMask names only mechanisms ControlMask names.
            if version != POWER_THROTTLING_VERSION || control & !POWER_THROTTLING_VALID != 0 || on & !control != 0 {
                return Err(ERROR_INVALID_PARAMETER);
            }
            state.throttle_control = control;
            state.throttle_state = on;
        }
        _ => {
            let flags = u32_at(data, 0);
            if flags & !LEAP_SECOND_SIXTY != 0 {
                return Err(ERROR_INVALID_PARAMETER);
            }
            state.leap_flags = flags;
        }
    }
    Ok(())
}

/// `GetProcessInformation(ProcessPowerThrottling)`: the sheet's rule `version_first`: "the
/// buffer's first field has to be 1 before the call"; with anything else the call fails with
/// `ERROR_INVALID_PARAMETER` and writes nothing.
pub fn power_throttling_version_ok(version_in_buffer: u32) -> bool {
    version_in_buffer == POWER_THROTTLING_VERSION
}

// ----- the shutdown parameters ----------------------------------------------------------------

/// The level every process starts with (Learn, `SetProcessShutdownParameters`: "All processes
/// start at shutdown level 0x280").
pub const SHUTDOWN_LEVEL_DEFAULT: u32 = 0x280;
/// `SHUTDOWN_NORETRY` (`winbase.h`).
pub const SHUTDOWN_NORETRY: u32 = 1;

/// The last error of a `SetProcessShutdownParameters` with a flag other than `SHUTDOWN_NORETRY`
/// (the sheet: 641, as observed).
pub const SHUTDOWN_BAD_FLAGS: u32 = 641;

/// `SetProcessShutdownParameters(level, flags)`
/// (`specs/windows/kernel32.dll/SetProcessShutdownParameters.yaml`): every level is accepted
/// and read back unchanged, also the ranges Learn reserves for the system (rule `levels`); the
/// flags are 0 or `SHUTDOWN_NORETRY`, any other bit fails with error 641 and leaves level and
/// flags as they were (rule `flags`).
pub fn shutdown_parameters(level: u32, flags: u32) -> Result<(u32, u32), u32> {
    if flags & !SHUTDOWN_NORETRY != 0 {
        return Err(SHUTDOWN_BAD_FLAGS);
    }
    Ok((level, flags))
}

// ----- cycle time -----------------------------------------------------------------------------

/// The processor frequency Poly reports cycle times at. Learn (`QueryProcessCycleTime`) defines
/// the result as "the number of CPU clock cycles used by the threads of the process" and warns
/// not to convert it to time, so only its growth with the CPU time used is meaningful: Poly
/// counts the CPU time it measured at a nominal 3 GHz.
pub const NOMINAL_CYCLES_PER_100NS: u64 = 300;

/// The cycle time of a process that used `kernel` and `user` CPU time (100 ns units).
pub fn cycle_time(kernel: u64, user: u64) -> u64 {
    kernel.saturating_add(user).saturating_mul(NOMINAL_CYCLES_PER_100NS)
}

// ----- mapped file names ----------------------------------------------------------------------

/// The module that holds `address`: `(base, size, full path)` entries of the loader's list.
pub fn module_at(modules: &[(u64, u64, Vec<u16>)], address: u64) -> Option<&[u16]> {
    modules.iter().find(|(base, size, _)| address >= *base && address - *base < *size).map(|(_, _, p)| p.as_slice())
}

/// `ERROR_INSUFFICIENT_BUFFER` (`winerror.h`).
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// `K32GetMappedFileNameW`'s result for a name and a buffer of `size` characters
/// (`specs/windows/kernel32.dll/K32GetMappedFileNameW.yaml`, rule `small_buffer`): `(characters
/// of the name to copy, whether a NUL follows them, the return value, the last error to set)`.
/// A buffer of the name's length + 1 holds the name and its NUL: the return value is the
/// length. A buffer of n characters that cannot: the first n - 1 characters and a NUL, the
/// return value n, `ERROR_INSUFFICIENT_BUFFER`; with n = 0 nothing is written and 0 returned.
pub fn mapped_name_copy(name: &[u16], size: u32) -> (usize, bool, u32, Option<u32>) {
    let size = size as usize;
    if size > name.len() {
        (name.len(), true, name.len() as u32, None)
    } else if size == 0 {
        (0, false, 0, Some(ERROR_INSUFFICIENT_BUFFER))
    } else {
        (size - 1, true, size as u32, Some(ERROR_INSUFFICIENT_BUFFER))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec;

    #[test]
    fn a_new_process_reads_the_documented_defaults() {
        let (s, m) = (ProcessState::NEW, AppMemory::default());
        assert_eq!(get(&s, &m, PROCESS_MEMORY_PRIORITY, 4), Ok(vec![5, 0, 0, 0]));
        assert_eq!(get(&s, &m, PROCESS_POWER_THROTTLING, 12), Ok(vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(get(&s, &m, PROCESS_PROTECTION_LEVEL_INFO, 4), Ok(vec![0xFE, 0xFF, 0xFF, 0xFF]));
        assert_eq!(get(&s, &m, PROCESS_LEAP_SECOND_INFO, 8), Ok(vec![0; 8]));
        assert_eq!(get(&s, &m, PROCESS_MACHINE_TYPE_INFO, 8), Ok(vec![0x64, 0x86, 0, 0, 3, 0, 0, 0]));
        let mem = AppMemory { available_commit: 1, private_commit: 2, peak_private_commit: 3, total_commit: 4 };
        let bytes = get(&s, &mem, PROCESS_APP_MEMORY_INFO, 32).unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[8], 2);
        assert_eq!(bytes[16], 3);
        assert_eq!(bytes[24], 4);
    }

    #[test]
    fn a_wrong_size_or_class_is_refused() {
        let (s, m) = (ProcessState::NEW, AppMemory::default());
        assert_eq!(get(&s, &m, PROCESS_MEMORY_PRIORITY, 8), Err(ERROR_BAD_LENGTH));
        assert_eq!(get(&s, &m, PROCESS_POWER_THROTTLING, 4), Err(ERROR_BAD_LENGTH));
        assert_eq!(get(&s, &m, 1, 4), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(get(&s, &m, 200, 4), Err(ERROR_INVALID_PARAMETER));
        // The sheet: ProcessInPrivateInfo is one byte; the caller's Version must be 1.
        assert_eq!(get(&s, &m, PROCESS_IN_PRIVATE_INFO, 1), Ok(vec![0]));
        assert!(power_throttling_version_ok(1));
        assert!(!power_throttling_version_ok(0));
        assert!(!power_throttling_version_ok(2));
        let mut s = s;
        // Read-only classes cannot be set.
        assert_eq!(set(&mut s, PROCESS_PROTECTION_LEVEL_INFO, &[0; 4]), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set(&mut s, PROCESS_MACHINE_TYPE_INFO, &[0; 8]), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set(&mut s, PROCESS_MEMORY_PRIORITY, &[5, 0]), Err(ERROR_BAD_LENGTH));
        assert_eq!(s, ProcessState::NEW);
    }

    #[test]
    fn what_is_set_is_read_back() {
        let mut s = ProcessState::NEW;
        let m = AppMemory::default();
        assert_eq!(set(&mut s, PROCESS_MEMORY_PRIORITY, &2u32.to_le_bytes()), Ok(()));
        assert_eq!(get(&s, &m, PROCESS_MEMORY_PRIORITY, 4), Ok(vec![2, 0, 0, 0]));
        assert_eq!(set(&mut s, PROCESS_MEMORY_PRIORITY, &0u32.to_le_bytes()), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set(&mut s, PROCESS_MEMORY_PRIORITY, &6u32.to_le_bytes()), Err(ERROR_INVALID_PARAMETER));
        // Power throttling: version 1, the execution-speed mechanism switched on.
        let on: Vec<u8> = [1u32, 1, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(set(&mut s, PROCESS_POWER_THROTTLING, &on), Ok(()));
        assert_eq!(get(&s, &m, PROCESS_POWER_THROTTLING, 12).unwrap(), on);
        // The system decides again: both masks zero.
        let auto: Vec<u8> = [1u32, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(set(&mut s, PROCESS_POWER_THROTTLING, &auto), Ok(()));
        assert_eq!((s.throttle_control, s.throttle_state), (0, 0));
        // A wrong version, an unknown mechanism, a state bit the control mask does not name.
        for bad in [[2u32, 1, 1], [1, 2, 0], [1, 1, 5]] {
            let b: Vec<u8> = bad.iter().flat_map(|v| v.to_le_bytes()).collect();
            assert_eq!(set(&mut s, PROCESS_POWER_THROTTLING, &b), Err(ERROR_INVALID_PARAMETER), "{bad:?}");
        }
        assert_eq!(set(&mut s, PROCESS_LEAP_SECOND_INFO, &[1, 0, 0, 0, 0, 0, 0, 0]), Ok(()));
        assert_eq!(get(&s, &m, PROCESS_LEAP_SECOND_INFO, 8), Ok(vec![1, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(set(&mut s, PROCESS_LEAP_SECOND_INFO, &[2, 0, 0, 0, 0, 0, 0, 0]), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn the_prefetch_override_class_reads_four_bytes_and_the_wrong_size_is_a_bad_length() {
        let (s, m) = (ProcessState::NEW, AppMemory::default());
        assert_eq!(get(&s, &m, PROCESS_MAX_OVERRIDE_PREFETCH_PARAMETER, 4), Ok(vec![0, 0, 0, 0]));
        for size in [0, 1, 2, 8, 12, 64] {
            assert_eq!(get(&s, &m, PROCESS_MAX_OVERRIDE_PREFETCH_PARAMETER, size), Err(ERROR_BAD_LENGTH), "{size}");
        }
        // The classes that cannot be read, whatever the size (the sheet).
        for class in [1, 5, 6, 10, 12, 13, 100, 0xFFFF_FFFF] {
            assert_eq!(get(&s, &m, class, 4), Err(ERROR_INVALID_PARAMETER), "{class}");
        }
    }

    #[test]
    fn shutdown_levels_are_the_application_ranges() {
        // Chrome's processes: level 0x100, no retry dialog.
        assert_eq!(shutdown_parameters(0x100, 1), Ok((0x100, 1)));
        assert_eq!(shutdown_parameters(SHUTDOWN_LEVEL_DEFAULT, 0), Ok((0x280, 0)));
        assert_eq!(shutdown_parameters(0x3FF, 0), Ok((0x3FF, 0)));
        // The sheet: every level is accepted, a flag other than SHUTDOWN_NORETRY is error 641.
        for level in [0, 0xFF, 0x400, 0x500, 0x999, 0xFFFF_FFFF] {
            assert_eq!(shutdown_parameters(level, 1), Ok((level, 1)));
        }
        for flags in [2, 3, 0x100, 0xFFFF_FFFF] {
            assert_eq!(shutdown_parameters(0x280, flags), Err(641));
        }
    }

    #[test]
    fn cycle_time_grows_with_cpu_time() {
        assert_eq!(cycle_time(0, 0), 0);
        assert_eq!(cycle_time(10, 20), 9000);
        assert!(cycle_time(1, 0) < cycle_time(1, 1));
        assert_eq!(cycle_time(u64::MAX, 5), u64::MAX);
    }

    #[test]
    fn an_address_finds_its_module_and_names_are_cut_to_the_buffer() {
        let name: Vec<u16> = "\\Device\\PolyFs0\\Windows\\System32\\ntdll.dll".encode_utf16().collect();
        let mods = vec![(0x1000u64, 0x2000u64, vec![1u16, 2]), (0x7ff0_0000_0000, 0x10_0000, name.clone())];
        assert_eq!(module_at(&mods, 0x7ff0_0000_1234), Some(name.as_slice()));
        assert_eq!(module_at(&mods, 0x1000), Some(&[1u16, 2][..]));
        assert_eq!(module_at(&mods, 0x2fff), Some(&[1u16, 2][..]));
        assert_eq!(module_at(&mods, 0x3000), None);
        assert_eq!(module_at(&mods, 0), None);
        let n = name.len();
        assert_eq!(mapped_name_copy(&name, 260), (n, true, n as u32, None));
        assert_eq!(mapped_name_copy(&name, n as u32 + 1), (n, true, n as u32, None));
        // The sheet: a buffer of the name's length or less is cut, with error 122.
        assert_eq!(mapped_name_copy(&name, n as u32), (n - 1, true, n as u32, Some(122)));
        assert_eq!(mapped_name_copy(&name, 10), (9, true, 10, Some(122)));
        assert_eq!(mapped_name_copy(&name, 0), (0, false, 0, Some(122)));
    }
}
