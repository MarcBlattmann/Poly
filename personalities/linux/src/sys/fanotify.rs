//! fanotify_init(2) and fanotify_mark(2): file system events for a
//! notification group (fanotify(7), UAPI `linux/fanotify.h`). The group,
//! its marks and its events are the kernel's (`poly_vfs::fanotify`); this
//! checks the arguments in the order the calls report errors.

use super::pipe::file_io;
use crate::args::Args;
use crate::cred;
use crate::errno::{Errno, SysResult};
use crate::fileio::NewFd;
use crate::kernel::LinuxKernel;
use crate::sys::path::nonempty_path;
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// Flags and mask bits (UAPI `linux/fanotify.h`).
pub mod bits {
    pub const FAN_CLOEXEC: u32 = 0x0000_0001;
    pub const FAN_NONBLOCK: u32 = 0x0000_0002;
    pub const FAN_CLASS_CONTENT: u32 = 0x0000_0004;
    pub const FAN_CLASS_PRE_CONTENT: u32 = 0x0000_0008;
    pub const FAN_CLASS_BITS: u32 = FAN_CLASS_CONTENT | FAN_CLASS_PRE_CONTENT;
    pub const FAN_UNLIMITED_QUEUE: u32 = 0x0000_0010;
    pub const FAN_UNLIMITED_MARKS: u32 = 0x0000_0020;
    pub const FAN_ENABLE_AUDIT: u32 = 0x0000_0040;
    pub const FAN_REPORT_PIDFD: u32 = 0x0000_0080;
    pub const FAN_REPORT_TID: u32 = 0x0000_0100;
    pub const FAN_REPORT_FID: u32 = 0x0000_0200;
    pub const FAN_REPORT_DIR_FID: u32 = 0x0000_0400;
    pub const FAN_REPORT_NAME: u32 = 0x0000_0800;
    pub const FAN_REPORT_TARGET_FID: u32 = 0x0000_1000;
    pub const FAN_REPORT_FD_ERROR: u32 = 0x0000_2000;
    pub const FAN_REPORT_MNT: u32 = 0x0000_4000;
    /// Every `fanotify_init` flag.
    pub const INIT_FLAGS: u32 = 0x0000_7fff;
    /// The flags that make a group report file ids.
    pub const FID_BITS: u32 = FAN_REPORT_FID | FAN_REPORT_DIR_FID | FAN_REPORT_NAME | FAN_REPORT_TARGET_FID;
    /// What only `CAP_SYS_ADMIN` may ask for (fanotify_init's spec sheet).
    pub const ADMIN_FLAGS: u32 =
        FAN_CLASS_BITS | FAN_UNLIMITED_QUEUE | FAN_UNLIMITED_MARKS | FAN_ENABLE_AUDIT | FAN_REPORT_PIDFD;

    pub const FAN_MARK_ADD: u32 = 0x0000_0001;
    pub const FAN_MARK_REMOVE: u32 = 0x0000_0002;
    pub const FAN_MARK_DONT_FOLLOW: u32 = 0x0000_0004;
    pub const FAN_MARK_ONLYDIR: u32 = 0x0000_0008;
    pub const FAN_MARK_MOUNT: u32 = 0x0000_0010;
    pub const FAN_MARK_IGNORED_MASK: u32 = 0x0000_0020;
    pub const FAN_MARK_IGNORED_SURV_MODIFY: u32 = 0x0000_0040;
    pub const FAN_MARK_FLUSH: u32 = 0x0000_0080;
    pub const FAN_MARK_FILESYSTEM: u32 = 0x0000_0100;
    pub const FAN_MARK_EVICTABLE: u32 = 0x0000_0200;
    pub const FAN_MARK_IGNORE: u32 = 0x0000_0400;
    /// Every `fanotify_mark` flag.
    pub const MARK_FLAGS: u32 = 0x0000_07ff;

