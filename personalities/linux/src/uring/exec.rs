//! How requests run: submission (link chains), the per-opcode work, the
//! retries of waiting requests (in io_uring_enter(2) and in the background,
//! [`background`]) and the `GETEVENTS` wait.
//!
//! Most requests are the system call of the same name: each runs that
//! call's handler from [`crate::table`] with arguments taken from the SQE,
//! so the checks and errors are the call's, reported in the CQE's `res`.
//! A request on a descriptor that is not ready (an empty pipe, a socket
//! without data or without a client) waits instead of blocking; regular
//! files are always ready.

use super::abi::{self, Cqe, Sqe, cqe_flags, err, op, res, sqe_flags, timeout};
use super::ring::{Instance, Pending, Wait};
use super::{FileHandle, UringKernel};
use crate::abi::Timespec;
use crate::args::Args;
use crate::errno::Errno;
use crate::fileio::{PollFd, WaitTime, events};
use crate::kernel::{Clock, Deadline, LinuxKernel, SigSet};
use crate::ns::lock::Mutex;
use crate::sys::poll::{Waited, wait_with_signals};
use crate::table::nr;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// `MSG_DONTWAIT` (`linux/socket.h`).
const MSG_DONTWAIT: u32 = 0x40;
/// `O_NONBLOCK` (`asm-generic/fcntl.h`).
const O_NONBLOCK: u32 = 0o4000;
/// How often a wait looks again at what it cannot sleep on (requests on
/// registered files or of other processes, CQEs other threads post): 10 ms.
const RECHECK_NS: u64 = 10_000_000;
/// `POLLPRI`: what a `GETEVENTS` wait asks of the ring's own descriptor,
/// which reports it while the instance is kicked (the kernel's ring file).
const KICK_EVENT: u16 = 0x002;

pub(crate) fn uring(k: &mut dyn LinuxKernel) -> Result<&mut dyn UringKernel, Errno> {
    k.uring().ok_or(Errno::ENOSYS)
}

/// What starting (or retrying) a request gave.
enum Step {
    Done(Result<u64, Errno>),
    /// A CQE with `IORING_CQE_F_MORE` now, and the request stays.
    More(i32, Wait),
    Wait(Wait),
}

/// Submit up to `max` SQEs from the SQ ring; returns how many were
/// consumed (dropped SQ array entries not counted).
pub fn submit(k: &mut dyn LinuxKernel, inst: &Arc<Instance>, max: u32) -> u32 {
    let sqes = inst.take_sqes(max);
    let n = sqes.len() as u32;
    let mut chain: VecDeque<Sqe> = VecDeque::new();
    for sqe in sqes {
        let links = sqe.links();
        chain.push_back(sqe);
        if !links {
            run_chain(k, inst, core::mem::take(&mut chain));
        }
    }
    if !chain.is_empty() {
        run_chain(k, inst, chain);
    }
    n
}

/// Run a link chain from its head until one request waits (the rest wait
/// with it) or the chain ends.
fn run_chain(k: &mut dyn LinuxKernel, inst: &Arc<Instance>, mut chain: VecDeque<Sqe>) {
    while let Some(sqe) = chain.pop_front() {
        match start(k, inst, &sqe) {
            Step::Done(r) => {
                let res = res(r);
                complete(inst, &sqe, res, 0);
                if breaks(&sqe, res) {
                    cancel_chain(inst, chain);
                    return;
                }
            }
            Step::More(res, wait) => {
                complete(inst, &sqe, res, cqe_flags::MORE);
                park(k, inst, sqe, wait, chain.into());
                return;
            }
            Step::Wait(wait) => {
                park(k, inst, sqe, wait, chain.into());
                return;
            }
        }
    }
}

/// Put a request that waits in `inst`'s pending list, owned by the calling
/// process, with a watch on its file (not for registered files, which are
/// opened again at each attempt), and note the instance for
/// [`background`].
fn park(k: &mut dyn LinuxKernel, inst: &Arc<Instance>, sqe: Sqe, wait: Wait, chain: Vec<Sqe>) {
    let owner = k.pid();
    let mut p = Pending::new(sqe, wait, chain, owner);
    let fixed = sqe.flags & sqe_flags::FIXED_FILE != 0;
    if let (false, Wait::Ready { events } | Wait::Poll { events, .. }) = (fixed, wait) {
        p.watch = k.uring().and_then(|u| u.watch(sqe.fd, events, inst));
    }
    inst.add_pending(p);
    note_parked(inst, owner);
}

