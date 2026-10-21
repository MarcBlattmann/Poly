//! Linux error numbers.
//!
//! Values from the Linux UAPI headers `asm-generic/errno-base.h` (1-34) and
//! `asm-generic/errno.h` (35 and up), which x86-64 uses unchanged.

use core::fmt;

/// A Linux error number. A system call that fails returns `-errno` in `rax`
/// (see [`Errno::to_rax`]); user space sees values in `[-4095, -1]` as errors.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Errno(u16);

/// The largest error number the ABI can return: `rax` values in
/// `[-MAX_ERRNO, -1]` are errors, everything else is a result.
pub const MAX_ERRNO: u16 = 4095;

impl Errno {
    pub const EPERM: Errno = Errno(1);
    pub const ENOENT: Errno = Errno(2);
    pub const ESRCH: Errno = Errno(3);
    pub const EINTR: Errno = Errno(4);
    pub const EIO: Errno = Errno(5);
    pub const ENXIO: Errno = Errno(6);
    pub const E2BIG: Errno = Errno(7);
    pub const ENOEXEC: Errno = Errno(8);
    pub const EBADF: Errno = Errno(9);
    pub const ECHILD: Errno = Errno(10);
    pub const EAGAIN: Errno = Errno(11);
    pub const ENOMEM: Errno = Errno(12);
    pub const EACCES: Errno = Errno(13);
    pub const EFAULT: Errno = Errno(14);
    pub const ENOTBLK: Errno = Errno(15);
    pub const EBUSY: Errno = Errno(16);
    pub const EEXIST: Errno = Errno(17);
    pub const EXDEV: Errno = Errno(18);
    pub const ENODEV: Errno = Errno(19);
    pub const ENOTDIR: Errno = Errno(20);
    pub const EISDIR: Errno = Errno(21);
    pub const EINVAL: Errno = Errno(22);
    pub const ENFILE: Errno = Errno(23);
    pub const EMFILE: Errno = Errno(24);
    pub const ENOTTY: Errno = Errno(25);
    pub const ETXTBSY: Errno = Errno(26);
    pub const EFBIG: Errno = Errno(27);
    pub const ENOSPC: Errno = Errno(28);
    pub const ESPIPE: Errno = Errno(29);
    pub const EROFS: Errno = Errno(30);
    pub const EMLINK: Errno = Errno(31);
    pub const EPIPE: Errno = Errno(32);
    pub const EDOM: Errno = Errno(33);
    pub const ERANGE: Errno = Errno(34);
    pub const ENAMETOOLONG: Errno = Errno(36);
    pub const ENOSYS: Errno = Errno(38);
    /// `EIDRM` (`asm-generic/errno.h`): "Identifier removed"; a semaphore
    /// set removed while semop(2) waited on it.
    pub const EIDRM: Errno = Errno(43);
    /// `ENODATA` (`asm-generic/errno.h`): "No data available"; an extended
    /// attribute that does not exist (getxattr(2)).
    pub const ENODATA: Errno = Errno(61);
    pub const EOVERFLOW: Errno = Errno(75);
    /// `EUSERS` (`asm-generic/errno.h`): "Too many users"; user
    /// namespaces nested too deep (clone(2), unshare(2)).
    pub const EUSERS: Errno = Errno(87);
    pub const EOPNOTSUPP: Errno = Errno(95);
    pub const ETIMEDOUT: Errno = Errno(110);

    // ---- internal: never returned to user space -----------------------
    //
    // A handler that was interrupted by a signal returns one of these.
    // The dispatcher turns it into `-EINTR` in `rax` and records how the
    // call may be restarted; signal delivery
    // ([`crate::signal::deliver::exit_to_user`]) then either leaves the
    // `EINTR` or rewinds the thread to the `syscall` instruction, the way
    // signal(7) describes for each kind of call. The values are Poly's own;
    // they lie above every error number Linux defines for user space.

    /// Restart unless a handler without `SA_RESTART` runs (signal(7):
    /// "automatically restarted after the signal handler returns if the
    /// SA_RESTART flag was used; otherwise the call fails with EINTR").
    pub const RESTART_IF_SA_RESTART: Errno = Errno(3840);
    /// Always restart, whatever the handler's flags.
    pub const RESTART_ALWAYS: Errno = Errno(3841);
    /// Restart only if no handler runs (a stop and `SIGCONT`, or a signal
    /// that is ignored after all); with a handler: `EINTR`.
    pub const RESTART_IF_NO_HANDLER: Errno = Errno(3842);
    /// Like [`Errno::RESTART_IF_NO_HANDLER`], but the call continues
    /// through restart_syscall(2) with the state saved in the thread: a
    /// wait with a relative timeout continues with the time left. With a
    /// handler it fails with `EINTR`, `SA_RESTART` or not: restarting it
    /// after a handler would start the timeout over, and restart_syscall(2)
    /// "ensures that the restarted call doesn't sleep for longer than the
    /// time it was told to" (to be confirmed by the diff runner).
    pub const RESTART_WITH_BLOCK: Errno = Errno(3843);

