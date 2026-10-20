//! Pipes, poll / select, epoll and the event files (ADR 0042) against the
//! fake kernel: argument rules, timeouts, signal masks, select's sets,
//! copying results, interrupted waits and SIGPIPE. The semantics of the
//! files themselves are tested in `poly-vfs`.

extern crate std;

use super::fake::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo, Timespec, le_u64};
use crate::errno::Errno;
use crate::fileio::events::*;
use crate::fileio::{EpollEvent, EpollOp, FileIo, ItimerSpec, NewFd, PollFd, WaitTime};
use crate::kernel::{Clock, Deadline, SigSet, Wakeup};
use crate::signal::ExitToUser;
use crate::sys::signalfd::siginfo_bytes;
use crate::{LinuxKernel, nr};
use std::collections::BTreeMap;
use std::vec;
use std::vec::Vec;

/// What the fake's [`FileIo`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Made {
    Pipe { fds: [i32; 2], new: NewFd, direct: bool },
    Epoll { fd: i32, cloexec: bool },
    EventFd { fd: i32, initval: u32, semaphore: bool, new: NewFd },
    TimerFd { fd: i32, clock: Clock, new: NewFd },
    SignalFd { fd: i32, mask: SigSet, new: NewFd },
}

/// The fake kernel's pipes, waits and event files: scripted readiness and
/// records of every call.
#[derive(Debug)]
pub struct FakeIo {
    pub limit: usize,
    /// The events that are true, per descriptor (none if absent).
    pub ready: BTreeMap<i32, u16>,
    /// Every poll: the entries as asked, the wait, `exact`.
    pub polls: Vec<(Vec<PollFd>, WaitTime, bool)>,
    pub made: Vec<Made>,
    /// Epoll sets: the interest list and the events a wait returns.
    pub epolls: BTreeMap<i32, (BTreeMap<i32, EpollEvent>, Vec<EpollEvent>)>,
    pub epoll_waits: Vec<(i32, usize, WaitTime)>,
    pub timers: BTreeMap<i32, (bool, ItimerSpec)>,
    pub readable: BTreeMap<i32, usize>,
    /// signal_queued calls.
    pub queued: Vec<i32>,
}

impl Default for FakeIo {
    fn default() -> Self {
        FakeIo {
            limit: 1024,
            ready: BTreeMap::new(),
            polls: Vec::new(),
            made: Vec::new(),
            epolls: BTreeMap::new(),
            epoll_waits: Vec::new(),
            timers: BTreeMap::new(),
            readable: BTreeMap::new(),
            queued: Vec::new(),
        }
    }
}

impl FakeKernel {
    fn new_fd(&mut self, kind: Kind) -> i32 {
        let fd = (0..).find(|fd| !self.files.contains_key(fd)).unwrap();
        self.files.insert(fd, File::new(kind));
        fd
    }

    fn deadline_of(wait: WaitTime) -> Option<Deadline> {
        match wait {
            WaitTime::Until(d) => Some(d),
            _ => None,
        }
    }

    /// Block as a real wait would; a wait nothing could ever end (no
    /// steps left, no deadline) counts as interrupted, so tests of calls
    /// that block forever finish.
    fn wait_once(&mut self, wait: WaitTime) -> Result<bool, Errno> {
        if wait == WaitTime::Poll {
            return Ok(false);
        }
        if wait == WaitTime::Forever && self.steps.is_empty() && !self.notified.contains(&self.tid) {
            return Err(Errno::EINTR);
        }
        match self.block(Self::deadline_of(wait)) {
            Wakeup::Signal => Err(Errno::EINTR),
            Wakeup::TimedOut => Ok(false),
            Wakeup::Woken => Ok(true),
        }
    }
}

impl FileIo for FakeKernel {
    fn open_files_limit(&self) -> usize {
        self.io.limit
    }

    fn pipe(&mut self, new: NewFd, direct: bool) -> Result<[i32; 2], Errno> {
        let fds = [self.new_fd(Kind::Pipe), self.new_fd(Kind::Pipe)];
        self.io.made.push(Made::Pipe { fds, new, direct });
        Ok(fds)
    }

    fn poll(&mut self, fds: &mut [PollFd], wait: WaitTime, exact: bool) -> Result<usize, Errno> {
        self.io.polls.push((fds.to_vec(), wait, exact));
        loop {
            let mut n = 0;
            for f in fds.iter_mut() {
                f.revents = if f.fd < 0 {
                    0
                } else if !self.files.contains_key(&f.fd) {
                    POLLNVAL
                } else {
                    let asked = if exact { f.events } else { f.events | POLLERR | POLLHUP };
                    self.io.ready.get(&f.fd).copied().unwrap_or(0) & asked
                };
                n += usize::from(f.revents != 0);
            }
            if n > 0 || !self.wait_once(wait)? {
                return Ok(n);
            }
        }
    }