/// Instances with requests waiting, and the process each request of the
/// entry belongs to, for [`background`]; dead or idle entries are dropped
/// there.
static PARKED: Mutex<Vec<(i32, Weak<Instance>)>> = Mutex::new(Vec::new());
/// Entries in [`PARKED`]: a lock-free answer to "is anything waiting".
static PARKED_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Whether some request of some instance may be waiting: the kernel calls
/// [`background`] on a thread's way to user mode only then.
pub fn parked_anywhere() -> bool {
    PARKED_COUNT.load(Ordering::Relaxed) > 0
}

pub(crate) fn note_parked(inst: &Arc<Instance>, owner: i32) {
    let mut list = PARKED.lock();
    if !list.iter().any(|(o, w)| *o == owner && core::ptr::eq(w.as_ptr(), Arc::as_ptr(inst))) {
        list.push((owner, Arc::downgrade(inst)));
    }
    PARKED_COUNT.store(list.len(), Ordering::Relaxed);
}

/// The instances with requests of process `owner` waiting; forgets
/// entries whose instance is gone or has none of its owner's requests.
pub(crate) fn parked_of(owner: i32) -> Vec<Arc<Instance>> {
    let mut mine = Vec::new();
    // References taken under the lock are dropped after it: the last one
    // may end an instance, whose watches then leave their wait queues.
    let mut others = Vec::new();
    {
        let mut list = PARKED.lock();
        list.retain(|(o, w)| match w.upgrade() {
            Some(i) if i.has_pending_of(*o) => {
                if *o == owner {
                    mine.push(i);
                } else {
                    others.push(i);
                }
                true
            }
            Some(i) => {
                others.push(i);
                false
            }
            None => false,
        });
        PARKED_COUNT.store(list.len(), Ordering::Relaxed);
    }
    drop(others);
    mine
}

/// On a thread's way back to user mode (after a system call, or an
/// interrupt while requests wait): try the waiting requests of its process
/// again, so one whose file became ready (data arrived, a client
/// connected, a poll fired) completes without another io_uring_enter(2),
/// as Linux completes such requests asynchronously. Kept-back CQEs are left
/// for the next io_uring_enter(2), which moves them into the ring.
pub fn background(k: &mut dyn LinuxKernel) {
    if !parked_anywhere() || k.uring().is_none() {
        return;
    }
    let owner = k.pid();
    for inst in parked_of(owner) {
        retry(k, &inst, Some(owner));
        signal(k, &inst);
    }
}

/// Whether a request that ended with `res` breaks its chain: a failure
/// (negative, or a short read or write) unless the link is a hard link.
pub fn breaks(sqe: &Sqe, res: i32) -> bool {
    if sqe.flags & sqe_flags::IO_HARDLINK != 0 {
        return false;
    }
    if res < 0 {
        return true;
    }
    matches!(sqe.opcode, op::READ | op::WRITE | op::READ_FIXED | op::WRITE_FIXED) && (res as u32) < sqe.len
}

/// Post the CQE of `sqe` (none for a success with `IOSQE_CQE_SKIP_SUCCESS`).
fn complete(inst: &Instance, sqe: &Sqe, res: i32, flags: u32) {
    if res >= 0 && flags & cqe_flags::MORE == 0 && sqe.flags & sqe_flags::CQE_SKIP_SUCCESS != 0 {
        return;
    }
    inst.post(Cqe { user_data: sqe.user_data, res, flags });
}

/// Complete every SQE of a broken chain with `-ECANCELED`.
fn cancel_chain(inst: &Instance, chain: impl IntoIterator<Item = Sqe>) {
    for sqe in chain {
        inst.post(Cqe { user_data: sqe.user_data, res: -i32::from(err::ECANCELED.get()), flags: 0 });
    }
}

/// Complete a pending request that was removed (cancelled) and its chain.
fn cancel_pending(inst: &Instance, p: Pending) {
    inst.post(Cqe { user_data: p.sqe.user_data, res: -i32::from(err::ECANCELED.get()), flags: 0 });
    cancel_chain(inst, p.chain);
}

