//! poll(2), ppoll(2), select(2), pselect6(2), and the waiting rules they
//! share with epoll_wait(2) (`sys::epoll`).
//!
//! **Timeouts.** `poll` takes milliseconds (negative: forever, 0: check
//! once); `ppoll` and `pselect6` a `timespec`, `select` a `timeval` (null:
//! forever; zero: check once; invalid: `EINVAL`). Each becomes a deadline
//! on `CLOCK_MONOTONIC`. `ppoll`, `select` and `pselect6` write the time
//! left back (ppoll(2) and select(2): the Linux system calls modify their
//! timeout argument), also when interrupted; a failure to write it back is
//! ignored (Poly's choice, to be confirmed by the diff runner).
//!
//! **Signals.** `ppoll`, `pselect6` and `epoll_pwait` install a signal
//! mask for the wait and put the old one back afterwards, atomically with
//! respect to signals: an interrupting signal finds the wait's mask, and
//! the old mask returns only after its handler (the same `saved_mask`
//! rt_sigsuspend uses). A signal the thread does not block, pending at
//! the start or arriving during the wait, ends the wait: if nothing was
//! ready, the call fails with `EINTR`. signal(7) lists `poll`, `ppoll`,
//! `select` and `pselect` among the calls "never restarted after being
//! interrupted by a signal handler"; without a handler (a stop and
//! `SIGCONT`) they restart, with the time left for the ones that write it
//! back, from the start for `poll` (to be confirmed by the diff runner).
//!
//! **select's sets.** select(2): a descriptor is ready for reading "if a
//! read operation will not block; in particular, a file descriptor is also
//! ready on end-of-file", for writing "if a write operation will not
//! block", and the exceptional set watches for urgent data. So a
//! descriptor counts as readable for `POLLIN`, `POLLRDNORM`,
//! `POLLRDBAND`, `POLLHUP` or `POLLERR`, writable for `POLLOUT`,
//! `POLLWRNORM`, `POLLWRBAND` or `POLLERR` (a write fails at once), and
//! exceptional for `POLLPRI`. A set descriptor that is not open is `EBADF`.

use super::io::bounce;
use super::pipe::file_io;
use crate::abi::Timespec;
use crate::abi::signal::SIGSET_SIZE;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{FileIo, PollFd, WaitTime, deadline_after, events::*, remaining};
use crate::kernel::{Clock, Deadline, LinuxKernel, SigSet, UserPtr};
use crate::signal::deliver::with_own;
use alloc::vec::Vec;

/// How a wait in [`wait_with_signals`] ended.
pub(crate) enum Waited<T> {
    Done(T),
    /// A signal (or the end of the process) ended it with nothing ready.
    Interrupted,
}

/// Install `mask` for the calling thread, keeping the old one to put back
/// (rt_sigsuspend's `saved_mask`).
fn install_mask(k: &mut dyn LinuxKernel, mask: SigSet) {
    let tid = k.tid();
    with_own(k, |p| {
        let t = p.thread_mut(tid);
        t.saved_mask.get_or_insert(t.mask);
        t.mask = mask.without(SigSet::UNBLOCKABLE);
    });
}

/// Put the mask [`install_mask`] replaced back.
fn restore_mask(k: &mut dyn LinuxKernel) {
    let tid = k.tid();
    with_own(k, |p| {
        let t = p.thread_mut(tid);
        if let Some(mask) = t.saved_mask.take() {
            t.mask = mask;
        }
    });
}

