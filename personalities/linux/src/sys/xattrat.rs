//! setxattrat(2), getxattrat(2), listxattrat(2), removexattrat(2): the
//! extended attribute calls with a directory descriptor and `at_flags`
//! (their spec sheets; everything about names, values and permissions is
//! as for [`super::xattr`]).
//!
//! - The file is `(dfd, path)` as for the other *at calls; a final
//!   symbolic link is followed unless `AT_SYMLINK_NOFOLLOW`; with
//!   `AT_EMPTY_PATH` an empty or NULL path names `dfd`'s file (`EBADF` for
//!   an `O_PATH` descriptor), without it an empty path is `ENOENT`. Other
//!   `at_flags` are `EINVAL`.
//! - setxattrat and getxattrat take the value and its size in a `struct
//!   xattr_args` (UAPI `linux/xattr.h`: `__aligned_u64 value; __u32 size;
//!   __u32 flags`) of `usize` bytes: at least `XATTR_ARGS_SIZE_VER0` (16,
//!   `EINVAL`), at most a page and larger only with zero bytes past the
//!   known ones (`E2BIG`) ([`ext_struct`]). setxattrat's `flags` are
//!   setxattr's; getxattrat's must be 0 (`EINVAL`).
//! - Order (setxattrat's spec sheet): the argument checks (`at_flags`,
//!   the structure, its flags, the name, the value) come before the path
//!   and the kernel's checks.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::XattrTarget;
use crate::fs::at::{AT_EMPTY_PATH, AT_SYMLINK_NOFOLLOW};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use crate::sys::path::{fs, nonempty_path, path_arg};
use crate::sys::xattr::{XATTR_CREATE, XATTR_LIST_MAX, XATTR_REPLACE, XATTR_SIZE_MAX, name_arg, reply};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::SETXATTRAT, setxattrat),
    (nr::GETXATTRAT, getxattrat),
    (nr::LISTXATTRAT, listxattrat),
    (nr::REMOVEXATTRAT, removexattrat),
];

/// `XATTR_ARGS_SIZE_VER0` (UAPI `linux/xattr.h`).
pub const XATTR_ARGS_SIZE_VER0: usize = 16;
/// The largest extensible structure the calls take: a page.
pub const EXT_STRUCT_MAX: u64 = 4096;

/// The `at_flags` these calls (and file_getattr / file_setattr) accept.
pub const AT_FLAGS: u32 = AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH;

/// `at_flags` with a bit other than [`AT_FLAGS`] is `EINVAL`.
pub fn check_at_flags(flags: u32) -> Result<(), Errno> {
    if flags & !AT_FLAGS != 0 { Err(Errno::EINVAL) } else { Ok(()) }
}

/// Whether the size of an extensible structure is acceptable: `EINVAL`
/// below `known` bytes, `E2BIG` above a page.
pub fn check_ext_size(usize: u64, known: usize) -> Result<usize, Errno> {
    if usize < known as u64 {
        return Err(Errno::EINVAL);
    }
    if usize > EXT_STRUCT_MAX {
        return Err(Errno::E2BIG);
    }
    Ok(usize as usize)
}

/// An extensible structure the caller passes in (`ptr`, `usize` bytes)
/// of which Poly knows the first `known` bytes: the size checked as
/// [`check_ext_size`], then copied in (`EFAULT`), then `E2BIG` if a byte
/// past the known ones is not zero. Returns the known bytes.
pub fn ext_struct(k: &dyn LinuxKernel, ptr: UserPtr, usize: u64, known: usize) -> Result<Vec<u8>, Errno> {
    let size = check_ext_size(usize, known)?;
    let mut buf = bounce(size)?;
    k.copy_from_user(ptr, &mut buf)?;
    if buf[known..].iter().any(|&b| b != 0) {
        return Err(Errno::E2BIG);
    }
    buf.truncate(known);
    Ok(buf)
}

/// The path of an *at call with `at_flags`: with `AT_EMPTY_PATH` it may be
/// empty or NULL, else it must not be empty (`ENOENT`).
pub fn at_path(k: &dyn LinuxKernel, p: UserPtr, at_flags: u32) -> Result<Vec<u8>, Errno> {
    if at_flags & AT_EMPTY_PATH != 0 {
        if p.is_null() { Ok(Vec::new()) } else { path_arg(k, p) }
    } else {
        nonempty_path(k, p)
    }
}