fn call(k: &mut dyn LinuxKernel, number: u64, a: [u64; 6]) -> Result<u64, Errno> {
    match crate::table::handler(number) {
        Some(h) => h(k, &Args(a)),
        None => Err(Errno::EINVAL),
    }
}

/// A descriptor as a system call argument (`int`, sign-extended).
fn fd_arg(fd: i32) -> u64 {
    i64::from(fd) as u64
}

/// Run `f` with the request's descriptor: `sqe.fd`, or with
/// `IOSQE_FIXED_FILE` the registered file in slot `sqe.fd`, opened on a
/// temporary descriptor for the call (`EBADF` for no table, an empty slot
/// or one past its end).
fn with_fd<R>(
    k: &mut dyn LinuxKernel,
    inst: &Instance,
    sqe: &Sqe,
    f: impl FnOnce(&mut dyn LinuxKernel, i32) -> R,
) -> Result<R, Errno> {
    if sqe.flags & sqe_flags::FIXED_FILE == 0 {
        return Ok(f(k, sqe.fd));
    }
    let handle: FileHandle = usize::try_from(sqe.fd).ok().and_then(|i| inst.file(i)).ok_or(Errno::EBADF)?;
    let fd = uring(k)?.install_handle(&handle)?;
    let r = f(k, fd);
    let _ = k.close(fd);
    Ok(r)
}

/// The poll(2) events of `fd` now (`POLLNVAL` if it is not open).
fn poll_now(k: &mut dyn LinuxKernel, fd: i32, ask: u16) -> u16 {
    let mut fds = [PollFd { fd, events: ask, revents: 0 }];
    match k.file_io() {
        Some(io) => match io.poll(&mut fds, WaitTime::Poll, false) {
            Ok(_) => fds[0].revents,
            Err(_) => events::POLLNVAL,
        },
        None => events::POLLNVAL,
    }
}

/// Whether a request on `fd` may run now: the descriptor is ready for
/// `ask` (or broken: the call reports why), or non-blocking (the call
/// then says `EAGAIN` itself).
fn ready(k: &mut dyn LinuxKernel, fd: i32, ask: u16) -> bool {
    if poll_now(k, fd, ask) != 0 {
        return true;
    }
    match k.fs() {
        Some(fs) => fs.status_flags(fd).is_ok_and(|f| f & O_NONBLOCK != 0),
        None => true,
    }
}

/// The file type bits of `fd`, `None` if it is not open.
fn file_type(k: &mut dyn LinuxKernel, fd: i32) -> Option<u32> {
    k.fstat(fd).ok().map(|st| st.mode & crate::abi::mode::S_IFMT)
}

/// Whether `fd` reads as a stream (no file position: pipes, sockets,
/// character devices, event files), where a request's offset is ignored.
fn is_stream(k: &mut dyn LinuxKernel, fd: i32) -> bool {
    use crate::abi::mode::{S_IFBLK, S_IFDIR, S_IFREG};
    file_type(k, fd).is_some_and(|t| t != S_IFREG && t != S_IFDIR && t != S_IFBLK)
}

/// A read or write request (`READ`, `WRITE`, their `_FIXED` and vectored
/// forms) on `fd`.
fn read_write(k: &mut dyn LinuxKernel, sqe: &Sqe, fd: i32, write: bool, vectored: bool) -> Step {
    let stream = is_stream(k, fd);
    let ask = if write { events::POLLOUT } else { events::POLLIN };
    if stream && !ready(k, fd, ask) {
        return Step::Wait(Wait::Ready { events: ask });
    }
    let (plain, positioned) = match (write, vectored) {
        (false, false) => (nr::READ, nr::PREAD64),
        (true, false) => (nr::WRITE, nr::PWRITE64),
        (false, true) => (nr::READV, nr::PREADV),
        (true, true) => (nr::WRITEV, nr::PWRITEV),
    };
    let a = [fd_arg(fd), sqe.addr, u64::from(sqe.len), sqe.off, 0, 0];
    // IORING_FEAT_RW_CUR_POS: an offset of -1 uses (and moves) the file
    // position; streams have none.
    Step::Done(if stream || sqe.off == u64::MAX { call(k, plain, a) } else { call(k, positioned, a) })
}

fn flatten(r: Result<Step, Errno>) -> Step {
    r.unwrap_or_else(|e| Step::Done(Err(e)))
}

