//! The error modes (Learn: "SetErrorMode", "GetErrorMode", "SetThreadErrorMode",
//! "GetThreadErrorMode"; `winbase.h` for the `SEM_*` values), as the sheet
//! `specs/windows/kernel32.dll/SetErrorMode.yaml` records them.
//!
//! The process mode keeps every bit it is given (undefined ones too: `0x10`, `0x100` and
//! `0xFFFFFFFF` read back), except `SEM_NOALIGNMENTFAULTEXCEPT`: once it is set it cannot be
//! cleared. A child starts with the mode in force when it is created. The thread's own mode is
//! separate and takes only `SEM_FAILCRITICALERRORS`, `SEM_NOGPFAULTERRORBOX` and
//! `SEM_NOOPENFILEERRORBOX`.

use crate::error::ERROR_INVALID_PARAMETER;

/// `SEM_FAILCRITICALERRORS`.
pub const SEM_FAILCRITICALERRORS: u32 = 0x0001;
/// `SEM_NOGPFAULTERRORBOX`.
pub const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;
/// `SEM_NOALIGNMENTFAULTEXCEPT`.
pub const SEM_NOALIGNMENTFAULTEXCEPT: u32 = 0x0004;
/// `SEM_NOOPENFILEERRORBOX`.
pub const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;

/// The bits a thread's error mode may hold.
pub const THREAD_MODE_BITS: u32 = SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX;

/// The process mode after `SetErrorMode(new)` when it was `old`: every bit as given, plus the
/// alignment bit if it was set.
pub fn process_mode_after(old: u32, new: u32) -> u32 {
    new | (old & SEM_NOALIGNMENTFAULTEXCEPT)
}

/// `SetThreadErrorMode(mode, ...)`: the three bits a thread's mode has, else `ERROR_INVALID_PARAMETER`.
pub fn thread_mode_check(mode: u32) -> Result<u32, u32> {
    if mode & !THREAD_MODE_BITS != 0 { Err(ERROR_INVALID_PARAMETER) } else { Ok(mode) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_mode_keeps_every_bit_it_is_given() {
        for m in [0u32, 1, 2, 0x8000, 0x8003, 0x10, 0x100, 0xFFFF_FFFB] {
            assert_eq!(process_mode_after(0, m), m);
            assert_eq!(process_mode_after(0x8003, m), m);
        }
    }

    #[test]
    fn the_alignment_bit_cannot_be_cleared() {
        assert_eq!(process_mode_after(4, 0), 4);
        assert_eq!(process_mode_after(4, 1), 5);
        assert_eq!(process_mode_after(4, 4), 4);
        assert_eq!(process_mode_after(0, 0xFFFF_FFFF), 0xFFFF_FFFF);
        assert_eq!(process_mode_after(0xFFFF_FFFF, 0), 4);
        assert_eq!(process_mode_after(0, 4), 4);
    }

    #[test]
    fn the_thread_mode_takes_three_bits() {
        assert_eq!(thread_mode_check(0x8003), Ok(0x8003));
        assert_eq!(thread_mode_check(0), Ok(0));
        assert_eq!(thread_mode_check(0x10), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(thread_mode_check(4), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(thread_mode_check(0x8003 | 0x100), Err(ERROR_INVALID_PARAMETER));
    }
}
