//! The message queue file system (mq_overview(7)) over the POSIX message
//! queues of the Linux personality (ADR 0178): `poly_procfs::MqueueFs`
//! shows what [`NsQueues`] says, the queue names of one IPC namespace.
//!
//! A mount made by mount(2) shows the IPC namespace of the process that
//! mounts it, for as long as it is mounted ([`for_current`]); the one the
//! kernel mounts on `/dev/mqueue` at boot shows the initial namespace's.

use crate::user;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::ipc::mqueue::{self, NAME_MAX};
use poly_personality_linux::ns::IpcNs;
use poly_procfs::{MqueueFs, MqueueSource, QueueEntry};
use poly_vfs::{Errno, FileSystem, Result};

/// The queue names of an IPC namespace (`None`: the initial one). Holding
/// the namespace keeps its queues alive while the file system is mounted.
struct NsQueues(Option<Arc<IpcNs>>);

fn errno(e: poly_personality_linux::errno::Errno) -> Errno {
    Errno::from_code(i32::from(e.get())).unwrap_or(Errno::EIO)
}

impl MqueueSource for NsQueues {
    fn queues(&self) -> Vec<QueueEntry> {
        // The names under the table's lock, each queue's size after it.
        let entries = mqueue::with_names(&self.0, |t| t.entries());
        entries
            .into_iter()
            .map(|(name, q)| QueueEntry {
                name,
                id: q.id,
                perm: q.mode,
                uid: q.uid,
                gid: q.gid,
                size: q.bytes() as u64,
            })
            .collect()
    }

    fn status(&self, id: u64) -> Option<Vec<u8>> {
        mqueue::with_names(&self.0, |t| t.by_id(id)).map(|q| q.status_line())
    }

    fn unlink(&self, name: &[u8]) -> Result<()> {
        mqueue::with_names(&self.0, |t| t.remove(name)).map_err(errno)
    }

    fn create(&self, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<u64> {
        if name.len() > NAME_MAX {
            return Err(Errno::ENAMETOOLONG);
        }
        mqueue::with_names(&self.0, |t| {
            if t.get(name).is_some() {
                return Err(Errno::EEXIST);
            }
            t.create(name, (perm, uid, gid), None, false).map(|q| q.id).map_err(errno)
        })
    }
}

/// A message queue file system over IPC namespace `ns` (`None`: the
/// initial one).
pub fn new(ns: Option<Arc<IpcNs>>) -> Arc<dyn FileSystem> {
    Arc::new(MqueueFs::new(Arc::new(NsQueues(ns)), super::clock().now()))
}

/// A message queue file system over the IPC namespace of the running
/// process (the initial one for a kernel thread or a process without
/// Linux state): what mount(2) of type `mqueue` mounts.
pub fn for_current() -> Arc<dyn FileSystem> {
    new(user::current_process().and_then(|p| p.linux.as_ref().and_then(|l| l.with(|s| s.ns.ipc.clone()))))
}
