//! splice(2), tee(2) and vmsplice(2): moving bytes into, out of and
//! between pipes without a trip through a user buffer.
//!
//! The pipes keep their buffers ([`FileIo::pipe_transfer`] and friends,
//! over `poly_vfs::pipe`): between two pipes each buffer of the input (or
//! the part of it within the count) becomes a buffer of the output, and
//! each piece vmsplice(2) puts in (a segment, cut at the page boundaries
//! of its address) is a buffer of its own, so the output's free buffers
//! bound what one call moves. A file, memory device or socket on the other
//! side goes through a kernel buffer: into a pipe at most the bytes of its
//! free pages (like sendfile(2)), out of a pipe at most what it holds,
//! peeked first and taken only once written, so an error of the output
//! leaves the bytes in the pipe. vmsplice(2) copies the caller's memory
//! at the call (Linux's pipe refers to the caller's pages until they are
//! read; Poly's pipes hold bytes).
//!
//! The checks follow the spec sheets' orders ([`check_splice`] has
//! splice's checks on the descriptors' kinds and the values). A call that
//! has to wait polls the pipe (or socket) it waits for, unless
//! `SPLICE_F_NONBLOCK` (or, for splice and tee, `O_NONBLOCK` on the pipe
//! that would wait) makes it `EAGAIN`; a signal ends the wait and the call
//! restarts under `SA_RESTART` (`EINTR` otherwise). `EPIPE` raises
//! `SIGPIPE` once per call.

use crate::abi::{IOV_MAX, Iovec, MAX_RW_COUNT, PAGE_SIZE};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{FileIo, PipeTransfer, PollFd, WaitTime, events};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::io::{bounce, load_iovecs, sigpipe_on_epipe};
use crate::sys::path::fs;
use crate::sys::sendfile::{Desc, FileKind, describe};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SPLICE, splice), (nr::TEE, tee), (nr::VMSPLICE, vmsplice)];

/// The flags (`linux/splice.h`); only `SPLICE_F_NONBLOCK` does anything.
pub const SPLICE_F_MOVE: u32 = 1;
pub const SPLICE_F_NONBLOCK: u32 = 2;
pub const SPLICE_F_MORE: u32 = 4;
pub const SPLICE_F_GIFT: u32 = 8;
const SPLICE_F_ALL: u32 = SPLICE_F_MOVE | SPLICE_F_NONBLOCK | SPLICE_F_MORE | SPLICE_F_GIFT;

/// Open file status bits (`asm-generic/fcntl.h`).
const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_APPEND: u32 = 0o2000;
const O_NONBLOCK: u32 = 0o4000;
const O_PATH: u32 = 0o10000000;

/// `SEEK_SET`, `SEEK_CUR` (`linux/fs.h`).
const SEEK_SET: u32 = 0;
const SEEK_CUR: u32 = 1;

/// The minor number of `/dev/null` among the memory devices
/// (`Documentation/admin-guide/devices.txt`: major 1, minor 3).
const NULL_MINOR: u32 = 3;

fn readable(d: Desc) -> bool {
    d.flags & O_PATH == 0 && d.flags & O_ACCMODE != O_WRONLY
}

fn writable(d: Desc) -> bool {
    d.flags & O_PATH == 0 && matches!(d.flags & O_ACCMODE, O_WRONLY | O_RDWR)
}

fn nonblocking(d: Desc) -> bool {
    d.flags & O_NONBLOCK != 0
}

/// What a splice(2) moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Between two pipes.
    PipeToPipe,
    /// From a file or memory device (read at a position) into a pipe.
    FileToPipe,
    /// From a socket (read as a stream) into a pipe.
    SocketToPipe,
    /// From a pipe into a file (written at a position), a memory device or
    /// a socket.
    FromPipe,
}

