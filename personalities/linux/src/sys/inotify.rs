//! inotify_init(2), inotify_init1(2), inotify_add_watch(2),
//! inotify_rm_watch(2): file system events (inotify(7)). The instance,
//! its queue and the events are the kernel's (`poly_vfs::inotify`); this
//! checks the arguments in the order the calls report errors.

use super::pipe::file_io;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{NewFd, flags};
use crate::kernel::LinuxKernel;
use crate::sys::path::nonempty_path;
use crate::table::{Handler, nr};

/// Bits of an `inotify_add_watch` mask (UAPI `linux/inotify.h`).
pub mod mask {
    /// Every event bit a watch can ask for (`IN_ACCESS` .. `IN_MOVE_SELF`).
    pub const IN_ALL_EVENTS: u32 = 0x0000_0fff;
    pub const IN_UNMOUNT: u32 = 0x0000_2000;
    pub const IN_Q_OVERFLOW: u32 = 0x0000_4000;
    pub const IN_IGNORED: u32 = 0x0000_8000;
    pub const IN_ONLYDIR: u32 = 0x0100_0000;
    pub const IN_DONT_FOLLOW: u32 = 0x0200_0000;
    pub const IN_EXCL_UNLINK: u32 = 0x0400_0000;
    pub const IN_MASK_CREATE: u32 = 0x1000_0000;
    pub const IN_MASK_ADD: u32 = 0x2000_0000;
    pub const IN_ISDIR: u32 = 0x4000_0000;
    pub const IN_ONESHOT: u32 = 0x8000_0000;
    /// Every bit the call accepts.
    pub const KNOWN: u32 = IN_ALL_EVENTS
        | IN_UNMOUNT
        | IN_Q_OVERFLOW
        | IN_IGNORED
        | IN_ONLYDIR
        | IN_DONT_FOLLOW
        | IN_EXCL_UNLINK
        | IN_MASK_CREATE
        | IN_MASK_ADD
        | IN_ISDIR
        | IN_ONESHOT;
}

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::INOTIFY_INIT, inotify_init),
    (nr::INOTIFY_ADD_WATCH, inotify_add_watch),
    (nr::INOTIFY_RM_WATCH, inotify_rm_watch),
    (nr::INOTIFY_INIT1, inotify_init1),
];

/// inotify_init(): inotify_init1(0).
pub fn inotify_init(k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    make(k, 0)
}

/// inotify_init1(flags): `IN_NONBLOCK` and `IN_CLOEXEC` (`O_NONBLOCK`,
/// `O_CLOEXEC`); any other bit of the low 32 is `EINVAL`, checked before
/// a descriptor is looked for.
pub fn inotify_init1(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make(k, a.uint(0))
}

fn make(k: &mut dyn LinuxKernel, bits: u32) -> SysResult {
    if bits & !(flags::O_NONBLOCK | flags::O_CLOEXEC) != 0 {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.inotify_init(NewFd::from_flags(bits)).map(|fd| fd as u64)
}

/// Whether `m` (the low 32 bits of the argument) is a mask the call
/// accepts: some known bit, no unknown one, not both `IN_MASK_ADD` and
/// `IN_MASK_CREATE`.
pub const fn valid_mask(m: u32) -> bool {
    use mask::*;
    m & KNOWN != 0 && m & !KNOWN == 0 && m & (IN_MASK_ADD | IN_MASK_CREATE) != (IN_MASK_ADD | IN_MASK_CREATE)
}

/// inotify_add_watch(fd, path, mask): the mask first (`EINVAL`), then the
/// descriptor (`EBADF`; `EINVAL` if it is no inotify instance), then the
/// path (`EFAULT`, `ENOENT` ...); the kernel then checks read permission
/// (`EACCES`), `IN_ONLYDIR` (`ENOTDIR`) and `IN_MASK_CREATE` (`EEXIST`).
pub fn inotify_add_watch(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let m = a.uint(2);
    if !valid_mask(m) {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    if !file_io(k)?.is_inotify(fd)? {
        return Err(Errno::EINVAL);
    }
    let path = nonempty_path(k, a.ptr(1))?;
    file_io(k)?.inotify_add_watch(fd, &path, m).map(|wd| wd as u64)
}

/// inotify_rm_watch(fd, wd): `EBADF`, then `EINVAL` for a descriptor that
/// is no inotify instance or a watch it does not have.
pub fn inotify_rm_watch(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    if !file_io(k)?.is_inotify(fd)? {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.inotify_rm_watch(fd, a.int(1)).map(|()| 0)
}

#[cfg(test)]
mod tests {
    use super::mask::*;
    use super::valid_mask;

    #[test]
    fn masks() {
        assert!(!valid_mask(0));
        assert!(valid_mask(IN_ONLYDIR | IN_MASK_ADD), "flag bits alone are accepted");
        assert!(valid_mask(IN_IGNORED));
        assert!(!valid_mask(0x100 | 0x1000));
        assert!(!valid_mask(0x100 | 0x10000));
        assert!(!valid_mask(0x100 | 0x80_0000));
        assert!(!valid_mask(0x100 | IN_MASK_ADD | IN_MASK_CREATE));
        assert!(valid_mask(IN_ALL_EVENTS | IN_ONESHOT | IN_EXCL_UNLINK | IN_DONT_FOLLOW));
    }
}