/// Start (or retry) request `sqe`.
fn start(k: &mut dyn LinuxKernel, inst: &Instance, sqe: &Sqe) -> Step {
    if sqe.flags & !sqe_flags::KNOWN != 0 || sqe.flags & sqe_flags::BUFFER_SELECT != 0 {
        return Step::Done(Err(Errno::EINVAL));
    }
    let s = *sqe;
    let fdv = fd_arg(s.fd);
    match s.opcode {
        op::NOP => Step::Done(Ok(0)),
        op::READ | op::WRITE => {
            flatten(with_fd(k, inst, sqe, |k, fd| read_write(k, &s, fd, s.opcode == op::WRITE, false)))
        }
        op::READV | op::WRITEV => {
            flatten(with_fd(k, inst, sqe, |k, fd| read_write(k, &s, fd, s.opcode == op::WRITEV, true)))
        }
        op::READ_FIXED | op::WRITE_FIXED => {
            if !inst.fixed_buffer_ok(s.buf_index, s.addr, s.len) {
                return Step::Done(Err(Errno::EFAULT));
            }
            flatten(with_fd(k, inst, sqe, |k, fd| read_write(k, &s, fd, s.opcode == op::WRITE_FIXED, false)))
        }
        op::FSYNC => {
            if s.op_flags & !abi::FSYNC_DATASYNC != 0 {
                return Step::Done(Err(Errno::EINVAL));
            }
            let number = if s.op_flags & abi::FSYNC_DATASYNC != 0 { nr::FDATASYNC } else { nr::FSYNC };
            flatten(with_fd(k, inst, sqe, |k, fd| Step::Done(call(k, number, [fd_arg(fd), 0, 0, 0, 0, 0]))))
        }
        op::POLL_ADD => {
            let ask = s.op_flags as u16;
            let multishot = s.len & abi::POLL_ADD_MULTI != 0;
            flatten(with_fd(k, inst, sqe, |k, fd| poll_step(k, fd, ask, multishot, true)))
        }
        op::POLL_REMOVE => Step::Done(match inst.remove_pending(s.addr, |p| matches!(p.wait, Wait::Poll { .. })) {
            Some(p) => {
                cancel_pending(inst, p);
                Ok(0)
            }
            None => Err(Errno::ENOENT),
        }),
        op::TIMEOUT => match timeout_wait(k, inst, &s) {
            Ok(w) => Step::Wait(w),
            Err(e) => Step::Done(Err(e)),
        },
        op::TIMEOUT_REMOVE => Step::Done(timeout_remove(k, inst, &s)),
        op::ASYNC_CANCEL => Step::Done(match inst.remove_pending(s.addr, |_| true) {
            Some(p) => {
                cancel_pending(inst, p);
                Ok(0)
            }
            None => Err(Errno::ENOENT),
        }),
        op::ACCEPT => flatten(with_fd(k, inst, sqe, |k, fd| {
            if !ready(k, fd, events::POLLIN) {
                return Step::Wait(Wait::Ready { events: events::POLLIN });
            }
            Step::Done(call(k, nr::ACCEPT4, [fd_arg(fd), s.addr, s.off, u64::from(s.op_flags), 0, 0]))
        })),
        op::CONNECT => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::CONNECT, [fd_arg(fd), s.addr, s.off, 0, 0, 0]))
        })),
        op::OPENAT => Step::Done(call(k, nr::OPENAT, [fdv, s.addr, u64::from(s.op_flags), u64::from(s.len), 0, 0])),
        op::OPENAT2 => Step::Done(call(k, nr::OPENAT2, [fdv, s.addr, s.off, u64::from(s.len), 0, 0])),
        op::CLOSE => Step::Done(call(k, nr::CLOSE, [fdv, 0, 0, 0, 0, 0])),
        op::STATX => Step::Done(call(k, nr::STATX, [fdv, s.addr, u64::from(s.op_flags), u64::from(s.len), s.off, 0])),
        op::SEND => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::SENDTO, [fd_arg(fd), s.addr, u64::from(s.len), u64::from(s.op_flags), 0, 0]))
        })),
        op::RECV | op::RECVMSG => flatten(with_fd(k, inst, sqe, |k, fd| {
            if file_type(k, fd).is_some_and(|t| t != crate::abi::mode::S_IFSOCK) {
                return Step::Done(Err(err::ENOTSOCK));
            }
            if s.op_flags & MSG_DONTWAIT == 0 && !ready(k, fd, events::POLLIN) {
                return Step::Wait(Wait::Ready { events: events::POLLIN });
            }
            Step::Done(if s.opcode == op::RECV {
                call(k, nr::RECVFROM, [fd_arg(fd), s.addr, u64::from(s.len), u64::from(s.op_flags), 0, 0])
            } else {
                call(k, nr::RECVMSG, [fd_arg(fd), s.addr, u64::from(s.op_flags), 0, 0, 0])
            })
        })),
        op::SENDMSG => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::SENDMSG, [fd_arg(fd), s.addr, u64::from(s.op_flags), 0, 0, 0]))
        })),
        op::FALLOCATE => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::FALLOCATE, [fd_arg(fd), u64::from(s.len), s.off, s.addr, 0, 0]))
        })),
        op::FADVISE => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::FADVISE64, [fd_arg(fd), s.off, u64::from(s.len), u64::from(s.op_flags), 0, 0]))
        })),
        op::MADVISE => Step::Done(call(k, nr::MADVISE, [s.addr, u64::from(s.len), u64::from(s.op_flags), 0, 0, 0])),
        op::SHUTDOWN => flatten(with_fd(k, inst, sqe, |k, fd| {
            Step::Done(call(k, nr::SHUTDOWN, [fd_arg(fd), u64::from(s.len), 0, 0, 0, 0]))
        })),
        op::RENAMEAT => {
            Step::Done(call(k, nr::RENAMEAT2, [fdv, s.addr, fd_arg(s.len as i32), s.off, u64::from(s.op_flags), 0]))
        }
        op::UNLINKAT => Step::Done(call(k, nr::UNLINKAT, [fdv, s.addr, u64::from(s.op_flags), 0, 0, 0])),
        op::MKDIRAT => Step::Done(call(k, nr::MKDIRAT, [fdv, s.addr, u64::from(s.len), 0, 0, 0])),
        op::SYMLINKAT => Step::Done(call(k, nr::SYMLINKAT, [s.addr, fdv, s.off, 0, 0, 0])),
        op::LINKAT => {
            Step::Done(call(k, nr::LINKAT, [fdv, s.addr, fd_arg(s.len as i32), s.off, u64::from(s.op_flags), 0]))
        }
        op::SOCKET => Step::Done(call(k, nr::SOCKET, [fdv, s.off, u64::from(s.len), 0, 0, 0])),
        _ => Step::Done(Err(Errno::EINVAL)),
    }
}