    fn bytes_readable(&mut self, fd: i32) -> Result<Option<usize>, Errno> {
        if !self.files.contains_key(&fd) {
            return Err(Errno::EBADF);
        }
        Ok(self.io.readable.get(&fd).copied())
    }

    fn epoll_create(&mut self, cloexec: bool) -> Result<i32, Errno> {
        let fd = self.new_fd(Kind::NullDevice);
        self.io.epolls.insert(fd, Default::default());
        self.io.made.push(Made::Epoll { fd, cloexec });
        Ok(fd)
    }

    fn epoll_ctl(&mut self, epfd: i32, op: EpollOp, fd: i32, event: EpollEvent) -> Result<(), Errno> {
        if !self.files.contains_key(&epfd) || !self.files.contains_key(&fd) {
            return Err(Errno::EBADF);
        }
        let (interest, _) = self.io.epolls.get_mut(&epfd).ok_or(Errno::EINVAL)?;
        match op {
            EpollOp::Add if interest.contains_key(&fd) => Err(Errno::EEXIST),
            EpollOp::Add => {
                interest.insert(fd, event);
                Ok(())
            }
            EpollOp::Mod => interest.get_mut(&fd).map(|e| *e = event).ok_or(Errno::ENOENT),
            EpollOp::Del => interest.remove(&fd).map(|_| ()).ok_or(Errno::ENOENT),
        }
    }

    fn epoll_wait(&mut self, epfd: i32, max: usize, wait: WaitTime) -> Result<Vec<EpollEvent>, Errno> {
        if !self.files.contains_key(&epfd) {
            return Err(Errno::EBADF);
        }
        self.io.epoll_waits.push((epfd, max, wait));
        loop {
            let (_, pending) = self.io.epolls.get_mut(&epfd).ok_or(Errno::EINVAL)?;
            if !pending.is_empty() {
                let n = pending.len().min(max);
                return Ok(pending.drain(..n).collect());
            }
            if !self.wait_once(wait)? {
                return Ok(Vec::new());
            }
        }
    }

    fn eventfd(&mut self, initval: u32, semaphore: bool, new: NewFd) -> Result<i32, Errno> {
        let fd = self.new_fd(Kind::NullDevice);
        self.io.made.push(Made::EventFd { fd, initval, semaphore, new });
        Ok(fd)
    }

    fn timerfd_create(&mut self, clock: Clock, new: NewFd) -> Result<i32, Errno> {
        let fd = self.new_fd(Kind::NullDevice);
        self.io.timers.insert(fd, (false, ItimerSpec::default()));
        self.io.made.push(Made::TimerFd { fd, clock, new });
        Ok(fd)
    }

    fn timerfd_settime(&mut self, fd: i32, absolute: bool, new: &ItimerSpec) -> Result<ItimerSpec, Errno> {
        if !self.files.contains_key(&fd) {
            return Err(Errno::EBADF);
        }
        let t = self.io.timers.get_mut(&fd).ok_or(Errno::EINVAL)?;
        let old = t.1;
        *t = (absolute, *new);
        Ok(old)
    }

    fn timerfd_gettime(&mut self, fd: i32) -> Result<ItimerSpec, Errno> {
        self.io.timers.get(&fd).map(|t| t.1).ok_or(Errno::EINVAL)
    }

    fn signalfd(&mut self, fd: i32, mask: SigSet, new: NewFd) -> Result<i32, Errno> {
        let fd = if fd == -1 { self.new_fd(Kind::NullDevice) } else { fd };
        self.io.made.push(Made::SignalFd { fd, mask, new });
        Ok(fd)
    }
}

const ARG: u64 = DATA + 0x800;
const BUF: u64 = DATA + 0x1000;
const HANDLER: u64 = 0x40_5000;
const RESTORER: u64 = 0x40_6000;

fn install(k: &mut FakeKernel, sig: u32, flags: u64) {
    let act = SigAction { handler: HANDLER, flags: flags | SA_RESTORER, restorer: RESTORER, mask: 0 };
    k.poke(ARG, &act.to_bytes());
    assert_eq!(sys(k, nr::RT_SIGACTION, &[sig as u64, ARG, 0, 8]), Ok(0));
}

fn pollfds(k: &mut FakeKernel, at: u64, fds: &[(i32, u16)]) {
    let raw: Vec<u8> = fds.iter().flat_map(|&(fd, events)| PollFd { fd, events, revents: 0xdead }.to_bytes()).collect();
    k.poke(at, &raw);
}

