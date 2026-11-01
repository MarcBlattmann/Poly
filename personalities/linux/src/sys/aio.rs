//! io_setup(2), io_destroy(2), io_submit(2), io_cancel(2),
//! io_getevents(2), io_pgetevents(2): native Linux AIO. The contexts and
//! their rings are [`crate::aio`]'s; this decodes the arguments in the
//! order the calls report errors (the spec sheets), does the I/O and posts
//! the completions into the caller's ring.

use super::pipe::file_io;
use crate::abi::mode::{S_IFDIR, S_IFIFO, S_IFSOCK};
use crate::abi::{IOV_MAX, Iovec, MAX_RW_COUNT, Timespec};
use crate::aio::{AioTable, EVENT_SIZE, Event, HEADER_LEN, IOCB_FLAG_RESFD, IOCB_SIZE, Iocb, Pending, cmd, header};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::events::{POLLERR, POLLHUP, POLLNVAL};
use crate::fileio::{PollFd, WaitTime, deadline_after};
use crate::kernel::{Backing, Clock, LinuxKernel, MapHints, MapRequest, Placement, Prot, UserPtr};
use crate::ns::lock::Mutex;
use crate::sys::io::{How, bounce, load_iovecs, read_into_how, write_from_how};
use crate::sys::path::{O_PATH, fs};
use crate::sys::poll::{Waited, sigmask_arg, wait_with_signals};
use crate::sys::rwv::{RWF_APPEND, RWF_DSYNC, RWF_HIPRI, RWF_NOAPPEND, RWF_NOSIGNAL, RWF_NOWAIT, RWF_SYNC};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::IO_SETUP, io_setup),
    (nr::IO_DESTROY, io_destroy),
    (nr::IO_GETEVENTS, io_getevents),
    (nr::IO_SUBMIT, io_submit),
    (nr::IO_CANCEL, io_cancel),
    (nr::IO_PGETEVENTS, io_pgetevents),
];

/// Every live context. The lock is held for table updates only, never
/// across user memory access or I/O.
static CONTEXTS: Mutex<AioTable> = Mutex::new(AioTable::new());

/// `EINPROGRESS` (`asm-generic/errno.h`): "Operation now in progress".
const EINPROGRESS: Errno = match Errno::new(115) {
    Some(e) => e,
    None => Errno::EINVAL,
};

/// The `RWF_*` bits a read or write accepts (the others: `EOPNOTSUPP`, as
/// preadv2 answers them).
const RWF_OK: u32 = RWF_HIPRI | RWF_DSYNC | RWF_SYNC | RWF_NOWAIT | RWF_APPEND | RWF_NOAPPEND | RWF_NOSIGNAL;

/// io_setup(nr_events, ctx_idp): the word at `ctx_idp` is read first
/// (`EFAULT`) and must be 0, `nr_events` (an `unsigned int`) not 0 nor
/// absurd (`EINVAL`), the system-wide total within aio-max-nr (`EAGAIN`).
/// Maps the ring and stores its address; on failure the word stays.
pub fn io_setup(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let nr_events = a.uint(0);
    let idp = a.ptr(1);
    let mut word = [0; 8];
    k.copy_from_user(idp, &mut word)?;
    if u64::from_le_bytes(word) != 0 || nr_events == 0 {
        return Err(Errno::EINVAL);
    }
    let (slots, len) = CONTEXTS.lock().check_new(nr_events)?;
    let base = k.mmap(&MapRequest {
        addr: 0,
        len,
        prot: Prot(crate::abi::prot::PROT_READ | crate::abi::prot::PROT_WRITE),
        placement: Placement::Hint,
        shared: true,
        backing: Backing::Anonymous,
        hints: MapHints::default(),
    })?;
    let (space, owner) = (k.address_space_id(), k.pid());
    let index = {
        let mut t = CONTEXTS.lock();
        // Checked again: another thread may have taken the room meanwhile.
        match t.check_new(nr_events) {
            Ok(_) => t.insert(space, owner, base, len, slots, nr_events).index,
            Err(e) => {
                drop(t);
                let _ = k.munmap(base, len);
                return Err(e);
            }
        }
    };
    let stored = k
        .copy_to_user(UserPtr(base), &header(index, slots, 0, 0))
        .and_then(|()| k.copy_to_user(idp, &base.to_le_bytes()));
    if let Err(e) = stored {
        CONTEXTS.lock().remove(space, base);
        let _ = k.munmap(base, len);
        return Err(e);
    }
    Ok(0)
}