/// One wait of `poll`, `select` or `epoll_wait`: with `mask` installed if
/// given, run `attempt` with `wait` (or only check, if a signal can be
/// delivered already), and tell a result from an interruption. `none`
/// says a result has nothing ready. After an interruption the installed
/// mask stays until the signal is delivered; otherwise it is put back.
pub(crate) fn wait_with_signals<T>(
    k: &mut dyn LinuxKernel,
    mask: Option<SigSet>,
    wait: WaitTime,
    mut attempt: impl FnMut(&mut dyn FileIo, WaitTime) -> Result<T, Errno>,
    none: impl Fn(&T) -> bool,
) -> Result<Waited<T>, Errno> {
    if let Some(mask) = mask {
        install_mask(k, mask);
    }
    // A signal sent from now on finds the mask above and notifies the
    // thread, which ends the kernel's wait; one pending already is seen
    // here.
    k.clear_signal_notification();
    let tid = k.tid();
    let deliverable = with_own(k, |p| p.has_deliverable(tid));
    let wait = if deliverable { WaitTime::Poll } else { wait };
    let result = match file_io(k) {
        Ok(io) => match attempt(io, wait) {
            Ok(r) if deliverable && none(&r) => Ok(Waited::Interrupted),
            Ok(r) => Ok(Waited::Done(r)),
            Err(Errno::EINTR) => Ok(Waited::Interrupted),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    if mask.is_some() && !matches!(result, Ok(Waited::Interrupted)) {
        restore_mask(k);
    }
    result
}

/// A `sigset_t` argument with its size: `None` for a null pointer,
/// `EINVAL` for a size other than 8, `EFAULT`.
pub(crate) fn sigmask_arg(k: &mut dyn LinuxKernel, ptr: UserPtr, size: u64) -> Result<Option<SigSet>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    if size != SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let mut raw = [0; 8];
    k.copy_from_user(ptr, &mut raw)?;
    Ok(Some(SigSet(u64::from_le_bytes(raw))))
}

/// A timeout in milliseconds: negative waits forever, 0 only checks.
pub(crate) fn ms_timeout(k: &mut dyn LinuxKernel, ms: i32) -> Result<WaitTime, Errno> {
    Ok(match ms {
        ms if ms < 0 => WaitTime::Forever,
        0 => WaitTime::Poll,
        ms => {
            let now = k.clock(Clock::Monotonic)?;
            WaitTime::Until(deadline_after(now, Clock::Monotonic, Timespec::from_nanos(ms as u64 * 1_000_000)))
        }
    })
}

/// A `timespec` timeout (null: forever) and its deadline, for writing the
/// time left back.
pub(crate) fn timespec_timeout(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<(WaitTime, Option<Deadline>), Errno> {
    if ptr.is_null() {
        return Ok((WaitTime::Forever, None));
    }
    let mut raw = [0; Timespec::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    timeout_from(k, Timespec::from_bytes(&raw))
}

fn timeout_from(k: &mut dyn LinuxKernel, t: Timespec) -> Result<(WaitTime, Option<Deadline>), Errno> {
    if !t.is_valid() {
        return Err(Errno::EINVAL);
    }
    let now = k.clock(Clock::Monotonic)?;
    let deadline = deadline_after(now, Clock::Monotonic, t);
    let wait = if t.is_zero() { WaitTime::Poll } else { WaitTime::Until(deadline) };
    Ok((wait, Some(deadline)))
}

/// `struct timeval` (`linux/time_types.h`'s `__kernel_old_timeval`):
/// seconds and microseconds, both `long`; 16 bytes.
const TIMEVAL_SIZE: usize = 16;

fn timeval_timeout(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<(WaitTime, Option<Deadline>), Errno> {
    if ptr.is_null() {
        return Ok((WaitTime::Forever, None));
    }
    let mut raw = [0; TIMEVAL_SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    let sec = crate::abi::le_u64(&raw[..8]) as i64;
    let usec = crate::abi::le_u64(&raw[8..]) as i64;
    if !(0..1_000_000).contains(&usec) {
        return Err(Errno::EINVAL);
    }
    timeout_from(k, Timespec { sec, nsec: usec * 1000 })
}

/// Write the time left to `deadline` to the timeout at `ptr`, as a
/// `timespec` or (`timeval`) a `timeval`. Errors are ignored.
fn write_back(k: &mut dyn LinuxKernel, ptr: UserPtr, deadline: Option<Deadline>, timeval: bool) {
    let Some(deadline) = deadline else { return };
    let Ok(now) = k.clock(Clock::Monotonic) else { return };
    let left = remaining(now, &deadline);
    let bytes = if timeval { Timespec { sec: left.sec, nsec: left.nsec / 1000 } } else { left }.to_bytes();
    let _ = k.copy_to_user(ptr, &bytes);
}

/// poll(fds, nfds, timeout)
pub fn poll(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let wait = ms_timeout(k, a.int(2))?;
    poll_fds(k, a.ptr(0), a.ulong(1), wait, None)
}

/// ppoll(fds, nfds, tmo_p, sigmask, sigsetsize)
pub fn ppoll(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (wait, deadline) = timespec_timeout(k, a.ptr(2))?;
    let mask = sigmask_arg(k, a.ptr(3), a.ulong(4))?;
    let result = poll_fds(k, a.ptr(0), a.ulong(1), wait, mask);
    if matches!(result, Ok(_) | Err(Errno::RESTART_IF_NO_HANDLER)) {
        write_back(k, a.ptr(2), deadline, false);
    }
    result
}

/// The `poll` part of poll and ppoll: poll(2), `EINVAL` "The nfds value
/// exceeds the RLIMIT_NOFILE value", `EFAULT` for the array (read before
/// the wait, `revents` written after it).
fn poll_fds(k: &mut dyn LinuxKernel, ptr: UserPtr, nfds: u64, wait: WaitTime, mask: Option<SigSet>) -> SysResult {
    let limit = file_io(k)?.open_files_limit() as u64;
    if nfds > limit {
        return Err(Errno::EINVAL);
    }
    let n = nfds as usize;
    let mut raw = bounce(n * PollFd::SIZE)?;
    if n > 0 {
        k.copy_from_user(ptr, &mut raw)?;
    }
    let mut fds: Vec<PollFd> = raw.chunks_exact(PollFd::SIZE).map(PollFd::from_bytes).collect();
    match wait_with_signals(k, mask, wait, |io, w| io.poll(&mut fds, w, false), |&n| n == 0)? {
        Waited::Done(ready) => {
            for (chunk, fd) in raw.chunks_exact_mut(PollFd::SIZE).zip(&fds) {
                chunk.copy_from_slice(&fd.to_bytes());
            }
            if n > 0 {
                k.copy_to_user(ptr, &raw)?;
            }
            Ok(ready as u64)
        }
        Waited::Interrupted => Err(Errno::RESTART_IF_NO_HANDLER),
    }
}

/// select(nfds, readfds, writefds, exceptfds, timeout)
pub fn select(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (wait, deadline) = timeval_timeout(k, a.ptr(4))?;
    let result = select_fds(k, a.int(0), [a.ptr(1), a.ptr(2), a.ptr(3)], wait, None);
    if matches!(result, Ok(_) | Err(Errno::RESTART_IF_NO_HANDLER)) {
        write_back(k, a.ptr(4), deadline, true);
    }
    result
}

/// pselect6(nfds, readfds, writefds, exceptfds, timeout, sigmask_and_size)
///
/// pselect(2), NOTES: the last argument is a pointer to `{ const sigset_t
/// *ss; size_t ss_len; }`; a null pointer, or a null `ss`, means no mask.
pub fn pselect6(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (wait, deadline) = timespec_timeout(k, a.ptr(4))?;
    let mask = if a.ptr(5).is_null() {
        None
    } else {
        let mut raw = [0; 16];
        k.copy_from_user(a.ptr(5), &mut raw)?;
        sigmask_arg(k, UserPtr(crate::abi::le_u64(&raw[..8])), crate::abi::le_u64(&raw[8..]))?
    };
    let result = select_fds(k, a.int(0), [a.ptr(1), a.ptr(2), a.ptr(3)], wait, mask);
    if matches!(result, Ok(_) | Err(Errno::RESTART_IF_NO_HANDLER)) {
        write_back(k, a.ptr(4), deadline, false);
    }
    result
}

/// What select asks poll for, per set (see the module documentation).
const SET_EVENTS: [u16; 3] =
    [POLLIN | POLLRDNORM | POLLRDBAND | POLLHUP | POLLERR, POLLOUT | POLLWRNORM | POLLWRBAND | POLLERR, POLLPRI];

fn bit(set: &[u64], fd: usize) -> bool {
    set.get(fd / 64).is_some_and(|w| w & (1 << (fd % 64)) != 0)
}

/// The `select` part of select and pselect6: select(2), `EINVAL` "nfds is
/// negative or exceeds the RLIMIT_NOFILE resource limit", `EBADF`,
/// `EFAULT`. The sets are read as `ceil(nfds / 64)` words and, on
/// success, written back the same size with only the ready descriptors
/// (bits from `nfds` up cleared); on an error they are left alone.
fn select_fds(
    k: &mut dyn LinuxKernel,
    nfds: i32,
    sets: [UserPtr; 3],
    wait: WaitTime,
    mask: Option<SigSet>,
) -> SysResult {
    if nfds < 0 {
        return Err(Errno::EINVAL);
    }
    let nfds = nfds as usize;
    if nfds > file_io(k)?.open_files_limit() {
        return Err(Errno::EINVAL);
    }
    let words = nfds.div_ceil(64);
    let mut input: [Vec<u64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (set, ptr) in input.iter_mut().zip(sets) {
        if ptr.is_null() || words == 0 {
            continue;
        }
        let mut raw = bounce(words * 8)?;
        k.copy_from_user(ptr, &mut raw)?;
        *set = raw.chunks_exact(8).map(crate::abi::le_u64).collect();
    }
    let mut fds: Vec<PollFd> = (0..nfds)
        .filter_map(|fd| {
            let events = (0..3).filter(|&s| bit(&input[s], fd)).fold(0, |e, s| e | SET_EVENTS[s]);
            (events != 0).then_some(PollFd { fd: fd as i32, events, revents: 0 })
        })
        .collect();
    let waited = wait_with_signals(k, mask, wait, |io, w| io.poll(&mut fds, w, true), |&n| n == 0)?;
    if matches!(waited, Waited::Interrupted) {
        return Err(Errno::RESTART_IF_NO_HANDLER);
    }
    if fds.iter().any(|f| f.revents & POLLNVAL != 0) {
        return Err(Errno::EBADF);
    }
    let mut output: [Vec<u64>; 3] = [alloc::vec![0; words], alloc::vec![0; words], alloc::vec![0; words]];
    let mut count = 0u64;
    for f in &fds {
        let fd = f.fd as usize;
        for s in 0..3 {
            if bit(&input[s], fd) && f.revents & SET_EVENTS[s] != 0 {
                output[s][fd / 64] |= 1 << (fd % 64);
                count += 1;
            }
        }
    }
    for (set, ptr) in output.iter().zip(sets) {
        if ptr.is_null() || words == 0 {
            continue;
        }
        let bytes: Vec<u8> = set.iter().flat_map(|w| w.to_le_bytes()).collect();
        k.copy_to_user(ptr, &bytes)?;
    }
    Ok(count)
}