/// `struct xattr_args`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XattrArgs {
    pub value: UserPtr,
    pub size: u32,
    pub flags: u32,
}

impl XattrArgs {
    pub fn from_bytes(b: &[u8]) -> XattrArgs {
        let word = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let mut value = [0u8; 8];
        value.copy_from_slice(&b[..8]);
        XattrArgs { value: UserPtr(u64::from_le_bytes(value)), size: word(8), flags: word(12) }
    }

    fn load(k: &dyn LinuxKernel, ptr: UserPtr, usize: u64) -> Result<XattrArgs, Errno> {
        Ok(XattrArgs::from_bytes(&ext_struct(k, ptr, usize, XATTR_ARGS_SIZE_VER0)?))
    }
}

fn target<'a>(dfd: i32, path: &'a [u8], at_flags: u32) -> XattrTarget<'a> {
    XattrTarget::At { dirfd: dfd, path, follow: at_flags & AT_SYMLINK_NOFOLLOW == 0 }
}

/// setxattrat(dfd, path, at_flags, name, uargs, usize)
pub fn setxattrat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, at_flags) = (a.fd(0), a.uint(2));
    check_at_flags(at_flags)?;
    let args = XattrArgs::load(k, a.ptr(4), a.ulong(5))?;
    if args.flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 {
        return Err(Errno::EINVAL);
    }
    let name = name_arg(k, a.ptr(3))?;
    if u64::from(args.size) > XATTR_SIZE_MAX {
        return Err(Errno::E2BIG);
    }
    let mut value = bounce(args.size as usize)?;
    if args.size > 0 {
        k.copy_from_user(args.value, &mut value)?;
    }
    let path = at_path(k, a.ptr(1), at_flags)?;
    fs(k)?.setxattr(target(dfd, &path, at_flags), &name, &value, args.flags).map(|()| 0)
}

/// getxattrat(dfd, path, at_flags, name, uargs, usize): the value's
/// length; with `size` 0 nothing is copied.
pub fn getxattrat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, at_flags) = (a.fd(0), a.uint(2));
    check_at_flags(at_flags)?;
    let args = XattrArgs::load(k, a.ptr(4), a.ulong(5))?;
    if args.flags != 0 {
        return Err(Errno::EINVAL);
    }
    let name = name_arg(k, a.ptr(3))?;
    let path = at_path(k, a.ptr(1), at_flags)?;
    let value = fs(k)?.getxattr(target(dfd, &path, at_flags), &name)?;
    reply(k, &value, args.value, u64::from(args.size), XATTR_SIZE_MAX)
}

/// listxattrat(dfd, path, at_flags, list, size)
pub fn listxattrat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, at_flags) = (a.fd(0), a.uint(2));
    check_at_flags(at_flags)?;
    let path = at_path(k, a.ptr(1), at_flags)?;
    let names = fs(k)?.listxattr(target(dfd, &path, at_flags))?;
    reply(k, &names, a.ptr(3), a.ulong(4), XATTR_LIST_MAX)
}

/// removexattrat(dfd, path, at_flags, name)
pub fn removexattrat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, at_flags) = (a.fd(0), a.uint(2));
    check_at_flags(at_flags)?;
    let name = name_arg(k, a.ptr(3))?;
    let path = at_path(k, a.ptr(1), at_flags)?;
    fs(k)?.removexattr(target(dfd, &path, at_flags), &name).map(|()| 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_flags() {
        assert_eq!(check_ext_size(8, 16), Err(Errno::EINVAL));
        assert_eq!(check_ext_size(16, 16), Ok(16));
        assert_eq!(check_ext_size(4096, 16), Ok(4096));
        assert_eq!(check_ext_size(4097, 16), Err(Errno::E2BIG));
        assert_eq!(check_at_flags(AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW), Ok(()));
        assert_eq!(check_at_flags(0x400), Err(Errno::EINVAL));
        assert_eq!(check_at_flags(0x800), Err(Errno::EINVAL), "AT_NO_AUTOMOUNT");
    }

    #[test]
    fn xattr_args_layout() {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        b[8..12].copy_from_slice(&3u32.to_le_bytes());
        b[12..].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(XattrArgs::from_bytes(&b), XattrArgs { value: UserPtr(0x1122_3344_5566_7788), size: 3, flags: 1 });
    }
}