/// io_destroy(ctx_id): `EINVAL` for anything but a live context of the
/// caller's address space. Pending operations and unreaped events are
/// dropped and the ring is unmapped.
pub fn io_destroy(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let space = k.address_space_id();
    let ctx = CONTEXTS.lock().remove(space, a.ulong(0)).ok_or(Errno::EINVAL)?;
    let _ = k.munmap(ctx.base, ctx.len);
    Ok(0)
}

/// munmap(2) removed `[addr, addr + len)`: a context whose ring was there
/// is gone (io_destroy's spec sheet).
pub fn unmapped(k: &mut dyn LinuxKernel, addr: u64, len: u64) {
    let space = k.address_space_id();
    CONTEXTS.lock().unmapped(space, addr, len);
}

/// The calling process left address space `space` (execve(2)) or ended:
/// its contexts go.
pub fn left_address_space(k: &mut dyn LinuxKernel, space: u64) {
    let pid = k.pid();
    CONTEXTS.lock().left(space, pid);
}

/// The calling process is exiting.
pub fn process_exit(k: &mut dyn LinuxKernel) {
    let space = k.address_space_id();
    left_address_space(k, space);
}

/// The caller's address space, if `id` is one of its contexts (`EINVAL`).
fn lookup(k: &mut dyn LinuxKernel, id: u64) -> Result<u64, Errno> {
    let space = k.address_space_id();
    CONTEXTS.lock().get(space, id).map(|c| c.space).ok_or(Errno::EINVAL)
}

/// Post `event` into context `id`'s ring (and signal `resfd`). A context
/// destroyed meanwhile takes nothing.
fn post(k: &mut dyn LinuxKernel, space: u64, id: u64, event: Event, resfd: Option<i32>) {
    let claimed = CONTEXTS.lock().get_mut(space, id).map(|c| {
        let slot = c.claim_slot();
        (c.slot_addr(slot), c.tail)
    });
    let Some((at, tail)) = claimed else { return };
    let _ = k.copy_to_user(UserPtr(at), &event.to_bytes());
    let _ = k.copy_to_user(UserPtr(id + 12), &tail.to_le_bytes());
    if let Some(efd) = resfd {
        let _ = k.write(efd, &1u64.to_le_bytes());
    }
}

/// Complete the pending polls of context `id` whose events are ready.
fn poll_pending(k: &mut dyn LinuxKernel, space: u64, id: u64) -> Result<(), Errno> {
    let pending: Vec<Pending> = match CONTEXTS.lock().get(space, id) {
        Some(c) if !c.pending.is_empty() => c.pending.clone(),
        _ => return Ok(()),
    };
    let mut fds = poll_fds(&pending);
    file_io(k)?.poll(&mut fds, WaitTime::Poll, false)?;
    for (p, fd) in pending.iter().zip(&fds) {
        if let Some(res) = poll_result(p, fd.revents)
            && CONTEXTS.lock().take_pending(space, id, p.iocb).is_some()
        {
            post(k, space, id, Event { data: p.data, obj: p.iocb, res, res2: 0 }, p.resfd);
        }
    }
    Ok(())
}

/// The `struct pollfd`s that wait for `pending`.
fn poll_fds(pending: &[Pending]) -> Vec<PollFd> {
    pending.iter().map(|p| PollFd { fd: p.fd, events: p.mask, revents: 0 }).collect()
}

/// A pending poll's result for `revents`: the events asked for plus
/// `POLLERR` / `POLLHUP`; `None` while nothing is ready (or the descriptor
/// was closed, `POLLNVAL`: the poll waits on).
fn poll_result(p: &Pending, revents: u16) -> Option<i64> {
    let res = revents & (p.mask | POLLERR | POLLHUP);
    (revents & POLLNVAL == 0 && res != 0).then_some(i64::from(res))
}

