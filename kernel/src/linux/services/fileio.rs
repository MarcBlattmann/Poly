//! Pipes, readiness waits and event files for the Linux personality
//! (`FileIo`, ADR 0042), and the blocking reads and writes behind
//! `LinuxKernel::read` / `write`.
//!
//! | `FileIo` | Kernel |
//! |---|---|
//! | `pipe` | `poly_vfs::pipe::Pipe`, one anonymous inode for both ends |
//! | `poll` | `poly_vfs::poll::poll` over the process's files |
//! | `epoll_*` | `poly_vfs::epoll::Epoll` |
//! | `eventfd` | `poly_vfs::eventfd::EventFd` |
//! | `timerfd_*` | `poly_vfs::timerfd::TimerFd` on [`fs::poll::timers`] |
//! | `signalfd` | [`SignalFd`](crate::linux::signalfd::SignalFd) |
//! | `inotify_*` | `poly_vfs::inotify::Inotify` (events from the VFS itself) |
//!
//! Waits block with [`ThreadBlocker`] (park / unpark), so the thread's
//! signal notification, or the end of its process, interrupts them: a
//! read or write interrupted before any byte moved returns
//! `RESTART_IF_SA_RESTART` (signal(7): reads and writes on "slow" devices
//! restart under `SA_RESTART`), a wait `EINTR`, which the personality
//! turns into what each call does.

use super::{Thread, vfs_errno};
use crate::fs::poll::{ThreadBlocker, timers};
use crate::fs::{self, Fd, FileType, OpenFile, OpenFlags};
use crate::linux::signalfd::SignalFd;
use crate::time;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::abi::Timespec;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fileio::{EpollEvent, EpollOp, FileIo, ItimerSpec, NewFd, PidFdTarget, PollFd, WaitTime};
use poly_personality_linux::kernel::{Clock, SigSet};
use poly_vfs::epoll::{self, Epoll};
use poly_vfs::eventfd::EventFd;
use poly_vfs::inotify::{Inotify, WatchKey};
use poly_vfs::pipe::{Pipe, PipeEnd};
use poly_vfs::poll::{PollEntry, PollTarget};
use poly_vfs::timerfd::{TimerClock, TimerFd, TimerSpec};
use poly_vfs::{FileObject, PollEvents, Timeout};

impl Thread<'_> {
    pub(in crate::linux) fn blocker(&self) -> ThreadBlocker<'_> {
        ThreadBlocker::new(self.process, Arc::clone(&self.flags))
    }

    fn file(&self, fd: Fd) -> Result<Arc<OpenFile>, Errno> {
        self.process.files.file(fd).map_err(vfs_errno)
    }

    /// Open `object` (on a new anonymous inode of `kind`) with `flags` and
    /// put it on the lowest free descriptor.
    fn install_object(
        &self,
        object: Arc<dyn FileObject>,
        kind: FileType,
        flags: OpenFlags,
        new: NewFd,
    ) -> Result<Fd, Errno> {
        let file = open_object(&self.process.files, object, kind, flags, new)?;
        self.process.files.install(file, new.cloexec).map_err(vfs_errno)
    }
}

fn open_object(
    files: &fs::Files,
    object: Arc<dyn FileObject>,
    kind: FileType,
    flags: OpenFlags,
    new: NewFd,
) -> Result<Arc<OpenFile>, Errno> {
    // Event files, timers, epoll sets and signal files (`Regular` here) are
    // anonymous inodes without a type, as on Linux.
    let node = if kind == FileType::Regular {
        fs::vfs().anon_inode(0o600, &files.cred())
    } else {
        fs::vfs().anon_node(kind, 0o600, &files.cred())
    }
    .map_err(vfs_errno)?;
    let flags = if new.nonblock { flags | OpenFlags::NONBLOCK } else { flags };
    Ok(Arc::new(OpenFile::on_object(node, object, flags)))
}

/// A wait interrupted before it got anywhere: the read or write restarts
/// under `SA_RESTART` (signal(7)).
fn io_errno(e: poly_vfs::Errno) -> Errno {
    if e == poly_vfs::Errno::EINTR { Errno::RESTART_IF_SA_RESTART } else { vfs_errno(e) }
}

/// `LinuxKernel::read`: blocks on files that wait (pipes, event files)
/// unless they are non-blocking.
pub(super) fn read(t: &mut Thread<'_>, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    let file = t.file(fd)?;
    let mut blocker = t.blocker();
    // A socket with SO_RCVTIMEO gives up then, and is not restarted after
    // a signal (signal(7); ADR 0048).
    match file.object().and_then(|o| o.timeouts().0) {
        Some(ns) => {
            file.read_wait_until(buf, &mut blocker, Timeout::At(time::now_ns().saturating_add(ns))).map_err(vfs_errno)
        }
        None => file.read_wait(buf, &mut blocker).map_err(io_errno),
    }
}

