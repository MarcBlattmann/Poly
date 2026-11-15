//! What a process keeps about itself that another process can set and read, and that a child
//! inherits (ADR 0155, Chrome stage 2): the default hard-error mode (`SetErrorMode`), the memory
//! priority and the power-throttling masks (`SetProcessInformation`).
//!
//! `NtSetInformationProcess` / `NtQueryInformationProcess` classes (`ntddk.h`, `winternl.h`):
//! `ProcessDefaultHardErrorMode` 12 (a `ULONG`), `ProcessMemoryPriority` 39
//! (`MEMORY_PRIORITY_INFORMATION`, a `ULONG`) and `ProcessPowerThrottlingState` 77
//! (`PROCESS_POWER_THROTTLING_STATE`: `Version`, `ControlMask`, `StateMask`). The user-mode
//! checks of `SetProcessInformation` are `poly_kernelbase::procinfo`'s; here only the sizes and
//! the shape are checked, so a wrong request through the NT service is a status, not a crash.

use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_INFO_LENGTH_MISMATCH, STATUS_INVALID_PARAMETER};

/// `ProcessDefaultHardErrorMode`.
pub const PROCESS_DEFAULT_HARD_ERROR_MODE: u64 = 12;
/// `ProcessMemoryPriority`.
pub const PROCESS_MEMORY_PRIORITY: u64 = 39;
/// `ProcessPowerThrottlingState`.
pub const PROCESS_POWER_THROTTLING_STATE: u64 = 77;

/// The value of a new process's memory priority (`MEMORY_PRIORITY_NORMAL`).
pub const MEMORY_PRIORITY_NORMAL: u32 = 5;

/// One process's set values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcAttrs {
    /// `SetErrorMode`'s value, inherited by a child.
    pub error_mode: u32,
    /// 1 to 5.
    pub memory_priority: u32,
    /// `PROCESS_POWER_THROTTLING_STATE.ControlMask` and `.StateMask`.
    pub throttle_control: u32,
    pub throttle_state: u32,
}

impl ProcAttrs {
    /// A new process (error mode as its creator's, the rest at the defaults).
    pub const fn new(error_mode: u32) -> ProcAttrs {
        ProcAttrs { error_mode, memory_priority: MEMORY_PRIORITY_NORMAL, throttle_control: 0, throttle_state: 0 }
    }

    /// The size of the class's structure, `None` for a class this module does not keep.
    pub fn class_size(class: u64) -> Option<usize> {
        match class {
            PROCESS_DEFAULT_HARD_ERROR_MODE | PROCESS_MEMORY_PRIORITY => Some(4),
            PROCESS_POWER_THROTTLING_STATE => Some(12),
            _ => None,
        }
    }

    /// Store a class's structure. `STATUS_INFO_LENGTH_MISMATCH` for a wrong length,
    /// `STATUS_INVALID_PARAMETER` for a memory priority outside 1 to 5 and for a power
    /// throttling state whose `Version` is not 1.
    pub fn set(&mut self, class: u64, data: &[u8]) -> Result<(), NtStatus> {
        let want = Self::class_size(class).ok_or(STATUS_INVALID_PARAMETER)?;
        if data.len() != want {
            return Err(STATUS_INFO_LENGTH_MISMATCH);
        }
        let word = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        match class {
            PROCESS_DEFAULT_HARD_ERROR_MODE => self.error_mode = word(0),
            PROCESS_MEMORY_PRIORITY => {
                let p = word(0);
                if !(1..=MEMORY_PRIORITY_NORMAL).contains(&p) {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                self.memory_priority = p;
            }
            _ => {
                if word(0) != 1 {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                self.throttle_control = word(4);
                self.throttle_state = word(8);
            }
        }
        Ok(())
    }

    /// The bytes of a class's structure (a power-throttling state has `Version` 1).
    pub fn get(&self, class: u64) -> Option<Vec<u8>> {
        let words: &[u32] = match class {
            PROCESS_DEFAULT_HARD_ERROR_MODE => &[self.error_mode],
            PROCESS_MEMORY_PRIORITY => &[self.memory_priority],
            PROCESS_POWER_THROTTLING_STATE => &[1, self.throttle_control, self.throttle_state],
            _ => return None,
        };
        Some(words.iter().flat_map(|w| w.to_le_bytes()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn a_new_process_has_the_normal_priority_and_the_creators_error_mode() {
        let a = ProcAttrs::new(0x8003);
        assert_eq!(a.get(PROCESS_DEFAULT_HARD_ERROR_MODE), Some(bytes(&[0x8003])));
        assert_eq!(a.get(PROCESS_MEMORY_PRIORITY), Some(bytes(&[5])));
        assert_eq!(a.get(PROCESS_POWER_THROTTLING_STATE), Some(bytes(&[1, 0, 0])));
        assert_eq!(a.get(7), None);
    }

    #[test]
    fn error_mode_keeps_every_bit() {
        let mut a = ProcAttrs::new(0);
        a.set(PROCESS_DEFAULT_HARD_ERROR_MODE, &bytes(&[0xFFFF_FFFF])).unwrap();
        assert_eq!(a.error_mode, 0xFFFF_FFFF);
        assert_eq!(a.set(PROCESS_DEFAULT_HARD_ERROR_MODE, &[0; 8]), Err(STATUS_INFO_LENGTH_MISMATCH));
    }

    #[test]
    fn memory_priority_takes_one_to_five() {
        let mut a = ProcAttrs::new(0);
        for p in 1..=5 {
            a.set(PROCESS_MEMORY_PRIORITY, &bytes(&[p])).unwrap();
            assert_eq!(a.memory_priority, p);
        }
        for p in [0, 6, 7] {
            assert_eq!(a.set(PROCESS_MEMORY_PRIORITY, &bytes(&[p])), Err(STATUS_INVALID_PARAMETER));
            assert_eq!(a.memory_priority, 5);
        }
        for n in [0usize, 3, 8, 12] {
            assert_eq!(a.set(PROCESS_MEMORY_PRIORITY, &alloc::vec![0u8; n]), Err(STATUS_INFO_LENGTH_MISMATCH));
        }
    }

    #[test]
    fn power_throttling_replaces_both_masks() {
        let mut a = ProcAttrs::new(0);
        a.set(PROCESS_POWER_THROTTLING_STATE, &bytes(&[1, 5, 5])).unwrap();
        assert_eq!((a.throttle_control, a.throttle_state), (5, 5));
        a.set(PROCESS_POWER_THROTTLING_STATE, &bytes(&[1, 4, 0])).unwrap();
        assert_eq!((a.throttle_control, a.throttle_state), (4, 0));
        assert_eq!(a.set(PROCESS_POWER_THROTTLING_STATE, &bytes(&[2, 1, 1])), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(a.set(PROCESS_POWER_THROTTLING_STATE, &bytes(&[1, 1])), Err(STATUS_INFO_LENGTH_MISMATCH));
        assert_eq!((a.throttle_control, a.throttle_state), (4, 0));
    }
}
