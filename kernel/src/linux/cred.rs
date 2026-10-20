//! Credentials in the kernel (ADR 0045): each Linux thread's
//! [`Creds`](poly_personality_linux::cred::Creds) live in the
//! personality's per-thread state (`LinuxThread::creds`, behind the
//! process's lock), where clone, fork and the `set*id` calls find them.
//! The kernel reads them for every file permission check
//! ([`current_vfs_cred`], which [`crate::fs::Files`] uses), changes them at
//! execve (set-user-ID programs, capabilities: [`exec_creds`]) and serves
//! the attribute calls (`chmod`, `chown`, `access`, `utimensat`, `umask`)
//! through the process's `Files`.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::Files;
use crate::user;
use poly_personality_linux::cred::{AttrChange, Creds, ExecFile, FileTarget, FsIds, TimeChange};
use poly_personality_linux::errno::Errno;
use poly_vfs::{Access, CapSet, Cred, OpenFile, SetTime, Timespec};

/// The VFS's view of `ids`. Capability numbers are Linux's on both sides.
pub fn vfs_cred(ids: &FsIds) -> Cred {
    Cred { uid: ids.uid, gid: ids.gid, groups: ids.groups.clone(), caps: CapSet(ids.caps.0) }
}

/// The credentials file permission checks use for the running thread: a
/// Linux thread's file system ids, supplementary groups and effective
/// capabilities; root for kernel threads and threads of processes without
/// Linux state.
pub fn current_vfs_cred() -> Cred {
    current_creds().map_or_else(Cred::root, |c| vfs_cred(&c.fs_ids()))
}

/// The running thread's Linux credentials, if it is a Linux thread (its
/// process's, should the thread not be registered with the personality:
/// never root by default).
pub fn current_creds() -> Option<Creds> {
    let process = user::current_process()?;
    let tid = user::current_tid()? as i32;
    let linux = process.linux.as_ref()?;
    let pid = process.pid as i32;
    linux.with(|p| if p.has_thread(tid) { Some(p.thread(tid).creds.clone()) } else { p.process_creds(pid).cloned() })
}

/// What execve(2) of `file` makes of the calling thread's credentials, and
/// whether the program runs "securely" (`AT_SECURE`). Nothing changes
/// yet: [`commit_exec`] installs them past the point of no return.
pub fn exec_creds(k: &Thread<'_>, file: &OpenFile) -> Result<(Creds, bool), Errno> {
    let attr = file.node().attr().map_err(vfs_errno)?;
    let exec = ExecFile { uid: attr.uid, gid: attr.gid, perm: attr.perm, nosuid: file.node().mount().flags().no_suid };
    let tid = k.tid as i32;
    Ok(k.linux.with(|p| p.thread(tid).creds.exec(&exec)))
}

/// The ids the ELF loader puts in the auxiliary vector.
pub fn aux_creds(c: &Creds) -> poly_elf::Credentials {
    poly_elf::Credentials { uid: c.uid, euid: c.euid, gid: c.gid, egid: c.egid }
}

/// Install the credentials and name of thread `tid` after a successful
/// execve.
pub fn commit_exec(k: &Thread<'_>, tid: u32, creds: Creds, path: &[u8]) {
    let comm = poly_personality_linux::sys::prctl::comm_from_path(path);
    k.linux.with(|p| {
        let t = p.thread_mut(tid as i32);
        t.creds = creds;
        t.comm = comm;
    });
}

fn set_time(t: Option<TimeChange>) -> Option<SetTime> {
    t.map(|t| match t {
        TimeChange::Now => SetTime::Now,
        TimeChange::At(ts) => SetTime::At(Timespec::new(ts.sec, ts.nsec as u32)),
    })
}

/// `LinuxKernel::change_attr`.
pub fn change_attr(files: &Files, file: FileTarget<'_>, change: &AttrChange, ids: &FsIds) -> Result<(), Errno> {
    let cred = vfs_cred(ids);
    let node = match file {
        FileTarget::Path { dirfd, path, follow } => files.lookup_as(&cred, dirfd, path, follow),
        FileTarget::Fd(fd) => files
            .file(fd)
            .and_then(|f| if f.is_path_only() { Err(poly_vfs::Errno::EBADF) } else { Ok(f.node().clone()) }),
        FileTarget::EmptyPath(fd) => files.empty_path(fd),
    }
    .map_err(vfs_errno)?;
    match *change {
        AttrChange::Mode(perm) => node.chmod(&cred, perm),
        AttrChange::Owner { uid, gid } => node.chown(&cred, uid, gid),
        AttrChange::Times { atime, mtime } => node.set_times(&cred, set_time(atime), set_time(mtime)),
    }
    .map_err(vfs_errno)
}

/// `LinuxKernel::check_access`.
pub fn check_access(files: &Files, file: FileTarget<'_>, mode: u32, ids: &FsIds) -> Result<(), Errno> {
    let cred = vfs_cred(ids);
    let want = Access::from_bits(mode).map_err(vfs_errno)?;
    let node = match file {
        FileTarget::Path { dirfd, path, follow } => files.lookup_as(&cred, dirfd, path, follow),
        FileTarget::Fd(fd) | FileTarget::EmptyPath(fd) => files.empty_path(fd),
    }
    .map_err(vfs_errno)?;
    node.access(&cred, want).map_err(vfs_errno)
}