fn revents(k: &FakeKernel, at: u64, n: usize) -> Vec<u16> {
    (0..n).map(|i| PollFd::from_bytes(&k.peek(at + 8 * i as u64, 8)).revents).collect()
}

fn ms(n: u64) -> Timespec {
    Timespec::from_nanos(n * 1_000_000)
}

// ---- pipes ---------------------------------------------------------------

#[test]
fn pipe2_returns_both_ends_and_checks_flags() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::PIPE2, &[BUF, 0o4000 | 0o2000000]), Ok(0));
    let fds = [k.peek_u32(BUF) as i32, k.peek_u32(BUF + 4) as i32];
    assert_eq!(fds, [3, 4], "the lowest free descriptors");
    assert_eq!(k.io.made, vec![Made::Pipe { fds, new: NewFd { nonblock: true, cloexec: true }, direct: false }]);
    assert_eq!(sys(&mut k, nr::PIPE2, &[BUF, 0o40000]), Ok(0), "O_DIRECT");
    assert!(matches!(k.io.made[1], Made::Pipe { direct: true, .. }));
    assert_eq!(sys(&mut k, nr::PIPE, &[BUF]), Ok(0));
    assert_eq!(sys(&mut k, nr::PIPE2, &[BUF, 0o200]), Err(Errno::EINVAL), "O_EXCL (O_NOTIFICATION_PIPE)");
    let before = k.files.len();
    assert_eq!(sys(&mut k, nr::PIPE2, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(k.files.len(), before, "no descriptors left behind");
}

#[test]
fn epipe_raises_sigpipe_for_the_writer() {
    let mut k = FakeKernel::new();
    k.files.get_mut(&1).unwrap().fail_after = Some((0, Errno::EPIPE));
    k.poke(BUF, b"x");
    assert_eq!(sys(&mut k, nr::WRITE, &[1, BUF, 1]), Err(Errno::EPIPE));
    let tid = k.tid;
    assert!(k.me().pending(tid).contains(SIGPIPE));
    assert!(k.notified.contains(&tid));
    assert_eq!(k.io.queued, vec![100], "signal files hear of it");
    let (exit, _) = run(&mut k, nr::GETPID, &[]);
    assert_eq!(exit, ExitToUser::Killed, "SIGPIPE's default action terminates");
    assert_eq!(k.exit, Some(Exit::Signal(SIGPIPE, false)));
    // Ignored: only EPIPE.
    let mut k = FakeKernel::new();
    k.files.get_mut(&1).unwrap().fail_after = Some((0, Errno::EPIPE));
    k.me().set_action(SIGPIPE, SigAction { handler: SIG_IGN, ..SigAction::default() });
    let iov = put_iovecs(&mut k, ARG, &[(BUF, 1)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, iov, 1]), Err(Errno::EPIPE));
    assert!(k.me().pending(100).is_empty());
}

#[test]
fn fionread_reports_the_bytes_in_a_pipe() {
    let mut k = FakeKernel::new();
    k.io.readable.insert(1, 42);
    assert_eq!(sys(&mut k, nr::IOCTL, &[1, 0x541B, BUF]), Ok(0));
    assert_eq!(k.peek_u32(BUF), 42);
    assert!(k.ioctls.is_empty(), "answered without the device");
    // A file that does not say goes to the device.
    let _ = sys(&mut k, nr::IOCTL, &[0, 0x541B, BUF]);
    assert_eq!(k.ioctls, vec![(0, 0x541B, BUF)]);
    k.io.readable.insert(1, 7);
    assert_eq!(sys(&mut k, nr::IOCTL, &[1, 0x541B, UNMAPPED]), Err(Errno::EFAULT));
}

// ---- poll ------------------------------------------------------------------

#[test]
fn poll_fills_revents_and_converts_the_timeout() {
    let mut k = FakeKernel::new();
    k.io.ready.insert(1, POLLOUT | POLLERR);
    pollfds(&mut k, BUF, &[(1, POLLIN | POLLOUT), (9, POLLIN), (-1, POLLIN), (0, POLLIN)]);
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 4, 0]), Ok(2));
    assert_eq!(revents(&k, BUF, 4), [POLLOUT | POLLERR, POLLNVAL, 0, 0]);
    assert_eq!(k.peek_u32(BUF + 8) as i32, 9, "fd and events are left alone");
    assert_eq!(k.io.polls[0].1, WaitTime::Poll);
    assert!(!k.io.polls[0].2, "poll reports POLLERR and POLLHUP unasked");
    // 250 ms from the monotonic clock; nothing comes: 0.
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 1, 250]), Ok(0));
    let at = k.monotonic.saturating_add(ms(250));
    assert_eq!(k.io.polls[1].1, WaitTime::Until(Deadline { clock: Clock::Monotonic, at }));
    assert_eq!(k.deadlines, vec![Some(Deadline { clock: Clock::Monotonic, at })]);
    // Negative: forever; a write by another thread wakes it.
    k.meanwhile(|k| {
        k.io.ready.insert(0, POLLIN);
        k.unblock(100);
    });
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 1, u64::MAX]), Ok(1));
    assert_eq!(k.io.polls[2].1, WaitTime::Forever);
    assert_eq!(revents(&k, BUF, 1), [POLLIN]);
}

