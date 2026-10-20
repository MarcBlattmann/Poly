//! File descriptor I/O: read(2), write(2), readv(2), writev(2), close(2),
//! fstat(2), ioctl(2).
//!
//! The VFS works on kernel buffers, so data moves through a bounce buffer
//! of at most [`CHUNK`] bytes: a write copies a chunk from user memory and
//! hands it to [`LinuxKernel::write`], until everything is written or the
//! file takes less than offered; a read does one [`LinuxKernel::read`] of
//! up to one chunk and copies the result out. Both may transfer fewer bytes
//! than asked, which read(2) and write(2) allow.

use crate::abi::signal::{SI_KERNEL, SIGPIPE};
use crate::abi::{FileStat, IOV_MAX, Iovec, MAX_RW_COUNT, SigInfo, ioctl, mode};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, Target, UserPtr};
use crate::signal::deliver::send;
use alloc::vec::Vec;

/// The largest bounce buffer: 64 KiB. Writes of up to this size reach the
/// file in one piece, which covers the atomicity pipes promise for writes
/// of up to `PIPE_BUF` (4 KiB) bytes and keeps writev(2)'s "single block"
/// for every vector up to 64 KiB.
pub const CHUNK: usize = 64 * 1024;

/// A zeroed kernel buffer of `len` bytes; `ENOMEM` if the heap cannot
/// provide it.
pub(crate) fn bounce(len: usize) -> Result<Vec<u8>, Errno> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Errno::ENOMEM)?;
    v.resize(len, 0);
    Ok(v)
}

/// A file descriptor argument: `EBADF` for negative values, which can never
/// name an open file (close(2), read(2), write(2): "fd is not a valid file
/// descriptor").
fn fd_arg(a: &Args, i: usize) -> Result<i32, Errno> {
    let fd = a.fd(i);
    if fd < 0 { Err(Errno::EBADF) } else { Ok(fd) }
}

/// read(fd, buf, count)
pub fn read(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let count = a.ulong(2).min(MAX_RW_COUNT);
    read_into(k, fd, &[Iovec { base: a.ptr(1).0, len: count }], count)
}

/// write(fd, buf, count)
pub fn write(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let count = a.ulong(2).min(MAX_RW_COUNT);
    let result = write_from(k, fd, &[Iovec { base: a.ptr(1).0, len: count }], count);
    sigpipe_on_epipe(k, result)
}

/// readv(fd, iov, iovcnt)
pub fn readv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let (iovs, total) = load_iovecs(k, a.ptr(1), a.int(2))?;
    read_into(k, fd, &iovs, total)
}

/// writev(fd, iov, iovcnt)
pub fn writev(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let (iovs, total) = load_iovecs(k, a.ptr(1), a.int(2))?;
    let result = write_from(k, fd, &iovs, total);
    sigpipe_on_epipe(k, result)
}

/// write(2): "EPIPE fd is connected to a pipe or socket whose reading end
/// is closed. When this happens the writing process will also receive a
/// SIGPIPE signal." It goes to the writing thread, with `SI_KERNEL` as its
/// code (Poly's choice, to be confirmed by the diff runner).
fn sigpipe_on_epipe(k: &mut dyn LinuxKernel, result: SysResult) -> SysResult {
    if result == Err(Errno::EPIPE) {
        let tid = k.tid();
        let info = SigInfo::new(SIGPIPE, SI_KERNEL);
        let _ = send(k, Target::Thread(tid), Some(tid), None, info);
    }
    result
}

/// close(fd)
pub fn close(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    k.close(fd).map(|()| 0)
}

/// fstat(fd, statbuf): `EBADF` for a closed fd, `EFAULT` if `statbuf`
/// cannot be written.
pub fn fstat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let st: FileStat = k.fstat(fd)?;
    k.copy_to_user(a.ptr(1), &st.to_bytes())?;
    Ok(0)
}

/// ioctl(fd, request, arg)
///
/// ioctl(2): `EBADF` for a closed fd; "ENOTTY fd is not associated with a
/// character special device". The personality answers the terminal
/// requests ([`ioctl::is_terminal_request`], e.g. `TCGETS`, which C
/// libraries use to ask "is this a terminal?") with `ENOTTY` for every file
/// that is not a character device; all other requests, and terminal
/// requests on character devices, go to the kernel, which returns `ENOTTY`
/// for devices that are not terminals.
pub fn ioctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let request = a.uint(1);
    let st = k.fstat(fd)?;
    if ioctl::is_terminal_request(request) && st.file_type() != mode::S_IFCHR {
        return Err(Errno::ENOTTY);
    }
    // FIONREAD on a pipe (and other byte streams the kernel knows the
    // length of): the bytes a read would return, as an int.
    if request == ioctl::FIONREAD
        && let Some(io) = k.file_io()
        && let Some(n) = io.bytes_readable(fd)?
    {
        let n = n.min(i32::MAX as usize) as i32;
        k.copy_to_user(a.ptr(2), &n.to_le_bytes())?;
        return Ok(0);
    }
    k.ioctl(fd, request, a.ulong(2))
}

