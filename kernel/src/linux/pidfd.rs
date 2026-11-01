//! Process descriptors (pidfd_open(2), clone3's `CLONE_PIDFD`): a
//! [`FileObject`] naming a process, or with `PIDFD_THREAD` a thread, by
//! its global id. Ids are never handed out again (`process::alloc_pid`),
//! so the id alone says which process it was, also after it was reaped.
//!
//! The file is readable (`POLLIN`) once its process has ended: a zombie,
//! or gone from the process tree. Every change of the tree notifies one
//! wait queue all pidfds share ([`changed`]); a waiter then looks at its
//! own process, as for signal files (`signalfd.rs`). Reads and writes are
//! `EINVAL`, as for any file that has no data.

use super::task;
use crate::fs::{self, Fd, FileType, OpenFile, OpenFlags};
use crate::process;
use crate::sync::SetOnce;
use alloc::sync::Arc;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fileio::PidFdTarget;
use poly_vfs::{FileObject, PollEvents, Result, WaitQueue};

static QUEUE: SetOnce<Arc<WaitQueue>> = SetOnce::new();

/// The queue every pidfd's waiters register on.
fn queue() -> &'static Arc<WaitQueue> {
    if let Some(q) = QUEUE.get() {
        return q;
    }
    let _ = QUEUE.set(Arc::new(WaitQueue::new()));
    loop {
        // Another thread may be storing its queue this very moment.
        if let Some(q) = QUEUE.get() {
            return q;
        }
        core::hint::spin_loop();
    }
}

/// The process tree changed (a process ended or was reaped) or a thread
/// ended: waiters on pidfds look again.
pub fn changed() {
    if let Some(q) = QUEUE.get() {
        q.notify(PollEvents::READABLE);
    }
}

/// One pidfd.
#[derive(Debug)]
pub struct PidFd {
    pid: i32,
    thread: bool,
    queue: Arc<WaitQueue>,
}

impl PidFd {
    pub fn new(pid: i32, thread: bool) -> PidFd {
        PidFd { pid, thread, queue: Arc::clone(queue()) }
    }

    /// Whether the process (or thread) has ended.
    fn ended(&self) -> bool {
        if task::exited_or_gone(self.pid) {
            // A thread that leads no group is no process of the tree.
            return !self.thread || u32::try_from(self.pid).ok().and_then(process::find_thread).is_none();
        }
        false
    }
}

impl FileObject for PidFd {
    fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.ended() { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[pidfd]")
    }
}

/// A pidfd for `pid` on the lowest free descriptor of `files`: `O_RDWR`,
/// `O_NONBLOCK` with `nonblock`, always close-on-exec (pidfd_open(2)).
pub fn install(files: &fs::Files, pid: i32, thread: bool, nonblock: bool) -> core::result::Result<Fd, Errno> {
    let node = fs::vfs().anon_node(FileType::Regular, 0o700, &files.cred()).map_err(super::services::vfs_errno)?;
    let flags = if nonblock { OpenFlags::RDWR | OpenFlags::NONBLOCK } else { OpenFlags::RDWR };
    let file = Arc::new(OpenFile::on_object(node, Arc::new(PidFd::new(pid, thread)), flags));
    files.install(file, true).map_err(super::services::vfs_errno)
}

/// What descriptor `fd` of `files` names: a pidfd, a `/proc/<pid>`
/// directory, or neither (`EBADF` if it is not open).
pub fn target(files: &fs::Files, fd: Fd) -> core::result::Result<Option<PidFdTarget>, Errno> {
    let file = files.file(fd).map_err(super::services::vfs_errno)?;
    if let Some(p) = file.object_as::<PidFd>() {
        return Ok(Some(PidFdTarget::PidFd { pid: p.pid, thread: p.thread }));
    }
    if file.node().mount().fs().info().name == "proc"
        && let Some(pid) = poly_procfs::process_dir(file.node().ino())
    {
        return Ok(Some(PidFdTarget::ProcDir(pid as i32)));
    }
    Ok(None)
}

/// `F_GETFL` of a pidfd shows `PIDFD_THREAD` as `O_EXCL` (pidfd_open's
/// spec sheet), which open files do not keep.
pub fn extra_status_flags(file: &OpenFile) -> u32 {
    match file.object_as::<PidFd>() {
        Some(p) if p.thread => OpenFlags::EXCL.bits(),
        _ => 0,
    }
}