/// A poll request on `fd` for `ask`: the ready events masked by the
/// request (plus `POLLERR` / `POLLHUP`), or a wait. `armed`: whether a
/// multishot request may fire now.
fn poll_step(k: &mut dyn LinuxKernel, fd: i32, ask: u16, multishot: bool, armed: bool) -> Step {
    let revents = poll_now(k, fd, ask);
    if revents & events::POLLNVAL != 0 {
        return Step::Done(Err(Errno::EBADF));
    }
    if revents == 0 {
        return Step::Wait(Wait::Poll { events: ask, multishot, armed: true, seen: 0 });
    }
    if !multishot {
        return Step::Done(Ok(u64::from(revents)));
    }
    if !armed {
        return Step::Wait(Wait::Poll { events: ask, multishot, armed: false, seen: 0 });
    }
    Step::More(i32::from(revents), Wait::Poll { events: ask, multishot, armed: false, seen: 0 })
}

fn now(k: &mut dyn LinuxKernel) -> Timespec {
    k.clock(Clock::Monotonic).unwrap_or_default()
}

fn after(t: Timespec, by: Timespec) -> Timespec {
    t.saturating_add(by)
}

fn ts_ns(t: Timespec) -> i128 {
    i128::from(t.sec) * 1_000_000_000 + i128::from(t.nsec)
}

fn read_timespec(k: &mut dyn LinuxKernel, addr: u64) -> Result<Timespec, Errno> {
    let mut raw = [0; Timespec::SIZE];
    k.copy_from_user(crate::kernel::UserPtr(addr), &mut raw)?;
    let t = Timespec::from_bytes(&raw);
    if !t.is_valid() {
        return Err(Errno::EINVAL);
    }
    Ok(t)
}