/// io_submit(ctx_id, nr, iocbpp): `EINVAL` for a negative `nr` or an
/// unknown context; then the iocbs in order, stopping at the first that
/// cannot be queued (its error if it is the first, else the count so far).
pub fn io_submit(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, count, array) = (a.ulong(0), a.long(1), a.ptr(2));
    if count < 0 {
        return Err(Errno::EINVAL);
    }
    let space = lookup(k, id)?;
    let slots = CONTEXTS.lock().get(space, id).map_or(0, |c| c.nr);
    let count = (count as u64).min(u64::from(slots));
    poll_pending(k, space, id)?;
    let mut done = 0;
    while done < count {
        let result = array
            .offset(done * 8)
            .ok_or(Errno::EFAULT)
            .and_then(|at| {
                let mut ptr = [0; 8];
                k.copy_from_user(at, &mut ptr).map(|()| u64::from_le_bytes(ptr))
            })
            .and_then(|iocb| submit_one(k, space, id, iocb));
        match result {
            Ok(()) => done += 1,
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done)
}

/// Queue the iocb at `at` (and, for everything but a poll, do it).
fn submit_one(k: &mut dyn LinuxKernel, space: u64, id: u64, at: u64) -> Result<(), Errno> {
    let mut raw = [0; IOCB_SIZE];
    k.copy_from_user(UserPtr(at), &mut raw)?;
    let cb = Iocb::from_bytes(&raw);
    if cb.reserved2 != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = cb.fildes as i32;
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let status = fs(k)?.status_flags(fd)?;
    if status & O_PATH != 0 {
        return Err(Errno::EBADF);
    }
    let resfd = if cb.flags & IOCB_FLAG_RESFD != 0 {
        let efd = cb.resfd as i32;
        if efd < 0 {
            return Err(Errno::EBADF);
        }
        if !file_io(k)?.is_eventfd(efd)? {
            return Err(Errno::EINVAL);
        }
        Some(efd)
    } else {
        None
    };
    let event = |res: i64| Event { data: cb.data, obj: at, res, res2: 0 };
    match cb.opcode {
        cmd::PREAD | cmd::PWRITE | cmd::PREADV | cmd::PWRITEV => {
            let write = matches!(cb.opcode, cmd::PWRITE | cmd::PWRITEV);
            let mode_ok = match status & 3 {
                2 => true,
                0 => !write,
                1 => write,
                _ => false,
            };
            if !mode_ok {
                return Err(Errno::EBADF);
            }
            if cb.offset < 0 {
                return Err(Errno::EINVAL);
            }
            if cb.rw_flags & !RWF_OK != 0 {
                return Err(Errno::EOPNOTSUPP);
            }
            if k.fstat(fd)?.file_type() == S_IFDIR {
                return Err(Errno::EINVAL);
            }
            let (iovs, total) = if matches!(cb.opcode, cmd::PREADV | cmd::PWRITEV) {
                if cb.nbytes > u64::from(IOV_MAX) {
                    return Err(Errno::EINVAL);
                }
                load_iovecs(k, UserPtr(cb.buf), cb.nbytes as i32)?
            } else {
                let len = cb.nbytes.min(MAX_RW_COUNT);
                (alloc::vec![Iovec { base: cb.buf, len }], len)
            };
            room(space, id)?;
            let how = How { at: Some(cb.offset as u64), append: None, nowait: cb.rw_flags & RWF_NOWAIT != 0 };
            let res = match total {
                0 => Ok(0),
                _ if write => write_from_how(k, fd, &iovs, total, how),
                _ => read_into_how(k, fd, &iovs, total, how),
            };
            if write && res.is_ok_and(|n| n > 0) && cb.rw_flags & (RWF_DSYNC | RWF_SYNC) != 0 {
                let _ = fs(k)?.fsync(fd, cb.rw_flags & RWF_SYNC == 0);
            }
            post(k, space, id, event(result_value(res)), resfd);
        }
        cmd::FSYNC | cmd::FDSYNC => {
            if cb.buf != 0 || cb.nbytes != 0 || cb.offset != 0 || cb.rw_flags != 0 {
                return Err(Errno::EINVAL);
            }
            if matches!(k.fstat(fd)?.file_type(), S_IFIFO | S_IFSOCK) {
                return Err(Errno::EINVAL);
            }
            room(space, id)?;
            let res = fs(k)?.fsync(fd, cb.opcode == cmd::FDSYNC).map(|()| 0);
            post(k, space, id, event(result_value(res)), resfd);
        }
        cmd::POLL => {
            if cb.offset != 0 || cb.nbytes != 0 || cb.rw_flags != 0 || cb.buf > 0xffff {
                return Err(Errno::EINVAL);
            }
            room(space, id)?;
            let p = Pending { iocb: at, data: cb.data, fd, mask: cb.buf as u16, resfd };
            let mut fds = poll_fds(core::slice::from_ref(&p));
            file_io(k)?.poll(&mut fds, WaitTime::Poll, false)?;
            match poll_result(&p, fds[0].revents) {
                Some(res) => post(k, space, id, event(res), resfd),
                None => match CONTEXTS.lock().get_mut(space, id) {
                    Some(c) => c.pending.push(p),
                    None => return Err(Errno::EINVAL),
                },
            }
        }
        _ => return Err(Errno::EINVAL),
    }
    Ok(())
}