#[test]
fn poll_checks_nfds_and_the_array() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 1025, 0]), Err(Errno::EINVAL), "above RLIMIT_NOFILE");
    assert_eq!(sys(&mut k, nr::POLL, &[UNMAPPED, 1, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::POLL, &[UNMAPPED, 0, 0]), Ok(0), "nothing to read");
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::POLL, &[ro, 1, 0]), Err(Errno::EFAULT), "revents cannot be written");
}

#[test]
fn interrupted_poll_is_eintr_after_a_handler_and_restarts_without() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    k.meanwhile(|k| {
        let me = k.tid;
        assert_eq!(sys(k, nr::TKILL, &[me as u64, SIGUSR1 as u64]), Ok(0));
    });
    let (_, ctx) = run(&mut k, nr::POLL, &[BUF, 1, u64::MAX]);
    assert_eq!(ctx.rip, HANDLER, "the handler runs");
    // It returns through the restorer: EINTR although SA_RESTART is set.
    k.ctx.rsp += 8;
    k.ctx.rip = RESTORER + 9;
    let (_, ctx) = run(&mut k, nr::RT_SIGRETURN, &[]);
    assert_eq!((ctx.rip, Errno::from_rax(ctx.rax)), (SYSCALL_RIP, Some(Errno::EINTR)));
    // Without a handler (a stop and SIGCONT) it restarts.
    let mut k = FakeKernel::new();
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    k.meanwhile(|k| {
        let me = k.tid;
        assert_eq!(sys(k, nr::TKILL, &[me as u64, SIGSTOP as u64]), Ok(0));
    });
    let (_, ctx) = run(&mut k, nr::POLL, &[BUF, 1, u64::MAX]);
    assert_eq!(k.stops, vec![SIGSTOP]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::POLL), "restarted");
}

#[test]
fn a_pending_signal_ends_a_poll_with_nothing_ready() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0);
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR1);
    assert_eq!(sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]), Ok(0));
    // ppoll with an empty mask: the pending signal is deliverable at once.
    k.poke(ARG, &0u64.to_le_bytes());
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, 0, ARG, 8]), Err(Errno::EINTR));
    assert_eq!(k.io.polls[0].1, WaitTime::Poll, "only checked");
    assert_eq!(k.blocks, 0);
    assert_eq!(k.mask(), SigSet::EMPTY, "the wait's mask stays until the signal is delivered");
    assert_eq!(k.thread_state().saved_mask, Some(SigSet::only(SIGUSR1)));
    // Something ready wins over the signal.
    let mut k = FakeKernel::new();
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR1);
    assert_eq!(sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]), Ok(0));
    k.io.ready.insert(0, POLLIN);
    k.poke(ARG, &0u64.to_le_bytes());
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, 0, ARG, 8]), Ok(1));
    assert_eq!(k.mask(), SigSet::only(SIGUSR1), "the old mask is back");
    assert_eq!(k.thread_state().saved_mask, None);
}

#[test]
fn ppoll_takes_a_timespec_and_writes_the_time_left_back() {
    let mut k = FakeKernel::new();
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    let t = Timespec { sec: 1, nsec: 500_000_000 };
    k.poke(ARG, &t.to_bytes());
    k.meanwhile(|k| k.monotonic = k.monotonic.saturating_add(Timespec { sec: 1, nsec: 0 }));
    // Nothing ready, the deadline passes in the fake: 0 and the rest.
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, ARG, 0, 8]), Ok(0));
    let left = Timespec::from_bytes(&k.peek(ARG, 16).try_into().unwrap());
    assert_eq!(left, Timespec { sec: 0, nsec: 500_000_000 });
    k.poke(ARG, &Timespec { sec: 0, nsec: 1_000_000_000 }.to_bytes());
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, ARG, 0, 8]), Err(Errno::EINVAL));
    k.poke(ARG, &Timespec::default().to_bytes());
    k.poke(ARG + 16, &0u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, ARG, ARG + 16, 16]), Err(Errno::EINVAL), "sigsetsize");
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, ARG, ARG + 16, 8]), Ok(0));
    assert_eq!(k.io.polls.last().unwrap().1, WaitTime::Poll, "a zero timespec only checks");
}

