//! Extended attributes (xattr(7)): getxattr(2), setxattr(2),
//! listxattr(2), removexattr(2) and their `l` and `f` forms (ADR 0059; GNU
//! `ls -l` asks for ACLs and security labels, `cp -a` copies attributes).
//!
//! No file system of Poly stores extended attributes yet, so every call
//! that names a file that exists answers as the manual pages say for a file
//! system without them: "ENOTSUP Extended attributes are not supported by
//! the filesystem, or are disabled". The file is looked up first, so a
//! missing one is `ENOENT` (`EBADF` for a descriptor that is not open), as
//! for stat(2); the `l` forms do not follow a final symbolic link. The
//! name is checked before the answer: an empty name, or one longer than
//! `XATTR_NAME_MAX` (255) bytes, is `ERANGE` ("the size of name is too
//! large"; the empty name is Poly's choice, to be confirmed by the diff
//! runner); `setxattr` with flags other than `XATTR_CREATE` and
//! `XATTR_REPLACE` (`linux/xattr.h`), or both, is `EINVAL`.

use crate::abi::MAX_RW_COUNT;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::at::AT_FDCWD;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::path::{fs, nonempty_path};
use crate::table::{Handler, nr};

/// `XATTR_NAME_MAX` (`linux/limits.h`).
pub const XATTR_NAME_MAX: usize = 255;
/// setxattr(2) flags (`linux/xattr.h`).
pub const XATTR_CREATE: u32 = 0x1;
pub const XATTR_REPLACE: u32 = 0x2;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::SETXATTR, setxattr),
    (nr::LSETXATTR, lsetxattr),
    (nr::FSETXATTR, fsetxattr),
    (nr::GETXATTR, getxattr),
    (nr::LGETXATTR, lgetxattr),
    (nr::FGETXATTR, fgetxattr),
    (nr::LISTXATTR, listxattr),
    (nr::LLISTXATTR, llistxattr),
    (nr::FLISTXATTR, flistxattr),
    (nr::REMOVEXATTR, removexattr),
    (nr::LREMOVEXATTR, lremovexattr),
    (nr::FREMOVEXATTR, fremovexattr),
];

/// Which file a call is about.
enum Subject {
    Path { follow: bool },
    Fd,
}

/// Look the file up (its existence is all that matters).
fn find(k: &mut dyn LinuxKernel, a: &Args, subject: Subject) -> Result<(), Errno> {
    match subject {
        Subject::Path { follow } => {
            let path = nonempty_path(k, a.ptr(0))?;
            fs(k)?.statat(AT_FDCWD, &path, follow).map(|_| ())
        }
        Subject::Fd => {
            let fd = a.fd(0);
            if fd < 0 {
                return Err(Errno::EBADF);
            }
            k.fstat(fd).map(|_| ())
        }
    }
}

/// The attribute name at `p`: `ERANGE` when empty or too long.
fn check_name(k: &dyn LinuxKernel, p: UserPtr) -> Result<(), Errno> {
    let mut buf = [0u8; XATTR_NAME_MAX + 1];
    let n = k.strncpy_from_user(p, &mut buf).map_err(|e| if e == Errno::ENAMETOOLONG { Errno::ERANGE } else { e })?;
    if n == 0 || n > XATTR_NAME_MAX { Err(Errno::ERANGE) } else { Ok(()) }
}

fn get(k: &mut dyn LinuxKernel, a: &Args, subject: Subject) -> SysResult {
    find(k, a, subject)?;
    check_name(k, a.ptr(1))?;
    Err(Errno::EOPNOTSUPP)
}

fn set(k: &mut dyn LinuxKernel, a: &Args, subject: Subject) -> SysResult {
    let flags = a.uint(4);
    if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 || flags == XATTR_CREATE | XATTR_REPLACE {
        return Err(Errno::EINVAL);
    }
    if a.ulong(3) > MAX_RW_COUNT {
        return Err(Errno::E2BIG);
    }
    get(k, a, subject)
}

fn list(k: &mut dyn LinuxKernel, a: &Args, subject: Subject) -> SysResult {
    find(k, a, subject)?;
    Err(Errno::EOPNOTSUPP)
}

/// getxattr(path, name, value, size)
pub fn getxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Path { follow: true })
}

/// lgetxattr(path, name, value, size)
pub fn lgetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Path { follow: false })
}

/// fgetxattr(fd, name, value, size)
pub fn fgetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Fd)
}

/// setxattr(path, name, value, size, flags)
pub fn setxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, Subject::Path { follow: true })
}

/// lsetxattr(path, name, value, size, flags)
pub fn lsetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, Subject::Path { follow: false })
}

/// fsetxattr(fd, name, value, size, flags)
pub fn fsetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, Subject::Fd)
}

/// listxattr(path, list, size)
pub fn listxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, Subject::Path { follow: true })
}

/// llistxattr(path, list, size)
pub fn llistxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, Subject::Path { follow: false })
}

/// flistxattr(fd, list, size)
pub fn flistxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, Subject::Fd)
}

/// removexattr(path, name)
pub fn removexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Path { follow: true })
}

/// lremovexattr(path, name)
pub fn lremovexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Path { follow: false })
}

/// fremovexattr(fd, name)
pub fn fremovexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, Subject::Fd)
}