    pub const FAN_OPEN_PERM: u64 = 0x0001_0000;
    pub const FAN_ACCESS_PERM: u64 = 0x0002_0000;
    pub const FAN_OPEN_EXEC_PERM: u64 = 0x0004_0000;
    pub const FAN_PRE_ACCESS: u64 = 0x0010_0000;
    pub const PERM_EVENTS: u64 = FAN_OPEN_PERM | FAN_ACCESS_PERM | FAN_OPEN_EXEC_PERM | FAN_PRE_ACCESS;
    /// `FAN_ATTRIB`, `FAN_MOVED_FROM`, `FAN_MOVED_TO`, `FAN_CREATE`,
    /// `FAN_DELETE`, `FAN_DELETE_SELF`, `FAN_MOVE_SELF` and `FAN_RENAME`:
    /// events a descriptor-reporting group cannot ask for (fanotify_mark(2),
    /// ERRORS).
    pub const FID_ONLY_EVENTS: u64 = 0x0000_0004 | 0x0000_0fc0 | FAN_RENAME;
    pub const FAN_FS_ERROR: u64 = 0x0000_8000;
    pub const FAN_EVENT_ON_CHILD: u64 = 0x0800_0000;
    pub const FAN_RENAME: u64 = 0x1000_0000;
    pub const FAN_ONDIR: u64 = 0x4000_0000;
    /// Every bit a mark's mask may have.
    pub const MASK_BITS: u64 = 0x0000_1fff | FAN_FS_ERROR | PERM_EVENTS | FAN_EVENT_ON_CHILD | FAN_RENAME | FAN_ONDIR;
}

use bits::*;

/// What `fanotify_mark` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkOp {
    Add,
    Remove,
    Flush,
}

/// What a mark is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkWhat {
    Inode,
    Mount,
    Filesystem,
}

/// A checked `fanotify_mark` request for the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkRequest<'a> {
    pub op: MarkOp,
    pub what: MarkWhat,
    /// The mask is the ignored mask (`FAN_MARK_IGNORED_MASK` /
    /// `FAN_MARK_IGNORE`).
    pub ignored: bool,
    pub mask: u32,
    /// The object: `path` from `dirfd`, or with no path `dirfd`'s own
    /// file (the working directory for `AT_FDCWD`).
    pub dirfd: i32,
    pub path: Option<&'a [u8]>,
    pub follow: bool,
    /// `FAN_MARK_ONLYDIR`: `ENOTDIR` for anything but a directory.
    pub onlydir: bool,
}

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::FANOTIFY_INIT, fanotify_init), (nr::FANOTIFY_MARK, fanotify_mark)];

/// The open flags `event_f_flags` may have (`O_ACCMODE`, `O_APPEND`,
/// `O_NONBLOCK`, `O_DSYNC`, `O_SYNC`, `O_LARGEFILE`, `O_NOATIME`,
/// `O_CLOEXEC`; UAPI `asm-generic/fcntl.h`).
pub const EVENT_F_BITS: u32 = 0o3 | 0o2000 | 0o4000 | 0o10000 | 0o4010000 | 0o100000 | 0o1000000 | 0o2000000;