/// `LinuxKernel::write`: on a blocking pipe, all of `buf` (or what went
/// out before a signal or the last reader's close).
pub(super) fn write(t: &mut Thread<'_>, fd: Fd, buf: &[u8]) -> Result<usize, Errno> {
    let file = t.file(fd)?;
    // A writer without CAP_FSETID clears set-user-ID / set-group-ID bits
    // (ADR 0045).
    t.process.files.before_write(&file, buf.len()).map_err(super::vfs_errno)?;
    let mut blocker = t.blocker();
    match file.object().and_then(|o| o.timeouts().1) {
        Some(ns) => {
            file.write_wait_until(buf, &mut blocker, Timeout::At(time::now_ns().saturating_add(ns))).map_err(vfs_errno)
        }
        None => file.write_wait(buf, &mut blocker).map_err(io_errno),
    }
}

fn timeout(wait: WaitTime) -> Timeout {
    match wait {
        WaitTime::Poll => Timeout::Now,
        WaitTime::Forever => Timeout::Never,
        WaitTime::Until(d) => {
            let ns = to_ns(d.at);
            Timeout::At(match d.clock {
                // The wall clock moves with the monotonic one (it cannot be
                // set yet), so a wall-clock deadline is a monotonic time.
                Clock::Realtime => ns.saturating_sub(time::realtime_ns().saturating_sub(time::now_ns())),
                _ => ns,
            })
        }
    }
}

fn to_ns(t: Timespec) -> u64 {
    if t.sec < 0 { 0 } else { (t.sec as u64).saturating_mul(1_000_000_000).saturating_add(t.nsec.max(0) as u64) }
}

fn spec(t: TimerSpec) -> ItimerSpec {
    ItimerSpec { interval: Timespec::from_nanos(t.interval_ns), value: Timespec::from_nanos(t.value_ns) }
}

fn epoll_of(file: &OpenFile) -> Result<Arc<Epoll>, Errno> {
    file.object_as::<Epoll>().ok_or(Errno::EINVAL)
}