#[test]
fn ppoll_installs_its_mask_for_the_wait() {
    let mut k = FakeKernel::new();
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR2);
    k.poke(ARG, &(SigSet::only(SIGUSR1).union(SigSet::only(SIGKILL))).0.to_le_bytes());
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    k.meanwhile(|k| {
        assert_eq!(k.mask(), SigSet::only(SIGUSR1), "the wait's mask, without SIGKILL");
        k.io.ready.insert(0, POLLIN);
        k.unblock(100);
    });
    assert_eq!(sys(&mut k, nr::PPOLL, &[BUF, 1, 0, ARG, 8]), Ok(1));
    assert_eq!(k.mask(), SigSet::only(SIGUSR2));
}

// ---- select ------------------------------------------------------------------

fn fdset(k: &mut FakeKernel, at: u64, fds: &[u32]) {
    let mut w = [0u64; 2];
    for &fd in fds {
        w[fd as usize / 64] |= 1 << (fd % 64);
    }
    k.poke(at, &w[0].to_le_bytes());
    k.poke(at + 8, &w[1].to_le_bytes());
}

fn fdset_read(k: &FakeKernel, at: u64) -> u64 {
    le_u64(&k.peek(at, 8))
}

#[test]
fn select_maps_sets_to_poll_and_back() {
    let mut k = FakeKernel::new();
    let (r, w, e) = (ARG, ARG + 16, ARG + 32);
    k.io.ready.insert(0, POLLIN);
    k.io.ready.insert(1, POLLOUT);
    k.io.ready.insert(2, POLLHUP);
    fdset(&mut k, r, &[0, 1, 2]);
    fdset(&mut k, w, &[1, 2]);
    fdset(&mut k, e, &[0]);
    k.poke(BUF, &[0u8; 16]);
    assert_eq!(sys(&mut k, nr::SELECT, &[3, r, w, e, BUF]), Ok(3));
    assert_eq!(fdset_read(&k, r), 0b101, "0 is readable, 2 at end of file");
    assert_eq!(fdset_read(&k, w), 0b010);
    assert_eq!(fdset_read(&k, e), 0);
    let (asked, wait, exact) = &k.io.polls[0];
    assert!(*exact);
    assert_eq!(*wait, WaitTime::Poll);
    assert_eq!(asked.iter().map(|p| p.fd).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(asked[0].events, POLLIN | POLLRDNORM | POLLRDBAND | POLLHUP | POLLERR | POLLPRI);
    assert_eq!(asked[1].events & POLLOUT, POLLOUT);
    // Bits from nfds up are neither looked at nor kept.
    fdset(&mut k, r, &[0, 5]);
    assert_eq!(sys(&mut k, nr::SELECT, &[1, r, 0, 0, BUF]), Ok(1));
    assert_eq!(fdset_read(&k, r), 1);
}

#[test]
fn select_errors_and_timeouts() {
    let mut k = FakeKernel::new();
    fdset(&mut k, ARG, &[7]);
    assert_eq!(sys(&mut k, nr::SELECT, &[8, ARG, 0, 0, 0]), Err(Errno::EBADF), "fd 7 is not open");
    assert_eq!(fdset_read(&k, ARG), 1 << 7, "sets unchanged on an error");
    assert_eq!(sys(&mut k, nr::SELECT, &[u64::MAX, 0, 0, 0, 0]), Err(Errno::EINVAL), "nfds -1");
    assert_eq!(sys(&mut k, nr::SELECT, &[1025, 0, 0, 0, 0]), Err(Errno::EINVAL));
    // timeval { 0, 1000000 } is invalid.
    k.poke(BUF, &0u64.to_le_bytes());
    k.poke(BUF + 8, &1_000_000u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SELECT, &[0, 0, 0, 0, BUF]), Err(Errno::EINVAL));
    // 2.25 s; 1 s passes; nothing ready: 0, sets cleared, 1.25 s left.
    k.poke(BUF + 8, &250_000u64.to_le_bytes());
    k.poke(BUF, &2u64.to_le_bytes());
    fdset(&mut k, ARG, &[0]);
    k.meanwhile(|k| k.monotonic = k.monotonic.saturating_add(Timespec { sec: 1, nsec: 0 }));
    assert_eq!(sys(&mut k, nr::SELECT, &[1, ARG, 0, 0, BUF]), Ok(0));
    assert_eq!(fdset_read(&k, ARG), 0);
    assert_eq!((le_u64(&k.peek(BUF, 8)), le_u64(&k.peek(BUF + 8, 8))), (1, 250_000));
    assert_eq!(sys(&mut k, nr::SELECT, &[1, UNMAPPED, 0, 0, 0]), Err(Errno::EFAULT));
}

