//! What the personality needs from the kernel for pipes, readiness waits
//! and event files (ADR 0042): [`FileIo`], reached through
//! [`LinuxKernel::file_io`](crate::LinuxKernel::file_io), and the types it
//! speaks in.
//!
//! The personality decodes and checks arguments, turns timeouts into
//! deadlines, swaps signal masks (`ppoll`, `pselect6`, `epoll_pwait`),
//! converts `select`'s descriptor sets, copies results to user memory and
//! decides how an interrupted wait ends. The kernel owns the files: it
//! makes them, looks descriptors up, and waits (a wait the thread's
//! signal notification ends returns `EINTR`).

use crate::abi::Timespec;
use crate::errno::Errno;
use crate::kernel::{Clock, Deadline, SigSet};
use alloc::vec::Vec;

/// Flag values from UAPI `asm-generic/fcntl.h` (x86-64 uses them), which
/// `pipe2`, `eventfd2` (`EFD_*`), `timerfd_create` (`TFD_*`),
/// `signalfd4` (`SFD_*`) and `epoll_create1` (`EPOLL_CLOEXEC`) share.
pub mod flags {
    pub const O_NONBLOCK: u32 = 0o4000;
    pub const O_DIRECT: u32 = 0o40000;
    pub const O_CLOEXEC: u32 = 0o2000000;
}

/// How long a wait may last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitTime {
    /// Check once, do not block.
    Poll,
    /// Until the deadline.
    Until(Deadline),
    /// Until something happens (or a signal).
    Forever,
}

/// `struct pollfd` (`asm-generic/poll.h`): `int fd; short events; short
/// revents;`, 8 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollFd {
    pub fd: i32,
    pub events: u16,
    pub revents: u16,
}

impl PollFd {
    pub const SIZE: usize = 8;

    pub fn from_bytes(b: &[u8]) -> PollFd {
        PollFd {
            fd: i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            events: u16::from_le_bytes([b[4], b[5]]),
            revents: u16::from_le_bytes([b[6], b[7]]),
        }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..4].copy_from_slice(&self.fd.to_le_bytes());
        b[4..6].copy_from_slice(&self.events.to_le_bytes());
        b[6..8].copy_from_slice(&self.revents.to_le_bytes());
        b
    }
}

/// Poll event bits (`asm-generic/poll.h`; `linux/eventpoll.h` uses the
/// same values for `EPOLLIN` and friends).
pub mod events {
    pub const POLLIN: u16 = 0x001;
    pub const POLLPRI: u16 = 0x002;
    pub const POLLOUT: u16 = 0x004;
    pub const POLLERR: u16 = 0x008;
    pub const POLLHUP: u16 = 0x010;
    pub const POLLNVAL: u16 = 0x020;
    pub const POLLRDNORM: u16 = 0x040;
    pub const POLLRDBAND: u16 = 0x080;
    pub const POLLWRNORM: u16 = 0x100;
    pub const POLLWRBAND: u16 = 0x200;
}

/// `struct epoll_event` (`linux/eventpoll.h`): `__u32 events; __u64
/// data;`, packed on x86-64 (`__EPOLL_PACKED`), so 12 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EpollEvent {
    pub events: u32,
    pub data: u64,
}

impl EpollEvent {
    pub const SIZE: usize = 12;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> EpollEvent {
        let mut data = [0; 8];
        data.copy_from_slice(&b[4..12]);
        EpollEvent { events: u32::from_le_bytes([b[0], b[1], b[2], b[3]]), data: u64::from_le_bytes(data) }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..4].copy_from_slice(&self.events.to_le_bytes());
        b[4..12].copy_from_slice(&self.data.to_le_bytes());
        b
    }
}

/// `epoll_ctl`'s operation (`EPOLL_CTL_ADD` 1, `EPOLL_CTL_DEL` 2,
/// `EPOLL_CTL_MOD` 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpollOp {
    Add,
    Del,
    Mod,
}

/// `struct itimerspec` (`linux/time_types.h`): the interval, then the
/// value; 32 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItimerSpec {
    pub interval: Timespec,
    pub value: Timespec,
}

impl ItimerSpec {
    pub const SIZE: usize = 32;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> ItimerSpec {
        let half = |o: usize| {
            let mut t = [0; Timespec::SIZE];
            t.copy_from_slice(&b[o..o + Timespec::SIZE]);
            Timespec::from_bytes(&t)
        };
        ItimerSpec { interval: half(0), value: half(Timespec::SIZE) }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[..Timespec::SIZE].copy_from_slice(&self.interval.to_bytes());
        b[Timespec::SIZE..].copy_from_slice(&self.value.to_bytes());
        b
    }
}

/// Status flags a new descriptor gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NewFd {
    /// `O_NONBLOCK` on the open file description.
    pub nonblock: bool,
    /// Close-on-exec on the descriptor.
    pub cloexec: bool,
}

impl NewFd {
    pub const fn from_flags(bits: u32) -> NewFd {
        NewFd { nonblock: bits & flags::O_NONBLOCK != 0, cloexec: bits & flags::O_CLOEXEC != 0 }
    }
}

