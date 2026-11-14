//! Process mitigation policies as one process has them (ADR 0171, ADR 0186): what
//! `GetProcessMitigationPolicy` reads, what `SetProcessMitigationPolicy` may change, and what
//! the creation-time attributes of `UpdateProcThreadAttribute` (`PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`,
//! `..._CHILD_PROCESS_POLICY`) put there before the child runs a line.
//!
//! The state is a flags word per policy (`PROCESS_MITIGATION_POLICY`, `winnt.h`; each policy's
//! structure is one `DWORD` of bit fields, DEP's is two). The creation words are the 2-bit fields
//! of `PROCESS_CREATION_MITIGATION_POLICY_*` and `PROCESS_CREATION_MITIGATION_POLICY2_*` in
//! `winbase.h`: 0 leaves the default, 1 turns the mitigation on, 2 turns it off, 3 turns it on
//! with an opt-out. The kernel keeps one [`State`] per process; `kernelbase` asks for it through
//! `NtQueryInformationProcess` and `NtSetInformationProcess` (class 52).

/// `PROCESS_MITIGATION_POLICY` values (`winnt.h`).
pub const DEP: u32 = 0;
pub const ASLR: u32 = 1;
pub const DYNAMIC_CODE: u32 = 2;
pub const STRICT_HANDLE: u32 = 3;
pub const SYSTEM_CALL_DISABLE: u32 = 4;
pub const OPTIONS_MASK: u32 = 5;
pub const EXTENSION_POINT_DISABLE: u32 = 6;
pub const CONTROL_FLOW_GUARD: u32 = 7;
pub const SIGNATURE: u32 = 8;
pub const FONT_DISABLE: u32 = 9;
pub const IMAGE_LOAD: u32 = 10;
pub const PAYLOAD_RESTRICTION: u32 = 12;
pub const CHILD_PROCESS: u32 = 13;
pub const SIDE_CHANNEL_ISOLATION: u32 = 14;
pub const USER_SHADOW_STACK: u32 = 15;
pub const USER_POINTER_AUTH: u32 = 17;
pub const MAX_POLICY: u32 = 18;

/// `ERROR_INVALID_PARAMETER`, `ERROR_NOT_SUPPORTED`, `ERROR_ACCESS_DENIED` (`winerror.h`).
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_ACCESS_DENIED: u32 = 5;

/// `PROCESS_CREATION_CHILD_PROCESS_RESTRICTED` and `..._OVERRIDE` (`winbase.h`): the values of
/// `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`.
pub const CHILD_PROCESS_RESTRICTED: u32 = 1;
pub const CHILD_PROCESS_OVERRIDE: u32 = 2;

/// A two-bit field of a creation word.
const fn field(word: u64, shift: u32) -> u64 {
    (word >> shift) & 3
}

/// Whether the field turns the mitigation on (1, or 3 with an opt-out).
const fn on(word: u64, shift: u32) -> bool {
    matches!(field(word, shift), 1 | 3)
}

/// Whether the field turns the mitigation off (2).
const fn off(word: u64, shift: u32) -> bool {
    field(word, shift) == 2
}

/// Whether `v` holds only `PROCESS_CREATION_MITIGATION_POLICY_*` values Windows defines
/// (`winbase.h`): the one-bit flags at bits 0 to 2 and two-bit fields at every fourth bit from
/// 8 to 60.
pub fn creation_defined(v: u64) -> bool {
    let mut ok: u64 = 0b111;
    let mut shift = 8;
    while shift <= 60 {
        ok |= 0b11 << shift;
        shift += 4;
    }
    v & !ok == 0
}

/// Why a policy could not be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetError {
    InvalidParameter,
    NotSupported,
    AccessDenied,
}

impl SetError {
    /// The Win32 error `SetProcessMitigationPolicy` reports.
    pub fn win32(self) -> u32 {
        match self {
            SetError::InvalidParameter => ERROR_INVALID_PARAMETER,
            SetError::NotSupported => ERROR_NOT_SUPPORTED,
            SetError::AccessDenied => ERROR_ACCESS_DENIED,
        }
    }
}

