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
//! - **setxattr**: `flags` with a bit other than `XATTR_CREATE` and
//!   `XATTR_REPLACE` are `EINVAL` (both together are valid: `EEXIST` if
//!   the name exists, `ENODATA` if not; ADR 0113), a value of more than
//!   `XATTR_SIZE_MAX` (64 KiB) bytes is `E2BIG`; a `size` of 0 sets an
//!   empty value (`value` is then not read).
//! - **Order** (the spec sheets): the flags, the name, the size and the
//!   value are checked before the file is looked at (a bad name with a
//!   NULL path is `ERANGE`, with a bad descriptor too); then the kernel's
//!   checks in the order `poly_vfs::xattr` gives.
//! - **Sizes**: a buffer size above the limit counts as the limit; a name
//!   list longer than `XATTR_LIST_MAX` (tmpfs keeps more) is `E2BIG` for
//!   a buffer of that size or more, `ERANGE` for a smaller one, and its
//!   length for size 0.
//! - **Sockets** have one attribute, `system.sockprotoname`, read-only
//!   ([`SOCKPROTONAME`]).
//! - The path calls resolve from the working directory (an empty path is
//!   `ENOENT`); the `l` calls do not follow a final symbolic link; the `f`
//!   calls take a descriptor (`EBADF` for a negative one).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::XattrTarget;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::net::{OptName, OptValue, abi};
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
pub(crate) fn name_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
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
/// and listxattr(2) do; returns its length. `size` 0 asks for the length
/// only; a size above `max` counts as `max`. Data longer than the buffer
/// is `ERANGE`, or `E2BIG` when the buffer is `max` bytes already (a name
/// list longer than `XATTR_LIST_MAX` can never be returned).
pub(crate) fn reply(k: &mut dyn LinuxKernel, data: &[u8], buf: UserPtr, size: u64, max: u64) -> SysResult {
    let len = data.len() as u64;
    if size == 0 {
        return Ok(len);
    }
    let size = size.min(max);
    if len > size {
        return Err(if size == max { Errno::E2BIG } else { Errno::ERANGE });
    }
    k.copy_to_user(buf, data)?;
    Ok(len)
}

/// A socket's one attribute (fgetxattr's spec sheet).
pub const SOCKPROTONAME: &[u8] = b"system.sockprotoname";

/// The value of `system.sockprotoname` if `fd` is a socket: its
/// protocol's name with the terminating NUL, `UNIX-STREAM` for a Unix
/// stream socket and `UNIX` for a Unix datagram socket, as the spec sheet
/// observed. `None` for anything else, also for sockets whose name Poly
/// does not know: the name then goes to the kernel, which answers
/// `EOPNOTSUPP` (no handler).
fn sock_proto_name(k: &mut dyn LinuxKernel, fd: i32) -> Option<&'static [u8]> {
    let net = k.net()?;
    let OptValue::Int(domain) = net.getsockopt(fd, OptName::Domain).ok()? else { return None };
    let OptValue::Int(kind) = net.getsockopt(fd, OptName::Type).ok()? else { return None };
    match (u16::try_from(domain).ok()?, u32::try_from(kind).ok()?) {
        (abi::AF_UNIX, abi::SOCK_STREAM) => Some(b"UNIX-STREAM\0"),
        (abi::AF_UNIX, abi::SOCK_DGRAM) => Some(b"UNIX\0"),
        _ => None,
    }
}

/// setxattr's flags, the low 32 bits of the register: `EINVAL` for any
/// bit but `XATTR_CREATE` and `XATTR_REPLACE` (both together are valid
/// and fail with `EEXIST` or `ENODATA`).
fn set_flags(a: &Args) -> Result<u32, Errno> {
    let flags = a.uint(4);
    if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 { Err(Errno::EINVAL) } else { Ok(flags) }
}

/// setxattr: the flags, the name, the size, the value, then the file
/// (setxattr's spec sheet: a bad name with a NULL path is `ERANGE`, a bad
/// value with a bad descriptor `EFAULT`).
fn set(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let flags = set_flags(a)?;
    let name = name_arg(k, a.ptr(1))?;
    let size = a.ulong(3);
    if size > XATTR_SIZE_MAX {
        return Err(Errno::E2BIG);
    }
    let mut value = bounce(size as usize)?;
    if size > 0 {
        k.copy_from_user(a.ptr(2), &mut value)?;
    }
    let target = Owned::load(k, a, by)?;
    fs(k)?.setxattr(target.target(), &name, &value, flags).map(|()| 0)
}

/// getxattr: the name, then the file.
fn get(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let name = name_arg(k, a.ptr(1))?;
    let target = Owned::load(k, a, by)?;
    if let Owned::Fd(fd) = target
        && name == SOCKPROTONAME
        && let Some(proto) = sock_proto_name(k, fd)
    {
        return reply(k, proto, a.ptr(2), a.ulong(3), XATTR_SIZE_MAX);
    }
    let value = fs(k)?.getxattr(target.target(), &name)?;
    reply(k, &value, a.ptr(2), a.ulong(3), XATTR_SIZE_MAX)
}

/// listxattr: the file's names, and a socket's `system.sockprotoname`.
fn list(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let target = Owned::load(k, a, by)?;
    let mut names = fs(k)?.listxattr(target.target())?;
    if let Owned::Fd(fd) = target
        && sock_proto_name(k, fd).is_some()
    {
        names.extend_from_slice(SOCKPROTONAME);
        names.push(0);
    }
    reply(k, &names, a.ptr(1), a.ulong(2), XATTR_LIST_MAX)
}

/// removexattr: the name, then the file.
fn remove(k: &mut dyn LinuxKernel, a: &Args, by: By) -> SysResult {
    let name = name_arg(k, a.ptr(1))?;
    let target = Owned::load(k, a, by)?;
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
