//! sendfile(2): copy between two descriptors without a trip through user
//! memory (ADR 0051). BusyBox's `cat` uses it and falls back to read and
//! write without it.
//!
//! Poly copies in chunks of [`CHUNK`] bytes through a kernel buffer:
//! reading at `*offset` (pread, the file offset untouched) or at the input
//! file's offset (read), then writing to the output descriptor. A chunk the
//! output takes only part of is not lost: with an offset pointer the
//! pointer says how far the copy got; without one the input's file offset
//! is moved back over what was read but not written. An error after some
//! bytes were copied returns the count so far (sendfile(2): "the number of
//! bytes written to out_fd").

use crate::abi::MAX_RW_COUNT;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::io::{CHUNK, bounce};
use crate::sys::path::fs;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SENDFILE, sendfile), (nr::COPY_FILE_RANGE, copy_file_range)];

/// `SEEK_CUR` (`linux/fs.h`).
const SEEK_CUR: u32 = 1;

/// sendfile(out_fd, in_fd, offset, count)
pub fn sendfile(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (out_fd, in_fd) = (a.fd(0), a.fd(1));
    if out_fd < 0 || in_fd < 0 {
        return Err(Errno::EBADF);
    }
    let offset_ptr = a.ptr(2);
    let mut offset = if offset_ptr.is_null() {
        None
    } else {
        let mut raw = [0u8; 8];
        k.copy_from_user(offset_ptr, &mut raw)?;
        // sendfile(2): "EINVAL ... offset is negative" (as for pread).
        Some(u64::try_from(i64::from_le_bytes(raw)).map_err(|_| Errno::EINVAL)?)
    };
    let total = a.ulong(3).min(MAX_RW_COUNT);
    let mut buf = bounce(total.clamp(1, CHUNK as u64) as usize)?;
    let mut done: u64 = 0;
    let result = loop {
        if done >= total {
            break Ok(());
        }
        let want = (total - done).min(buf.len() as u64) as usize;
        let read = match offset {
            Some(at) => fs(k)?.pread(in_fd, &mut buf[..want], at),
            None => k.read(in_fd, &mut buf[..want]),
        };
        let n = match read {
            Ok(0) => break Ok(()),
            Ok(n) => n.min(want),
            Err(e) => break Err(e),
        };
        let mut written = 0;
        let mut failed = None;
        while written < n {
            match k.write(out_fd, &buf[written..n]) {
                Ok(0) => break,
                Ok(w) => written += w.min(n - written),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        done += written as u64;
        match &mut offset {
            Some(at) => *at += written as u64,
            None if written < n => {
                // Give back what was read but not written (a pipe cannot:
                // those bytes are gone, as a short write leaves them).
                let _ = fs(k).and_then(|f| f.lseek(in_fd, -((n - written) as i64), SEEK_CUR));
            }
            None => {}
        }
        if let Some(e) = failed {
            break Err(e);
        }
        if written < n {
            break Ok(());
        }
    };
    if let Some(at) = offset {
        k.copy_to_user(offset_ptr, &(at as i64).to_le_bytes())?;
    }
    match result {
        Err(e) if done == 0 => Err(e),
        _ => Ok(done),
    }
}

/// Open file status bits (`asm-generic/fcntl.h`).
const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_APPEND: u32 = 0o2000;

/// An `loff_t *` argument of copy_file_range(2): `None` for a null pointer
/// (the file offset is used), else the offset it holds ("EINVAL" when
/// negative, as for pread).
fn offset_arg(k: &mut dyn LinuxKernel, ptr: crate::kernel::UserPtr) -> Result<Option<u64>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    let mut raw = [0u8; 8];
    k.copy_from_user(ptr, &mut raw)?;
    u64::try_from(i64::from_le_bytes(raw)).map(Some).map_err(|_| Errno::EINVAL)
}

/// copy_file_range(fd_in, off_in, fd_out, off_out, len, flags) (ADR 0059;
/// GNU `cp` tries it first).
///
/// copy_file_range(2): `flags` must be 0 (`EINVAL`); `EBADF` when a
/// descriptor is not open, `fd_in` not for reading, `fd_out` not for
/// writing or for appending; `EISDIR` for a directory, `EINVAL` for any
/// other file that is not a regular file and for overlapping ranges of
/// one file; `EXDEV` when the files are on different file systems (no
/// file system of Poly copies across file systems; callers fall back to
/// read and write). A null offset pointer means the file offset, which
/// then moves; otherwise `*off` moves and the file offset stays. The copy
/// goes through a kernel buffer in chunks, like [`sendfile`]; it stops at
/// the end of the input or at a short write and returns the count (an
/// error after some bytes returns the count so far). The order of the
/// checks is ours, to be confirmed by the diff runner.
pub fn copy_file_range(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd_in, fd_out) = (a.fd(0), a.fd(2));
    if fd_in < 0 || fd_out < 0 {
        return Err(Errno::EBADF);
    }
    let in_flags = fs(k)?.status_flags(fd_in)?;
    let out_flags = fs(k)?.status_flags(fd_out)?;
    if in_flags & O_ACCMODE == O_WRONLY || !matches!(out_flags & O_ACCMODE, O_WRONLY | O_RDWR) {
        return Err(Errno::EBADF);
    }
    if out_flags & O_APPEND != 0 {
        return Err(Errno::EBADF);
    }
    if a.uint(5) != 0 {
        return Err(Errno::EINVAL);
    }
    let (st_in, st_out) = (k.fstat(fd_in)?, k.fstat(fd_out)?);
    for st in [&st_in, &st_out] {
        match st.file_type() {
            crate::abi::mode::S_IFREG => {}
            crate::abi::mode::S_IFDIR => return Err(Errno::EISDIR),
            _ => return Err(Errno::EINVAL),
        }
    }
    if st_in.dev != st_out.dev {
        return Err(Errno::EXDEV);
    }
    let (in_ptr, out_ptr) = (a.ptr(1), a.ptr(3));
    let mut off_in = offset_arg(k, in_ptr)?;
    let mut off_out = offset_arg(k, out_ptr)?;
    let total = a.ulong(4).min(MAX_RW_COUNT);
    if st_in.ino == st_out.ino {
        // The same file: the ranges must not overlap.
        let fs = fs(k)?;
        let start_in = match off_in {
            Some(at) => at,
            None => fs.lseek(fd_in, 0, SEEK_CUR)?,
        };
        let start_out = match off_out {
            Some(at) => at,
            None => fs.lseek(fd_out, 0, SEEK_CUR)?,
        };
        if start_in < start_out.saturating_add(total) && start_out < start_in.saturating_add(total) {
            return Err(Errno::EINVAL);
        }
    }
    let mut buf = bounce(total.clamp(1, CHUNK as u64) as usize)?;
    let mut done: u64 = 0;
    let result = loop {
        if done >= total {
            break Ok(());
        }
        let want = (total - done).min(buf.len() as u64) as usize;
        let read = match off_in {
            Some(at) => fs(k)?.pread(fd_in, &mut buf[..want], at),
            None => k.read(fd_in, &mut buf[..want]),
        };
        let n = match read {
            Ok(0) => break Ok(()),
            Ok(n) => n.min(want),
            Err(e) => break Err(e),
        };
        let mut written = 0;
        let mut failed = None;
        while written < n {
            let w = match off_out {
                Some(at) => fs(k)?.pwrite(fd_out, &buf[written..n], at + written as u64),
                None => k.write(fd_out, &buf[written..n]),
            };
            match w {
                Ok(0) => break,
                Ok(w) => written += w.min(n - written),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        done += written as u64;
        if let Some(at) = &mut off_out {
            *at += written as u64;
        }
        match &mut off_in {
            Some(at) => *at += written as u64,
            None if written < n => {
                let _ = fs(k).and_then(|f| f.lseek(fd_in, -((n - written) as i64), SEEK_CUR));
            }
            None => {}
        }
        if let Some(e) = failed {
            break Err(e);
        }
        if written < n {
            break Ok(());
        }
    };
    if let Some(at) = off_in {
        k.copy_to_user(in_ptr, &(at as i64).to_le_bytes())?;
    }
    if let Some(at) = off_out {
        k.copy_to_user(out_ptr, &(at as i64).to_le_bytes())?;
    }
    match result {
        Err(e) if done == 0 => Err(e),
        _ => Ok(done),
    }
}