/// The kernel's pipes, waits and event files for the calling thread.
/// Descriptor arguments are not negative unless a method says otherwise;
/// a descriptor that is not open is `EBADF`.
pub trait FileIo {
    /// The most descriptors the process may have (`RLIMIT_NOFILE`), which
    /// bounds `poll`'s and `select`'s `nfds`.
    fn open_files_limit(&self) -> usize;

    /// A pipe (pipe(7)): `[read end, write end]`, each on the lowest free
    /// descriptor. `direct`: packet mode. `EMFILE`, `ENFILE`.
    fn pipe(&mut self, fd: NewFd, direct: bool) -> Result<[i32; 2], Errno>;

    /// Fill in every entry's `revents` (`POLLNVAL` for a descriptor that
    /// is not open, 0 for a negative one) and return how many are not 0,
    /// waiting as `wait` allows while none is. `revents` holds the events
    /// asked for that are true, plus `POLLERR` and `POLLHUP` unless
    /// `exact` (select(2) asks for exactly what it maps). `EINTR` when the
    /// thread's signal notification (or the end of its process) ends the
    /// wait.
    fn poll(&mut self, fds: &mut [PollFd], wait: WaitTime, exact: bool) -> Result<usize, Errno>;

    /// Bytes a read would return now (`FIONREAD`); `None` for files that
    /// do not say (`EBADF` if `fd` is not open).
    fn bytes_readable(&mut self, fd: i32) -> Result<Option<usize>, Errno>;

    /// A new epoll set.
    fn epoll_create(&mut self, cloexec: bool) -> Result<i32, Errno>;

    /// `epoll_ctl(2)`. `EBADF` for either descriptor, `EINVAL` if `epfd`
    /// is no epoll set or `fd == epfd`, `EPERM`, `EEXIST`, `ENOENT`,
    /// `ELOOP` as epoll_ctl(2) describes.
    fn epoll_ctl(&mut self, epfd: i32, op: EpollOp, fd: i32, event: EpollEvent) -> Result<(), Errno>;

    /// Up to `max` (at least 1) events of the set, waiting as `wait`
    /// allows while there are none; `EINTR` as for [`poll`](Self::poll),
    /// `EINVAL` if `epfd` is no epoll set.
    fn epoll_wait(&mut self, epfd: i32, max: usize, wait: WaitTime) -> Result<Vec<EpollEvent>, Errno>;

    /// An event counter starting at `initval`; `semaphore`:
    /// `EFD_SEMAPHORE`.
    fn eventfd(&mut self, initval: u32, semaphore: bool, fd: NewFd) -> Result<i32, Errno>;

    /// A timer file on `clock` (`Realtime` or `Monotonic`).
    fn timerfd_create(&mut self, clock: Clock, fd: NewFd) -> Result<i32, Errno>;

    /// Arm or disarm timer `fd` (checked values; `absolute`:
    /// `TFD_TIMER_ABSTIME`); returns the old setting. `EINVAL` if `fd` is
    /// not a timer file.
    fn timerfd_settime(&mut self, fd: i32, absolute: bool, new: &ItimerSpec) -> Result<ItimerSpec, Errno>;

    /// Timer `fd`'s setting: the time to the next expiration and the
    /// interval. `EINVAL` if `fd` is not a timer file.
    fn timerfd_gettime(&mut self, fd: i32) -> Result<ItimerSpec, Errno>;

    /// A signal file for `mask` (`fd` = -1), or a new mask for the signal
    /// file `fd` (`EINVAL` if it is not one); returns the descriptor.
    /// Reading it takes the reading thread's pending signals of the mask
    /// (signalfd(2)), encoded by [`crate::sys::signalfd::siginfo_bytes`].
    fn signalfd(&mut self, fd: i32, mask: SigSet, new: NewFd) -> Result<i32, Errno>;

    /// `F_GETPIPE_SZ`: the capacity of the pipe `fd` is an end of, in
    /// bytes. `EINVAL` if it is not a pipe.
    fn pipe_size(&mut self, fd: i32) -> Result<usize, Errno> {
        let _ = fd;
        Err(Errno::EINVAL)
    }

    /// `F_SETPIPE_SZ`: give the pipe `fd` is an end of at least `bytes` of
    /// room, rounded up to a power-of-two number of pages; returns the new
    /// capacity. `EBUSY` if what it holds would not fit, `EINVAL` if it is
    /// not a pipe.
    fn set_pipe_size(&mut self, fd: i32, bytes: usize) -> Result<usize, Errno> {
        let _ = (fd, bytes);
        Err(Errno::EINVAL)
    }
}

/// The deadline `timeout` after now on `clock`.
pub fn deadline_after(now: Timespec, clock: Clock, timeout: Timespec) -> Deadline {
    Deadline { clock, at: now.saturating_add(timeout) }
}

/// The time from `now` to `deadline`, 0 once it has passed.
pub fn remaining(now: Timespec, deadline: &Deadline) -> Timespec {
    let to_ns = |t: Timespec| (t.sec.max(0) as u128) * 1_000_000_000 + t.nsec.max(0) as u128;
    let left = to_ns(deadline.at).saturating_sub(to_ns(now));
    Timespec::from_nanos(left.min(u64::MAX as u128) as u64)
}
