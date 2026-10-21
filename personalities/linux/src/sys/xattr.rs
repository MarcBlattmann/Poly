//! Extended attributes (xattr(7)): getxattr(2), lgetxattr, fgetxattr,
//! setxattr(2), lsetxattr, fsetxattr, listxattr(2), llistxattr,
//! flistxattr, removexattr(2), lremovexattr, fremovexattr.
//!
//! The personality copies the arguments in and the results out and checks
//! what the man pages say about sizes; the kernel ([`LinuxFs::getxattr`]
//! and friends, over `poly_vfs::xattr`) checks namespaces and permissions
//! and keeps the attributes.
//!
//! - A **name** is a NUL-terminated string of 1 to `XATTR_NAME_MAX` (255)
//!   bytes; an empty or longer one is `ERANGE` ("the size of name ...
//!   exceeds a filesystem-specific limit").
//! - **getxattr** with `size` 0 returns the value's size without copying
//!   ("an empty buffer of size zero can be passed in order to obtain the
//!   current size of the named extended attribute"); a value larger than
//!   `size` is `ERANGE`. The same for **listxattr** and its name list.
//! - **setxattr**: `flags` other than `XATTR_CREATE` and `XATTR_REPLACE`,
//!   or both (ADR 0059's choice, to be confirmed by the diff runner), are
//!   `EINVAL`, a value of more than `XATTR_SIZE_MAX` (64 KiB) bytes is
//!   `E2BIG`; a `size` of 0 sets an empty value (`value` is then not read).
//! - The path calls resolve from the working directory (an empty path is
//!   `ENOENT`); the `l` calls do not follow a final symbolic link; the `f`
//!   calls take a descriptor (`EBADF` for a negative one).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::XattrTarget;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use crate::sys::path::{fs, nonempty_path};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

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

/// Limits (UAPI `linux/limits.h`).
pub const XATTR_NAME_MAX: usize = 255;
pub const XATTR_SIZE_MAX: u64 = 65536;
pub const XATTR_LIST_MAX: u64 = 65536;
/// setxattr(2) flags (UAPI `linux/xattr.h`).
pub const XATTR_CREATE: u32 = 0x1;
pub const XATTR_REPLACE: u32 = 0x2;

/// Which file: argument 0 as a path or a descriptor.
#[derive(Clone, Copy)]
enum By {
    Path { follow: bool },
    Fd,
}

/// The target in argument 0, owned (a path is copied in).
enum Owned {
    Path(Vec<u8>, bool),
    Fd(i32),
}

impl Owned {
    fn load(k: &dyn LinuxKernel, a: &Args, by: By) -> Result<Owned, Errno> {
        match by {
            By::Path { follow } => Ok(Owned::Path(nonempty_path(k, a.ptr(0))?, follow)),
            By::Fd if a.fd(0) < 0 => Err(Errno::EBADF),
            By::Fd => Ok(Owned::Fd(a.fd(0))),
        }
    }

    fn target(&self) -> XattrTarget<'_> {
        match self {
            Owned::Path(p, follow) => XattrTarget::Path { path: p, follow: *follow },
            Owned::Fd(fd) => XattrTarget::Fd(*fd),
        }
    }
}

/// An attribute name: 1 to [`XATTR_NAME_MAX`] bytes before the NUL.
fn name_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    let mut buf = bounce(XATTR_NAME_MAX + 1)?;
    let n = match k.strncpy_from_user(p, &mut buf) {
        Err(Errno::ENAMETOOLONG) => return Err(Errno::ERANGE),
        r => r?,
    };
    if n == 0 {
        return Err(Errno::ERANGE);
    }
    buf.truncate(n);
    Ok(buf)
}

/// Copy `data` out to a caller's buffer of `size` bytes as getxattr(2)
/// and listxattr(2) do; returns its length.
fn reply(k: &mut dyn LinuxKernel, data: &[u8], buf: UserPtr, size: u64, max: u64) -> SysResult {
    let len = data.len() as u64;
    if len > max {
        return Err(Errno::E2BIG);
    }
    if size == 0 {
        return Ok(len);
    }
    if len > size {
        return Err(Errno::ERANGE);
    }
    k.copy_to_user(buf, data)?;
    Ok(len)
}

fn set(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let target = Owned::load(k, a, by)?;
    let flags = a.uint(4);
    if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 || flags == XATTR_CREATE | XATTR_REPLACE {
        return Err(Errno::EINVAL);
    }
    let name = name_arg(k, a.ptr(1))?;
    let size = a.ulong(3);
    if size > XATTR_SIZE_MAX {
        return Err(Errno::E2BIG);
    }
    let mut value = bounce(size as usize)?;
    if size > 0 {
        k.copy_from_user(a.ptr(2), &mut value)?;
    }
    fs(k)?.setxattr(target.target(), &name, &value, flags).map(|()| 0)
}

fn get(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let target = Owned::load(k, a, by)?;
    let name = name_arg(k, a.ptr(1))?;
    let value = fs(k)?.getxattr(target.target(), &name)?;
    reply(k, &value, a.ptr(2), a.ulong(3), XATTR_SIZE_MAX)
}

fn list(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let target = Owned::load(k, a, by)?;
    let names = fs(k)?.listxattr(target.target())?;
    reply(k, &names, a.ptr(1), a.ulong(2), XATTR_LIST_MAX)
}

fn remove(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let target = Owned::load(k, a, by)?;
    let name = name_arg(k, a.ptr(1))?;
    fs(k)?.removexattr(target.target(), &name).map(|()| 0)
}

const FOLLOW: By = By::Path { follow: true };
const NOFOLLOW: By = By::Path { follow: false };

/// setxattr(path, name, value, size, flags)
pub fn setxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, FOLLOW)
}

/// lsetxattr(path, name, value, size, flags)
pub fn lsetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, NOFOLLOW)
}

/// fsetxattr(fd, name, value, size, flags)
pub fn fsetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    set(k, a, By::Fd)
}

/// getxattr(path, name, value, size)
pub fn getxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, FOLLOW)
}

/// lgetxattr(path, name, value, size)
pub fn lgetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, NOFOLLOW)
}

/// fgetxattr(fd, name, value, size)
pub fn fgetxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    get(k, a, By::Fd)
}

/// listxattr(path, list, size)
pub fn listxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, FOLLOW)
}

/// llistxattr(path, list, size)
pub fn llistxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, NOFOLLOW)
}

/// flistxattr(fd, list, size)
pub fn flistxattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    list(k, a, By::Fd)
}

/// removexattr(path, name)
pub fn removexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    remove(k, a, FOLLOW)
}

/// lremovexattr(path, name)
pub fn lremovexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    remove(k, a, NOFOLLOW)
}

/// fremovexattr(fd, name)
pub fn fremovexattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    remove(k, a, By::Fd)
}
