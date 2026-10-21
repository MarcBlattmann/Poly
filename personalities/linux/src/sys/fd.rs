//! System calls on descriptors beyond plain reading and writing: lseek(2),
//! pread64(2), pwrite64(2), dup(2), dup2(2), dup3(2), fcntl(2),
//! getdents64(2) and getdents(2), ftruncate(2), fsync(2), fdatasync(2),
//! sync(2), syncfs(2), fstatfs(2), close_range(2), flock(2).

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
    let total = crate::sys::fsize::allowed(k, fd, Some(offset), total)?;
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

/// fcntl(fd, cmd, arg): descriptor duplication, the close-on-exec flag,
/// the status flags and record locks ([`record_lock`]). The descriptor is
/// checked first (`EBADF`), then the command: signal-driven I/O ownership,
/// leases, pipe sizes and seals are not supported yet and give `EINVAL`,
/// as fcntl(2) says for "cmd is not recognized by this kernel".
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
        // An O_PATH descriptor's status flags cannot be set (EBADF).
        F_SETFL if flags & crate::sys::path::O_PATH != 0 => Err(Errno::EBADF),
        F_SETFL => fs.set_status_flags(fd, a.uint(2)).map(|()| 0),
        cmd @ (F_GETLK | F_SETLK | F_SETLKW | F_OFD_GETLK | F_OFD_SETLK | F_OFD_SETLKW) => {
            record_lock(k, fd, cmd, a.ptr(2), flags)
        }
        _ => Err(Errno::EINVAL),
    }
}

/// Record lock commands (`asm-generic/fcntl.h`, `linux/fcntl.h`).
pub const F_GETLK: u32 = 5;
pub const F_SETLK: u32 = 6;
pub const F_SETLKW: u32 = 7;
pub const F_OFD_GETLK: u32 = 36;
pub const F_OFD_SETLK: u32 = 37;
pub const F_OFD_SETLKW: u32 = 38;
/// `l_type` values.
pub const F_RDLCK: i16 = 0;
pub const F_WRLCK: i16 = 1;
pub const F_UNLCK: i16 = 2;
/// `O_ACCMODE` bits of the status flags.
const O_WRONLY: u32 = 1;
const O_RDWR: u32 = 2;

/// `struct flock` (`asm-generic/fcntl.h`, x86-64): `short l_type`, `short
/// l_whence`, (padding), `off_t l_start`, `off_t l_len`, `pid_t l_pid`,
/// (padding): 32 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Flock {
    kind: i16,
    whence: i16,
    start: i64,
    len: i64,
    pid: i32,
}

impl Flock {
    const SIZE: usize = 32;

    fn read(b: &[u8; Self::SIZE]) -> Flock {
        let i16_at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
        Flock {
            kind: i16_at(0),
            whence: i16_at(2),
            start: i64_at(8),
            len: i64_at(16),
            pid: i32::from_le_bytes(b[24..28].try_into().expect("4 bytes")),
        }
    }

    fn bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..2].copy_from_slice(&self.kind.to_le_bytes());
        b[2..4].copy_from_slice(&self.whence.to_le_bytes());
        b[8..16].copy_from_slice(&self.start.to_le_bytes());
        b[16..24].copy_from_slice(&self.len.to_le_bytes());
        b[24..28].copy_from_slice(&self.pid.to_le_bytes());
        b
    }
}

