//! System calls that name files by path: openat(2) and open(2), creat(2),
//! the stat(2) family and statx(2), readlink(2), access(2) and
//! faccessat2(2), mkdir(2), unlink(2), rmdir(2), rename(2) and
//! renameat2(2), symlink(2), link(2), chmod(2), truncate(2), statfs(2),
//! getcwd(2), chdir(2), umask(2).
//!
//! Each handler checks its flags, copies the path (at most `PATH_MAX`
//! bytes with the NUL: `ENAMETOOLONG` beyond, `EFAULT` for a bad pointer,
//! `ENOENT` for an empty path unless `AT_EMPTY_PATH` allows it) and calls
//! the kernel's [`LinuxFs`]; the open calls check the descriptor limit
//! (`EMFILE`) before anything else. The order of the checks: flags (`EINVAL`),
//! then the paths in argument order, then the kernel; for linkat and
//! renameat2 the second path's string errors are handed to the kernel,
//! which reports them in their place after the first path's walk (the
//! order of path errors, `poly_vfs`'s `path.rs`).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::at::*;
use crate::fs::{LinuxFs, access, fcntl, statx};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::bounce;
use alloc::vec::Vec;

/// `PATH_MAX` (Linux UAPI `linux/limits.h`): the longest path, its NUL
/// included.
pub const PATH_MAX: usize = 4096;

/// `O_CREAT` and the `O_TMPFILE` bit (`asm-generic/fcntl.h`): the flags
/// that make `open` use its mode.
const O_CREAT: u32 = 0o100;
const O_TMPFILE_BIT: u32 = 0o20000000;
/// `O_LARGEFILE` and `O_PATH` (`asm-generic/fcntl.h`).
pub const O_LARGEFILE: u32 = 0o100000;
pub const O_PATH: u32 = 0o10000000;

/// The kernel's file system services, `ENOSYS` without them.
pub(crate) fn fs(k: &mut dyn LinuxKernel) -> Result<&mut dyn LinuxFs, Errno> {
    k.fs().ok_or(Errno::ENOSYS)
}

/// The path at `p`, without its NUL (possibly empty).
pub(crate) fn path_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    let mut buf = bounce(PATH_MAX)?;
    let n = k.strncpy_from_user(p, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// The path at `p`, which must not be empty (`ENOENT`).
pub(crate) fn nonempty_path(k: &dyn LinuxKernel, p: UserPtr) -> Result<Vec<u8>, Errno> {
    let path = path_arg(k, p)?;
    if path.is_empty() { Err(Errno::ENOENT) } else { Ok(path) }
}

/// A path that may be empty when `flags` has `AT_EMPTY_PATH` (and then
/// may also be NULL, as Linux accepts since 6.11 for the stat calls).
fn maybe_empty_path(k: &dyn LinuxKernel, p: UserPtr, flags: u32) -> Result<Vec<u8>, Errno> {
    if flags & AT_EMPTY_PATH != 0 {
        if p.is_null() { Ok(Vec::new()) } else { path_arg(k, p) }
    } else {
        nonempty_path(k, p)
    }
}

// ---- open --------------------------------------------------------------

fn open_common(k: &mut dyn LinuxKernel, dirfd: i32, p: UserPtr, flags: u32, mode: u32) -> SysResult {
    // The descriptor limit comes first: with no number free, even a bad
    // path is EMFILE and O_CREAT creates nothing (the spec of open).
    fs(k)?.check_fd_free()?;
    let path = nonempty_path(k, p)?;
    // open(2): "The mode argument must be supplied if O_CREAT or O_TMPFILE
    // is specified in flags; if neither ... then mode is ignored."
    let mode = if flags & (O_CREAT | O_TMPFILE_BIT) != 0 { mode & 0o7777 } else { 0 };
    // On x86-64 every open is a large-file open: F_GETFL reports
    // O_LARGEFILE for all but O_PATH descriptors (the spec of fcntl).
    let flags = if flags & O_PATH == 0 { flags | O_LARGEFILE } else { flags };
    crate::sys::landlock::check_open(k, dirfd, &path, flags)?;
    fs(k)?.openat(dirfd, &path, flags, mode).map(|fd| fd as u64)
}

/// openat(dirfd, pathname, flags, mode)
pub fn openat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    open_common(k, a.fd(0), a.ptr(1), a.uint(2), a.uint(3))
}

