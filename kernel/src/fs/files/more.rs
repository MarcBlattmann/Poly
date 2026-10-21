//! More of [`Files`]: the calls the Linux personality's file system
//! services need beyond ADR 0037's set (ADR 0043), and what procfs reads
//! about a process's files.

use super::Files;
use crate::fs::vfs;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{
    AT_FDCWD, Access, At, Cred, Errno, Fd, FsContext, FsStats, NodeRef, OpenFile, PathArg, RenameFlags, Result,
};

/// A path and where it starts ([`Files::start`]) as a [`PathArg`]: their
/// errors are left to the VFS, which reports them in their place.
fn path_arg<'a>(path: Result<&'a [u8]>, start: &'a Result<Option<NodeRef>>) -> PathArg<'a> {
    PathArg { path, at: start.as_ref().map(|s| s.as_ref().map_or(At::Cwd, At::Node)).map_err(|e| *e) }
}

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

    /// Set the soft `RLIMIT_NOFILE`: descriptors at or above it cannot be
    /// opened.
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
        self.at(dirfd, path, |vfs, ctx, at| vfs.lookup(ctx, &self.cred(), at, path, follow))
    }

    /// `faccessat2(2)` as `cred`; an empty `path` checks `dirfd`'s file.
    /// The caller picks `cred`: access(2) checks with the real ids, and
    /// with `AT_EACCESS` the effective ones (ADR 0045).
    pub fn accessat(&self, cred: &Cred, dirfd: Fd, path: &[u8], want: Access, follow: bool) -> Result<()> {
        if path.is_empty() {
            return self.node_at(dirfd)?.access(cred, want);
        }
        self.at(dirfd, path, |vfs, ctx, at| vfs.access(ctx, cred, at, path, want, follow))
    }

    /// `renameat2(2)`. `new` is the second path or why the caller could
    /// not read it; that error and a bad `newdirfd` come in their place in
    /// the order of path errors ([`poly_vfs::Vfs::rename_paths`]).
    pub fn renameat(
        &self,
        olddirfd: Fd,
        old: &[u8],
        newdirfd: Fd,
        new: Result<&[u8]>,
        flags: RenameFlags,
    ) -> Result<()> {
        let o = self.start(olddirfd, old);
        let n = new.and_then(|new| self.start(newdirfd, new));
        let ctx = self.context();
        vfs().rename_paths(&ctx, &self.cred(), path_arg(Ok(old), &o), path_arg(new, &n), flags)
    }

    /// `linkat(2)`, `new` as for [`renameat`](Self::renameat): every error
    /// of `old` comes first ([`poly_vfs::Vfs::link_paths`]). An empty `old`
    /// (`AT_EMPTY_PATH`) names the file `olddirfd` is open on.
    pub fn linkat(&self, olddirfd: Fd, old: &[u8], newdirfd: Fd, new: Result<&[u8]>, follow: bool) -> Result<()> {
        let n = new.and_then(|new| self.start(newdirfd, new));
        let ctx = self.context();
        if old.is_empty() {
            // AT_EMPTY_PATH (the personality passes an empty path only
            // with it): the file `olddirfd` is open on. linkat(2): it needs
            // CAP_DAC_READ_SEARCH, except to name an O_TMPFILE file (the
            // spec of open: an unprivileged caller names its own).
            let file = if olddirfd == AT_FDCWD { None } else { self.file(olddirfd).ok() };
            let tmpfile = file.as_ref().is_some_and(|f| f.is_tmpfile());
            if !tmpfile && !self.cred().has(poly_vfs::cap::DAC_READ_SEARCH) {
                return Err(Errno::ENOENT);
            }
            if let Some(file) = file {
                return vfs().link_file(&ctx, &self.cred(), &file, path_arg(new, &n));
            }
            let node = self.node_at(olddirfd)?;
            return vfs().link_node(&ctx, &self.cred(), &node, path_arg(new, &n));
        }
        let o = self.start(olddirfd, old);
        vfs().link_paths(&ctx, &self.cred(), path_arg(Ok(old), &o), path_arg(new, &n), follow)
    }

    /// `fchmodat(2)` (following links: the system call has no flags).
    pub fn fchmodat(&self, dirfd: Fd, path: &[u8], perm: u16) -> Result<()> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.chmod(ctx, &self.cred(), at, path, perm, true))
    }

    /// `fchmod(2)`: not on an `O_PATH` descriptor (`EBADF`).
    pub fn fchmod(&self, fd: Fd, perm: u16) -> Result<()> {
        let file = self.file(fd)?;
        if file.is_path_only() {
            return Err(Errno::EBADF);
        }
        file.node().chmod(&self.cred(), perm)
    }

    /// `truncate(2)`.
    pub fn truncate(&self, path: &[u8], len: u64) -> Result<()> {
        self.at(AT_FDCWD, path, |vfs, ctx, at| vfs.truncate(ctx, &self.cred(), at, path, len))
    }

    /// `statfs(2)`.
    pub fn statfs(&self, path: &[u8]) -> Result<FsStats> {
        self.at(AT_FDCWD, path, |vfs, ctx, at| vfs.lookup(ctx, &self.cred(), at, path, true))?.statfs()
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
