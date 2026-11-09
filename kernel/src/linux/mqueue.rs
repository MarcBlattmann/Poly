//! POSIX message queue descriptors (mq_overview(7)): the open file of a
//! [`Queue`] of the Linux personality. Reading gives the queue's status
//! line, writing is `EINVAL`; it is readable while messages wait and
//! writable while there is room, for `poll`, `select` and `epoll`. When the
//! open file goes for good, a notification registration made through it
//! goes too.

use crate::fs::{self, Fd, FileType, OpenFile, OpenFlags};
use alloc::sync::Arc;
use core::any::Any;
use poly_personality_linux::errno::Errno as LinuxErrno;
use poly_personality_linux::ipc::mqueue::{MqFd, Queue, QueueWatch, new_file_id};
use poly_vfs::{Cred, Errno, FileObject, PollEvents, Result, WaitQueue};

/// One open file of a queue.
pub struct MqFile {
    queue: Arc<Queue>,
    id: u64,
    waiters: Arc<WaitQueue>,
}

/// The wait queue every open file of a queue shares, told about each
/// change of its messages.
struct PollWatch(Arc<WaitQueue>);

impl QueueWatch for PollWatch {
    fn changed(&self) {
        self.0.notify(PollEvents::IN | PollEvents::RDNORM | PollEvents::OUT | PollEvents::WRNORM);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl MqFile {
    fn new(queue: Arc<Queue>) -> MqFile {
        let watch = queue.watch_or_init(|| Arc::new(PollWatch(Arc::new(WaitQueue::new()))));
        let waiters = match watch.as_any().downcast_ref::<PollWatch>() {
            Some(w) => w.0.clone(),
            None => Arc::new(WaitQueue::new()),
        };
        MqFile { queue, id: new_file_id(), waiters }
    }
}

impl Drop for MqFile {
    fn drop(&mut self) {
        self.queue.file_closed(self.id);
    }
}

impl FileObject for MqFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let line = self.queue.status_line();
        let n = line.len().min(buf.len());
        buf[..n].copy_from_slice(&line[..n]);
        Ok(n)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        let (readable, writable) = self.queue.readiness();
        let mut events = PollEvents::NONE;
        if readable {
            events = events | PollEvents::IN | PollEvents::RDNORM;
        }
        if writable {
            events = events | PollEvents::OUT | PollEvents::WRNORM;
        }
        events
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.waiters)
    }

    fn seek_stays_at_zero(&self) -> bool {
        true
    }
}

/// `FileIo::mq_install`: `queue` on a new regular-file inode with its mode
/// and owner, open with `flags`, close-on-exec, on the lowest free
/// descriptor.
pub fn install(files: &fs::Files, queue: Arc<Queue>, flags: u32) -> core::result::Result<Fd, LinuxErrno> {
    let owner = Cred { uid: queue.uid, gid: queue.gid, ..Cred::root() };
    let node = fs::vfs().anon_node(FileType::Regular, queue.mode, &owner).map_err(super::services::vfs_errno)?;
    let object: Arc<dyn FileObject> = Arc::new(MqFile::new(queue));
    let file = Arc::new(OpenFile::on_object(node, object, OpenFlags::from_bits(flags)));
    files.install(file, true).map_err(super::services::vfs_errno)
}

/// `FileIo::mq_file`: the queue `fd` is open on, if it is a queue.
pub fn file(files: &fs::Files, fd: Fd) -> core::result::Result<Option<MqFd>, LinuxErrno> {
    let file = files.file(fd).map_err(super::services::vfs_errno)?;
    Ok(file.object_as::<MqFile>().map(|m| MqFd { queue: m.queue.clone(), file: m.id, flags: file.flags().bits() }))
}