/// fcntl(2)'s record locks: `F_GETLK`, `F_SETLK`, `F_SETLKW` and the open
/// file description forms `F_OFD_*` (ADR 0059; SQLite, Python's
/// `fcntl.lockf`).
///
/// The `struct flock` at `arg` names the range from `l_whence`
/// (`SEEK_SET`, `SEEK_CUR`: the file offset, `SEEK_END`: the file's size)
/// plus `l_start`, `l_len` bytes long (0: to the end of the file, however
/// it grows; negative: the bytes before `l_start`). fcntl(2): `EINVAL` for
/// an unknown `l_type` or `l_whence`, a range that starts before byte 0 or
/// does not fit in `off_t`, and for the `F_OFD_*` forms an `l_pid` other
/// than 0; `EBADF` for a read lock on a descriptor not open for reading or
/// a write lock on one not open for writing; `EAGAIN` when `F_SETLK` meets
/// a lock of another owner ("EACCES or EAGAIN": Poly's choice); `F_SETLKW`
/// waits instead, and is ended by a signal (`EINTR`, or restarted under
/// `SA_RESTART`, signal(7)). `F_GETLK` writes back the first lock in the
/// way (`l_whence` `SEEK_SET`, `l_pid` -1 for an open file description's
/// lock), or `l_type` `F_UNLCK` and nothing else changed. Deadlocks are
/// not detected (`EDEADLK` is never returned). The order of the checks is
/// ours, to be confirmed by the diff runner.
fn record_lock(k: &mut dyn LinuxKernel, fd: i32, cmd: u32, arg: UserPtr, flags: u32) -> SysResult {
    let mut raw = [0u8; Flock::SIZE];
    k.copy_from_user(arg, &mut raw)?;
    let mut l = Flock::read(&raw);
    let ofd = matches!(cmd, F_OFD_GETLK | F_OFD_SETLK | F_OFD_SETLKW);
    let write = match l.kind {
        F_RDLCK => Some(false),
        F_WRLCK => Some(true),
        F_UNLCK => None,
        _ => return Err(Errno::EINVAL),
    };
    if ofd && l.pid != 0 {
        return Err(Errno::EINVAL);
    }
    let base: i64 = match l.whence {
        0 => 0,
        1 => i64::try_from(fs(k)?.lseek(fd, 0, 1)?).map_err(|_| Errno::EOVERFLOW)?,
        2 => k.fstat(fd)?.size,
        _ => return Err(Errno::EINVAL),
    };
    let at = base.checked_add(l.start).ok_or(Errno::EOVERFLOW)?;
    let (start, end) = match l.len {
        0 => (at, None),
        len if len > 0 => (at, Some(at.checked_add(len).ok_or(Errno::EOVERFLOW)?)),
        len => (at.checked_add(len).ok_or(Errno::EINVAL)?, Some(at)),
    };
    if start < 0 {
        return Err(Errno::EINVAL);
    }
    let (start, end) = (start as u64, end.map_or(u64::MAX, |e| e as u64));
    if matches!(cmd, F_GETLK | F_OFD_GETLK) {
        // F_GETLK with F_UNLCK is EINVAL ("l_type ... F_RDLCK or F_WRLCK").
        let write = write.ok_or(Errno::EINVAL)?;
        match fs(k)?.test_lock(fd, ofd, write, start, end)? {
            Some(h) => {
                l.kind = if h.write { F_WRLCK } else { F_RDLCK };
                l.whence = 0;
                l.start = h.start as i64;
                l.len = if h.end == u64::MAX { 0 } else { (h.end - h.start) as i64 };
                l.pid = h.pid;
            }
            None => l.kind = F_UNLCK,
        }
        k.copy_to_user(arg, &l.bytes())?;
        return Ok(0);
    }
    let access = flags & 3;
    match write {
        Some(false) if access == O_WRONLY => return Err(Errno::EBADF),
        Some(true) if access != O_WRONLY && access != O_RDWR => return Err(Errno::EBADF),
        _ => {}
    }
    let wait = matches!(cmd, F_SETLKW | F_OFD_SETLKW);
    fs(k)?.set_lock(fd, ofd, write, start, end, wait).map(|()| 0)
}

/// flock(2) operations (`asm-generic/fcntl.h`... `linux/fs.h` has them as
/// `LOCK_SH` 1, `LOCK_EX` 2, `LOCK_NB` 4, `LOCK_UN` 8).
pub const LOCK_SH: u32 = 1;
pub const LOCK_EX: u32 = 2;
pub const LOCK_NB: u32 = 4;
pub const LOCK_UN: u32 = 8;

/// flock(fd, operation) (ADR 0059): "EINVAL operation is invalid" unless
/// it is exactly one of `LOCK_SH`, `LOCK_EX`, `LOCK_UN`, optionally with
/// `LOCK_NB`; "EWOULDBLOCK The file is locked and the LOCK_NB flag was
/// selected"; without `LOCK_NB` the call waits and a signal ends it
/// (`EINTR`, restarted under `SA_RESTART`, signal(7)).
pub fn flock(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let op = a.uint(1);
    let exclusive = match op & !LOCK_NB {
        LOCK_SH => Some(false),
        LOCK_EX => Some(true),
        LOCK_UN => None,
        _ => return Err(Errno::EINVAL),
    };
    fs(k)?.status_flags(fd)?;
    fs(k)?.flock(fd, exclusive, op & LOCK_NB == 0).map(|()| 0)
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

/// close_range(first, last, flags) (ADR 0059; Python's `subprocess` closes
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
    crate::sys::fsize::check_truncate(k, len)?;
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