/// When a timeout given as `t` with `flags` is due, on the monotonic clock.
fn due(k: &mut dyn LinuxKernel, t: Timespec, flags: u32) -> Timespec {
    let mono = now(k);
    if flags & timeout::ABS == 0 {
        return after(mono, t);
    }
    if flags & timeout::REALTIME != 0 {
        let real = k.clock(Clock::Realtime).unwrap_or_default();
        let left = ts_ns(t) - ts_ns(real);
        if left <= 0 {
            return mono;
        }
        return after(mono, Timespec::from_nanos(left.min(i128::from(u64::MAX)) as u64));
    }
    t
}

/// `IORING_OP_TIMEOUT`: `len` must be 1, `addr` a valid timespec; `off`
/// completions end it early.
fn timeout_wait(k: &mut dyn LinuxKernel, inst: &Instance, s: &Sqe) -> Result<Wait, Errno> {
    if s.len != 1 || s.op_flags & !timeout::KNOWN != 0 {
        return Err(Errno::EINVAL);
    }
    let t = read_timespec(k, s.addr)?;
    let at = due(k, t, s.op_flags);
    let target = if s.off > 0 { Some(inst.posted().saturating_add(s.off)) } else { None };
    Ok(Wait::Timeout { deadline: Deadline { clock: Clock::Monotonic, at }, target })
}

/// `IORING_OP_TIMEOUT_REMOVE`: cancel the timeout with user data `addr`,
/// or with `IORING_TIMEOUT_UPDATE` give it the time at `off`.
fn timeout_remove(k: &mut dyn LinuxKernel, inst: &Instance, s: &Sqe) -> Result<u64, Errno> {
    let is_timeout = |p: &Pending| matches!(p.wait, Wait::Timeout { .. });
    if s.op_flags & timeout::UPDATE != 0 {
        let t = read_timespec(k, s.off)?;
        let at = due(k, t, s.op_flags);
        let found = inst.update_pending(s.addr, is_timeout, |p| {
            if let Wait::Timeout { deadline, .. } = &mut p.wait {
                deadline.at = at;
            }
        });
        return if found { Ok(0) } else { Err(Errno::ENOENT) };
    }
    if s.op_flags != 0 {
        return Err(Errno::EINVAL);
    }
    match inst.remove_pending(s.addr, is_timeout) {
        Some(p) => {
            cancel_pending(inst, p);
            Ok(0)
        }
        None => Err(Errno::ENOENT),
    }
}

/// Try every waiting request again: run those whose descriptor is ready,
/// end timeouts that are due or counted out, fire polls. Repeats while
/// something completed (a completion may count a timeout out).
pub fn progress(k: &mut dyn LinuxKernel, inst: &Arc<Instance>) {
    retry(k, inst, None);
    inst.flush_overflow();
    signal(k, inst);
}

/// [`progress`] without moving kept-back CQEs or signalling, for the
/// requests of process `only` (all with `None`).
fn retry(k: &mut dyn LinuxKernel, inst: &Arc<Instance>, only: Option<i32>) {
    inst.take_kick();
    for _ in 0..16 {
        let mut pending = inst.take_pending();
        let mut kept = Vec::new();
        if let Some(owner) = only {
            let (mine, others): (Vec<Pending>, Vec<Pending>) = pending.into_iter().partition(|p| p.owner == owner);
            pending = mine;
            kept = others;
        }
        if pending.is_empty() {
            inst.restore_pending(kept);
            break;
        }
        let mut changed = false;
        for p in pending {
            match p.wait {
                Wait::Timeout { deadline, target } => {
                    let r = if target.is_some_and(|t| inst.posted() >= t) {
                        Some(0)
                    } else if ts_ns(now(k)) >= ts_ns(deadline.at) {
                        Some(-i32::from(err::ETIME.get()))
                    } else {
                        None
                    };
                    match r {
                        Some(res) => {
                            changed = true;
                            finish(k, inst, p, res);
                        }
                        None => kept.push(p),
                    }
                }
                Wait::Ready { events: ask } => {
                    let go = with_fd(k, inst, &p.sqe, |k, fd| ready(k, fd, ask)).unwrap_or(true);
                    if go {
                        changed = true;
                        let mut chain: VecDeque<Sqe> = p.chain.into();
                        chain.push_front(p.sqe);
                        run_chain(k, inst, chain);
                    } else {
                        kept.push(p);
                    }
                }
                Wait::Poll { events: ask, multishot, armed, seen } => {
                    // With a watch, a fired multishot poll fires again at
                    // the file's next wake-up (the count is read before
                    // the file is, so a wake-up meanwhile is not lost).
                    let (armed, count) = match &p.watch {
                        Some(w) => {
                            let c = w.count();
                            (armed || c != seen, c)
                        }
                        None => (armed, seen),
                    };
                    let step = flatten(with_fd(k, inst, &p.sqe, |k, fd| poll_step(k, fd, ask, multishot, armed)));
                    let seen_now = |w: Wait| match w {
                        Wait::Poll { events, multishot, armed, .. } => {
                            Wait::Poll { events, multishot, armed, seen: count }
                        }
                        other => other,
                    };
                    match step {
                        Step::Done(r) => {
                            changed = true;
                            finish(k, inst, p, res(r));
                        }
                        Step::More(res, wait) => {
                            changed = true;
                            complete(inst, &p.sqe, res, cqe_flags::MORE);
                            kept.push(Pending { wait: seen_now(wait), ..p });
                        }
                        Step::Wait(wait) => kept.push(Pending { wait: seen_now(wait), ..p }),
                    }
                }
            }
        }
        inst.restore_pending(kept);
        if !changed {
            break;
        }
    }
}