/// open(pathname, flags, mode)
pub fn open(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    open_common(k, AT_FDCWD, a.ptr(0), a.uint(1), a.uint(2))
}

/// `sizeof(struct open_how)` of the first version (`OPEN_HOW_SIZE_VER0`,
/// `linux/openat2.h`): `u64 flags, mode, resolve`.
pub const OPEN_HOW_SIZE_VER0: usize = 24;
/// The largest `struct open_how` openat2(2) takes: a page (openat2's spec
/// sheet: larger is `E2BIG`).
const OPEN_HOW_SIZE_MAX: u64 = 4096;
/// Every `O_*` flag of `asm-generic/fcntl.h`: the access mode and bits 6
/// (`O_CREAT`) to 22 (`__O_TMPFILE`), each of which is a flag.
const O_VALID: u64 = 0o3 | 0o37777700;
const O_ACCMODE: u64 = 0o3;
const O_TRUNC: u64 = 0o1000;
const O_DIRECTORY: u64 = 0o200000;
const O_NOFOLLOW: u64 = 0o400000;
const O_CLOEXEC: u64 = 0o2000000;
/// `RESOLVE_*` (`linux/openat2.h`).
pub mod resolve {
    pub const NO_XDEV: u64 = 0x01;
    pub const NO_MAGICLINKS: u64 = 0x02;
    pub const NO_SYMLINKS: u64 = 0x04;
    pub const BENEATH: u64 = 0x08;
    pub const IN_ROOT: u64 = 0x10;
    pub const CACHED: u64 = 0x20;
    pub const ALL: u64 = 0x3f;
}

/// The checks openat2(2) makes on a `struct open_how` (openat2's spec
/// sheet, in its order): unknown flags, `O_PATH` with flags other than
/// `O_DIRECTORY`, `O_NOFOLLOW` and `O_CLOEXEC`, `O_TMPFILE` without write
/// access, `O_DIRECTORY | O_CREAT`, a mode without `O_CREAT` /
/// `O_TMPFILE` or with bits outside 07777, unknown resolve bits and
/// `RESOLVE_BENEATH | RESOLVE_IN_ROOT` are `EINVAL`; `RESOLVE_CACHED`
/// with a flag that creates or truncates is `EAGAIN`.
pub fn check_open_how(flags: u64, mode: u64, res: u64) -> Result<(), Errno> {
    let tmpfile = u64::from(O_TMPFILE_BIT);
    let creat = u64::from(O_CREAT);
    if flags & !O_VALID != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & u64::from(O_PATH) != 0 && flags & !(u64::from(O_PATH) | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & tmpfile != 0 && !matches!(flags & O_ACCMODE, 1 | 2) {
        return Err(Errno::EINVAL);
    }
    if flags & O_DIRECTORY != 0 && flags & creat != 0 {
        return Err(Errno::EINVAL);
    }
    let mode_ok = if flags & (creat | tmpfile) != 0 { mode & !0o7777 == 0 } else { mode == 0 };
    if !mode_ok {
        return Err(Errno::EINVAL);
    }
    if res & !resolve::ALL != 0 || res & (resolve::BENEATH | resolve::IN_ROOT) == resolve::BENEATH | resolve::IN_ROOT {
        return Err(Errno::EINVAL);
    }
    if res & resolve::CACHED != 0 && flags & (creat | O_TRUNC | tmpfile) != 0 {
        return Err(Errno::EAGAIN);
    }
    Ok(())
}

/// openat2(dirfd, pathname, how, size): openat with its parameters in a
/// `struct open_how` of `size` bytes and `resolve` restrictions on the
/// walk. `size` below [`OPEN_HOW_SIZE_VER0`] is `EINVAL`, above a page
/// `E2BIG`; bytes past the known fields must be zero (`E2BIG`). Then the
/// fields' checks ([`check_open_how`]), the pathname, the descriptor limit
/// and the kernel, which carries out the restrictions.
pub fn openat2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.ulong(3);
    if size < OPEN_HOW_SIZE_VER0 as u64 {
        return Err(Errno::EINVAL);
    }
    if size > OPEN_HOW_SIZE_MAX {
        return Err(Errno::E2BIG);
    }
    let mut how = alloc::vec![0u8; size as usize];
    k.copy_from_user(a.ptr(2), &mut how)?;
    if how[OPEN_HOW_SIZE_VER0..].iter().any(|&b| b != 0) {
        return Err(Errno::E2BIG);
    }
    let field = |i: usize| u64::from_le_bytes(how[i * 8..i * 8 + 8].try_into().unwrap_or_default());
    let (flags, mode, res) = (field(0), field(1), field(2));
    check_open_how(flags, mode, res)?;
    let path = nonempty_path(k, a.ptr(1))?;
    fs(k)?.check_fd_free()?;
    let flags = flags as u32;
    let flags = if flags & O_PATH == 0 { flags | O_LARGEFILE } else { flags };
    // Landlock's file access rights apply as for openat(2).
    crate::sys::landlock::check_open(k, a.fd(0), &path, flags)?;
    fs(k)?.openat2(a.fd(0), &path, flags, mode as u32, res).map(|fd| fd as u64)
}