/// `EAGAIN` when context `id` holds as many completions as it can.
fn room(space: u64, id: u64) -> Result<(), Errno> {
    match CONTEXTS.lock().get(space, id) {
        Some(c) if c.has_room() => Ok(()),
        Some(_) => Err(Errno::EAGAIN),
        None => Err(Errno::EINVAL),
    }
}

/// An I/O result as an event's `res`: bytes or `-errno`.
fn result_value(r: Result<u64, Errno>) -> i64 {
    match r {
        Ok(n) => n as i64,
        Err(e) => -i64::from(e.get()),
    }
}

/// io_cancel(ctx_id, iocb, result): reads the iocb's `aio_key` first
/// (`EFAULT`; not 0: `EINVAL`), then the context and the iocb must be
/// pending there (`EINVAL`). A pending poll is cancelled: its event (res
/// 0) goes to the ring and the call says `EINPROGRESS`; `result` is never
/// written.
pub fn io_cancel(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, iocb) = (a.ulong(0), a.ulong(1));
    let mut key = [0; 4];
    k.copy_from_user(UserPtr(iocb).offset(8).ok_or(Errno::EFAULT)?, &mut key)?;
    if u32::from_le_bytes(key) != 0 {
        return Err(Errno::EINVAL);
    }
    let space = lookup(k, id)?;
    let p = CONTEXTS.lock().take_pending(space, id, iocb).ok_or(Errno::EINVAL)?;
    post(k, space, id, Event { data: p.data, obj: p.iocb, res: 0, res2: 0 }, p.resfd);
    Err(EINPROGRESS)
}

/// io_getevents(ctx_id, min_nr, nr, events, timeout)
pub fn io_getevents(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let wait = timeout_arg(k, a.ptr(4))?;
    getevents(k, a, wait, None)
}

/// io_pgetevents(ctx_id, min_nr, nr, events, timeout, usig): the timeout
/// (`EFAULT`), then `usig` (`EFAULT`), its mask's size (`EINVAL`) and the
/// mask (`EFAULT`), then io_getevents with the mask installed for the
/// wait, as ppoll(2) does.
pub fn io_pgetevents(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let wait = timeout_arg(k, a.ptr(4))?;
    let mask = if a.ptr(5).is_null() {
        None
    } else {
        let mut raw = [0; 16];
        k.copy_from_user(a.ptr(5), &mut raw)?;
        let ptr = UserPtr(crate::abi::le_u64(&raw[..8]));
        sigmask_arg(k, ptr, crate::abi::le_u64(&raw[8..]))?
    };
    getevents(k, a, wait, mask)
}

