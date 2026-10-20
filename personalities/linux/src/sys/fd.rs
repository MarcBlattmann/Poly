//! System calls on descriptors beyond plain reading and writing: lseek(2),
//! pread64(2), pwrite64(2), dup(2), dup2(2), dup3(2), fcntl(2),
//! getdents64(2) and getdents(2), ftruncate(2), fsync(2), fdatasync(2),
//! sync(2), syncfs(2), fstatfs(2), close_range(2).

use crate::abi::MAX_RW_COUNT;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::{DirEntry, encode_dirent, encode_dirent64, fcntl::*};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::{CHUNK, bounce};
use crate::sys::path::fs;

/// A descriptor argument: `EBADF` when negative.
fn fd_arg(a: &Args, i: usize) -> Result<i32, Errno> {
    let fd = a.fd(i);
    if fd < 0 { Err(Errno::EBADF) } else { Ok(fd) }
}

/// lseek(fd, offset, whence)
pub fn lseek(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    fs(k)?.lseek(fd, a.long(1), a.uint(2))
}

/// An offset argument of pread/pwrite: "EINVAL ... offset is negative"
/// (pread(2)).
fn offset_arg(a: &Args, i: usize) -> Result<u64, Errno> {
    u64::try_from(a.long(i)).map_err(|_| Errno::EINVAL)
}

/// pread64(fd, buf, count, offset): one read of up to [`CHUNK`] bytes.
pub fn pread64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let offset = offset_arg(a, 3)?;
    let count = a.ulong(2).min(MAX_RW_COUNT).min(CHUNK as u64) as usize;
    let mut buf = bounce(count)?;
    let n = fs(k)?.pread(fd, &mut buf, offset)?.min(count);
    k.copy_to_user(a.ptr(1), &buf[..n])?;
    Ok(n as u64)
}

/// pwrite64(fd, buf, count, offset): chunk by chunk until done or short;
/// an error after some bytes went out returns the count so far.
pub fn pwrite64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let offset = offset_arg(a, 3)?;
    let total = a.ulong(2).min(MAX_RW_COUNT);
    let src = a.ptr(1);
    if total == 0 {
        return fs(k)?.pwrite(fd, &[], offset).map(|n| n as u64);
    }
    let mut chunk = bounce(total.min(CHUNK as u64) as usize)?;
    let mut done: u64 = 0;
    while done < total {
        let want = (total - done).min(CHUNK as u64) as usize;
        let at = src.offset(done).ok_or(Errno::EFAULT)?;
        if let Err(e) = k.copy_from_user(at, &mut chunk[..want]) {
            return if done == 0 { Err(e) } else { Ok(done) };
        }
        let pos = offset.checked_add(done).ok_or(Errno::EINVAL)?;
        match fs(k)?.pwrite(fd, &chunk[..want], pos) {
            Ok(n) => {
                done += n.min(want) as u64;
                if n < want {
                    break;
                }
            }
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done)
}

/// dup(oldfd)
pub fn dup(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    fs(k)?.dup(fd).map(|fd| fd as u64)
}

/// dup2(oldfd, newfd): "If oldfd is a valid file descriptor, and newfd has
/// the same value as oldfd, then dup2() does nothing, and returns newfd."
pub fn dup2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let old = fd_arg(a, 0)?;
    let new = fd_arg(a, 1)?;
    let fs = fs(k)?;
    if old == new {
        fs.cloexec(old)?;
        return Ok(new as u64);
    }
    fs.dup3(old, new, false).map(|fd| fd as u64)
}

/// dup3(oldfd, newfd, flags): "EINVAL (dup3()) flags contain an invalid
/// value"; "EINVAL (dup3()) oldfd was equal to newfd."
pub fn dup3(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(2);
    if flags & !O_CLOEXEC != 0 {
        return Err(Errno::EINVAL);
    }
    let old = fd_arg(a, 0)?;
    let new = fd_arg(a, 1)?;
    if old == new {
        return Err(Errno::EINVAL);
    }
    fs(k)?.dup3(old, new, flags & O_CLOEXEC != 0).map(|fd| fd as u64)
}