/// creat(pathname, mode): open(2) "equivalent to open() with flags equal
/// to O_CREAT|O_WRONLY|O_TRUNC".
pub fn creat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    open_common(k, AT_FDCWD, a.ptr(0), fcntl::CREAT_FLAGS, a.uint(1))
}

// ---- stat ----------------------------------------------------------------

fn stat_common(k: &mut dyn LinuxKernel, dirfd: i32, p: UserPtr, buf: UserPtr, flags: u32) -> SysResult {
    if flags & !(AT_EMPTY_PATH | AT_NO_AUTOMOUNT | AT_SYMLINK_NOFOLLOW) != 0 {
        return Err(Errno::EINVAL);
    }
    let path = maybe_empty_path(k, p, flags)?;
    let st = fs(k)?.statat(dirfd, &path, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    copy_whole_to_user(k, buf, &st.stat.to_bytes())?;
    Ok(0)
}

/// Copy `src` to `dst` all or nothing: a buffer that runs into an
/// unmapped or read-only page is `EFAULT` with its writable start left as
/// it was (the stat calls' spec: "on any error the buffer is left
/// unchanged").
fn copy_whole_to_user(k: &mut dyn LinuxKernel, dst: UserPtr, src: &[u8]) -> Result<(), Errno> {
    // What a partial copy could overwrite: the readable start of the buffer.
    let mut old = alloc::vec![0u8; src.len()];
    let saved = if k.copy_from_user(dst, &mut old).is_ok() {
        src.len()
    } else {
        let mut n = 0;
        while n < src.len()
            && let Some(p) = dst.offset(n as u64)
            && k.copy_from_user(p, &mut old[n..n + 1]).is_ok()
        {
            n += 1;
        }
        n
    };
    let r = k.copy_to_user(dst, src);
    if r.is_err() && saved > 0 {
        // Writing the old bytes back stops at the same fault.
        let _ = k.copy_to_user(dst, &old[..saved]);
    }
    r
}

/// newfstatat(dirfd, pathname, statbuf, flags)
pub fn newfstatat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    stat_common(k, a.fd(0), a.ptr(1), a.ptr(2), a.uint(3))
}

/// stat(pathname, statbuf)
pub fn stat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    stat_common(k, AT_FDCWD, a.ptr(0), a.ptr(1), 0)
}

/// lstat(pathname, statbuf)
pub fn lstat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    stat_common(k, AT_FDCWD, a.ptr(0), a.ptr(1), AT_SYMLINK_NOFOLLOW)
}

/// statx(dirfd, pathname, flags, mask, statxbuf)
///
/// statx(2): `EINVAL` for unknown flags, for both `AT_STATX_FORCE_SYNC`
/// and `AT_STATX_DONT_SYNC`, and for `STATX__RESERVED` in `mask`. The
/// basic fields and the mount id are always filled, whatever `mask` asks
/// ("the kernel may return fields that weren't requested"); the birth time
/// is not known, so `STATX_BTIME` is never set in `stx_mask`.
pub fn statx(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dirfd, p, flags, mask, buf) = (a.fd(0), a.ptr(1), a.uint(2), a.uint(3), a.ptr(4));
    let known = AT_EMPTY_PATH | AT_NO_AUTOMOUNT | AT_SYMLINK_NOFOLLOW | AT_STATX_SYNC_TYPE;
    if flags & !known != 0 || flags & AT_STATX_SYNC_TYPE == AT_STATX_SYNC_TYPE || mask & statx::STATX_RESERVED != 0 {
        return Err(Errno::EINVAL);
    }
    let path = maybe_empty_path(k, p, flags)?;
    let mut st = fs(k)?.statat(dirfd, &path, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    // `STATX_MNT_ID_UNIQUE` asked for: the unique 64-bit id in
    // `stx_mnt_id` instead of the legacy one (statx(2)).
    let id_bit = if mask & statx::STATX_MNT_ID_UNIQUE != 0 {
        st.mount_id = crate::mountapi::unique_id(st.mount_id);
        statx::STATX_MNT_ID_UNIQUE
    } else {
        statx::STATX_MNT_ID
    };
    k.copy_to_user(buf, &st.to_statx(statx::STATX_BASIC_STATS | id_bit))?;
    Ok(0)
}