/// splice(2)'s checks once both descriptors are open and the offsets were
/// read (`off_in`, `off_out`: the values, `None` for null pointers; an
/// offset for a pipe side was `ESPIPE` before), in the spec sheet's
/// order: `EBADF` for an input not open for reading or an output not open
/// for writing; then `EINVAL` for neither side a pipe, a side that is not
/// a regular file, memory device (not `/dev/null` as input) or socket, an
/// offset for a socket, an output with `O_APPEND`, a negative offset, and
/// a count with bit 63 set or going past 2^63-1 from its offset when a
/// file or device is read or written. `null_input`: the input is
/// `/dev/null`.
pub fn check_splice(
    input: Desc,
    output: Desc,
    null_input: bool,
    off_in: Option<i64>,
    off_out: Option<i64>,
    len: u64,
) -> Result<Case, Errno> {
    if !readable(input) || !writable(output) || output.kind == FileKind::Directory {
        return Err(Errno::EBADF);
    }
    let case = match (input.kind, output.kind) {
        (FileKind::Pipe, FileKind::Pipe) => Case::PipeToPipe,
        (FileKind::Pipe, FileKind::Regular) if output.flags & O_APPEND != 0 => return Err(Errno::EINVAL),
        (FileKind::Pipe, FileKind::Regular | FileKind::Memory) => Case::FromPipe,
        (FileKind::Pipe, FileKind::Socket) if off_out.is_none() => Case::FromPipe,
        (FileKind::Memory, FileKind::Pipe) if null_input => return Err(Errno::EINVAL),
        (FileKind::Regular | FileKind::Memory, FileKind::Pipe) => Case::FileToPipe,
        (FileKind::Socket, FileKind::Pipe) if off_in.is_none() => Case::SocketToPipe,
        _ => return Err(Errno::EINVAL),
    };
    let positioned = |kind: FileKind| matches!(kind, FileKind::Regular | FileKind::Memory);
    for (off, kind) in [(off_in, input.kind), (off_out, output.kind)] {
        if !positioned(kind) {
            continue;
        }
        if len > i64::MAX as u64 {
            return Err(Errno::EINVAL);
        }
        if let Some(at) = off {
            let at = u64::try_from(at).map_err(|_| Errno::EINVAL)?;
            if at.checked_add(len).is_none_or(|end| end > i64::MAX as u64) {
                return Err(Errno::EINVAL);
            }
        }
    }
    Ok(case)
}

/// The kernel's pipes, `EINVAL` without them.
fn io(k: &mut dyn LinuxKernel) -> Result<&mut dyn FileIo, Errno> {
    k.file_io().ok_or(Errno::EINVAL)
}

/// Wait until `fd` has one of `events` (or an error or hang-up); a signal
/// ends the wait, and the call restarts under `SA_RESTART`.
fn wait(k: &mut dyn LinuxKernel, fd: i32, events: u16) -> Result<(), Errno> {
    let mut fds = [PollFd { fd, events, revents: 0 }];
    match io(k)?.poll(&mut fds, WaitTime::Forever, false) {
        Ok(_) => Ok(()),
        Err(Errno::EINTR) => Err(Errno::RESTART_IF_SA_RESTART),
        Err(e) => Err(e),
    }
}

/// Whether `fd` has `events` now.
fn ready(k: &mut dyn LinuxKernel, fd: i32, events: u16) -> Result<bool, Errno> {
    let mut fds = [PollFd { fd, events, revents: 0 }];
    io(k)?.poll(&mut fds, WaitTime::Poll, false)?;
    Ok(fds[0].revents != 0)
}

/// An `loff_t *` argument: `None` for a null pointer.
fn read_offset(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Option<i64>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    let mut raw = [0u8; 8];
    k.copy_from_user(ptr, &mut raw)?;
    Ok(Some(i64::from_le_bytes(raw)))
}

/// Move between two pipes (splice, or tee with `keep`), waiting as
/// allowed.
fn between_pipes(k: &mut dyn LinuxKernel, fd_in: i32, fd_out: i32, len: u64, keep: bool, nonblock: bool) -> SysResult {
    let len = len.min(MAX_RW_COUNT) as usize;
    loop {
        match io(k)?.pipe_transfer(fd_in, fd_out, len, keep)? {
            PipeTransfer::Moved(n) => return Ok(n as u64),
            _ if nonblock => return Err(Errno::EAGAIN),
            PipeTransfer::WaitInput => wait(k, fd_in, events::POLLIN)?,
            PipeTransfer::WaitOutput => {
                // A full pipe nobody reads never gets room.
                if io(k)?.pipe_status(fd_out)?.readers == 0 {
                    return Err(Errno::EPIPE);
                }
                wait(k, fd_out, events::POLLOUT)?;
            }
        }
    }
}

/// Room in the pipe `fd` for bytes from a file or socket: `EPIPE` without
/// readers; without a free page `EAGAIN` or a wait.
fn pipe_room(k: &mut dyn LinuxKernel, fd: i32, nonblock: bool) -> Result<usize, Errno> {
    loop {
        let st = io(k)?.pipe_status(fd)?;
        if st.readers == 0 {
            return Err(Errno::EPIPE);
        }
        if st.room > 0 {
            return Ok(st.room);
        }
        if nonblock {
            return Err(Errno::EAGAIN);
        }
        wait(k, fd, events::POLLOUT)?;
    }
}

