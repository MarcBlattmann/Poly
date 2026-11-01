//! The kernel's side of Landlock and fanotify (ADR 0122): the open file a
//! Landlock ruleset descriptor is, and the hooks fanotify uses to report
//! the acting process and give its reader descriptors for events. The
//! rules themselves are the personality's (`poly_personality_linux::landlock`),
//! the groups and marks the VFS's (`poly_vfs::fanotify`).

use alloc::sync::Arc;
use poly_personality_linux::landlock::{ObjectKey, Ruleset};
use poly_vfs::fanotify::{Hooks, set_hooks};
use poly_vfs::inotify::WatchKey;
use poly_vfs::{Errno, FileObject, NodeRef, OpenFlags, PollEvents, Result, WaitQueue};

/// What a Landlock ruleset descriptor is open on: an anonymous inode
/// (`[landlock-ruleset]`) whose reads and writes are `EINVAL`.
pub struct RulesetFile(pub Arc<Ruleset>);

impl FileObject for RulesetFile {
    fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        PollEvents::NONE
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        None
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[landlock-ruleset]")
    }
}

/// The personality's name for an object: the file system's identity and
/// the inode number.
pub fn object_key(key: WatchKey) -> ObjectKey {
    let (fs, ino) = key.parts();
    (fs as u64, ino)
}

/// The pid fanotify events report: the running process's.
fn actor_pid() -> i32 {
    crate::user::current_process().map_or(0, |p| p.pid as i32)
}

/// A descriptor of the running process for an event's object (fanotify's
/// reader): opened quietly with the group's `event_f_flags` (plus
/// `O_LARGEFILE`, as every x86-64 open), `FD_CLOEXEC` if they ask for it.
fn open_event_fd(node: &NodeRef, flags: OpenFlags) -> Result<i32> {
    let process = crate::user::current_process().ok_or(Errno::ESRCH)?;
    let file = crate::fs::vfs().open_node_quiet(node, flags.union(OpenFlags::LARGEFILE))?;
    process.files.install(file, flags.contains(OpenFlags::CLOEXEC))
}

/// Install fanotify's hooks (at boot, with the namespaces').
pub fn init() {
    set_hooks(Hooks { actor_pid, open_event_fd });
}
