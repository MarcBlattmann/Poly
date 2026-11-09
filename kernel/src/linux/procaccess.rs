//! Another process's resources for kcmp(2), pidfd_getfd(2) and
//! process_vm_readv(2) / process_vm_writev(2) (ADR 0121). The personality
//! (`sys::procaccess`) has found the process and checked the caller's
//! access; here are the objects.
//!
//! kcmp identities are the addresses of the kernel objects: an open file
//! description, the memory, the descriptor table, the directories and
//! umask (shared with `CLONE_FS`). The signal handler table is a number
//! (`LinuxProcess::sighand`) that a `CLONE_SIGHAND` child takes from its
//! creator, since Poly copies the handlers to it (see `task.rs`). Poly
//! has no I/O contexts and keeps semaphore undo lists
//! with the semaphores, so every process compares equal for `KCMP_IO` and
//! `KCMP_SYSVSEM`, as two processes without either do (kcmp's spec
//! sheet).

use super::services::vfs_errno;
use crate::process::{self, Process};
use crate::user::Memory;
use alloc::sync::Arc;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::kernel::KcmpResource;
use poly_vfs::epoll::Epoll;

/// Process `pid` (a thread group id), alive or a zombie.
pub fn process(pid: i32) -> Result<Arc<Process>, Errno> {
    let pid = u32::try_from(pid).map_err(|_| Errno::ESRCH)?;
    process::find(pid).or_else(|| super::task::zombie(pid)).ok_or(Errno::ESRCH)
}

/// Process `pid` while it still runs: `ESRCH` once it has ended or all its
/// threads have started to exit (`Process::is_ending`), which is when a
/// thread waiting for its `clear_child_tid` word learns of it.
pub fn live_process(pid: i32) -> Result<Arc<Process>, Errno> {
    let p = process(pid)?;
    if p.is_ending() { Err(Errno::ESRCH) } else { Ok(p) }
}

/// The memory of live process `pid`.
pub fn memory(pid: i32) -> Result<Arc<Memory>, Errno> {
    live_process(pid)?;
    crate::fs::proc::memory_of(pid as u32).ok_or(Errno::ESRCH)
}

/// The identity of `what` of process `pid` (`LinuxKernel::kcmp_id`).
pub fn kcmp_id(pid: i32, what: KcmpResource) -> Result<u64, Errno> {
    let p = process(pid)?;
    let file_id = |fd: i32| -> Result<u64, Errno> {
        let file = p.files.file(fd).map_err(vfs_errno)?;
        Ok(Arc::as_ptr(&file) as usize as u64)
    };
    Ok(match what {
        KcmpResource::File(fd) => file_id(fd)?,
        // A zombie's memory is gone: an identity no memory has.
        KcmpResource::Vm => memory(pid).map_or(u64::MAX - pid as u64, |m| Arc::as_ptr(&m) as usize as u64),
        KcmpResource::Files => p.files.table_id(),
        KcmpResource::Fs => p.files.context_id(),
        KcmpResource::Sighand => match p.linux.as_ref() {
            Some(l) => l.with(|s| s.sighand),
            None => core::ptr::from_ref(&p.linux) as usize as u64,
        },
        KcmpResource::Io | KcmpResource::SysvSem => 0,
        KcmpResource::EpollTarget { efd, tfd, toff } => {
            let file = p.files.file(efd).map_err(vfs_errno)?;
            let set = file.object_as::<Epoll>().ok_or(Errno::EINVAL)?;
            let target = set.target(tfd, toff as usize).ok_or(Errno::ENOENT)?;
            Arc::as_ptr(&target) as usize as u64
        }
    })
}