/// The bytes in the pipe `fd`: 0 once it is empty without writers;
/// while it is empty with writers `EAGAIN` or a wait.
fn pipe_bytes(k: &mut dyn LinuxKernel, fd: i32, nonblock: bool) -> Result<usize, Errno> {
    loop {
        let st = io(k)?.pipe_status(fd)?;
        if st.len > 0 || st.writers == 0 {
            return Ok(st.len);
        }
        if nonblock {
            return Err(Errno::EAGAIN);
        }
        wait(k, fd, events::POLLIN)?;
    }
}

/// Take `n` bytes from the front of the pipe `fd` (they are there).
fn consume(k: &mut dyn LinuxKernel, fd: i32, n: usize) -> Result<(), Errno> {
    let mut sink = bounce(n.min(64 * 1024))?;
    let mut left = n;
    while left > 0 {
        let want = left.min(sink.len());
        match k.read(fd, &mut sink[..want])? {
            0 => break,
            got => left -= got.min(want),
        }
    }
    Ok(())
}

/// splice(fd_in, off_in, fd_out, off_out, len, flags)
pub fn splice(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let len = a.ulong(4);
    if len == 0 {
        return Ok(0);
    }
    let flags = a.uint(5);
    if flags & !SPLICE_F_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let (fd_in, fd_out) = (a.fd(0), a.fd(2));
    if fd_in < 0 {
        return Err(Errno::EBADF);
    }
    let input = describe(k, fd_in)?;
    if fd_out < 0 {
        return Err(Errno::EBADF);
    }
    let output = describe(k, fd_out)?;
    let (in_ptr, out_ptr) = (a.ptr(1), a.ptr(3));
    if (!in_ptr.is_null() && input.kind == FileKind::Pipe) || (!out_ptr.is_null() && output.kind == FileKind::Pipe) {
        return Err(Errno::ESPIPE);
    }
    let off_in = read_offset(k, in_ptr)?;
    let off_out = read_offset(k, out_ptr)?;
    let null_input = input.kind == FileKind::Memory && crate::fs::minor(k.fstat(fd_in)?.rdev) == NULL_MINOR;
    let case = check_splice(input, output, null_input, off_in, off_out, len)?;
    let flag_nonblock = flags & SPLICE_F_NONBLOCK != 0;
    let moved = match case {
        Case::PipeToPipe => {
            let nonblock = flag_nonblock || nonblocking(input) || nonblocking(output);
            let result = between_pipes(k, fd_in, fd_out, len, false, nonblock);
            sigpipe_on_epipe(k, result)?
        }
        Case::FileToPipe | Case::SocketToPipe => {
            let room = pipe_room(k, fd_out, flag_nonblock || nonblocking(output)).map(|r| r as u64);
            let room = sigpipe_on_epipe(k, room)?;
            let n = len.min(room) as usize;
            let mut buf = bounce(n)?;
            let got = if case == Case::SocketToPipe {
                if flag_nonblock && !ready(k, fd_in, events::POLLIN | events::POLLHUP)? {
                    return Err(Errno::EAGAIN);
                }
                k.read(fd_in, &mut buf)?
            } else {
                let start = match off_in {
                    Some(at) => Some(at as u64),
                    None => match fs(k)?.lseek(fd_in, 0, SEEK_CUR) {
                        Ok(at) => Some(at),
                        Err(Errno::ESPIPE) => None,
                        Err(e) => return Err(e),
                    },
                };
                let got = match start {
                    Some(at) => fs(k)?.pread(fd_in, &mut buf, at)?,
                    None => k.read(fd_in, &mut buf)?,
                };
                let got = got.min(n);
                let written = if got > 0 { k.write(fd_out, &buf[..got])? } else { 0 };
                if off_in.is_none()
                    && let Some(at) = start
                    && written > 0
                {
                    fs(k)?.lseek(fd_in, (at + written as u64) as i64, SEEK_SET)?;
                }
                if let Some(at) = off_in {
                    k.copy_to_user(in_ptr, &(at as u64 + written as u64).to_le_bytes())?;
                }
                return Ok(written as u64);
            };
            let got = got.min(n);
            if got > 0 { k.write(fd_out, &buf[..got])? as u64 } else { 0 }
        }
        Case::FromPipe => {
            let have = pipe_bytes(k, fd_in, flag_nonblock || nonblocking(input))?;
            let n = (len.min(have as u64)) as usize;
            if n == 0 {
                0
            } else {
                let mut buf = bounce(n)?;
                let n = io(k)?.pipe_peek(fd_in, &mut buf)?;
                let written = match (output.kind, off_out) {
                    (FileKind::Regular, Some(at)) => fs(k)?.pwrite(fd_out, &buf[..n], at as u64),
                    _ => k.write(fd_out, &buf[..n]),
                };
                let written = sigpipe_on_epipe(k, written.map(|w| w as u64))? as usize;
                consume(k, fd_in, written.min(n))?;
                written.min(n) as u64
            }
        }
    };
    if let Some(at) = off_out {
        k.copy_to_user(out_ptr, &(at as u64 + moved).to_le_bytes())?;
    }
    Ok(moved)
}