/// fcntl(fd, cmd, arg): descriptor duplication, the close-on-exec flag
/// and the status flags. The descriptor is checked first (`EBADF`), then
/// the command: record locks, signal-driven I/O ownership, leases, pipe
/// sizes and seals are not supported yet and give `EINVAL`, as
/// fcntl(2) says for "cmd is not recognized by this kernel".
pub fn fcntl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let fs = fs(k)?;
    let flags = fs.status_flags(fd)?;
    match a.uint(1) {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            // "EINVAL (F_DUPFD) arg is negative or is greater than the
            // maximum allowable value."
            let min = a.int(2);
            if min < 0 {
                return Err(Errno::EINVAL);
            }
            fs.dup_min(fd, min, a.uint(1) == F_DUPFD_CLOEXEC).map(|fd| fd as u64)
        }
        F_GETFD => fs.cloexec(fd).map(|c| if c { FD_CLOEXEC } else { 0 }),
        F_SETFD => fs.set_cloexec(fd, a.ulong(2) & FD_CLOEXEC != 0).map(|()| 0),
        F_GETFL => Ok(u64::from(flags)),
        F_SETFL => fs.set_status_flags(fd, a.uint(2)).map(|()| 0),
        _ => Err(Errno::EINVAL),
    }
}

/// The common part of getdents64(2) and getdents(2): records encoded by
/// `encode` (of `len` bytes each) into a buffer of at most `count`
/// bytes, copied to `dirp`. "EINVAL Result buffer is too small" when not
/// even the first entry fits.
fn getdents_common(
    k: &mut dyn LinuxKernel,
    fd: i32,
    dirp: UserPtr,
    count: u32,
    len: fn(&[u8]) -> usize,
    encode: fn(&DirEntry<'_>, &mut [u8]) -> usize,
) -> SysResult {
    let size = (count as usize).min(CHUNK);
    let mut buf = bounce(size)?;
    let mut used = 0;
    let mut too_small = false;
    fs(k)?.getdents(fd, &mut |e| {
        let n = len(e.name);
        if used + n > size {
            too_small = used == 0;
            return false;
        }
        encode(e, &mut buf[used..]);
        used += n;
        true
    })?;
    if too_small {
        return Err(Errno::EINVAL);
    }
    k.copy_to_user(dirp, &buf[..used])?;
    Ok(used as u64)
}

/// close_range(2) flags (`linux/close_range.h`).
pub const CLOSE_RANGE_UNSHARE: u32 = 1 << 1;
pub const CLOSE_RANGE_CLOEXEC: u32 = 1 << 2;

/// close_range(first, last, flags) (ADR 0057; Python's `subprocess` closes
/// a child's descriptors with it): "EINVAL flags is not valid, or first is
/// greater than last". Descriptors in the range that are not open are
/// skipped; `CLOSE_RANGE_CLOEXEC` marks instead of closing.
/// `CLOSE_RANGE_UNSHARE` first gives the caller a descriptor table of its
/// own: a Poly process's table is already its own (ADR 0054), so this is
/// nothing to do for a single thread; with other threads sharing it Poly
/// cannot split it and refuses (`EINVAL`, Poly's choice, as unshare(2)
/// with `CLONE_FILES`).
pub fn close_range(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (first, last, flags) = (a.uint(0), a.uint(1), a.uint(2));
    if flags & !(CLOSE_RANGE_UNSHARE | CLOSE_RANGE_CLOEXEC) != 0 || first > last {
        return Err(Errno::EINVAL);
    }
    if flags & CLOSE_RANGE_UNSHARE != 0 && crate::signal::deliver::with_own(k, |p| p.thread_ids().count()) > 1 {
        return Err(Errno::EINVAL);
    }
    fs(k)?.close_range(first, last, flags & CLOSE_RANGE_CLOEXEC != 0).map(|()| 0)
}

/// getdents64(fd, dirp, count)
pub fn getdents64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    getdents_common(k, fd, a.ptr(1), a.uint(2), crate::fs::dirent64_len, encode_dirent64)
}

/// getdents(fd, dirp, count): the old `struct linux_dirent`.
pub fn getdents(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    getdents_common(k, fd, a.ptr(1), a.uint(2), crate::fs::dirent_len, encode_dirent)
}

/// ftruncate(fd, length): "EINVAL The argument length is negative".
pub fn ftruncate(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let len = u64::try_from(a.long(1)).map_err(|_| Errno::EINVAL)?;
    fs(k)?.ftruncate(fd, len).map(|()| 0)
}

/// fsync(fd)
pub fn fsync(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    fs(k)?.fsync(fd, false).map(|()| 0)
}

/// fdatasync(fd)
pub fn fdatasync(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    fs(k)?.fsync(fd, true).map(|()| 0)
}

/// sync(): "sync() is always successful."
pub fn sync(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    fs(k)?.sync();
    Ok(0)
}

/// syncfs(fd)
pub fn syncfs(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    fs(k)?.syncfs(fd).map(|()| 0)
}

/// fstatfs(fd, buf)
pub fn fstatfs(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let st = fs(k)?.fstatfs(fd)?;
    k.copy_to_user(a.ptr(1), &st.to_bytes())?;
    Ok(0)
}