impl FileIo for Thread<'_> {
    fn open_files_limit(&self) -> usize {
        self.process.files.fd_limit()
    }

    fn pipe(&mut self, new: NewFd, direct: bool) -> Result<[i32; 2], Errno> {
        let (r, w) = Pipe::pair(direct);
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Fifo, 0o600, &files.cred()).map_err(vfs_errno)?;
        let mut flags = if new.nonblock { OpenFlags::NONBLOCK } else { OpenFlags::RDONLY };
        if direct {
            flags = flags | OpenFlags::DIRECT;
        }
        let read = Arc::new(OpenFile::on_object(node.clone(), Arc::new(r), flags | OpenFlags::RDONLY));
        let write = Arc::new(OpenFile::on_object(node, Arc::new(w), flags | OpenFlags::WRONLY));
        let rfd = files.install(read, new.cloexec).map_err(vfs_errno)?;
        match files.install(write, new.cloexec) {
            Ok(wfd) => Ok([rfd, wfd]),
            Err(e) => {
                let _ = files.close(rfd);
                Err(vfs_errno(e))
            }
        }
    }

    fn poll(&mut self, fds: &mut [PollFd], wait: WaitTime, exact: bool) -> Result<usize, Errno> {
        let mut entries: Vec<PollEntry> = fds
            .iter()
            .map(|p| {
                let target = if p.fd < 0 {
                    PollTarget::Ignore
                } else {
                    self.process.files.file(p.fd).map_or(PollTarget::Closed, PollTarget::File)
                };
                let events = PollEvents::from_bits(u32::from(p.events));
                if exact { PollEntry::exact(target, events) } else { PollEntry::new(target, events) }
            })
            .collect();
        let mut blocker = self.blocker();
        let ready = poly_vfs::poll::poll(&mut entries, &mut blocker, timeout(wait)).map_err(vfs_errno)?;
        for (p, e) in fds.iter_mut().zip(&entries) {
            p.revents = e.revents.bits() as u16;
        }
        Ok(ready)
    }

    fn bytes_readable(&mut self, fd: i32) -> Result<Option<usize>, Errno> {
        Ok(self.file(fd)?.bytes_readable())
    }

    fn epoll_create(&mut self, cloexec: bool) -> Result<i32, Errno> {
        let new = NewFd { nonblock: false, cloexec };
        self.install_object(Epoll::new(), FileType::Regular, OpenFlags::RDWR, new)
    }

    fn epoll_ctl(&mut self, epfd: i32, op: EpollOp, fd: i32, event: EpollEvent) -> Result<(), Errno> {
        let set = self.file(epfd)?;
        let file = self.file(fd)?;
        let ep = epoll_of(&set)?;
        if Arc::ptr_eq(&set, &file) {
            return Err(Errno::EINVAL);
        }
        let op = match op {
            EpollOp::Add => epoll::EpollOp::Add,
            EpollOp::Mod => epoll::EpollOp::Mod,
            EpollOp::Del => epoll::EpollOp::Del,
        };
        let event = epoll::EpollEvent { events: event.events, data: event.data };
        ep.ctl(op, fd, &file, event).map_err(vfs_errno)
    }

    fn epoll_wait(&mut self, epfd: i32, max: usize, wait: WaitTime) -> Result<Vec<EpollEvent>, Errno> {
        let set = self.file(epfd)?;
        let ep = epoll_of(&set)?;
        let mut blocker = self.blocker();
        let events = ep.wait(max, &mut blocker, timeout(wait)).map_err(vfs_errno)?;
        Ok(events.into_iter().map(|e| EpollEvent { events: e.events, data: e.data }).collect())
    }

    fn eventfd(&mut self, initval: u32, semaphore: bool, new: NewFd) -> Result<i32, Errno> {
        let counter = Arc::new(EventFd::new(u64::from(initval), semaphore));
        self.install_object(counter, FileType::Regular, OpenFlags::RDWR, new)
    }

    fn timerfd_create(&mut self, clock: Clock, new: NewFd) -> Result<i32, Errno> {
        let clock = if clock == Clock::Realtime { TimerClock::Realtime } else { TimerClock::Monotonic };
        let timer = TimerFd::new(clock, timers());
        self.install_object(timer, FileType::Regular, OpenFlags::RDWR, new)
    }

    fn timerfd_settime(&mut self, fd: i32, absolute: bool, new: &ItimerSpec) -> Result<ItimerSpec, Errno> {
        let timer = self.file(fd)?.object_as::<TimerFd>().ok_or(Errno::EINVAL)?;
        let set = TimerSpec { value_ns: to_ns(new.value), interval_ns: to_ns(new.interval) };
        Ok(spec(timer.set(absolute, set)))
    }

    fn timerfd_gettime(&mut self, fd: i32) -> Result<ItimerSpec, Errno> {
        let timer = self.file(fd)?.object_as::<TimerFd>().ok_or(Errno::EINVAL)?;
        Ok(spec(timer.get()))
    }

    fn signalfd(&mut self, fd: i32, mask: SigSet, new: NewFd) -> Result<i32, Errno> {
        if fd != -1 {
            let file = self.file(fd)?;
            let sfd = file.object_as::<SignalFd>().ok_or(Errno::EINVAL)?;
            sfd.set_mask(mask);
            return Ok(fd);
        }
        self.install_object(Arc::new(SignalFd::new(mask)), FileType::Regular, OpenFlags::RDWR, new)
    }

    fn pidfd_open(&mut self, pid: i32, thread: bool, nonblock: bool) -> Result<i32, Errno> {
        crate::linux::pidfd::install(&self.process.files, pid, thread, nonblock)
    }

    fn pidfd_target(&mut self, fd: i32) -> Result<Option<PidFdTarget>, Errno> {
        crate::linux::pidfd::target(&self.process.files, fd)
    }

    fn is_object(&mut self, fd: i32) -> Result<bool, Errno> {
        Ok(self.file(fd)?.object().is_some())
    }

    fn pipe_size(&mut self, fd: i32) -> Result<usize, Errno> {
        let end = self.file(fd)?.object_as::<PipeEnd>().ok_or(Errno::EINVAL)?;
        Ok(end.pipe().capacity())
    }

    fn pipe_free_room(&mut self, fd: i32) -> Result<usize, Errno> {
        let end = self.file(fd)?.object_as::<PipeEnd>().ok_or(Errno::EINVAL)?;
        Ok(end.pipe().free_room())
    }

    fn set_pipe_size(&mut self, fd: i32, bytes: usize) -> Result<usize, Errno> {
        let end = self.file(fd)?.object_as::<PipeEnd>().ok_or(Errno::EINVAL)?;
        end.pipe().set_capacity(bytes).map_err(vfs_errno)
    }

    fn inotify_init(&mut self, new: NewFd) -> Result<i32, Errno> {
        self.install_object(Inotify::new(), FileType::Regular, OpenFlags::RDONLY, new)
    }

    fn is_inotify(&mut self, fd: i32) -> Result<bool, Errno> {
        Ok(self.file(fd)?.object_as::<Inotify>().is_some())
    }

    fn inotify_add_watch(&mut self, fd: i32, path: &[u8], mask: u32) -> Result<i32, Errno> {
        use poly_personality_linux::sys::inotify::mask::{IN_DONT_FOLLOW, IN_ONLYDIR};
        let instance = self.file(fd)?.object_as::<Inotify>().ok_or(Errno::EINVAL)?;
        let files = &self.process.files;
        let node = files.lookupat(fs::AT_FDCWD, path, mask & IN_DONT_FOLLOW == 0).map_err(vfs_errno)?;
        node.access(&files.cred(), poly_vfs::Access::READ).map_err(vfs_errno)?;
        if mask & IN_ONLYDIR != 0 && node.kind().map_err(vfs_errno)? != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        instance.add_watch(WatchKey::of_node(&node), mask).map_err(vfs_errno)
    }

    fn inotify_rm_watch(&mut self, fd: i32, wd: i32) -> Result<(), Errno> {
        let instance = self.file(fd)?.object_as::<Inotify>().ok_or(Errno::EINVAL)?;
        instance.rm_watch(wd).map_err(vfs_errno)
    }
}