// ---- links -------------------------------------------------------------

fn readlink_common(k: &mut dyn LinuxKernel, dirfd: i32, p: UserPtr, buf: UserPtr, size: i32) -> SysResult {
    // readlink(2): "EINVAL bufsiz is not positive."
    if size <= 0 {
        return Err(Errno::EINVAL);
    }
    // readlinkat(2) with an empty path reads the link dirfd is open on;
    // the kernel says ENOENT for AT_FDCWD.
    let path = if dirfd == AT_FDCWD { nonempty_path(k, p)? } else { path_arg(k, p)? };
    let target = fs(k)?.readlinkat(dirfd, &path)?;
    // "readlink() does not append a terminating null byte ... It will
    // (silently) truncate the contents ... in case the buffer is too small."
    let n = target.len().min(size as usize);
    k.copy_to_user(buf, &target[..n])?;
    Ok(n as u64)
}

/// readlinkat(dirfd, pathname, buf, bufsiz)
pub fn readlinkat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    readlink_common(k, a.fd(0), a.ptr(1), a.ptr(2), a.int(3))
}

/// readlink(pathname, buf, bufsiz)
pub fn readlink(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    readlink_common(k, AT_FDCWD, a.ptr(0), a.ptr(1), a.int(2))
}

fn symlink_common(k: &mut dyn LinuxKernel, target: UserPtr, dirfd: i32, p: UserPtr) -> SysResult {
    // symlink(2): "ENOENT ... target or linkpath is an empty string."
    let target = nonempty_path(k, target)?;
    let path = nonempty_path(k, p)?;
    crate::sys::landlock::check_parent(k, dirfd, &path, crate::landlock::abi::LANDLOCK_ACCESS_FS_MAKE_SYM)?;
    fs(k)?.symlinkat(&target, dirfd, &path).map(|()| 0)
}

/// symlinkat(target, newdirfd, linkpath)
pub fn symlinkat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    symlink_common(k, a.ptr(0), a.fd(1), a.ptr(2))
}

/// symlink(target, linkpath)
pub fn symlink(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    symlink_common(k, a.ptr(0), AT_FDCWD, a.ptr(1))
}

fn link_common(
    k: &mut dyn LinuxKernel,
    olddirfd: i32,
    old: UserPtr,
    newdirfd: i32,
    new: UserPtr,
    flags: u32,
) -> SysResult {
    if flags & !(AT_SYMLINK_FOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let old = maybe_empty_path(k, old, flags & AT_EMPTY_PATH)?;
    // newpath's string errors come after every error of oldpath, which the
    // kernel finds (linkat's path_order_two_paths).
    let new = nonempty_path(k, new);
    if let Ok(n) = &new {
        crate::sys::landlock::check_move(k, olddirfd, &old, newdirfd, n, false)?;
    }
    fs(k)?.linkat(olddirfd, &old, newdirfd, new.as_deref().map_err(|&e| e), flags & AT_SYMLINK_FOLLOW != 0).map(|()| 0)
}

/// linkat(olddirfd, oldpath, newdirfd, newpath, flags)
pub fn linkat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    link_common(k, a.fd(0), a.ptr(1), a.fd(2), a.ptr(3), a.uint(4))
}

/// link(oldpath, newpath): a symbolic link `oldpath` is not followed
/// (link(2): "Linux ... does not dereference oldpath if it is a symbolic
/// link").
pub fn link(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    link_common(k, AT_FDCWD, a.ptr(0), AT_FDCWD, a.ptr(1), 0)
}

// ---- access ------------------------------------------------------------