/// A waiting request ended with `res`: post it and go on with its chain.
fn finish(k: &mut dyn LinuxKernel, inst: &Arc<Instance>, p: Pending, res: i32) {
    complete(inst, &p.sqe, res, 0);
    if breaks(&p.sqe, res) {
        cancel_chain(inst, p.chain);
    } else if !p.chain.is_empty() {
        run_chain(k, inst, p.chain.into());
    }
}

/// Signal the registered eventfd if CQEs were posted since last time.
pub fn signal(k: &mut dyn LinuxKernel, inst: &Instance) {
    if let Some(h) = inst.take_signal()
        && let Some(u) = k.uring()
    {
        u.eventfd_signal(&h);
    }
}

/// Wait until the CQ ring holds `min` CQEs: retry waiting requests, and
/// sleep in a poll(2) of their descriptors until one is ready, a timeout
/// (theirs or `until`) is due, or a signal comes (with `mask` installed
/// for the wait, as ppoll(2) does). `ETIME` when `until` passed first;
/// an interruption is `RESTART_IF_NO_HANDLER` (`EINTR` after a handler).
pub fn wait(
    k: &mut dyn LinuxKernel,
    inst: &Arc<Instance>,
    ring_fd: i32,
    min: u32,
    until: Option<Timespec>,
    mask: Option<SigSet>,
) -> Result<(), Errno> {
    loop {
        progress(k, inst);
        if inst.cq_ready() >= min {
            return Ok(());
        }
        let t = now(k);
        if until.is_some_and(|u| ts_ns(t) >= ts_ns(u)) {
            return Err(err::ETIME);
        }
        let mut targets = inst.wait_targets(k.pid());
        // CQEs other threads post wake nothing here: look again now and
        // then.
        targets.recheck = true;
        let mut fds: Vec<PollFd> =
            targets.fds.iter().map(|&(fd, ask)| PollFd { fd, events: ask, revents: 0 }).collect();
        // The ring's own descriptor reports a kick: a watched file woke.
        fds.push(PollFd { fd: ring_fd, events: KICK_EVENT, revents: 0 });
        let mut wake = until;
        let mut earliest = |x: Timespec| {
            if wake.is_none_or(|w| ts_ns(x) < ts_ns(w)) {
                wake = Some(x);
            }
        };
        if let Some(d) = targets.timeout {
            earliest(d);
        }
        if targets.recheck {
            earliest(after(t, Timespec::from_nanos(RECHECK_NS)));
        }
        let wait = match wake {
            Some(at) => WaitTime::Until(Deadline { clock: Clock::Monotonic, at }),
            None => WaitTime::Forever,
        };
        match wait_with_signals(k, mask, wait, |io, w| io.poll(&mut fds, w, false), |&n| n == 0)? {
            Waited::Done(_) => {}
            Waited::Interrupted => return Err(Errno::RESTART_IF_NO_HANDLER),
        }
    }
}
