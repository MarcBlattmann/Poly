//! sendfile(2): copy between two descriptors without a trip through user
//! memory (ADR 0051). BusyBox's `cat` uses it and falls back to read and
//! write without it.
//!
//! The checks run in the order of sendfile's spec sheet: the offset
//! pointer is read first (`EFAULT`), then the descriptors and their access
//! modes (`EBADF`), then what the files are and the offset and count values
//! ([`check`]: `EINVAL`, `ESPIPE`, `EOVERFLOW`); then a non-blocking pipe
//! without a free page is `EAGAIN` ([`pipe_share`]) and `RLIMIT_FSIZE` may
//! cut the count or refuse it (`EFBIG`, `SIGXFSZ`).
//!
//! The input is read at a position (a regular file or a memory device:
//! `*offset`, or its file offset), except that a socket may feed a pipe as
//! a stream. Poly copies in chunks of [`CHUNK`] bytes through a kernel
//! buffer, reading at the position (pread, so nothing is taken that is not
//! written) and writing to the output at its file offset, or into the pipe
//! or socket. It stops at the end of the input, at a short write or at an
//! error, which is returned only if nothing moved (with `SIGPIPE` for
//! `EPIPE`); else the count (sendfile(2): "the number of bytes written to
//! out_fd"). The position then moves by what was written: `*offset`, which
//! is written back (`EFAULT` if it cannot be, the bytes moved all the
//! same), or the input's file offset. With one descriptor as both input and
//! output and no offset pointer, the bytes are read at the file offset and
//! written over themselves, and the offset moves by the count once.

use crate::abi::{MAX_RW_COUNT, PAGE_SIZE, mode};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::io::{CHUNK, bounce, sigpipe_on_epipe};
use crate::sys::path::fs;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SENDFILE, sendfile), (nr::COPY_FILE_RANGE, copy_file_range)];

/// `SEEK_SET`, `SEEK_CUR` (`linux/fs.h`).
const SEEK_SET: u32 = 0;
const SEEK_CUR: u32 = 1;

/// Open file status bits (`asm-generic/fcntl.h`).
const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_APPEND: u32 = 0o2000;
const O_NONBLOCK: u32 = 0o4000;
const O_PATH: u32 = 0o10000000;

/// The largest offset a read of sendfile may start at: the last page
/// boundary below 2^63. The spec sheet has `EOVERFLOW` for an `*offset`
/// beyond the largest offset the input file allows, which
/// 0x7fffffffffffff00 is and 2^40 is not; where between them the limit lies
/// is Poly's choice, to be confirmed by the diff runner.
pub const LAST_OFFSET: u64 = (i64::MAX as u64) & !(PAGE_SIZE - 1);

/// What a descriptor is, as far as sendfile cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Regular,
    /// A memory device (major 1: `/dev/null`, `/dev/zero`, ...).
    Memory,
    Directory,
    /// A pipe or a FIFO.
    Pipe,
    Socket,
    /// Anything else: other devices, event counters, timers, ...
    Other,
}

/// A descriptor: its kind and its access mode and status flags (`F_GETFL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desc {
    pub kind: FileKind,
    pub flags: u32,
}

/// How the input is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// At a position: `*offset`, or the input's file offset.
    Positioned,
    /// With read, as a stream (a socket into a pipe).
    Stream,
}

/// The checks of sendfile once both descriptors are known to be open, in
/// the spec sheet's order; `offset` is `*offset` (`None` for a null
/// pointer). Returns how the input is read.
///
/// - `EBADF`: the input not open for reading or the output not for writing
///   (`O_PATH` descriptors neither; a directory is never open for writing).
/// - The input: a regular file or a memory device; a directory is
///   `EINVAL`; anything else with an offset pointer `ESPIPE` (sendfile(2):
///   "offset is not NULL but the input file is not seekable"), without one
///   `EINVAL`, except a socket into a pipe.
/// - `EINVAL`: an output with `O_APPEND`, a count or `*offset` below 0, or
///   `*offset + count` past 2^63-1; `EOVERFLOW` for an `*offset` past
///   [`LAST_OFFSET`] ([`check_range`]).
pub fn check(input: Desc, output: Desc, offset: Option<i64>, count: u64) -> Result<Source, Errno> {
    if input.flags & O_PATH != 0 || input.flags & O_ACCMODE == O_WRONLY {
        return Err(Errno::EBADF);
    }
    if output.flags & O_PATH != 0
        || !matches!(output.flags & O_ACCMODE, O_WRONLY | O_RDWR)
        || output.kind == FileKind::Directory
    {
        return Err(Errno::EBADF);
    }
    let source = match input.kind {
        FileKind::Regular | FileKind::Memory => Source::Positioned,
        FileKind::Directory => return Err(Errno::EINVAL),
        _ if offset.is_some() => return Err(Errno::ESPIPE),
        FileKind::Socket if output.kind == FileKind::Pipe => Source::Stream,
        _ => return Err(Errno::EINVAL),
    };
    if output.flags & O_APPEND != 0 || count > i64::MAX as u64 {
        return Err(Errno::EINVAL);
    }
    if let Some(at) = offset {
        check_range(u64::try_from(at).map_err(|_| Errno::EINVAL)?, count)?;
    }
    Ok(source)
}

/// A read of `count` bytes at `at` (both below 2^63): `EINVAL` if it would
/// end past 2^63-1, `EOVERFLOW` if it starts past [`LAST_OFFSET`].
pub fn check_range(at: u64, count: u64) -> Result<(), Errno> {
    if at.checked_add(count).is_none_or(|end| end > i64::MAX as u64) {
        return Err(Errno::EINVAL);
    }
    if at > LAST_OFFSET {
        return Err(Errno::EOVERFLOW);
    }
    Ok(())
}