/// io_getevents' timeout: a relative `timespec` that is not validated
/// (io_getevents' spec sheet): a `tv_nsec` of a second or more adds to the
/// seconds, a negative `tv_sec` waits for ever, NULL waits for ever, zero
/// only checks.
fn timeout_arg(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<WaitTime, Errno> {
    if ptr.is_null() {
        return Ok(WaitTime::Forever);
    }
    let mut raw = [0; Timespec::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    let t = Timespec::from_bytes(&raw);
    match relative_ns(t.sec, t.nsec) {
        None => Ok(WaitTime::Forever),
        Some(0) => Ok(WaitTime::Poll),
        Some(ns) => {
            let now = k.clock(Clock::Monotonic)?;
            Ok(WaitTime::Until(deadline_after(now, Clock::Monotonic, Timespec::from_nanos(ns))))
        }
    }
}

/// A timeout in nanoseconds, `None` for "no limit" (a negative `tv_sec`).
fn relative_ns(sec: i64, nsec: i64) -> Option<u64> {
    if sec < 0 {
        return None;
    }
    Some((sec as u64).saturating_mul(1_000_000_000).saturating_add(nsec.max(0) as u64))
}

/// The common part of io_getevents and io_pgetevents.
fn getevents(k: &mut dyn LinuxKernel, a: &Args, wait: WaitTime, mask: Option<crate::kernel::SigSet>) -> SysResult {
    let (id, min_nr, max_nr, events) = (a.ulong(0), a.long(1), a.long(2), a.ptr(3));
    if min_nr < 0 || max_nr < 0 || min_nr > max_nr {
        return Err(Errno::EINVAL);
    }
    let space = lookup(k, id)?;
    loop {
        poll_pending(k, space, id)?;
        let (available, pending) = {
            let mut t = CONTEXTS.lock();
            let c = t.get_mut(space, id).ok_or(Errno::EINVAL)?;
            (c.available(), c.pending.clone())
        };
        let available = refresh_head(k, space, id).unwrap_or(available);
        if i64::from(available) >= min_nr || matches!(wait, WaitTime::Poll) {
            break;
        }
        let mut fds = poll_fds(&pending);
        match wait_with_signals(k, mask, wait, |io, w| io.poll(&mut fds, w, false), |&n| n == 0)? {
            Waited::Done(0) if timed_out(k, wait) => break,
            Waited::Done(_) => {}
            Waited::Interrupted => return Err(Errno::RESTART_IF_NO_HANDLER),
        }
    }
    reap(k, space, id, max_nr as u64, events)
}

/// Whether the deadline of `wait` has passed.
fn timed_out(k: &mut dyn LinuxKernel, wait: WaitTime) -> bool {
    match wait {
        WaitTime::Poll => true,
        WaitTime::Forever => false,
        WaitTime::Until(d) => k.clock(d.clock).is_ok_and(|now| crate::fileio::remaining(now, &d).is_zero()),
    }
}

/// Take the head user space may have advanced from the ring header;
/// returns the events available then.
fn refresh_head(k: &mut dyn LinuxKernel, space: u64, id: u64) -> Option<u32> {
    let mut raw = [0; 4];
    k.copy_from_user(UserPtr(id + 8), &mut raw).ok()?;
    let mut t = CONTEXTS.lock();
    let c = t.get_mut(space, id)?;
    c.adopt_head(u32::from_le_bytes(raw));
    Some(c.available())
}

/// Copy up to `max` events from the ring to `events`, oldest first, and
/// advance the head. Nothing is consumed if `events` cannot be written.
fn reap(k: &mut dyn LinuxKernel, space: u64, id: u64, max: u64, events: UserPtr) -> SysResult {
    let (head, n, slots) = {
        let t = CONTEXTS.lock();
        let c = t.get(space, id).ok_or(Errno::EINVAL)?;
        (c.head, u64::from(c.available()).min(max) as u32, c.nr)
    };
    if n == 0 {
        return Ok(0);
    }
    let mut buf = bounce(n as usize * EVENT_SIZE as usize)?;
    for (i, chunk) in buf.chunks_exact_mut(EVENT_SIZE as usize).enumerate() {
        let slot = (head + i as u32) % slots;
        k.copy_from_user(UserPtr(id + HEADER_LEN + u64::from(slot) * EVENT_SIZE), chunk)?;
    }
    k.copy_to_user(events, &buf)?;
    let new_head = {
        let mut t = CONTEXTS.lock();
        let c = t.get_mut(space, id).ok_or(Errno::EINVAL)?;
        if c.head == head {
            c.consume(n);
        }
        c.head
    };
    let _ = k.copy_to_user(UserPtr(id + 8), &new_head.to_le_bytes());
    Ok(u64::from(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(relative_ns(-1, 0), None);
        assert_eq!(relative_ns(0, 0), Some(0));
        assert_eq!(relative_ns(0, 1_000_000_000), Some(1_000_000_000), "not normalised, just added");
        assert_eq!(relative_ns(2, 5), Some(2_000_000_005));
        assert_eq!(relative_ns(i64::MAX, 0), Some(u64::MAX));
    }

    #[test]
    fn poll_results() {
        let p = Pending { iocb: 1, data: 2, fd: 3, mask: 0x1, resfd: None };
        assert_eq!(poll_result(&p, 0), None);
        assert_eq!(poll_result(&p, 0x1), Some(0x1));
        assert_eq!(poll_result(&p, 0x10), Some(0x10), "POLLHUP always");
        assert_eq!(poll_result(&p, 0x4), None, "not asked for");
        assert_eq!(poll_result(&p, POLLNVAL), None);
    }
}