/// tee(fd_in, fd_out, len, flags)
pub fn tee(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(3);
    if flags & !SPLICE_F_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let len = a.ulong(2);
    if len == 0 {
        return Ok(0);
    }
    let (fd_in, fd_out) = (a.fd(0), a.fd(1));
    if fd_in < 0 {
        return Err(Errno::EBADF);
    }
    let input = describe(k, fd_in)?;
    if !readable(input) {
        return Err(Errno::EBADF);
    }
    if fd_out < 0 {
        return Err(Errno::EBADF);
    }
    let output = describe(k, fd_out)?;
    if !writable(output) || output.kind == FileKind::Directory {
        return Err(Errno::EBADF);
    }
    if input.kind != FileKind::Pipe || output.kind != FileKind::Pipe {
        return Err(Errno::EINVAL);
    }
    let nonblock = flags & SPLICE_F_NONBLOCK != 0 || nonblocking(input) || nonblocking(output);
    let result = between_pipes(k, fd_in, fd_out, len, true, nonblock);
    sigpipe_on_epipe(k, result)
}

/// The pieces vmsplice(2) makes of the segments `iovs` for a pipe with
/// `slots` free buffers: each non-empty segment cut at the page boundaries
/// of its address, at most `slots` pieces, as `(address, length)`.
pub fn pieces(iovs: &[Iovec], slots: usize) -> Vec<(u64, usize)> {
    let mut out = Vec::new();
    for v in iovs {
        let (mut at, mut left) = (v.base, v.len);
        while left > 0 {
            if out.len() == slots {
                return out;
            }
            let to_boundary = PAGE_SIZE - at % PAGE_SIZE;
            let n = left.min(to_boundary);
            out.push((at, n as usize));
            at += n;
            left -= n;
        }
    }
    out
}

