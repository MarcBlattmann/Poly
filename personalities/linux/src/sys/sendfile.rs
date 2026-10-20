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
pub const CALLS: &[(u64, Handler)] = &[(nr::SENDFILE, sendfile)];

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
