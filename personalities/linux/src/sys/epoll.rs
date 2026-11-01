//! epoll_create(2), epoll_create1(2), epoll_ctl(2), epoll_wait(2),
//! epoll_pwait(2), epoll_pwait2(2).
//!
//! The interest and ready lists, level- and edge-triggered reporting and
//! `EPOLLONESHOT` are the kernel's (`poly_vfs::epoll`); here are the
//! argument rules, the timeouts and the signal mask of the `pwait`
//! variants (as for `ppoll`, see `sys::poll`). An interrupted wait fails
//! with `EINTR`, handler or not: signal(7) lists `epoll_wait` and
//! `epoll_pwait` both among the calls never restarted after a handler and
//! among those that fail with `EINTR` after a stop signal.

use super::pipe::file_io;
use super::poll::{Waited, ms_timeout, sigmask_arg, timespec_timeout, wait_with_signals};
use crate::abi::USER_ADDRESS_LIMIT;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{EpollEvent, EpollOp, WaitTime, flags::O_CLOEXEC};
use crate::kernel::{LinuxKernel, SigSet, UserPtr};
use alloc::vec::Vec;

/// `EP_MAX_EVENTS`: epoll_wait(2) "EINVAL ... maxevents is less than or
/// equal to zero"; more events than an `int`'s worth of bytes cannot be
/// asked for either (Poly's bound, `INT_MAX / sizeof(struct epoll_event)`).
const MAX_EVENTS: i32 = i32::MAX / EpollEvent::SIZE as i32;

/// epoll_create(size): "size is ignored, but must be greater than zero"
/// (`EINVAL` otherwise).
pub fn epoll_create(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.int(0) <= 0 {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.epoll_create(false).map(|fd| fd as u64)
}

/// epoll_create1(flags): only `EPOLL_CLOEXEC` (`EINVAL` otherwise).
pub fn epoll_create1(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(0);
    if flags & !O_CLOEXEC != 0 {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.epoll_create(flags != 0).map(|fd| fd as u64)
}

/// epoll_ctl(epfd, op, fd, event): `EINVAL` for an unknown `op`; `event`
/// is read for `ADD` and `MOD` (`EFAULT`) and ignored for `DEL` ("Since
/// Linux 2.6.9, event can be specified as NULL when using
/// EPOLL_CTL_DEL"). Descriptors are the kernel's to check (`EBADF`
/// first, then `EINVAL` / `EPERM` / `EEXIST` / `ENOENT` / `ELOOP`).
pub fn epoll_ctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (epfd, fd) = (a.fd(0), a.fd(2));
    let op = match a.int(1) {
        1 => EpollOp::Add,
        2 => EpollOp::Del,
        3 => EpollOp::Mod,
        _ => return Err(Errno::EINVAL),
    };
    if epfd < 0 || fd < 0 {
        return Err(Errno::EBADF);
    }
    let event = if op == EpollOp::Del {
        EpollEvent::default()
    } else {
        let mut raw = [0; EpollEvent::SIZE];
        k.copy_from_user(a.ptr(3), &mut raw)?;
        EpollEvent::from_bytes(&raw)
    };
    file_io(k)?.epoll_ctl(epfd, op, fd, event).map(|()| 0)
}

/// epoll_wait(epfd, events, maxevents, timeout)
pub fn epoll_wait(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let max = max_events(a.int(2))?;
    let wait = ms_timeout(k, a.int(3))?;
    wait_events(k, a.fd(0), a.ptr(1), max, wait, None)
}

/// epoll_pwait(epfd, events, maxevents, timeout, sigmask, sigsetsize)
pub fn epoll_pwait(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let max = max_events(a.int(2))?;
    let wait = ms_timeout(k, a.int(3))?;
    let mask = sigmask_arg(k, a.ptr(4), a.ulong(5))?;
    wait_events(k, a.fd(0), a.ptr(1), max, wait, mask)
}

/// epoll_pwait2(epfd, events, maxevents, timeout, sigmask, sigsetsize):
/// epoll_pwait with a `timespec` timeout (null: forever).
///
/// Checks in the order of epoll_pwait2's spec sheet: the timeout (`EFAULT`,
/// `EINVAL`), `sigsetsize` (`EINVAL`), the mask (`EFAULT`), `epfd`
/// (`EBADF`), `maxevents` (`EINVAL`), the range of `events` (`EFAULT` when
/// `events + maxevents * 12` is not within user space), then the kernel's
/// (not an epoll instance: `EINVAL`), and writing the events (`EFAULT`).
pub fn epoll_pwait2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (wait, _) = timespec_timeout(k, a.ptr(3))?;
    let mask = sigmask_arg(k, a.ptr(4), a.ulong(5))?;
    let epfd = a.fd(0);
    // Whether `epfd` is open, asked of the descriptor table (without a
    // file system service the kernel's epoll_wait says it later).
    let open = epfd >= 0 && crate::sys::path::fs(k).map_or(true, |fs| fs.cloexec(epfd).is_ok());
    if !open {
        return Err(Errno::EBADF);
    }
    let max = max_events(a.int(2))?;
    events_in_range(a.ptr(1), max)?;
    wait_events(k, epfd, a.ptr(1), max, wait, mask)
}

/// `EFAULT` unless `max` events at `events` lie within user space.
fn events_in_range(events: UserPtr, max: usize) -> Result<(), Errno> {
    let end = events.0.checked_add((max * EpollEvent::SIZE) as u64).ok_or(Errno::EFAULT)?;
    if end > USER_ADDRESS_LIMIT { Err(Errno::EFAULT) } else { Ok(()) }
}

fn max_events(max: i32) -> Result<usize, Errno> {
    if max <= 0 || max > MAX_EVENTS { Err(Errno::EINVAL) } else { Ok(max as usize) }
}

/// Wait on the set and copy the events out (`EFAULT` if they cannot be
/// written; the events are consumed then, Poly's choice).
fn wait_events(
    k: &mut dyn LinuxKernel,
    epfd: i32,
    out: UserPtr,
    max: usize,
    wait: WaitTime,
    mask: Option<SigSet>,
) -> SysResult {
    if epfd < 0 {
        return Err(Errno::EBADF);
    }
    // A zero timeout only polls: the mask is put back before anything is
    // delivered, so a pending signal it unblocks stays pending
    // (epoll_pwait2's spec sheet, "sigmask_poll"), as if there were none.
    let mask = if wait == WaitTime::Poll { None } else { mask };
    let events = match wait_with_signals(k, mask, wait, |io, w| io.epoll_wait(epfd, max, w), Vec::is_empty)? {
        Waited::Done(events) => events,
        Waited::Interrupted => return Err(Errno::EINTR),
    };
    let bytes: Vec<u8> = events.iter().flat_map(|e| e.to_bytes()).collect();
    if !bytes.is_empty() {
        k.copy_to_user(out, &bytes)?;
    }
    Ok(events.len() as u64)
}