/// vmsplice(fd, iov, nr_segs, flags)
pub fn vmsplice(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(3);
    if flags & !SPLICE_F_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let d = describe(k, fd)?;
    if d.flags & O_PATH != 0 {
        return Err(Errno::EBADF);
    }
    let nr = a.uint(2);
    if nr > IOV_MAX {
        return Err(Errno::EINVAL);
    }
    let (iovs, total) = load_iovecs(k, a.ptr(1), nr as i32)?;
    if total == 0 {
        return Ok(0);
    }
    if d.kind != FileKind::Pipe {
        return Err(Errno::EBADF);
    }
    let nonblock = flags & SPLICE_F_NONBLOCK != 0;
    if writable(d) {
        let result = vmsplice_in(k, fd, &iovs, nonblock);
        return sigpipe_on_epipe(k, result);
    }
    // The read end: like readv(2), but the bytes stay in the pipe if they
    // cannot be stored.
    let have = pipe_bytes(k, fd, nonblock)?;
    let n = (total.min(have as u64)) as usize;
    if n == 0 {
        return Ok(0);
    }
    let mut buf = bounce(n)?;
    let n = io(k)?.pipe_peek(fd, &mut buf)?;
    let mut done = 0;
    for v in iovs.iter().filter(|v| v.len > 0) {
        if done == n {
            break;
        }
        let take = (v.len.min((n - done) as u64)) as usize;
        match k.copy_to_user(UserPtr(v.base), &buf[done..done + take]) {
            Ok(()) => done += take,
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    consume(k, fd, done)?;
    Ok(done as u64)
}

/// vmsplice(2) into a pipe: the segments' pieces become buffers while
/// there are free ones; stops at the first piece that cannot be read.
fn vmsplice_in(k: &mut dyn LinuxKernel, fd: i32, iovs: &[Iovec], nonblock: bool) -> SysResult {
    loop {
        let room = pipe_room(k, fd, nonblock)?;
        let slots = room / PAGE_SIZE as usize;
        let mut data: Vec<Vec<u8>> = Vec::new();
        for (at, len) in pieces(iovs, slots) {
            let mut piece = bounce(len)?;
            match k.copy_from_user(UserPtr(at), &mut piece) {
                Ok(()) => data.push(piece),
                Err(e) if data.is_empty() => return Err(e),
                Err(_) => break,
            }
        }
        let refs: Vec<&[u8]> = data.iter().map(|p| &p[..]).collect();
        match io(k)?.pipe_push(fd, &refs) {
            // Another writer took the pages meanwhile.
            Err(Errno::EAGAIN) => continue,
            r => return r.map(|n| n as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: Desc = Desc { kind: FileKind::Regular, flags: O_RDWR };
    const PIPE_R: Desc = Desc { kind: FileKind::Pipe, flags: 0 };
    const PIPE_W: Desc = Desc { kind: FileKind::Pipe, flags: O_WRONLY };
    const SOCK: Desc = Desc { kind: FileKind::Socket, flags: O_RDWR };

    #[test]
    fn splice_checks_in_order() {
        assert_eq!(check_splice(REG, PIPE_W, false, None, None, 5), Ok(Case::FileToPipe));
        assert_eq!(check_splice(PIPE_R, REG, false, None, Some(3), 5), Ok(Case::FromPipe));
        assert_eq!(check_splice(PIPE_R, PIPE_W, false, None, None, u64::MAX), Ok(Case::PipeToPipe));
        assert_eq!(check_splice(SOCK, PIPE_W, false, None, None, 5), Ok(Case::SocketToPipe));
        // Access modes before kinds and values.
        let wronly = Desc { flags: O_WRONLY, ..REG };
        assert_eq!(check_splice(wronly, PIPE_W, false, Some(-5), None, 5), Err(Errno::EBADF));
        assert_eq!(check_splice(REG, PIPE_R, false, Some(-5), None, 5), Err(Errno::EBADF));
        let opath = Desc { flags: O_PATH, ..REG };
        assert_eq!(check_splice(opath, PIPE_W, false, None, None, 5), Err(Errno::EBADF));
        // Kinds.
        assert_eq!(check_splice(REG, REG, false, None, None, 5), Err(Errno::EINVAL));
        assert_eq!(check_splice(SOCK, PIPE_W, false, Some(0), None, 5), Err(Errno::EINVAL));
        assert_eq!(check_splice(PIPE_R, SOCK, false, None, Some(0), 5), Err(Errno::EINVAL));
        assert_eq!(check_splice(SOCK, REG, false, None, None, 5), Err(Errno::EINVAL));
        let null = Desc { kind: FileKind::Memory, flags: O_RDWR };
        assert_eq!(check_splice(null, PIPE_W, true, None, None, 5), Err(Errno::EINVAL));
        assert_eq!(check_splice(PIPE_R, null, false, None, None, 5), Ok(Case::FromPipe));
        let dir = Desc { kind: FileKind::Directory, flags: 0 };
        assert_eq!(check_splice(dir, PIPE_W, false, None, None, 5), Err(Errno::EINVAL));
        let other = Desc { kind: FileKind::Other, flags: O_RDWR };
        assert_eq!(check_splice(PIPE_R, other, false, None, None, 5), Err(Errno::EINVAL));
        let append = Desc { flags: O_WRONLY | O_APPEND, ..REG };
        assert_eq!(check_splice(PIPE_R, append, false, None, Some(0), 1), Err(Errno::EINVAL));
        // Values.
        assert_eq!(check_splice(REG, PIPE_W, false, Some(-1), None, 1), Err(Errno::EINVAL));
        assert_eq!(check_splice(REG, PIPE_W, false, Some(0), None, 1 << 63), Err(Errno::EINVAL));
        assert_eq!(check_splice(PIPE_R, REG, false, None, Some(0), u64::MAX), Err(Errno::EINVAL));
        assert_eq!(check_splice(REG, PIPE_W, false, Some(0x7fff_ffff_ffff_ff00), None, 0x1000), Err(Errno::EINVAL));
    }

    #[test]
    fn vmsplice_pieces_follow_page_boundaries() {
        let v = |base, len| Iovec { base, len };
        assert_eq!(pieces(&[v(0x1000 - 50, 200)], 16), alloc::vec![(0x1000 - 50, 50), (0x1000, 150)]);
        assert_eq!(pieces(&[v(0x1064, 5000)], 16), alloc::vec![(0x1064, 3996), (0x2000, 1004)]);
        assert_eq!(pieces(&[v(0, 0), v(0x2000, 3), v(0x3000, 0)], 16), alloc::vec![(0x2000, 3)]);
        assert_eq!(pieces(&[v(0x10000, 200_000)], 16).len(), 16);
        let ones: Vec<Iovec> = (0..20).map(|i| v(0x1000 + i, 1)).collect();
        assert_eq!(pieces(&ones, 15).len(), 15);
    }
}