    /// An error number from its value. Returns `None` for 0 and for values
    /// above [`MAX_ERRNO`], which the ABI cannot express.
    pub const fn new(value: u16) -> Option<Errno> {
        if value == 0 || value > MAX_ERRNO { None } else { Some(Errno(value)) }
    }

    /// True for the internal restart codes, which the dispatcher must
    /// never return to user space.
    pub const fn is_restart(self) -> bool {
        self.0 >= 3840 && self.0 <= 3843
    }

    /// The positive error number.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The value returned in `rax`: `-errno` as a 64-bit two's complement.
    pub const fn to_rax(self) -> u64 {
        (self.0 as u64).wrapping_neg()
    }

    /// Decode a raw `rax` value: `Some(errno)` if it is in the error range.
    pub const fn from_rax(rax: u64) -> Option<Errno> {
        let negated = rax.wrapping_neg();
        if negated >= 1 && negated <= MAX_ERRNO as u64 { Some(Errno(negated as u16)) } else { None }
    }

    /// The symbolic name, for the errors defined here.
    pub const fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            1 => "EPERM",
            2 => "ENOENT",
            3 => "ESRCH",
            4 => "EINTR",
            5 => "EIO",
            6 => "ENXIO",
            7 => "E2BIG",
            8 => "ENOEXEC",
            9 => "EBADF",
            10 => "ECHILD",
            11 => "EAGAIN",
            12 => "ENOMEM",
            13 => "EACCES",
            14 => "EFAULT",
            15 => "ENOTBLK",
            16 => "EBUSY",
            17 => "EEXIST",
            18 => "EXDEV",
            19 => "ENODEV",
            20 => "ENOTDIR",
            21 => "EISDIR",
            22 => "EINVAL",
            23 => "ENFILE",
            24 => "EMFILE",
            25 => "ENOTTY",
            26 => "ETXTBSY",
            27 => "EFBIG",
            28 => "ENOSPC",
            29 => "ESPIPE",
            30 => "EROFS",
            31 => "EMLINK",
            32 => "EPIPE",
            33 => "EDOM",
            34 => "ERANGE",
            36 => "ENAMETOOLONG",
            38 => "ENOSYS",
            43 => "EIDRM",
            61 => "ENODATA",
            75 => "EOVERFLOW",
            87 => "EUSERS",
            95 => "EOPNOTSUPP",
            110 => "ETIMEDOUT",
            3840 => "RESTART_IF_SA_RESTART",
            3841 => "RESTART_ALWAYS",
            3842 => "RESTART_IF_NO_HANDLER",
            3843 => "RESTART_WITH_BLOCK",
            _ => return None,
        })
    }
}

impl fmt::Debug for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "Errno({})", self.0),
        }
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// What a system call handler returns: the value for `rax`, or an error
/// that becomes `-errno` in `rax`.
pub type SysResult = Result<u64, Errno>;

/// Encode a handler's result for `rax`.
pub const fn encode(result: SysResult) -> u64 {
    match result {
        Ok(value) => value,
        Err(errno) => errno.to_rax(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rax_encoding_round_trips() {
        assert_eq!(Errno::ENOSYS.to_rax() as i64, -38);
        assert_eq!(Errno::from_rax(Errno::EBADF.to_rax()), Some(Errno::EBADF));
        assert_eq!(Errno::from_rax(0), None);
        assert_eq!(Errno::from_rax(4095u64.wrapping_neg()), Errno::new(4095));
        assert_eq!(Errno::from_rax(4096u64.wrapping_neg()), None);
        assert_eq!(encode(Ok(5)), 5);
        assert_eq!(encode(Err(Errno::EINVAL)) as i64, -22);
    }

    #[test]
    fn new_rejects_values_outside_the_abi() {
        assert_eq!(Errno::new(0), None);
        assert_eq!(Errno::new(4096), None);
        assert_eq!(Errno::new(14), Some(Errno::EFAULT));
    }

    #[test]
    fn names() {
        extern crate std;
        use std::format;
        assert_eq!(format!("{:?}", Errno::ENOTTY), "ENOTTY");
        assert_eq!(format!("{}", Errno::new(200).unwrap()), "Errno(200)");
    }
}