fn access_common(k: &mut dyn LinuxKernel, dirfd: i32, p: UserPtr, mode: u32, flags: u32) -> SysResult {
    // access(2): "EINVAL mode was incorrectly specified"; "EINVAL Invalid
    // flag specified in flags."
    if mode & !(access::R_OK | access::W_OK | access::X_OK) != 0
        || flags & !(AT_EACCESS | AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0
    {
        return Err(Errno::EINVAL);
    }
    let path = maybe_empty_path(k, p, flags & AT_EMPTY_PATH)?;
    fs(k)?.accessat(dirfd, &path, mode, flags & AT_EACCESS != 0, flags & AT_SYMLINK_NOFOLLOW == 0).map(|()| 0)
}

/// access(pathname, mode)
pub fn access(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_common(k, AT_FDCWD, a.ptr(0), a.uint(1), 0)
}

/// faccessat(dirfd, pathname, mode): the system call has no flags
/// argument (access(2), "C library/kernel differences").
pub fn faccessat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_common(k, a.fd(0), a.ptr(1), a.uint(2), 0)
}

/// faccessat2(dirfd, pathname, mode, flags)
pub fn faccessat2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_common(k, a.fd(0), a.ptr(1), a.uint(2), a.uint(3))
}

// ---- directories and names -----------------------------------------------

/// mkdirat(dirfd, pathname, mode)
pub fn mkdirat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(1))?;
    crate::sys::landlock::check_parent(k, a.fd(0), &path, crate::landlock::abi::LANDLOCK_ACCESS_FS_MAKE_DIR)?;
    fs(k)?.mkdirat(a.fd(0), &path, a.uint(2) & 0o7777).map(|()| 0)
}

/// mkdir(pathname, mode)
pub fn mkdir(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    crate::sys::landlock::check_parent(k, AT_FDCWD, &path, crate::landlock::abi::LANDLOCK_ACCESS_FS_MAKE_DIR)?;
    fs(k)?.mkdirat(AT_FDCWD, &path, a.uint(1) & 0o7777).map(|()| 0)
}

/// unlinkat(dirfd, pathname, flags): "EINVAL An invalid flag value was
/// specified in flags."
pub fn unlinkat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(2);
    if flags & !AT_REMOVEDIR != 0 {
        return Err(Errno::EINVAL);
    }
    let path = nonempty_path(k, a.ptr(1))?;
    crate::sys::landlock::check_remove(k, a.fd(0), &path, flags & AT_REMOVEDIR != 0)?;
    fs(k)?.unlinkat(a.fd(0), &path, flags & AT_REMOVEDIR != 0).map(|()| 0)
}

/// unlink(pathname)
pub fn unlink(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    crate::sys::landlock::check_remove(k, AT_FDCWD, &path, false)?;
    fs(k)?.unlinkat(AT_FDCWD, &path, false).map(|()| 0)
}

/// rmdir(pathname)
pub fn rmdir(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    crate::sys::landlock::check_remove(k, AT_FDCWD, &path, true)?;
    fs(k)?.unlinkat(AT_FDCWD, &path, true).map(|()| 0)
}

fn rename_common(
    k: &mut dyn LinuxKernel,
    olddirfd: i32,
    old: UserPtr,
    newdirfd: i32,
    new: UserPtr,
    flags: u32,
) -> SysResult {
    let old = nonempty_path(k, old)?;
    // newpath's string errors come after oldpath's directories, which the
    // kernel walks (renameat2's path_order_two_paths).
    let new = nonempty_path(k, new);
    if let Ok(n) = &new {
        crate::sys::landlock::check_move(k, olddirfd, &old, newdirfd, n, true)?;
    }
    fs(k)?.renameat(olddirfd, &old, newdirfd, new.as_deref().map_err(|&e| e), flags).map(|()| 0)
}

/// renameat2(olddirfd, oldpath, newdirfd, newpath, flags)
pub fn renameat2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    rename_common(k, a.fd(0), a.ptr(1), a.fd(2), a.ptr(3), a.uint(4))
}

/// renameat(olddirfd, oldpath, newdirfd, newpath)
pub fn renameat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    rename_common(k, a.fd(0), a.ptr(1), a.fd(2), a.ptr(3), 0)
}

/// rename(oldpath, newpath)
pub fn rename(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    rename_common(k, AT_FDCWD, a.ptr(0), AT_FDCWD, a.ptr(1), 0)
}

