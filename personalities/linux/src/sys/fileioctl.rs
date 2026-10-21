//! ioctl(2) requests on files of a file system, answered by the
//! personality before the request reaches the kernel's device ioctls
//! ([`crate::sys::io::ioctl`] asks here first).
//!
//! - **`FIONREAD` on a regular file**: ioctl(2)'s generic `FIONREAD` "get
//!   the number of bytes in the input buffer"; for a regular file that is
//!   the bytes between the file offset and the end of the file (0 at or
//!   past the end), stored as an `int`. Other kinds of files (pipes,
//!   sockets, terminals) go on to the kernel as before.
//! - **`FICLONE`, `FICLONERANGE`** (ioctl_ficlone(2)) and
//!   **`FIDEDUPERANGE`** (ioctl_fideduperange(2)): no file system of Poly
//!   shares extents between files, so a request that passes the checks the
//!   man pages list fails with `EOPNOTSUPP` ("the filesystem does not
//!   support reflinking either file descriptor"), which is what tells cp(1)
//!   to copy the data instead. Before that, in this order: `EBADF` for a
//!   source that is not open for reading or a destination that is not open
//!   for writing (or is append-only); `EISDIR` for a directory; `EINVAL`
//!   for a device, FIFO or socket; `EXDEV` for files on different mounts.

use crate::abi::mode;
use crate::errno::{Errno, SysResult};
use crate::fs::LinuxFs;
use crate::kernel::{LinuxKernel, UserPtr};

/// Request numbers (UAPI `linux/fs.h`: `_IOW(0x94, 9, int)`,
/// `_IOW(0x94, 13, struct file_clone_range)`, `_IOWR(0x94, 54, struct
/// file_dedupe_range)`; `asm-generic/ioctls.h` for `FIONREAD`).
pub const FICLONE: u32 = 0x4004_9409;
pub const FICLONERANGE: u32 = 0x4020_940d;
pub const FIDEDUPERANGE: u32 = 0xc018_9436;
pub const FIONREAD: u32 = crate::abi::ioctl::FIONREAD;

/// `struct file_clone_range` (`linux/fs.h`): `__s64 src_fd; __u64
/// src_offset, src_length, dest_offset;`.
const FILE_CLONE_RANGE_SIZE: usize = 32;

/// Open flags (`asm-generic/fcntl.h`).
const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_APPEND: u32 = 0o2000;
const O_PATH: u32 = 0o10000000;
/// `SEEK_CUR`.
const SEEK_CUR: u32 = 1;

/// The answer to `request` on `fd` if this module handles it, `None` to
/// let the caller go on (other requests, `FIONREAD` on what is not a
/// regular file, or a kernel without file system services).
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: u64) -> Option<SysResult> {
    match request {
        FIONREAD => fionread(k, fd, UserPtr(arg)),
        FICLONE => Some(clone_check(k, arg as u32 as i32, fd)),
        FICLONERANGE => Some(clone_range(k, fd, UserPtr(arg))),
        FIDEDUPERANGE => Some(dedupe(k, fd)),
        _ => None,
    }
}

fn fionread(k: &mut dyn LinuxKernel, fd: i32, arg: UserPtr) -> Option<SysResult> {
    let f = k.fs()?;
    let st = f.statat(fd, b"", true).ok()?.stat;
    if st.file_type() != mode::S_IFREG || f.status_flags(fd).ok()? & O_PATH != 0 {
        return None;
    }
    let left = match f.lseek(fd, 0, SEEK_CUR) {
        Ok(pos) => (st.size.max(0) as u64).saturating_sub(pos),
        Err(e) => return Some(Err(e)),
    };
    let n = left.min(i32::MAX as u64) as i32;
    Some(k.copy_to_user(arg, &n.to_le_bytes()).map(|()| 0))
}

/// The open flags of `fd`, `EBADF` if it is not open or is an `O_PATH`
/// descriptor (which is not open for I/O).
fn io_flags(f: &mut dyn LinuxFs, fd: i32) -> Result<u32, Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let flags = f.status_flags(fd)?;
    if flags & O_PATH != 0 { Err(Errno::EBADF) } else { Ok(flags) }
}

const fn readable(flags: u32) -> bool {
    flags & O_ACCMODE != O_WRONLY
}

const fn writable(flags: u32) -> bool {
    matches!(flags & O_ACCMODE, O_WRONLY | O_RDWR)
}

/// `EISDIR` for a directory, `EINVAL` for anything else that is not a
/// regular file.
pub(crate) fn regular(file_type: u32) -> Result<(), Errno> {
    match file_type {
        mode::S_IFREG => Ok(()),
        mode::S_IFDIR => Err(Errno::EISDIR),
        _ => Err(Errno::EINVAL),
    }
}

/// The checks of ioctl_ficlone(2) for cloning `src` into `dest`, then
/// `EOPNOTSUPP`.
fn clone_check(k: &mut dyn LinuxKernel, src: i32, dest: i32) -> SysResult {
    let f = k.fs().ok_or(Errno::ENOSYS)?;
    let dest_flags = io_flags(f, dest)?;
    let src_flags = io_flags(f, src)?;
    if !readable(src_flags) || !writable(dest_flags) || dest_flags & O_APPEND != 0 {
        return Err(Errno::EBADF);
    }
    let (s, d) = (f.statat(src, b"", true)?, f.statat(dest, b"", true)?);
    regular(s.stat.file_type())?;
    regular(d.stat.file_type())?;
    if s.mount_id != d.mount_id {
        return Err(Errno::EXDEV);
    }
    Err(Errno::EOPNOTSUPP)
}

fn clone_range(k: &mut dyn LinuxKernel, dest: i32, arg: UserPtr) -> SysResult {
    let mut raw = [0u8; FILE_CLONE_RANGE_SIZE];
    k.copy_from_user(arg, &mut raw)?;
    let src = i64::from_le_bytes(raw[..8].try_into().expect("8 bytes"));
    let src = i32::try_from(src).map_err(|_| Errno::EBADF)?;
    clone_check(k, src, dest)
}

/// ioctl_fideduperange(2) on the source `fd`: it must be open for reading
/// and a regular file; then `EOPNOTSUPP`.
fn dedupe(k: &mut dyn LinuxKernel, fd: i32) -> SysResult {
    let f = k.fs().ok_or(Errno::ENOSYS)?;
    if !readable(io_flags(f, fd)?) {
        return Err(Errno::EBADF);
    }
    regular(f.statat(fd, b"", true)?.stat.file_type())?;
    Err(Errno::EOPNOTSUPP)
}
