//! Calls on names and their attributes (ADR 0246, addendum): `mkdir`,
//! `rmdir`, `unlink`, `link`, `symlink`, `readlink`, the `rename` family,
//! `chmod`, `umask`, `mknod`, `truncate`, and extended attributes
//! (`specs/macos/bsd/{rename,renameat,renameatx_np,getxattr,setxattr,
//! listxattr,removexattr,fgetxattr,fsetxattr,flistxattr,fremovexattr}.yaml`).
//!
//! Extended attributes of macOS have free names (`com.apple.FinderInfo`,
//! `test`); the kernel keeps them as `user.<name>` attributes of the VFS and
//! this layer adds and takes away the prefix.

use super::errno::*;
use super::file::AT_FDCWD;
use super::{BsdKernel, PATH_MAX, Ret, ok, put, read_cstr};
use alloc::vec;
use alloc::vec::Vec;

/// `<fcntl.h>`: `AT_REMOVEDIR`, `AT_SYMLINK_FOLLOW`.
pub const AT_REMOVEDIR: u32 = 0x80;
pub const AT_SYMLINK_FOLLOW: u32 = 0x40;
/// `<stdio.h>` renamex_np flags.
pub const RENAME_SWAP: u32 = 0x2;
pub const RENAME_EXCL: u32 = 0x4;
/// `<sys/xattr.h>`.
pub const XATTR_NOFOLLOW: u32 = 0x1;
pub const XATTR_CREATE: u32 = 0x2;
pub const XATTR_REPLACE: u32 = 0x4;
pub const XATTR_MAXNAMELEN: usize = 127;
pub const RESOURCE_FORK: &[u8] = b"com.apple.ResourceFork";
pub const FINDER_INFO: &[u8] = b"com.apple.FinderInfo";
/// `ENOATTR` (`<sys/errno.h>`).
pub const ENOATTR: u32 = 93;

/// How a rename treats the destination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenameHow {
    /// Exchange the two names.
    pub swap: bool,
    /// Fail with `EEXIST` if the destination exists.
    pub excl: bool,
}

/// What an attribute call names: a path (following a last link or not) or
/// an open descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    Path { path: &'a [u8], follow: bool },
    Fd(i32),
}

