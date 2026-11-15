//! The per-thread switch of WOW64's file-system redirection (ADR 0186, stage 2; Learn:
//! `Wow64DisableWow64FsRedirection`, `Wow64RevertWow64FsRedirection`,
//! `Wow64EnableWow64FsRedirection`).
//!
//! The state is one `u32` in the thread's TEB (`poly_rtl::layout::TEB_FS_REDIRECTION_OFF`; zero
//! is "redirection on", as in every new thread) that the NT personality reads when it converts a
//! 32-bit process's file name. These functions are its rules, over the flag's value, so `kernelbase`
//! only has to load and store it.
//!
//! `Wow64DisableWow64FsRedirection`'s `OldValue` is, to the caller, "for system use only"
//! (Learn): here it is the flag's value before the call, so that nested pairs restore what was
//! there ("Every successful call to the Wow64DisableWow64FsRedirection function must have a
//! matching call to the Wow64RevertWow64FsRedirection function").

/// The flag's value for "redirection off".
pub const OFF: u32 = 1;

/// Whether a flag value switches redirection off.
pub const fn is_off(flag: u32) -> bool {
    flag != 0
}

/// `Wow64DisableWow64FsRedirection` on a thread whose flag is `flag`: the new flag and the
/// `OldValue` to give back.
pub const fn disable(flag: u32) -> (u32, usize) {
    (OFF, flag as usize)
}

/// `Wow64RevertWow64FsRedirection(old_value)`: the flag it restores.
pub const fn revert(old_value: usize) -> u32 {
    if old_value != 0 { OFF } else { 0 }
}

/// `Wow64EnableWow64FsRedirection(enable)`: the new flag.
pub const fn enable(enable: bool) -> u32 {
    if enable { 0 } else { OFF }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_and_revert_nest() {
        // A new thread: redirection on.
        let flag = 0;
        assert!(!is_off(flag));
        let (flag, outer) = disable(flag);
        assert!(is_off(flag));
        assert_eq!(outer, 0);
        // A second pair inside the first: its OldValue says "already off", so reverting it
        // leaves the outer pair's state.
        let (flag, inner) = disable(flag);
        assert!(is_off(flag));
        assert_eq!(inner, 1);
        let flag = revert(inner);
        assert!(is_off(flag));
        let flag = revert(outer);
        assert!(!is_off(flag));
    }

    #[test]
    fn enable_sets_the_state_it_names() {
        assert_eq!(enable(false), OFF);
        assert!(is_off(enable(false)));
        assert!(!is_off(enable(true)));
        // `Wow64EnableWow64FsRedirection(TRUE)` after a disable is as good as a revert.
        let (flag, _) = disable(0);
        assert!(is_off(flag));
        assert!(!is_off(enable(true)));
    }
}
