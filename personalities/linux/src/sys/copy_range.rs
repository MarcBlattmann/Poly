//! copy_file_range(2): copy bytes from one regular file to another
//! without a trip through user memory.
//!
//! Poly has no file system that shares or offloads copies, so this is the
//! generic copy the man page describes the kernel falling back to: chunks
//! of [`CHUNK`] bytes read with `pread` and written with `pwrite` through a
//! kernel buffer. A NULL offset pointer means the descriptor's file offset,
//! which is used and then moved by the bytes copied; a pointer gives the
//! offset, which is updated and the file offset is left alone. Bytes that
//! reach descriptors 1 and 2 are the program's output in a traced run like
//! those of `write` (the kernel's `pwrite` reports them, ADR 0027).
//!
//! Checks, as copy_file_range(2)'s ERRORS list them (the order is ours):
//!
//! 1. `EBADF`: a descriptor that is not open (or an `O_PATH` one), `fd_in`
//!    not open for reading, `fd_out` not open for writing or opened with
//!    `O_APPEND`;
//! 2. `EINVAL`: `flags` is not 0;
//! 3. `EISDIR` for a directory, `EINVAL` for anything else that is not a
//!    regular file (a pipe, a terminal: copy_file_range(2) "Either fd_in
//!    or fd_out is not a regular file");
//! 4. `EFAULT` for an offset pointer that cannot be read, `EINVAL` for a
//!    negative offset, `EOVERFLOW` when offset plus length does not fit in
//!    an `loff_t`;
//! 5. `EINVAL` when both are the same file and the ranges overlap (the
//!    source range counted up to the end of the file);
//! 6. `EXDEV` for files on different file systems (devices): "not on the
//!    same filesystem, and the source and target filesystems are not of
//!    the same type, or do not support cross-filesystem copy" — no file
//!    system of Poly supports a cross-filesystem copy, whatever its type.
//!
//! A length of 0, or a source offset at or past the end of the file,
//! copies nothing and returns 0. At most [`MAX_RW_COUNT`] bytes are copied
//! per call. An error after some bytes were copied returns the count so
//! far.

use crate::abi::MAX_RW_COUNT;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::{FileStatus, LinuxFs};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::fileioctl::regular;
use crate::sys::io::{CHUNK, bounce};
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::COPY_FILE_RANGE, copy_file_range)];

const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_APPEND: u32 = 0o2000;
const O_PATH: u32 = 0o10000000;
const SEEK_SET: u32 = 0;
const SEEK_CUR: u32 = 1;

/// One side of the copy: its descriptor, status, where the copy starts
/// and the user pointer to update (NULL: the file offset).
struct Side {
    fd: i32,
    status: FileStatus,
    ptr: UserPtr,
    at: u64,
}

fn open_side(f: &mut dyn LinuxFs, fd: i32, write: bool) -> Result<(FileStatus, u32), Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let flags = f.status_flags(fd)?;
    let mode = flags & O_ACCMODE;
    let ok = if write { matches!(mode, O_WRONLY | O_RDWR) && flags & O_APPEND == 0 } else { mode != O_WRONLY };
    if flags & O_PATH != 0 || !ok {
        return Err(Errno::EBADF);
    }
    Ok((f.statat(fd, b"", true)?, flags))
}

/// The start offset of a side: `*ptr`, or the file offset.
fn start(k: &mut dyn LinuxKernel, fd: i32, ptr: UserPtr) -> Result<u64, Errno> {
    if ptr.is_null() {
        return fs(k)?.lseek(fd, 0, SEEK_CUR);
    }
    let mut raw = [0u8; 8];
    k.copy_from_user(ptr, &mut raw)?;
    u64::try_from(i64::from_le_bytes(raw)).map_err(|_| Errno::EINVAL)
}

fn fs(k: &mut dyn LinuxKernel) -> Result<&mut dyn LinuxFs, Errno> {
    k.fs().ok_or(Errno::ENOSYS)
}

/// copy_file_range(fd_in, off_in, fd_out, off_out, len, flags)
pub fn copy_file_range(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd_in, fd_out) = (a.fd(0), a.fd(2));
    let f = fs(k)?;
    let (in_status, _) = open_side(f, fd_in, false)?;
    let (out_status, _) = open_side(f, fd_out, true)?;
    if a.uint(5) != 0 {
        return Err(Errno::EINVAL);
    }
    regular(in_status.stat.file_type())?;
    regular(out_status.stat.file_type())?;
    let mut src = Side { fd: fd_in, status: in_status, ptr: a.ptr(1), at: 0 };
    let mut dst = Side { fd: fd_out, status: out_status, ptr: a.ptr(3), at: 0 };
    src.at = start(k, src.fd, src.ptr)?;
    dst.at = start(k, dst.fd, dst.ptr)?;
    let len = a.ulong(4);
    if src.at.checked_add(len).is_none_or(|e| e > i64::MAX as u64)
        || dst.at.checked_add(len).is_none_or(|e| e > i64::MAX as u64)
    {
        return Err(Errno::EOVERFLOW);
    }
    // What there is to copy: up to the end of the source.
    let size = src.status.stat.size.max(0) as u64;
    let len = len.min(size.saturating_sub(src.at)).min(MAX_RW_COUNT);
    let same = src.status.stat.dev == dst.status.stat.dev && src.status.stat.ino == dst.status.stat.ino;
    if same && len > 0 && src.at < dst.at + len && dst.at < src.at + len {
        return Err(Errno::EINVAL);
    }
    if src.status.stat.dev != dst.status.stat.dev {
        return Err(Errno::EXDEV);
    }
    let (done, result) = copy(fs(k)?, &src, &dst, len);
    // The offsets move by what was copied, also when an error ended it.
    for (side, moved) in [(&mut src, done), (&mut dst, done)] {
        side.at += moved;
        if side.ptr.is_null() {
            if moved > 0 {
                fs(k)?.lseek(side.fd, side.at as i64, SEEK_SET)?;
            }
        } else {
            k.copy_to_user(side.ptr, &(side.at as i64).to_le_bytes())?;
        }
    }
    match result {
        Err(e) if done == 0 => Err(e),
        _ => Ok(done),
    }
}

/// Copy up to `len` bytes; returns the count and how it ended.
fn copy(f: &mut dyn LinuxFs, src: &Side, dst: &Side, len: u64) -> (u64, Result<(), Errno>) {
    if len == 0 {
        return (0, Ok(()));
    }
    let mut buf = match bounce(len.min(CHUNK as u64) as usize) {
        Ok(b) => b,
        Err(e) => return (0, Err(e)),
    };
    let mut done: u64 = 0;
    while done < len {
        let want = (len - done).min(buf.len() as u64) as usize;
        let n = match f.pread(src.fd, &mut buf[..want], src.at + done) {
            Ok(0) => break,
            Ok(n) => n.min(want),
            Err(e) => return (done, Err(e)),
        };
        let mut written = 0;
        while written < n {
            match f.pwrite(dst.fd, &buf[written..n], dst.at + done + written as u64) {
                Ok(0) => return (done + written as u64, Ok(())),
                Ok(w) => written += w.min(n - written),
                Err(e) => return (done + written as u64, Err(e)),
            }
        }
        done += n as u64;
    }
    (done, Ok(()))
}