// ---- attributes --------------------------------------------------------

/// chmod(pathname, mode)
pub fn chmod(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    fs(k)?.fchmodat(AT_FDCWD, &path, a.uint(1) & 0o7777).map(|()| 0)
}

/// fchmodat(dirfd, pathname, mode): the system call has no flags (chmod(2),
/// "C library/kernel differences") and follows symbolic links.
pub fn fchmodat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(1))?;
    fs(k)?.fchmodat(a.fd(0), &path, a.uint(2) & 0o7777).map(|()| 0)
}

/// fchmodat2(dirfd, pathname, mode, flags): fchmodat with
/// `AT_SYMLINK_NOFOLLOW` and `AT_EMPTY_PATH` (fchmodat2's spec sheet; any
/// other flag bit is `EINVAL`, first). The pathname is read even with
/// `AT_EMPTY_PATH` (NULL is `EFAULT`).
pub fn fchmodat2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(3);
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let path = if flags & AT_EMPTY_PATH != 0 { path_arg(k, a.ptr(1))? } else { nonempty_path(k, a.ptr(1))? };
    let follow = flags & AT_SYMLINK_NOFOLLOW == 0;
    fs(k)?.fchmodat2(a.fd(0), &path, a.uint(2) & 0o7777, follow).map(|()| 0)
}

/// fchmod(fd, mode)
pub fn fchmod(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    fs(k)?.fchmod(fd, a.uint(1) & 0o7777).map(|()| 0)
}

/// truncate(path, length): "EINVAL The argument length is negative",
/// before the path is looked at. Growing past `RLIMIT_FSIZE` is `EFBIG`
/// with `SIGXFSZ`, after every error of the path and the file (the spec
/// of truncate: type, `EROFS`, `EACCES`, then `EFBIG`).
pub fn truncate(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let len = u64::try_from(a.long(1)).map_err(|_| Errno::EINVAL)?;
    let path = nonempty_path(k, a.ptr(0))?;
    if crate::sys::fsize::check_truncate(k, len).is_err() {
        // The file's own errors first: its type, then a read-only mount
        // and write permission as the truncation would check them.
        let st = fs(k)?.statat(AT_FDCWD, &path, true)?;
        match st.stat.file_type() {
            crate::abi::mode::S_IFDIR => return Err(Errno::EISDIR),
            crate::abi::mode::S_IFREG => {}
            _ => return Err(Errno::EINVAL),
        }
        // Only growing past the limit is refused; shrinking (to anything)
        // works (the spec of truncate: `rlimit_fsize`).
        if len > u64::try_from(st.stat.size).unwrap_or(0) {
            fs(k)?.accessat(AT_FDCWD, &path, access::W_OK, true, true)?;
            return crate::sys::fsize::too_big_signal(k);
        }
    }
    crate::sys::landlock::check_object(k, AT_FDCWD, &path, true, crate::landlock::abi::LANDLOCK_ACCESS_FS_TRUNCATE)?;
    fs(k)?.truncate(&path, len).map(|()| 0)
}

/// statfs(path, buf)
pub fn statfs(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    let st = fs(k)?.statfs(&path)?;
    k.copy_to_user(a.ptr(1), &st.to_bytes())?;
    Ok(0)
}

// ---- the working directory and the mask ----------------------------------

/// getcwd(buf, size): the system call returns the length of the path with
/// its NUL (getcwd(2), "C library/kernel differences"); `ERANGE` when it
/// does not fit in `size` bytes.
pub fn getcwd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.ulong(1);
    let mut cwd = fs(k)?.getcwd()?;
    cwd.push(0);
    if (cwd.len() as u64) > size {
        return Err(Errno::ERANGE);
    }
    k.copy_to_user(a.ptr(0), &cwd)?;
    Ok(cwd.len() as u64)
}

/// chdir(path)
pub fn chdir(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    fs(k)?.chdir(&path).map(|()| 0)
}

/// fchdir(fd)
pub fn fchdir(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    fs(k)?.fchdir(fd).map(|()| 0)
}

/// umask(mask): "This system call always succeeds and the previous value
/// of the mask is returned"; only `mask & 0777` is used.
pub fn umask(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    Ok(u64::from(fs(k)?.umask(a.uint(0) & 0o777)))
}