/// fanotify_init's checks, in its order: without `CAP_SYS_ADMIN` a group
/// that reports no file ids or asks for an admin flag is `EPERM`; then
/// unknown flags, bad `event_f_flags`, two classes, file ids with a
/// permission class, `FAN_REPORT_NAME` without `FAN_REPORT_DIR_FID`,
/// `FAN_REPORT_TARGET_FID` without the others, `FAN_REPORT_TID` with
/// `FAN_REPORT_PIDFD`: `EINVAL`.
pub fn check_init(flags: u32, event_flags: u32, privileged: bool) -> Result<(), Errno> {
    if !privileged && (flags & ADMIN_FLAGS != 0 || flags & FID_BITS == 0) {
        return Err(Errno::EPERM);
    }
    if flags & !INIT_FLAGS != 0 || event_flags & !EVENT_F_BITS != 0 || event_flags & 0o3 == 0o3 {
        return Err(Errno::EINVAL);
    }
    let class = flags & FAN_CLASS_BITS;
    if class == FAN_CLASS_BITS {
        return Err(Errno::EINVAL);
    }
    let fid = flags & (FAN_REPORT_FID | FAN_REPORT_DIR_FID | FAN_REPORT_NAME | FAN_REPORT_TARGET_FID) != 0;
    if fid && class != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & FAN_REPORT_NAME != 0 && flags & FAN_REPORT_DIR_FID == 0 {
        return Err(Errno::EINVAL);
    }
    let target = FAN_REPORT_FID | FAN_REPORT_DIR_FID | FAN_REPORT_NAME;
    if flags & FAN_REPORT_TARGET_FID != 0 && flags & target != target {
        return Err(Errno::EINVAL);
    }
    if flags & (FAN_REPORT_TID | FAN_REPORT_PIDFD) == FAN_REPORT_TID | FAN_REPORT_PIDFD {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// fanotify_init(flags, event_f_flags).
pub fn fanotify_init(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (flags, event_flags) = (a.uint(0), a.uint(1));
    let privileged = cred::current(k).capable(crate::cred::cap::SYS_ADMIN);
    check_init(flags, event_flags, privileged)?;
    let fd = NewFd { cloexec: flags & FAN_CLOEXEC != 0, nonblock: flags & FAN_NONBLOCK != 0 };
    file_io(k)?.fanotify_init(flags, event_flags, privileged, fd).map(|fd| fd as u64)
}

/// The checks of fanotify_mark's flags and mask that need no group:
/// unknown flags, both `FAN_MARK_MOUNT` and `FAN_MARK_FILESYSTEM`, not
/// exactly one of add, remove and flush, unknown mask bits, an empty mask
/// to add: `EINVAL`.
pub fn decode_mark(flags: u32, mask: u64) -> Result<(MarkOp, MarkWhat), Errno> {
    if flags & !MARK_FLAGS != 0 {
        return Err(Errno::EINVAL);
    }
    let what = match flags & (FAN_MARK_MOUNT | FAN_MARK_FILESYSTEM) {
        0 => MarkWhat::Inode,
        FAN_MARK_MOUNT => MarkWhat::Mount,
        FAN_MARK_FILESYSTEM => MarkWhat::Filesystem,
        _ => return Err(Errno::EINVAL),
    };
    let op = match flags & (FAN_MARK_ADD | FAN_MARK_REMOVE | FAN_MARK_FLUSH) {
        FAN_MARK_ADD => MarkOp::Add,
        FAN_MARK_REMOVE => MarkOp::Remove,
        FAN_MARK_FLUSH => MarkOp::Flush,
        _ => return Err(Errno::EINVAL),
    };
    if op != MarkOp::Flush && mask & !MASK_BITS != 0 {
        return Err(Errno::EINVAL);
    }
    if op == MarkOp::Add && mask & !(FAN_ONDIR | FAN_EVENT_ON_CHILD) == 0 {
        return Err(Errno::EINVAL);
    }
    Ok((op, what))
}

/// fanotify_mark(fanotify_fd, flags, mask, dirfd, pathname): the flags and
/// mask ([`decode_mark`]), the group (`EBADF`; `EINVAL` if no group),
/// permission events on a notification group and file-id-only events on
/// a descriptor group (`EINVAL`), mount and file system marks from a group
/// made without `CAP_SYS_ADMIN` (`EPERM`), then the path (the kernel:
/// `ENOENT`, `ENOTDIR` with `FAN_MARK_ONLYDIR`, `ENOENT` to remove what is
/// not marked).
pub fn fanotify_mark(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, flags, mask, dirfd) = (a.fd(0), a.uint(1), a.ulong(2), a.fd(3));
    let (op, what) = decode_mark(flags, mask)?;
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let (gflags, privileged) = file_io(k)?.fanotify_group(fd)?.ok_or(Errno::EINVAL)?;
    if op != MarkOp::Flush {
        if mask & PERM_EVENTS != 0 && gflags & FAN_CLASS_BITS == 0 {
            return Err(Errno::EINVAL);
        }
        if mask & FID_ONLY_EVENTS != 0 && gflags & (FAN_REPORT_FID | FAN_REPORT_DIR_FID) == 0 {
            return Err(Errno::EINVAL);
        }
    }
    if what != MarkWhat::Inode && !privileged {
        return Err(Errno::EPERM);
    }
    let path: Option<Vec<u8>> =
        if op == MarkOp::Flush || a.ptr(4).is_null() { None } else { Some(nonempty_path(k, a.ptr(4))?) };
    let req = MarkRequest {
        op,
        what,
        ignored: flags & (FAN_MARK_IGNORED_MASK | FAN_MARK_IGNORE) != 0,
        mask: mask as u32,
        dirfd,
        path: path.as_deref(),
        follow: flags & FAN_MARK_DONT_FOLLOW == 0,
        onlydir: flags & FAN_MARK_ONLYDIR != 0,
    };
    file_io(k)?.fanotify_mark(fd, &req).map(|()| 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_rules() {
        assert_eq!(check_init(1 << 30, 0, true), Err(Errno::EINVAL));
        assert_eq!(check_init(FAN_CLASS_BITS, 0, true), Err(Errno::EINVAL));
        assert_eq!(check_init(0, 0o100, true), Err(Errno::EINVAL), "O_CREAT");
        assert_eq!(check_init(FAN_REPORT_NAME, 0, true), Err(Errno::EINVAL));
        assert_eq!(check_init(FAN_REPORT_FID | FAN_CLASS_CONTENT, 0, true), Err(Errno::EINVAL));
        assert_eq!(check_init(FAN_CLOEXEC | FAN_NONBLOCK, 0, true), Ok(()));
        assert_eq!(check_init(FAN_CLASS_CONTENT | FAN_UNLIMITED_QUEUE | FAN_UNLIMITED_MARKS, 0, true), Ok(()));
        assert_eq!(check_init(FAN_REPORT_FID | FAN_REPORT_DIR_FID | FAN_REPORT_NAME, 0, true), Ok(()));
        // Without CAP_SYS_ADMIN.
        assert_eq!(check_init(0, 0, false), Err(Errno::EPERM));
        assert_eq!(check_init(FAN_REPORT_FID | FAN_UNLIMITED_QUEUE, 0, false), Err(Errno::EPERM));
        assert_eq!(check_init(FAN_REPORT_FID | FAN_CLASS_CONTENT, 0, false), Err(Errno::EPERM));
        assert_eq!(check_init(FAN_REPORT_NAME, 0, false), Err(Errno::EINVAL), "a lone FAN_REPORT_NAME");
        assert_eq!(check_init(FAN_REPORT_FID | FAN_CLOEXEC | FAN_NONBLOCK, 0, false), Ok(()));
        assert_eq!(check_init(FAN_REPORT_DIR_FID | FAN_REPORT_NAME, 0, false), Ok(()));
    }

    #[test]
    fn mark_flags() {
        assert_eq!(decode_mark(FAN_MARK_ADD, 0x20), Ok((MarkOp::Add, MarkWhat::Inode)));
        assert_eq!(decode_mark(FAN_MARK_ADD | FAN_MARK_REMOVE, 0x20), Err(Errno::EINVAL));
        assert_eq!(decode_mark(0, 0x20), Err(Errno::EINVAL));
        assert_eq!(decode_mark(FAN_MARK_ADD, 0), Err(Errno::EINVAL));
        assert_eq!(decode_mark(FAN_MARK_ADD, FAN_ONDIR), Err(Errno::EINVAL));
        assert_eq!(decode_mark(FAN_MARK_ADD, 1 << 40), Err(Errno::EINVAL));
        assert_eq!(decode_mark(1 << 20, 0x20), Err(Errno::EINVAL));
        assert_eq!(decode_mark(FAN_MARK_ADD | FAN_MARK_MOUNT | FAN_MARK_FILESYSTEM, 0x20), Err(Errno::EINVAL));
        assert_eq!(decode_mark(FAN_MARK_FLUSH, 0), Ok((MarkOp::Flush, MarkWhat::Inode)));
        assert_eq!(decode_mark(FAN_MARK_FLUSH | FAN_MARK_MOUNT, 0), Ok((MarkOp::Flush, MarkWhat::Mount)));
        assert_eq!(decode_mark(FAN_MARK_REMOVE | FAN_MARK_FILESYSTEM, 0x8), Ok((MarkOp::Remove, MarkWhat::Filesystem)));
    }
}
