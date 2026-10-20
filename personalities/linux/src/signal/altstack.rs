//! Alternate signal stacks: sigaltstack(2).

use crate::abi::StackT;
use crate::abi::signal::{MINSIGSTKSZ, SS_AUTODISARM, SS_DISABLE, SS_ONSTACK};
use crate::errno::Errno;

/// A thread's alternate signal stack: `[sp, sp + size)`, or disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AltStack {
    pub sp: u64,
    pub size: u64,
    /// `SS_DISABLE`, or 0 for an enabled stack; plus `SS_AUTODISARM`.
    pub flags: u32,
}

impl Default for AltStack {
    fn default() -> Self {
        AltStack::DISABLED
    }
}

impl AltStack {
    /// No alternate stack: what a new process has, reported as
    /// `{ss_sp=NULL, ss_flags=SS_DISABLE, ss_size=0}`.
    pub const DISABLED: AltStack = AltStack { sp: 0, size: 0, flags: SS_DISABLE };

    pub const fn is_enabled(&self) -> bool {
        self.flags & SS_DISABLE == 0
    }

    /// `SS_AUTODISARM`: the stack is cleared while a handler runs on it
    /// and restored by rt_sigreturn.
    pub const fn autodisarm(&self) -> bool {
        self.flags & SS_AUTODISARM != 0
    }

    /// The thread runs on this stack: its stack pointer `rsp` lies in
    /// `(sp, sp + size]` (a stack grows down from `sp + size`; a pointer at
    /// the very top has nothing pushed yet but already belongs to it).
    /// The exact bounds are to be confirmed by the diff runner.
    pub const fn on_stack(&self, rsp: u64) -> bool {
        self.is_enabled() && rsp > self.sp && rsp - self.sp <= self.size
    }

    /// The `stack_t` that sigaltstack(2) reports in `old_ss` (and a signal
    /// frame saves in `uc_stack`) for a thread at `rsp`: "SS_ONSTACK: The
    /// thread is currently executing on the alternate signal stack",
    /// "SS_DISABLE: The alternate signal stack is currently disabled",
    /// else 0; plus `SS_AUTODISARM` if it was set (to be confirmed by the
    /// diff runner).
    pub const fn report(&self, rsp: u64) -> StackT {
        let state = if self.on_stack(rsp) {
            SS_ONSTACK
        } else if !self.is_enabled() {
            SS_DISABLE
        } else {
            0
        };
        StackT { sp: self.sp, flags: state | (self.flags & SS_AUTODISARM), size: self.size }
    }

    /// The stack sigaltstack(2) installs for `new`, for a thread at `rsp`.
    ///
    /// - `EPERM` "An attempt was made to change the alternate signal stack
    ///   while it was active (i.e., the thread was already executing on the
    ///   current alternate signal stack)".
    /// - `EINVAL` if `ss_flags` is not 0, `SS_ONSTACK` (accepted as 0:
    ///   sigaltstack(2), NOTES) or `SS_DISABLE`, each optionally with
    ///   `SS_AUTODISARM`.
    /// - `ENOMEM` "The specified size of the new alternate signal stack
    ///   ss.ss_size was less than MINSIGSTKSZ" (not checked for
    ///   `SS_DISABLE`, which ignores `ss_sp` and `ss_size`).
    ///
    /// The man page does not order these; Poly checks them as listed (to
    /// be confirmed by the diff runner).
    pub fn change(&self, new: StackT, rsp: u64) -> Result<AltStack, Errno> {
        if self.on_stack(rsp) {
            return Err(Errno::EPERM);
        }
        match new.flags & !SS_AUTODISARM {
            SS_DISABLE => Ok(AltStack::DISABLED),
            0 | SS_ONSTACK if new.size < MINSIGSTKSZ => Err(Errno::ENOMEM),
            0 | SS_ONSTACK => Ok(AltStack { sp: new.sp, size: new.size, flags: new.flags & SS_AUTODISARM }),
            _ => Err(Errno::EINVAL),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STACK: AltStack = AltStack { sp: 0x1_0000, size: 0x4000, flags: 0 };

    #[test]
    fn on_stack_bounds() {
        assert!(!STACK.on_stack(0x1_0000), "the lowest byte is the stack's end, not inside");
        assert!(STACK.on_stack(0x1_0001));
        assert!(STACK.on_stack(0x1_4000), "the top");
        assert!(!STACK.on_stack(0x1_4001));
        assert!(!AltStack::DISABLED.on_stack(1));
        assert!(!AltStack { flags: SS_DISABLE, ..STACK }.on_stack(0x1_2000));
    }

    #[test]
    fn report_flags() {
        assert_eq!(AltStack::DISABLED.report(0x7fff_0000), StackT { sp: 0, flags: SS_DISABLE, size: 0 });
        assert_eq!(STACK.report(0x7fff_0000).flags, 0);
        assert_eq!(STACK.report(0x1_2000).flags, SS_ONSTACK);
        let disarm = AltStack { flags: SS_AUTODISARM, ..STACK };
        assert_eq!(disarm.report(0x1_2000).flags, SS_ONSTACK | SS_AUTODISARM);
    }

    #[test]
    fn change_rules() {
        let new = StackT { sp: 0x2_0000, flags: 0, size: MINSIGSTKSZ };
        let off = 0x7fff_0000;
        assert_eq!(STACK.change(new, off), Ok(AltStack { sp: 0x2_0000, size: 2048, flags: 0 }));
        assert_eq!(STACK.change(StackT { flags: SS_ONSTACK, ..new }, off).map(|s| s.flags), Ok(0));
        assert_eq!(STACK.change(StackT { flags: SS_AUTODISARM, ..new }, off).map(|s| s.flags), Ok(SS_AUTODISARM));
        assert_eq!(STACK.change(StackT { size: MINSIGSTKSZ - 1, ..new }, off), Err(Errno::ENOMEM));
        let disable = StackT { sp: 0, flags: SS_DISABLE, size: 11952 };
        assert_eq!(STACK.change(disable, off), Ok(AltStack::DISABLED));
        assert_eq!(STACK.change(StackT { size: 0, ..disable }, off), Ok(AltStack::DISABLED), "size ignored");
        for bad in [SS_ONSTACK | SS_DISABLE, 4, 0x8000, SS_AUTODISARM | 8] {
            assert_eq!(STACK.change(StackT { flags: bad, ..new }, off), Err(Errno::EINVAL), "flags {bad:#x}");
        }
        assert_eq!(STACK.change(new, 0x1_2000), Err(Errno::EPERM), "running on it");
        assert_eq!(STACK.change(StackT { flags: 99, ..new }, 0x1_2000), Err(Errno::EPERM), "EPERM first");
    }
}