/// How many of `count` bytes go into a pipe with `room` bytes of free pages
/// and a capacity of `capacity`: at most the room (bytes go in whole pages,
/// a page partly filled or partly read takes none). With no room a
/// non-blocking pipe is `EAGAIN`, even for a count of 0; a blocking one
/// takes up to a pipe's worth, waiting for room as the write does.
pub fn pipe_share(count: u64, room: usize, capacity: usize, nonblock: bool) -> Result<u64, Errno> {
    match room {
        0 if nonblock => Err(Errno::EAGAIN),
        0 => Ok(count.min(capacity as u64)),
        room => Ok(count.min(room as u64)),
    }
}

/// sendfile(out_fd, in_fd, offset, count)
pub fn sendfile(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let offset_ptr = a.ptr(2);
    let offset = if offset_ptr.is_null() {
        None
    } else {
        let mut raw = [0u8; 8];
        k.copy_from_user(offset_ptr, &mut raw)?;
        Some(i64::from_le_bytes(raw))
    };
    let (out_fd, in_fd) = (a.fd(0), a.fd(1));
    if out_fd < 0 || in_fd < 0 {
        return Err(Errno::EBADF);
    }
    let input = describe(k, in_fd)?;
    let output = describe(k, out_fd)?;
    let count = a.ulong(3);
    let source = check(input, output, offset, count)?;
    // Where the input is read: at *offset, or at its file offset.
    let start = match (offset, source) {
        (Some(at), _) => Some(at as u64),
        // A memory device without an offset is read as a stream.
        (None, Source::Positioned) => match fs(k)?.lseek(in_fd, 0, SEEK_CUR) {
            Ok(at) => {
                check_range(at, count)?;
                Some(at)
            }
            Err(Errno::ESPIPE) => None,
            Err(e) => return Err(e),
        },
        (None, Source::Stream) => None,
    };
    let mut total = count.min(MAX_RW_COUNT);
    if output.kind == FileKind::Pipe
        && let Some(io) = k.file_io()
        && let Ok(room) = io.pipe_free_room(out_fd)
    {
        let capacity = io.pipe_size(out_fd).unwrap_or(CHUNK);
        total = pipe_share(total, room, capacity, output.flags & O_NONBLOCK != 0)?;
    }
    // RLIMIT_FSIZE: the transfer may be cut short, or refused.
    let total = crate::sys::fsize::allowed(k, out_fd, None, total)?;
    let (done, failed) = copy(k, in_fd, out_fd, start, total);
    if offset.is_none()
        && let Some(at) = start
        && done > 0
    {
        // The input's file offset moves by what was written (when it is
        // the output's too, the write has moved it there already).
        let _ = fs(k).and_then(|f| f.lseek(in_fd, (at + done) as i64, SEEK_SET));
    }
    if let Some(e) = failed
        && done == 0
    {
        return sigpipe_on_epipe(k, Err(e));
    }
    if let Some(at) = offset {
        k.copy_to_user(offset_ptr, &(at as u64 + done).to_le_bytes())?;
    }
    Ok(done)
}

/// What `fd` is and how it is open; `EBADF` if it is not.
fn describe(k: &mut dyn LinuxKernel, fd: i32) -> Result<Desc, Errno> {
    let st = k.fstat(fd)?;
    let object = match k.file_io() {
        Some(io) => io.is_object(fd)?,
        None => false,
    };
    let kind = match st.file_type() {
        mode::S_IFIFO => FileKind::Pipe,
        mode::S_IFSOCK => FileKind::Socket,
        mode::S_IFDIR => FileKind::Directory,
        // Event counters, timers and the like say they are regular files.
        _ if object => FileKind::Other,
        mode::S_IFREG => FileKind::Regular,
        mode::S_IFCHR if crate::fs::major(st.rdev) == 1 => FileKind::Memory,
        _ => FileKind::Other,
    };
    // A descriptor the file system's table does not know (only the test
    // fakes have such) counts as open for reading and writing.
    let flags = fs(k).ok().and_then(|f| f.status_flags(fd).ok()).unwrap_or(O_RDWR);
    Ok(Desc { kind, flags })
}

/// Copy up to `total` bytes from `in_fd`, read at `start` (or as a stream
/// for `None`), to `out_fd`, chunk by chunk, until the input ends or the
/// output takes less than offered. Returns the bytes written and the error
/// that ended the copy, if one did.
fn copy(k: &mut dyn LinuxKernel, in_fd: i32, out_fd: i32, start: Option<u64>, total: u64) -> (u64, Option<Errno>) {
    if total == 0 {
        return (0, None);
    }
    let mut buf = match bounce(total.min(CHUNK as u64) as usize) {
        Ok(buf) => buf,
        Err(e) => return (0, Some(e)),
    };
    let mut done: u64 = 0;
    while done < total {
        let want = (total - done).min(buf.len() as u64) as usize;
        let read = match start {
            Some(at) => fs(k).and_then(|f| f.pread(in_fd, &mut buf[..want], at + done)),
            None => k.read(in_fd, &mut buf[..want]),
        };
        let n = match read {
            Ok(0) => break,
            Ok(n) => n.min(want),
            Err(e) => return (done, Some(e)),
        };
        match k.write(out_fd, &buf[..n]) {
            Ok(w) => {
                let w = w.min(n);
                done += w as u64;
                if w < n {
                    break;
                }
            }
            Err(e) => return (done, Some(e)),
        }
    }
    (done, None)
}

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
