//! What the file attribute calls ask of the kernel: which file, and what
//! to change or check ([`crate::kernel::LinuxKernel::change_attr`],
//! [`crate::kernel::LinuxKernel::check_access`]).

use crate::abi::Timespec;

/// The file an attribute call acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileTarget<'a> {
    /// `path` (not empty), relative to `dirfd` (`AT_FDCWD` or a directory
    /// descriptor) unless absolute; a final symbolic link is followed if
    /// `follow` is set.
    Path { dirfd: i32, path: &'a [u8], follow: bool },
    /// The open file `fd` (fchmod(2), fchown(2), futimens(3)): `EBADF` if
    /// it is not open or was opened with `O_PATH`.
    Fd(i32),
    /// What `dirfd` names with an empty path and `AT_EMPTY_PATH`: the file
    /// it is open on (`O_PATH` descriptors included), or the working
    /// directory for `AT_FDCWD`.
    EmptyPath(i32),
}

/// A time for utimensat(2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeChange {
    /// `UTIME_NOW`.
    Now,
    /// An explicit time (nanoseconds already checked to be 0-999999999).
    At(Timespec),
}

/// An attribute change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrChange {
    /// chmod(2): the permission bits, set-user-ID, set-group-ID and sticky
    /// bit (`mode & 07777`).
    Mode(u16),
    /// chown(2): `None` for `-1`, "unchanged".
    Owner { uid: Option<u32>, gid: Option<u32> },
    /// utimensat(2): `None` for `UTIME_OMIT`; never both `None`.
    Times { atime: Option<TimeChange>, mtime: Option<TimeChange> },
}