/// An attribute operation the kernel does on the VFS name `name` (with the
/// `user.` prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XattrOp<'a> {
    Get { name: &'a [u8] },
    Set { name: &'a [u8], value: &'a [u8], create: bool, replace: bool },
    List,
    Remove { name: &'a [u8] },
}

fn path(k: &dyn BsdKernel, addr: u64) -> Result<Vec<u8>, u32> {
    let p = read_cstr(k, addr, PATH_MAX)?;
    if p.is_empty() { Err(ENOENT) } else { Ok(p) }
}

pub fn mkdir(k: &mut dyn BsdKernel, dirfd: i32, p: u64, mode: u32) -> Result<Ret, u32> {
    let p = path(k, p)?;
    k.mkdir(dirfd, &p, (mode & 0o7777) as u16).and_then(|()| ok(0))
}

pub fn unlink(k: &mut dyn BsdKernel, dirfd: i32, p: u64, flags: u32) -> Result<Ret, u32> {
    if flags & !AT_REMOVEDIR != 0 {
        return Err(EINVAL);
    }
    let p = path(k, p)?;
    k.unlink(dirfd, &p, flags & AT_REMOVEDIR != 0).and_then(|()| ok(0))
}

pub fn link(k: &mut dyn BsdKernel, fd1: i32, old: u64, fd2: i32, new: u64, flags: u32) -> Result<Ret, u32> {
    if flags & !AT_SYMLINK_FOLLOW != 0 {
        return Err(EINVAL);
    }
    let (old, new) = (path(k, old)?, path(k, new)?);
    k.link(fd1, &old, fd2, &new, flags & AT_SYMLINK_FOLLOW != 0).and_then(|()| ok(0))
}

pub fn symlink(k: &mut dyn BsdKernel, target: u64, dirfd: i32, p: u64) -> Result<Ret, u32> {
    let target = read_cstr(k, target, PATH_MAX)?;
    let p = path(k, p)?;
    k.symlink(&target, dirfd, &p).and_then(|()| ok(0))
}

/// readlink: the target, cut to the buffer; the number of bytes stored.
pub fn readlink(k: &mut dyn BsdKernel, dirfd: i32, p: u64, buf: u64, size: u64) -> Result<Ret, u32> {
    let p = path(k, p)?;
    let target = k.readlink(dirfd, &p)?;
    let n = target.len().min(size as usize);
    put(k, buf, &target[..n])?;
    ok(n as u64)
}

/// renameatx_np's flags: 0 and 1 are plain renames; SWAP and EXCL may not
/// come together; any other bit is `EINVAL`.
pub fn rename_how(flags: u32) -> Result<RenameHow, u32> {
    if flags & !(1 | RENAME_SWAP | RENAME_EXCL) != 0 || flags & (RENAME_SWAP | RENAME_EXCL) == RENAME_SWAP | RENAME_EXCL
    {
        return Err(EINVAL);
    }
    Ok(RenameHow { swap: flags & RENAME_SWAP != 0, excl: flags & RENAME_EXCL != 0 })
}

pub fn rename(k: &mut dyn BsdKernel, fd1: i32, old: u64, fd2: i32, new: u64, flags: u32) -> Result<Ret, u32> {
    let how = rename_how(flags)?;
    let (old, new) = (path(k, old)?, path(k, new)?);
    k.rename(fd1, &old, fd2, &new, how).and_then(|()| ok(0))
}

pub fn chmod(k: &mut dyn BsdKernel, target: Target<'_>, mode: u32) -> Result<Ret, u32> {
    k.chmod(target, (mode & 0o7777) as u16).and_then(|()| ok(0))
}

pub fn chmod_path(k: &mut dyn BsdKernel, dirfd: i32, p: u64, mode: u32, follow: bool) -> Result<Ret, u32> {
    let p = path(k, p)?;
    k.chmod_at(dirfd, &p, (mode & 0o7777) as u16, follow).and_then(|()| ok(0))
}

pub fn mknod(k: &mut dyn BsdKernel, p: u64, mode: u32, dev: u32) -> Result<Ret, u32> {
    let p = path(k, p)?;
    k.mknod(&p, mode, dev).and_then(|()| ok(0))
}

pub fn truncate(k: &mut dyn BsdKernel, target: Target<'_>, len: i64) -> Result<Ret, u32> {
    if len < 0 {
        return Err(EINVAL);
    }
    k.truncate(target, len as u64).and_then(|()| ok(0))
}

// ----- extended attributes -----

/// An attribute name as the program gave it: UTF-8, 1 to 127 bytes.
fn xattr_name(k: &dyn BsdKernel, addr: u64) -> Result<Vec<u8>, u32> {
    let name =
        read_cstr(k, addr, XATTR_MAXNAMELEN + 1).map_err(|e| if e == ENAMETOOLONG { ENAMETOOLONG } else { e })?;
    if name.is_empty() || core::str::from_utf8(&name).is_err() {
        return Err(EINVAL);
    }
    if name.len() > XATTR_MAXNAMELEN {
        return Err(ENAMETOOLONG);
    }
    Ok(name)
}

/// The VFS name of a macOS attribute.
fn vfs_name(name: &[u8]) -> Vec<u8> {
    let mut v = b"user.".to_vec();
    v.extend_from_slice(name);
    v
}

/// The target of a path or descriptor form. The descriptor forms take no
/// options where the path forms take `XATTR_NOFOLLOW` (`EINVAL`).
fn xattr_target(k: &dyn BsdKernel, fd_form: bool, a0: u64, options: u32) -> Result<(Vec<u8>, Option<i32>, bool), u32> {
    if fd_form {
        if options & XATTR_NOFOLLOW != 0 {
            return Err(EINVAL);
        }
        Ok((Vec::new(), Some(a0 as i32), true))
    } else {
        Ok((path(k, a0)?, None, options & XATTR_NOFOLLOW == 0))
    }
}

fn target<'a>(p: &'a [u8], fd: Option<i32>, follow: bool) -> Target<'a> {
    match fd {
        Some(fd) => Target::Fd(fd),
        None => Target::Path { path: p, follow },
    }
}

/// An attribute's value, `ENOATTR` if absent (an all-zero FinderInfo reads
/// as absent).
fn get_value(k: &mut dyn BsdKernel, t: Target<'_>, name: &[u8]) -> Result<Vec<u8>, u32> {
    let v = k.xattr(t, XattrOp::Get { name: &vfs_name(name) })?;
    if name == FINDER_INFO && v.iter().all(|b| *b == 0) {
        return Err(ENOATTR);
    }
    Ok(v)
}

/// getxattr / fgetxattr(target, name, value, size, position, options).
pub fn getxattr(k: &mut dyn BsdKernel, fd_form: bool, a: &[u64; 8]) -> Result<Ret, u32> {
    let (value, size, position, options) = (a[2], a[3], a[4] as u32, a[5] as u32);
    let (p, fd, follow) = xattr_target(k, fd_form, a[0], options)?;
    let name = xattr_name(k, a[1])?;
    let fork = name == RESOURCE_FORK;
    if position != 0 && !fork {
        return Err(EINVAL);
    }
    let v = get_value(k, target(&p, fd, follow), &name)?;
    // A size query (no buffer, or size 0) gives the whole length.
    if value == 0 || size == 0 {
        return ok(v.len() as u64);
    }
    if fork {
        // The resource fork reads like a file: from the position, as much
        // as the buffer takes.
        let from = (position as usize).min(v.len());
        let n = (v.len() - from).min(size as usize);
        put(k, value, &v[from..from + n])?;
        return ok(n as u64);
    }
    if (size as usize) < v.len() {
        return Err(ERANGE);
    }
    put(k, value, &v)?;
    ok(v.len() as u64)
}