/// Read an iovec array from user memory and check it (readv(2)): `EINVAL`
/// if `count` is negative or above `IOV_MAX`, or if the lengths, as
/// `ssize_t`, are negative or add up past `SSIZE_MAX`; `EFAULT` if the
/// array cannot be read. Returns the vectors and the byte total, capped at
/// [`MAX_RW_COUNT`].
fn load_iovecs(k: &mut dyn LinuxKernel, iov: UserPtr, count: i32) -> Result<(Vec<Iovec>, u64), Errno> {
    if count < 0 || count as u32 > IOV_MAX {
        return Err(Errno::EINVAL);
    }
    let count = count as usize;
    let mut raw = bounce(count * Iovec::SIZE)?;
    if count > 0 {
        k.copy_from_user(iov, &mut raw)?;
    }
    let mut iovs = Vec::new();
    iovs.try_reserve_exact(count).map_err(|_| Errno::ENOMEM)?;
    let mut total: i64 = 0;
    for entry in raw.chunks_exact(Iovec::SIZE) {
        let v = Iovec::from_bytes(entry.try_into().expect("chunks_exact yields whole entries"));
        let len = i64::try_from(v.len).map_err(|_| Errno::EINVAL)?;
        total = total.checked_add(len).ok_or(Errno::EINVAL)?;
        iovs.push(v);
    }
    Ok((iovs, (total as u64).min(MAX_RW_COUNT)))
}

/// Walks the bytes described by a list of iovecs, at most `total` of them.
struct Cursor<'a> {
    iovs: &'a [Iovec],
    index: usize,
    offset: u64,
    left: u64,
}

impl<'a> Cursor<'a> {
    fn new(iovs: &'a [Iovec], total: u64) -> Self {
        Cursor { iovs, index: 0, offset: 0, left: total }
    }

    /// The next contiguous user range of at most `max` bytes, or `None`
    /// when everything has been walked.
    fn next(&mut self, max: u64) -> Option<(UserPtr, u64)> {
        while self.left > 0 && self.index < self.iovs.len() {
            let v = self.iovs[self.index];
            let remaining = v.len - self.offset;
            if remaining == 0 {
                self.index += 1;
                self.offset = 0;
                continue;
            }
            let n = remaining.min(max).min(self.left);
            let at = UserPtr(v.base.wrapping_add(self.offset));
            self.offset += n;
            self.left -= n;
            return Some((at, n));
        }
        None
    }
}

/// Write the `total` bytes described by `iovs` to `fd`, chunk by chunk.
/// An error after some bytes went out ends the call with the count so far
/// (write(2): a partial write is a success).
fn write_from(k: &mut dyn LinuxKernel, fd: i32, iovs: &[Iovec], total: u64) -> SysResult {
    if total == 0 {
        return k.write(fd, &[]).map(|n| n as u64);
    }
    let mut chunk = bounce(total.min(CHUNK as u64) as usize)?;
    let mut cursor = Cursor::new(iovs, total);
    let mut done: u64 = 0;
    while done < total {
        let want = (total - done).min(CHUNK as u64) as usize;
        let mut filled = 0;
        let mut fault = None;
        while filled < want {
            let (at, n) = cursor.next((want - filled) as u64).expect("iovecs cover the total");
            if let Err(e) = k.copy_from_user(at, &mut chunk[filled..filled + n as usize]) {
                fault = Some(e);
                break;
            }
            filled += n as usize;
        }
        if let Some(e) = fault {
            return if done == 0 { Err(e) } else { Ok(done) };
        }
        match k.write(fd, &chunk[..want]) {
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

/// Read from `fd` into the `total` bytes described by `iovs`: one read of
/// up to [`CHUNK`] bytes, scattered in order (readv(2) fills `iov[0]`
/// before `iov[1]`).
fn read_into(k: &mut dyn LinuxKernel, fd: i32, iovs: &[Iovec], total: u64) -> SysResult {
    if total == 0 {
        return k.read(fd, &mut []).map(|n| n as u64);
    }
    let mut buf = bounce(total.min(CHUNK as u64) as usize)?;
    let n = k.read(fd, &mut buf)?.min(buf.len());
    let mut cursor = Cursor::new(iovs, n as u64);
    let mut copied = 0;
    while let Some((at, len)) = cursor.next(u64::MAX) {
        k.copy_to_user(at, &buf[copied..copied + len as usize])?;
        copied += len as usize;
    }
    Ok(n as u64)
}