#[test]
fn pselect6_takes_a_mask_through_a_pointer() {
    let mut k = FakeKernel::new();
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR2);
    // { const sigset_t *ss; size_t ss_len; }
    k.poke(ARG, &SigSet::only(SIGUSR1).0.to_le_bytes());
    k.poke(ARG + 8, &ARG.to_le_bytes());
    k.poke(ARG + 16, &8u64.to_le_bytes());
    k.io.ready.insert(0, POLLIN);
    fdset(&mut k, BUF, &[0]);
    k.meanwhile(|_| {});
    assert_eq!(sys(&mut k, nr::PSELECT6, &[1, BUF, 0, 0, 0, ARG + 8]), Ok(1));
    assert_eq!(k.mask(), SigSet::only(SIGUSR2));
    k.poke(ARG + 16, &4u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::PSELECT6, &[1, BUF, 0, 0, 0, ARG + 8]), Err(Errno::EINVAL));
    // A null ss: no mask.
    k.poke(ARG + 8, &0u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::PSELECT6, &[1, BUF, 0, 0, 0, ARG + 8]), Ok(1));
}

// ---- epoll -------------------------------------------------------------------

#[test]
fn epoll_create_ctl_and_wait() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::EPOLL_CREATE, &[0]), Err(Errno::EINVAL), "size must be positive");
    assert_eq!(sys(&mut k, nr::EPOLL_CREATE1, &[1]), Err(Errno::EINVAL));
    let ep = sys(&mut k, nr::EPOLL_CREATE1, &[0o2000000]).unwrap();
    assert_eq!(k.io.made, vec![Made::Epoll { fd: 3, cloexec: true }]);
    assert_eq!(sys(&mut k, nr::EPOLL_CREATE, &[1]), Ok(4));
    // struct epoll_event is packed: events at 0, data at 4.
    let mut ev = [0u8; 12];
    ev[..4].copy_from_slice(&(0x8000_0001u32).to_le_bytes());
    ev[4..].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    k.poke(ARG, &ev);
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 1, 0, ARG]), Ok(0));
    assert_eq!(k.io.epolls[&3].0[&0], EpollEvent { events: 0x8000_0001, data: 0x1122_3344_5566_7788 });
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 1, 0, ARG]), Err(Errno::EEXIST));
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 4, 0, ARG]), Err(Errno::EINVAL), "unknown op");
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 3, 0, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 2, 0, 0]), Ok(0), "DEL takes a null event");
    assert_eq!(sys(&mut k, nr::EPOLL_CTL, &[ep, 1, u64::from(u32::MAX), ARG]), Err(Errno::EBADF));
    // Waiting.
    for max in [0u64, u64::from(u32::MAX), (i32::MAX / 12 + 1) as u64] {
        assert_eq!(sys(&mut k, nr::EPOLL_WAIT, &[ep, BUF, max, 0]), Err(Errno::EINVAL), "maxevents {max}");
    }
    k.io.epolls.get_mut(&3).unwrap().1 = vec![
        EpollEvent { events: 1, data: 10 },
        EpollEvent { events: 4, data: 20 },
        EpollEvent { events: 5, data: 30 },
    ];
    assert_eq!(sys(&mut k, nr::EPOLL_WAIT, &[ep, BUF, 2, 100]), Ok(2));
    assert_eq!(EpollEvent::from_bytes(&k.peek(BUF + 12, 12).try_into().unwrap()), EpollEvent { events: 4, data: 20 });
    let at = k.monotonic.saturating_add(ms(100));
    assert_eq!(k.io.epoll_waits[0], (3, 2, WaitTime::Until(Deadline { clock: Clock::Monotonic, at })));
    assert_eq!(sys(&mut k, nr::EPOLL_WAIT, &[ep, UNMAPPED, 2, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::EPOLL_WAIT, &[ep, BUF, 2, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::EPOLL_WAIT, &[0, BUF, 2, 0]), Err(Errno::EINVAL), "not an epoll set");
}

#[test]
fn interrupted_epoll_wait_is_never_restarted() {
    for handler in [true, false] {
        let mut k = FakeKernel::new();
        let ep = sys(&mut k, nr::EPOLL_CREATE1, &[0]).unwrap();
        if handler {
            install(&mut k, SIGUSR1, SA_RESTART);
        }
        let sig = if handler { SIGUSR1 } else { SIGSTOP };
        k.meanwhile(move |k| {
            let me = k.tid;
            assert_eq!(sys(k, nr::TKILL, &[me as u64, sig as u64]), Ok(0));
        });
        let (_, mut ctx) = run(&mut k, nr::EPOLL_WAIT, &[ep, BUF, 4, u64::MAX]);
        if handler {
            assert_eq!(ctx.rip, HANDLER);
            k.ctx.rsp += 8;
            k.ctx.rip = RESTORER + 9;
            ctx = run(&mut k, nr::RT_SIGRETURN, &[]).1;
        } else {
            assert_eq!(k.stops, vec![SIGSTOP]);
        }
        assert_eq!(Errno::from_rax(ctx.rax), Some(Errno::EINTR), "handler: {handler}");
        assert_eq!(ctx.rip, SYSCALL_RIP, "not restarted (handler: {handler})");
    }
}

#[test]
fn epoll_pwait_masks_and_pwait2_takes_a_timespec() {
    let mut k = FakeKernel::new();
    let ep = sys(&mut k, nr::EPOLL_CREATE1, &[0]).unwrap();
    k.poke(ARG, &SigSet::only(SIGINT).0.to_le_bytes());
    k.meanwhile(|k| {
        assert_eq!(k.mask(), SigSet::only(SIGINT));
        k.io.epolls.get_mut(&3).unwrap().1.push(EpollEvent { events: 1, data: 1 });
        k.unblock(100);
    });
    assert_eq!(sys(&mut k, nr::EPOLL_PWAIT, &[ep, BUF, 1, u64::MAX, ARG, 8]), Ok(1));
    assert_eq!(k.mask(), SigSet::EMPTY);
    assert_eq!(sys(&mut k, nr::EPOLL_PWAIT, &[ep, BUF, 1, 0, ARG, 9]), Err(Errno::EINVAL));
    k.poke(ARG + 8, &Timespec { sec: 0, nsec: 5 }.to_bytes());
    assert_eq!(sys(&mut k, nr::EPOLL_PWAIT2, &[ep, BUF, 1, ARG + 8, 0, 8]), Ok(0));
    let at = k.monotonic.saturating_add(Timespec { sec: 0, nsec: 5 });
    assert_eq!(k.io.epoll_waits.last().unwrap().2, WaitTime::Until(Deadline { clock: Clock::Monotonic, at }));
}

// ---- event files ---------------------------------------------------------------

#[test]
fn eventfd_flags() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::EVENTFD2, &[5, 1 | 0o4000]), Ok(3));
    assert_eq!(
        k.io.made[0],
        Made::EventFd { fd: 3, initval: 5, semaphore: true, new: NewFd { nonblock: true, cloexec: false } }
    );
    assert_eq!(sys(&mut k, nr::EVENTFD2, &[0, 2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::EVENTFD, &[u64::from(u32::MAX)]), Ok(4));
    assert!(matches!(k.io.made[1], Made::EventFd { initval: u32::MAX, semaphore: false, .. }));
}