/// The flags word of every policy of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    flags: [u32; MAX_POLICY as usize + 1],
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

impl State {
    /// A new process: DEP enabled and permanent (x64), bottom-up and high-entropy ASLR, and the
    /// shadow-stack policy reads `CetDynamicApisOutOfProcOnly` (bit 8) as the oracle's children do.
    pub const fn new() -> State {
        let mut flags = [0u32; MAX_POLICY as usize + 1];
        flags[DEP as usize] = 3;
        flags[ASLR as usize] = 5;
        flags[USER_SHADOW_STACK as usize] = 0x100;
        State { flags }
    }

    /// The flags word of `policy` (0 for a policy this model does not keep).
    pub fn get(&self, policy: u32) -> u32 {
        self.flags.get(policy as usize).copied().unwrap_or(0)
    }

    /// The state of a child created with the two mitigation words of
    /// `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY` (`w1` is the second word of a 16-byte value).
    pub fn with_creation_words(mut self, w0: u64, w1: u64) -> State {
        let f = &mut self.flags;
        // `PROCESS_CREATION_MITIGATION_POLICY_DEP_ENABLE` is bit 0 and leaves DEP as it is (x64).
        // ASLR: force relocate (8), bottom-up (16), high entropy (20).
        let aslr = &mut f[ASLR as usize];
        for (shift, bit) in [(16, 1u32), (8, 2), (20, 4)] {
            if on(w0, shift) {
                *aslr |= bit;
            } else if off(w0, shift) {
                *aslr &= !bit;
            }
        }
        // Strict handle checks (24): raise on invalid handles, permanent.
        if on(w0, 24) {
            f[STRICT_HANDLE as usize] |= 3;
        }
        // Win32k system calls (28) and FSCTL system calls (word 1, 56).
        if on(w0, 28) {
            f[SYSTEM_CALL_DISABLE as usize] |= 1;
        }
        if on(w1, 56) {
            f[SYSTEM_CALL_DISABLE as usize] |= 4;
        }
        // Extension points (32).
        if on(w0, 32) {
            f[EXTENSION_POINT_DISABLE as usize] |= 1;
        }
        // Dynamic code (36): prohibit (and allow a thread to opt out with 3).
        if on(w0, 36) {
            f[DYNAMIC_CODE as usize] |= if field(w0, 36) == 3 { 3 } else { 1 };
        }
        // Control flow guard (40).
        if on(w0, 40) {
            f[CONTROL_FLOW_GUARD as usize] |= 1;
        }
        // Block non-Microsoft binaries (44): Microsoft-signed only with the opt-in bit; 3 allows
        // the Store as well.
        match field(w0, 44) {
            1 => f[SIGNATURE as usize] |= 1 | 4,
            3 => f[SIGNATURE as usize] |= 2 | 4,
            _ => {}
        }
        // Fonts (48); images: no remote (52), no low label (56), prefer System32 (60).
        if on(w0, 48) {
            f[FONT_DISABLE as usize] |= 1;
        }
        for (shift, bit) in [(52, 1u32), (56, 2), (60, 4)] {
            if on(w0, shift) {
                f[IMAGE_LOAD as usize] |= bit;
            }
        }
        // Second word: restrict indirect branch prediction (16), speculative store bypass (20).
        if on(w1, 16) {
            f[SIDE_CHANNEL_ISOLATION as usize] |= 1;
        }
        if on(w1, 20) {
            f[SIDE_CHANNEL_ISOLATION as usize] |= 8;
        }
        // CET user shadow stacks (28; 3 is strict mode), set-context IP validation (32) and
        // blocking binaries without CET (36).
        match field(w1, 28) {
            1 => f[USER_SHADOW_STACK as usize] |= 1,
            3 => f[USER_SHADOW_STACK as usize] |= 1 | 0x10,
            _ => {}
        }
        if on(w1, 32) {
            f[USER_SHADOW_STACK as usize] |= 4;
        }
        if on(w1, 36) {
            f[USER_SHADOW_STACK as usize] |= 0x20;
        }
        self
    }