/// setxattr / fsetxattr(target, name, value, size, position, options).
pub fn setxattr(k: &mut dyn BsdKernel, fd_form: bool, a: &[u64; 8]) -> Result<Ret, u32> {
    let (value, size, position, options) = (a[2], a[3], a[4] as u32, a[5] as u32);
    if options & (0x8 | 0x10) != 0 || options & (XATTR_CREATE | XATTR_REPLACE) == XATTR_CREATE | XATTR_REPLACE {
        return Err(EINVAL);
    }
    let (p, fd, follow) = xattr_target(k, fd_form, a[0], options)?;
    let name = xattr_name(k, a[1])?;
    let fork = name == RESOURCE_FORK;
    if position != 0 && !fork {
        return Err(EINVAL);
    }
    // The file's errors come before the value's (a missing file with an
    // unreadable value is ENOENT).
    k.xattr(target(&p, fd, follow), XattrOp::List)?;
    let mut bytes = vec![0u8; size as usize];
    if size != 0 {
        k.copy_in(value, &mut bytes).map_err(|()| EFAULT)?;
    }
    if name == FINDER_INFO && bytes.len() != 32 {
        return Err(ERANGE);
    }
    let t = target(&p, fd, follow);
    if fork && position != 0 {
        // A write into the resource fork at an offset: the rest stays, a gap
        // fills with zeros.
        let mut old = match get_value(k, t, &name) {
            Ok(v) => v,
            Err(ENOATTR) => Vec::new(),
            Err(e) => return Err(e),
        };
        let at = position as usize;
        if old.len() < at + bytes.len() {
            old.resize(at + bytes.len(), 0);
        }
        old[at..at + bytes.len()].copy_from_slice(&bytes);
        bytes = old;
    }
    let create = options & XATTR_CREATE != 0;
    let replace = options & XATTR_REPLACE != 0;
    k.xattr(t, XattrOp::Set { name: &vfs_name(&name), value: &bytes, create, replace }).map(|_| ())?;
    ok(0)
}

/// The macOS names on a file: the `user.` attributes without the prefix,
/// in byte order, an all-zero FinderInfo and an empty resource fork left
/// out.
fn names(k: &mut dyn BsdKernel, t: Target<'_>) -> Result<Vec<Vec<u8>>, u32> {
    let raw = k.xattr(t, XattrOp::List)?;
    let mut out: Vec<Vec<u8>> = raw
        .split(|b| *b == 0)
        .filter_map(|n| n.strip_prefix(b"user.".as_slice()))
        .filter(|n| !n.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    let mut keep = Vec::with_capacity(out.len());
    for n in out.drain(..) {
        let shown = if n == FINDER_INFO || n == RESOURCE_FORK {
            get_value(k, t, &n).is_ok_and(|v| !v.is_empty())
        } else {
            true
        };
        if shown {
            keep.push(n);
        }
    }
    keep.sort();
    Ok(keep)
}

/// listxattr / flistxattr(target, namebuf, size, options).
pub fn listxattr(k: &mut dyn BsdKernel, fd_form: bool, a: &[u64; 8]) -> Result<Ret, u32> {
    let (buf, size, options) = (a[1], a[2], a[3] as u32);
    let (p, fd, follow) = xattr_target(k, fd_form, a[0], options)?;
    let mut out = Vec::new();
    for n in names(k, target(&p, fd, follow))? {
        out.extend_from_slice(&n);
        out.push(0);
    }
    if buf == 0 || size == 0 {
        return ok(out.len() as u64);
    }
    if (size as usize) < out.len() {
        return Err(ERANGE);
    }
    put(k, buf, &out)?;
    ok(out.len() as u64)
}

/// removexattr / fremovexattr(target, name, options).
pub fn removexattr(k: &mut dyn BsdKernel, fd_form: bool, a: &[u64; 8]) -> Result<Ret, u32> {
    let options = a[2] as u32;
    let (p, fd, follow) = xattr_target(k, fd_form, a[0], options)?;
    let name = xattr_name(k, a[1])?;
    let t = target(&p, fd, follow);
    if name == FINDER_INFO {
        // An all-zero FinderInfo is absent.
        get_value(k, t, &name)?;
    }
    k.xattr(t, XattrOp::Remove { name: &vfs_name(&name) }).map(|_| ())?;
    ok(0)
}

/// The descriptor of a path form's directory: `AT_FDCWD`.
pub const CWD: i32 = AT_FDCWD;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_flags() {
        assert_eq!(rename_how(0), Ok(RenameHow::default()));
        assert_eq!(rename_how(1), Ok(RenameHow::default()));
        assert_eq!(rename_how(RENAME_SWAP).map(|h| h.swap), Ok(true));
        assert_eq!(rename_how(RENAME_EXCL).map(|h| h.excl), Ok(true));
        assert_eq!(rename_how(RENAME_SWAP | RENAME_EXCL), Err(EINVAL));
        assert_eq!(rename_how(8), Err(EINVAL));
        assert_eq!(rename_how(0x100), Err(EINVAL));
    }
}
