//! More of [`Files`]: the calls the Linux personality's file system
//! services need beyond ADR 0037's set (ADR 0043), and what procfs reads
//! about a process's files.

use super::Files;
use crate::fs::vfs;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{AT_FDCWD, Access, At, Errno, Fd, FsContext, FsStats, NodeRef, OpenFile, RenameFlags, Result};

impl Files {
    /// A copy of the context: root, working directory, umask.
    pub fn context(&self) -> FsContext {
        self.ctx.lock().clone()
    }

    /// The file creation mask, for `/proc/<pid>/status`. Read without
    /// waiting: the reader may be this process in the middle of a
    /// `chdir` into `/proc`, holding the context; then the default
    /// (`022`) is reported.
    pub fn current_umask(&self) -> u16 {
        self.ctx.try_lock().map_or(FsContext::DEFAULT_UMASK, |c| c.umask())
    }

    /// Every open descriptor with its file and close-on-exec flag, in
    /// increasing order.
    pub fn fd_list(&self) -> Vec<(Fd, Arc<OpenFile>, bool)> {
        self.fds.lock().iter().map(|(fd, f, cloexec)| (fd, f.clone(), cloexec)).collect()
    }

    /// The soft `RLIMIT_NOFILE`: descriptors at or above it cannot be
    /// opened.
    pub fn fd_limit(&self) -> usize {
        self.fds.lock().limit()
    }

    pub fn set_fd_limit(&self, limit: usize) {
        self.fds.lock().set_limit(limit);
    }

    /// The node `fd` is open on, or the working directory for
    /// [`AT_FDCWD`]: what an empty path with `AT_EMPTY_PATH` means.
    pub fn node_at(&self, dirfd: Fd) -> Result<NodeRef> {
        if dirfd == AT_FDCWD { Ok(self.ctx.lock().cwd().clone()) } else { Ok(self.file(dirfd)?.node().clone()) }
    }

    /// Where a relative path starts: `None` for the working directory (or
    /// an absolute path), else the node `dirfd` is open on.
    fn start(&self, dirfd: Fd, path: &[u8]) -> Result<Option<NodeRef>> {
        if dirfd == AT_FDCWD || path.first() == Some(&b'/') {
            Ok(None)
        } else {
            Ok(Some(self.file(dirfd)?.node().clone()))
        }
    }

    /// The node `path` names from `dirfd` (`stat(2)` and friends).
    pub fn lookupat(&self, dirfd: Fd, path: &[u8], follow: bool) -> Result<NodeRef> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.lookup(ctx, &self.cred, at, path, follow))
    }

    /// `faccessat2(2)`; an empty `path` checks `dirfd`'s file. `cred` is
    /// the process's (real and effective ids are the same until users
    /// arrive, so `AT_EACCESS` changes nothing).
    pub fn accessat(&self, dirfd: Fd, path: &[u8], want: Access, follow: bool) -> Result<()> {
        if path.is_empty() {
            return self.node_at(dirfd)?.access(&self.cred, want);
        }
        self.at(dirfd, path, |vfs, ctx, at| vfs.access(ctx, &self.cred, at, path, want, follow))
    }

    /// `renameat2(2)`.
    pub fn renameat(&self, olddirfd: Fd, old: &[u8], newdirfd: Fd, new: &[u8], flags: RenameFlags) -> Result<()> {
        let o = self.start(olddirfd, old)?;
        let n = self.start(newdirfd, new)?;
        let ctx = self.context();
        vfs().rename(
            &ctx,
            &self.cred,
            o.as_ref().map_or(At::Cwd, At::Node),
            old,
            n.as_ref().map_or(At::Cwd, At::Node),
            new,
            flags,
        )
    }

    /// `linkat(2)`. An empty `old` (`AT_EMPTY_PATH`, meant for files made
    /// with `O_TMPFILE`, which Poly does not have) is `ENOENT`.
    pub fn linkat(&self, olddirfd: Fd, old: &[u8], newdirfd: Fd, new: &[u8], follow: bool) -> Result<()> {
        if old.is_empty() {
            return Err(Errno::ENOENT);
        }
        let o = self.start(olddirfd, old)?;
        let n = self.start(newdirfd, new)?;
        let ctx = self.context();
        vfs().link(
            &ctx,
            &self.cred,
            o.as_ref().map_or(At::Cwd, At::Node),
            old,
            n.as_ref().map_or(At::Cwd, At::Node),
            new,
            follow,
        )
    }

    /// `fchmodat(2)` (following links: the system call has no flags).
    pub fn fchmodat(&self, dirfd: Fd, path: &[u8], perm: u16) -> Result<()> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.chmod(ctx, &self.cred, at, path, perm, true))
    }

    /// `fchmod(2)`.
    pub fn fchmod(&self, fd: Fd, perm: u16) -> Result<()> {
        self.file(fd)?.node().chmod(&self.cred, perm)
    }

    /// `truncate(2)`.
    pub fn truncate(&self, path: &[u8], len: u64) -> Result<()> {
        self.at(AT_FDCWD, path, |vfs, ctx, at| vfs.truncate(ctx, &self.cred, at, path, len))
    }

    /// `statfs(2)`.
    pub fn statfs(&self, path: &[u8]) -> Result<FsStats> {
        self.at(AT_FDCWD, path, |vfs, ctx, at| vfs.lookup(ctx, &self.cred, at, path, true))?.statfs()
    }

    /// `fstatfs(2)`.
    pub fn fstatfs(&self, fd: Fd) -> Result<FsStats> {
        self.file(fd)?.node().statfs()
    }

    /// `syncfs(2)`: the file system `fd` is on.
    pub fn syncfs(&self, fd: Fd) -> Result<()> {
        self.file(fd)?.node().mount().fs().sync()
    }

    /// `readlinkat(2)` with an empty path: the link `dirfd` is open on
    /// (`O_PATH | O_NOFOLLOW`); `ENOENT` for anything else, as readlink(2)
    /// says for an empty path.
    pub fn readlink_fd(&self, dirfd: Fd) -> Result<Vec<u8>> {
        if dirfd == AT_FDCWD {
            return Err(Errno::ENOENT);
        }
        self.file(dirfd)?.node().readlink().map_err(|_| Errno::ENOENT)
    }
}