    /// The state of a child created with `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY` `value`
    /// (`PROCESS_CREATION_CHILD_PROCESS_RESTRICTED` stops it making processes).
    pub fn with_child_policy(mut self, value: u32) -> State {
        if value & CHILD_PROCESS_RESTRICTED != 0 {
            self.flags[CHILD_PROCESS as usize] |= 1;
        }
        self
    }

    /// Whether `CreateProcessW` is refused (`ERROR_CHILD_PROCESS_BLOCKED`).
    pub fn child_processes_blocked(&self) -> bool {
        self.flags[CHILD_PROCESS as usize] & 1 != 0
    }

    /// Whether memory cannot be made executable (the dynamic-code policy).
    pub fn dynamic_code_blocked(&self) -> bool {
        self.flags[DYNAMIC_CODE as usize] & 1 != 0
    }

    /// Whether every win32k system call is refused (`DisallowWin32kSystemCalls`).
    pub fn win32k_disabled(&self) -> bool {
        self.flags[SYSTEM_CALL_DISABLE as usize] & 1 != 0
    }

    /// `SetProcessMitigationPolicy`: most policies can only be switched on, a permanent
    /// strict-handle policy cannot be weakened, control flow guard and shadow stacks cannot be
    /// set at run time, and the options mask and payload restriction are not settable (the
    /// sheets of `SetProcessMitigationPolicy`).
    pub fn set(&mut self, policy: u32, flags: u32) -> Result<(), SetError> {
        if policy > MAX_POLICY {
            return Err(SetError::InvalidParameter);
        }
        let slot = &mut self.flags[policy as usize];
        match policy {
            DEP => return Err(SetError::NotSupported),
            CONTROL_FLOW_GUARD | USER_SHADOW_STACK => return Err(SetError::AccessDenied),
            PAYLOAD_RESTRICTION | OPTIONS_MASK => return Err(SetError::InvalidParameter),
            STRICT_HANDLE => {
                // Raise-on-invalid and permanent go together; once permanent it stays.
                if *slot & 2 != 0 && flags & 2 == 0 {
                    return Err(SetError::AccessDenied);
                }
                if flags & 3 != 0 && flags & 3 != 3 {
                    return Err(SetError::InvalidParameter);
                }
                *slot |= flags;
            }
            ASLR => {
                if flags & 1 != 0 {
                    *slot |= 5;
                }
            }
            // Microsoft-signed-only turns on the opt-in bit as well.
            SIGNATURE => *slot |= if flags & 1 != 0 { flags | 4 } else { flags },
            _ => *slot |= flags,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 16-byte values Chrome passes (the sheet of `UpdateProcThreadAttribute`).
    const GPU: (u64, u64) = (0x0111_0001_0001_1000, 0x0100_0000_0001_0000);
    const STORAGE: (u64, u64) = (0x0111_0001_1001_1000, 0x0100_0000_0001_0000);
    const RENDERER: (u64, u64) = (0x0111_1001_1001_1000, 0x0100_0000_2001_1000);

    fn child(words: (u64, u64)) -> State {
        State::new().with_creation_words(words.0, words.1).with_child_policy(1)
    }

    #[test]
    fn a_new_process_has_the_defaults() {
        let s = State::new();
        assert_eq!(s.get(DEP), 3);
        assert_eq!(s.get(ASLR), 5);
        assert_eq!(s.get(USER_SHADOW_STACK), 0x100);
        assert_eq!(s.get(SYSTEM_CALL_DISABLE), 0);
        assert!(!s.child_processes_blocked() && !s.dynamic_code_blocked() && !s.win32k_disabled());
    }

    #[test]
    fn the_gpu_words_as_the_child_reads_them() {
        let s = child(GPU);
        assert_eq!(s.get(EXTENSION_POINT_DISABLE), 1);
        assert_eq!(s.get(FONT_DISABLE), 1);
        assert_eq!(s.get(IMAGE_LOAD), 3);
        assert_eq!(s.get(SYSTEM_CALL_DISABLE), 4);
        assert_eq!(s.get(SIDE_CHANNEL_ISOLATION), 1);
        assert_eq!(s.get(CHILD_PROCESS), 1);
        assert_eq!(s.get(SIGNATURE), 0);
        assert_eq!(s.get(STRICT_HANDLE), 0);
        assert_eq!(s.get(ASLR), 5);
        assert_eq!(s.get(USER_SHADOW_STACK), 0x100);
        assert_eq!(s.get(DYNAMIC_CODE), 0);
        assert!(!s.win32k_disabled() && s.child_processes_blocked());
    }

    #[test]
    fn the_storage_service_words_disable_win32k() {
        let s = child(STORAGE);
        assert_eq!(s.get(SYSTEM_CALL_DISABLE), 5);
        assert!(s.win32k_disabled());
        assert_eq!(s.get(SIGNATURE), 0);
    }

    #[test]
    fn the_renderer_words_add_microsoft_signed_only() {
        let s = child(RENDERER);
        assert_eq!(s.get(SYSTEM_CALL_DISABLE), 5);
        assert_eq!(s.get(SIGNATURE), 5);
        assert_eq!(s.get(FONT_DISABLE), 1);
        assert_eq!(s.get(IMAGE_LOAD), 3);
        // The shadow-stack field of word 1 is 2 ("off"): the default stays.
        assert_eq!(s.get(USER_SHADOW_STACK), 0x100);
    }

    #[test]
    fn the_child_policy_alone_only_blocks_children() {
        let s = State::new().with_child_policy(1);
        assert_eq!(s.get(CHILD_PROCESS), 1);
        assert_eq!(s.get(SYSTEM_CALL_DISABLE), 0);
        assert!(!State::new().with_child_policy(CHILD_PROCESS_OVERRIDE).child_processes_blocked());
    }

    #[test]
    fn dynamic_code_and_a_four_byte_value() {
        let s = State::new().with_creation_words(1 << 36, 0);
        assert!(s.dynamic_code_blocked());
        // A field value of 2 turns an ASLR default off.
        let s = State::new().with_creation_words(2 << 16, 0);
        assert_eq!(s.get(ASLR), 4);
    }

    #[test]
    fn creation_values_are_checked_against_the_defined_fields() {
        assert!(creation_defined(GPU.0) && creation_defined(RENDERER.0) && creation_defined(1));
        assert!(!creation_defined(1 << 62));
        assert!(!creation_defined(1 << 3));
    }

    #[test]
    fn set_follows_the_sheets() {
        let mut s = State::new();
        assert_eq!(s.set(DEP, 1), Err(SetError::NotSupported));
        assert_eq!(s.set(CONTROL_FLOW_GUARD, 1), Err(SetError::AccessDenied));
        assert_eq!(s.set(USER_SHADOW_STACK, 1), Err(SetError::AccessDenied));
        assert_eq!(s.set(OPTIONS_MASK, 1), Err(SetError::InvalidParameter));
        assert_eq!(s.set(PAYLOAD_RESTRICTION, 1), Err(SetError::InvalidParameter));
        assert_eq!(s.set(19, 1), Err(SetError::InvalidParameter));
        assert_eq!(s.set(STRICT_HANDLE, 1), Err(SetError::InvalidParameter));
        assert_eq!(s.set(STRICT_HANDLE, 3), Ok(()));
        assert_eq!(s.set(STRICT_HANDLE, 0), Err(SetError::AccessDenied));
        assert_eq!(s.set(SIGNATURE, 1), Ok(()));
        assert_eq!(s.get(SIGNATURE), 5);
        assert_eq!(s.set(CHILD_PROCESS, 1), Ok(()));
        assert!(s.child_processes_blocked());
        // A policy only goes on: setting zero changes nothing.
        assert_eq!(s.set(CHILD_PROCESS, 0), Ok(()));
        assert!(s.child_processes_blocked());
        assert_eq!(s.set(ASLR, 1), Ok(()));
        assert_eq!(s.get(ASLR), 5);
        assert_eq!(s.set(SYSTEM_CALL_DISABLE, 1), Ok(()));
        assert!(s.win32k_disabled());
    }
}