#[test]
fn timerfd_clocks_flags_and_settings() {
    let mut k = FakeKernel::new();
    for (id, clock) in [(0, Clock::Realtime), (1, Clock::Monotonic), (7, Clock::Monotonic), (8, Clock::Realtime)] {
        let fd = sys(&mut k, nr::TIMERFD_CREATE, &[id, 0o2000000]).unwrap() as i32;
        assert!(k.io.made.contains(&Made::TimerFd { fd, clock, new: NewFd { nonblock: false, cloexec: true } }));
    }
    for id in [2u64, 4, 5, 6, 10, u64::MAX] {
        assert_eq!(sys(&mut k, nr::TIMERFD_CREATE, &[id, 0]), Err(Errno::EINVAL), "clock {id}");
    }
    assert_eq!(sys(&mut k, nr::TIMERFD_CREATE, &[1, 1]), Err(Errno::EINVAL));
    let new = ItimerSpec { interval: ms(10), value: Timespec { sec: 3, nsec: 0 } };
    k.poke(ARG, &new.to_bytes());
    assert_eq!(sys(&mut k, nr::TIMERFD_SETTIME, &[3, 1, ARG, BUF]), Ok(0));
    assert_eq!(k.io.timers[&3], (true, new));
    assert_eq!(k.peek(BUF, 32), ItimerSpec::default().to_bytes(), "the old setting");
    assert_eq!(sys(&mut k, nr::TIMERFD_SETTIME, &[3, 4, ARG, 0]), Err(Errno::EINVAL), "flags");
    assert_eq!(sys(&mut k, nr::TIMERFD_SETTIME, &[3, 2, ARG, 0]), Ok(0), "TFD_TIMER_CANCEL_ON_SET");
    k.poke(ARG + 8, &1_000_000_000u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::TIMERFD_SETTIME, &[3, 0, ARG, 0]), Err(Errno::EINVAL), "bad nsec");
    assert_eq!(sys(&mut k, nr::TIMERFD_SETTIME, &[3, 0, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::TIMERFD_GETTIME, &[3, BUF]), Ok(0));
    assert_eq!(ItimerSpec::from_bytes(&k.peek(BUF, 32).try_into().unwrap()), new);
    assert_eq!(sys(&mut k, nr::TIMERFD_GETTIME, &[0, BUF]), Err(Errno::EINVAL), "not a timer");
    assert_eq!(sys(&mut k, nr::TIMERFD_GETTIME, &[u64::from(u32::MAX), BUF]), Err(Errno::EBADF));
}

#[test]
fn signalfd_masks_and_flags() {
    let mut k = FakeKernel::new();
    let mask = SigSet::only(SIGUSR1).union(SigSet::only(SIGKILL)).union(SigSet::only(SIGSTOP));
    k.poke(ARG, &mask.0.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SIGNALFD4, &[u64::from(u32::MAX), ARG, 8, 0o4000]), Ok(3));
    assert_eq!(
        k.io.made[0],
        Made::SignalFd { fd: 3, mask: SigSet::only(SIGUSR1), new: NewFd { nonblock: true, cloexec: false } }
    );
    assert_eq!(sys(&mut k, nr::SIGNALFD4, &[3, ARG, 8, 0]), Ok(3), "a new mask for fd 3");
    assert_eq!(sys(&mut k, nr::SIGNALFD4, &[u64::from(u32::MAX), ARG, 4, 0]), Err(Errno::EINVAL), "sizemask");
    assert_eq!(sys(&mut k, nr::SIGNALFD4, &[u64::from(u32::MAX), ARG, 8, 1]), Err(Errno::EINVAL), "flags");
    assert_eq!(sys(&mut k, nr::SIGNALFD4, &[u64::from(u32::MAX), UNMAPPED, 8, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SIGNALFD, &[u64::from(u32::MAX - 1), ARG, 8]), Err(Errno::EBADF), "fd -2");
    assert_eq!(sys(&mut k, nr::SIGNALFD, &[u64::from(u32::MAX), ARG, 8]), Ok(4));
}

#[test]
fn signalfd_siginfo_layout() {
    let user = siginfo_bytes(&SigInfo::from_process(SIGUSR1, SI_USER, 77, 1000));
    assert_eq!(u32::from_le_bytes(user[0..4].try_into().unwrap()), SIGUSR1);
    assert_eq!(i32::from_le_bytes(user[8..12].try_into().unwrap()), SI_USER);
    assert_eq!(u32::from_le_bytes(user[12..16].try_into().unwrap()), 77, "ssi_pid");
    assert_eq!(u32::from_le_bytes(user[16..20].try_into().unwrap()), 1000, "ssi_uid");
    let mut queued = SigInfo::from_process(SIGRTMIN, SI_QUEUE, 5, 6);
    queued.fields[8..16].copy_from_slice(&0x1234_5678_9abc_def0u64.to_le_bytes());
    let q = siginfo_bytes(&queued);
    assert_eq!(i32::from_le_bytes(q[44..48].try_into().unwrap()), 0x9abc_def0u32 as i32, "ssi_int");
    assert_eq!(le_u64(&q[48..56]), 0x1234_5678_9abc_def0, "ssi_ptr");
    let fault = siginfo_bytes(&SigInfo::fault(SIGSEGV, 1, 0xdead_b000));
    assert_eq!(le_u64(&fault[72..80]), 0xdead_b000, "ssi_addr");
    assert_eq!(u32::from_le_bytes(fault[12..16].try_into().unwrap()), 0, "no pid for a fault");
    assert_eq!(user.len(), 128);
}

#[test]
fn signals_queued_for_a_process_are_announced() {
    let mut k = FakeKernel::new();
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR1);
    assert_eq!(sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]), Ok(0));
    assert_eq!(k.io.queued, vec![100], "blocked, but a signal file may read it");
    assert_eq!(sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]), Ok(0));
    assert_eq!(k.io.queued, vec![100], "merged with the pending one: nothing new");
    assert!(!k.notified.contains(&100));
}

#[test]
fn nfds_is_bounded_by_the_descriptor_limit() {
    let mut k = FakeKernel::new();
    k.io.limit = 0;
    pollfds(&mut k, BUF, &[(0, POLLIN)]);
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::POLL, &[BUF, 0, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::SELECT, &[1, 0, 0, 0, 0]), Err(Errno::EINVAL));
}
