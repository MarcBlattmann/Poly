//! A process's open files: the kernel side of `open`, `read`, `write`,
//! `close`, `lseek`, `dup`, `fstat`, `getdents` and friends.
//!
//! Every call works on kernel byte buffers and returns a [`poly_vfs::Errno`]
//! (Linux's numbers) on failure. Copying from and to user memory, and
//! encoding results (`struct stat`, `linux_dirent64`), is the system call
//! layer's job: a page fault on user memory never happens with a lock of
//! this module or the VFS held. Flags and `whence` values are Linux's
//! (`OpenFlags::from_bits`, `Whence::from_linux`); other personalities
//! translate theirs.
//!
//! Descriptor semantics come from `poly_vfs::FdTable` (host-tested): the
//! lowest free number, `dup`/`dup2`/`dup3`, the close-on-exec flag per
//! descriptor, shared offsets between duplicates.

use super::vfs;
use crate::sync::Mutex;

mod more;
mod mounts;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{
    AT_FDCWD, At, Cred, DirEntry, Errno, Fd, FdTable, FileType, FsContext, NodeRef, OpenFile, OpenFlags, Result, Stat,
    Whence,
};

/// `AT_REMOVEDIR` (Linux UAPI `linux/fcntl.h`): `unlinkat` removes a
/// directory.
pub const AT_REMOVEDIR: u32 = 0x200;

mod perm;

/// One process's file state: its descriptor table and its root and working
/// directories and umask ([`FsContext`]). Permission checks use the
/// credentials of the thread that calls ([`Files::cred`], ADR 0045).
/// Locks: the context, then the table; neither is held during a read or
/// write.
#[derive(Debug)]
pub struct Files {
    ctx: Mutex<FsContext>,
    fds: Mutex<FdTable>,
}

impl Default for Files {
    fn default() -> Files {
        Files::new()
    }
}

impl Files {
    /// No descriptors, root and working directory at `/`, umask 022.
    pub fn new() -> Files {
        let ctx = vfs().root_context().expect("the root directory is always there");
        Files { ctx: Mutex::new(ctx), fds: Mutex::new(FdTable::new()) }
    }

    /// What a new process starts with: `/dev/console` open for reading and
    /// writing on descriptors 0, 1 and 2, all three one open file
    /// description (as `init` gets them). Without a console (a root with
    /// no `/dev/console`), no descriptors.
    pub fn with_console() -> Files {
        let files = Files::new();
        match files.open(b"/dev/console", OpenFlags::RDWR | OpenFlags::NOCTTY, 0) {
            Ok(0) => {
                let ok = files.dup(0).is_ok() && files.dup(0).is_ok();
                debug_assert!(ok, "fds 1 and 2 are free");
            }
            Ok(fd) => debug_assert!(false, "a new table gave fd {fd} first"),
            Err(e) => crate::kprintln!("fs: a new process gets no console: /dev/console: {}", e.name()),
        }
        files
    }

    /// A copy for `fork`: the same open file descriptions (offsets shared),
    /// close-on-exec flags, directories and umask.
    pub fn fork(&self) -> Files {
        let ctx = self.ctx.lock().clone();
        let fds = self.fds.lock().clone();
        Files { ctx: Mutex::new(ctx), fds: Mutex::new(fds) }
    }

    /// The credentials permission checks use: the calling thread's file
    /// system ids, groups and effective capabilities (a Linux thread's,
    /// `linux::cred`), root for the kernel's own threads.
    pub fn cred(&self) -> Cred {
        crate::linux::cred::current_vfs_cred()
    }

    // ---- descriptors ----------------------------------------------------

    /// The open file behind `fd` (`EBADF`), for callers that need more than
    /// this API offers (mapping a file, exec, waiting on a device).
    pub fn file(&self, fd: Fd) -> Result<Arc<OpenFile>> {
        self.fds.lock().get(fd).cloned()
    }

    /// Open descriptors.
    pub fn count(&self) -> usize {
        self.fds.lock().count()
    }

    /// Put `file` at the lowest free descriptor: an open file that does
    /// not come from a path (a pipe end, an event file; ADR 0042).
    pub fn install(&self, file: Arc<OpenFile>, cloexec: bool) -> Result<Fd> {
        self.fds.lock().install(file, cloexec)
    }

    /// The most descriptors the process may have (`RLIMIT_NOFILE`).
    pub fn fd_limit(&self) -> usize {
        self.fds.lock().limit()
    }

    /// `close(2)`.
    pub fn close(&self, fd: Fd) -> Result<()> {
        // The file is dropped after the table's lock is released.
        let file = self.fds.lock().close(fd)?;
        drop(file);
        Ok(())
    }

    /// `dup(2)`: the lowest free descriptor, close-on-exec clear.
    pub fn dup(&self, fd: Fd) -> Result<Fd> {
        self.fds.lock().dup(fd)
    }

    /// `dup2(2)`.
    pub fn dup2(&self, old: Fd, new: Fd) -> Result<Fd> {
        let replaced = {
            let mut fds = self.fds.lock();
            let replaced = fds.get(new).ok().cloned();
            fds.dup2(old, new)?;
            replaced
        };
        // A replaced file's last reference goes here, unlocked.
        drop(replaced);
        Ok(new)
    }

    /// `dup3(2)`: `flags` may hold only `O_CLOEXEC`.
    pub fn dup3(&self, old: Fd, new: Fd, flags: OpenFlags) -> Result<Fd> {
        let replaced = {
            let mut fds = self.fds.lock();
            let replaced = fds.get(new).ok().cloned();
            fds.dup3(old, new, flags)?;
            replaced
        };
        drop(replaced);
        Ok(new)
    }

    /// `fcntl(F_DUPFD)` / `fcntl(F_DUPFD_CLOEXEC)`: the lowest free
    /// descriptor at or above `min`.
    pub fn dup_from(&self, fd: Fd, min: Fd, cloexec: bool) -> Result<Fd> {
        self.fds.lock().dup_from(fd, min, cloexec)
    }

    /// `fcntl(F_GETFD)`: whether close-on-exec is set.
    pub fn cloexec(&self, fd: Fd) -> Result<bool> {
        self.fds.lock().cloexec(fd)
    }

    /// `fcntl(F_SETFD)`.
    pub fn set_cloexec(&self, fd: Fd, cloexec: bool) -> Result<()> {
        self.fds.lock().set_cloexec(fd, cloexec)
    }

    /// `fcntl(F_GETFL)`: access mode and status flags.
    pub fn status_flags(&self, fd: Fd) -> Result<OpenFlags> {
        Ok(self.file(fd)?.flags())
    }

    /// `fcntl(F_SETFL)`: changes `O_APPEND`, `O_NONBLOCK`, `O_ASYNC`,
    /// `O_DIRECT`, `O_NOATIME`; ignores the rest. Turning `O_NOATIME` on
    /// needs ownership or `CAP_FOWNER` (`EPERM`, as for open(2)).
    pub fn set_status_flags(&self, fd: Fd, flags: OpenFlags) -> Result<()> {
        let file = self.file(fd)?;
        self.check_noatime(&file, flags)?;
        file.set_flags(flags);
        Ok(())
    }

    /// At `execve`: close every descriptor marked close-on-exec.
    pub fn close_on_exec(&self) {
        let closed = {
            let mut fds = self.fds.lock();
            let before = fds.clone();
            fds.close_on_exec();
            before
        };
        // The old table's references go unlocked.
        drop(closed);
    }

    /// `close_range(2)`: close (or, with `cloexec`, mark close-on-exec)
    /// the open descriptors from `first` to `last`.
    pub fn close_range(&self, first: u32, last: u32, cloexec: bool) {
        let closed = self.fds.lock().close_range(first, last, cloexec);
        // The files' last references go unlocked.
        drop(closed);
    }

    /// At exit: close everything.
    pub fn close_all(&self) {
        let old = core::mem::take(&mut *self.fds.lock());
        drop(old);
    }

    // ---- I/O on descriptors (no lock held during the transfer) ------------

    /// `read(2)`: up to `buf.len()` bytes from the offset; 0 at end of file.
    pub fn read(&self, fd: Fd, buf: &mut [u8]) -> Result<usize> {
        self.file(fd)?.read(buf)
    }

    /// `write(2)`.
    pub fn write(&self, fd: Fd, buf: &[u8]) -> Result<usize> {
        let file = self.file(fd)?;
        self.before_write(&file, buf.len())?;
        file.write(buf)
    }

    /// `pread64(2)`: at `offset`, leaving the file offset alone.
    pub fn pread(&self, fd: Fd, buf: &mut [u8], offset: u64) -> Result<usize> {
        self.file(fd)?.read_at(offset, buf)
    }

    /// `pwrite64(2)`.
    pub fn pwrite(&self, fd: Fd, buf: &[u8], offset: u64) -> Result<usize> {
        let file = self.file(fd)?;
        self.before_write(&file, buf.len())?;
        file.write_at(offset, buf)
    }

    /// `lseek(2)`: returns the new offset.
    pub fn lseek(&self, fd: Fd, offset: i64, whence: Whence) -> Result<u64> {
        self.file(fd)?.seek(offset, whence)
    }

    /// `fstat(2)`.
    pub fn fstat(&self, fd: Fd) -> Result<Stat> {
        self.file(fd)?.stat()
    }

    /// `getdents64(2)` without the encoding: lists the directory open as
    /// `fd` from its offset, calling `emit` for each entry (`.` and `..`
    /// included) until it returns `false` (no room: that entry comes again
    /// next time) or the directory ends. Returns the entries accepted; 0 at
    /// the end. `emit` runs with the file's offset lock held (POSIX makes
    /// the offset update atomic), so it must not use `fd` itself.
    pub fn getdents(&self, fd: Fd, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<usize> {
        self.file(fd)?.read_dir(emit)
    }

    /// `ioctl(2)` on a device; `ENOTTY` for anything else.
    pub fn ioctl(&self, fd: Fd, request: u32, arg: u64) -> Result<u64> {
        self.file(fd)?.ioctl(request, arg)
    }

    /// `ftruncate(2)`.
    pub fn ftruncate(&self, fd: Fd, len: u64) -> Result<()> {
        let file = self.file(fd)?;
        self.before_write(&file, 1)?;
        file.truncate(len)
    }

    /// `fsync(2)`.
    pub fn fsync(&self, fd: Fd) -> Result<()> {
        self.file(fd)?.sync()
    }

    // ---- paths ----------------------------------------------------------

    /// Runs `f` with the VFS, the context and where `path` starts for a
    /// `*at` call: the working directory for [`AT_FDCWD`] or an absolute
    /// path (which ignores `dirfd`, even a bad one), else the node `dirfd`
    /// is open on (`EBADF`). The walk runs on a copy of the context, with
    /// no lock of this `Files` held: following `/proc/self/cwd` asks for
    /// this very context (ADR 0043).
    fn at<R>(
        &self,
        dirfd: Fd,
        path: &[u8],
        f: impl FnOnce(&poly_vfs::Vfs, &FsContext, At<'_>) -> Result<R>,
    ) -> Result<R> {
        let dir: Option<NodeRef> = if dirfd == AT_FDCWD || path.first() == Some(&b'/') {
            None
        } else {
            Some(self.fds.lock().get(dirfd)?.node().clone())
        };
        let ctx = self.ctx.lock().clone();
        f(vfs(), &ctx, dir.as_ref().map_or(At::Cwd, At::Node))
    }

    /// `open(2)`: [`openat`](Self::openat) from the working directory.
    pub fn open(&self, path: &[u8], flags: OpenFlags, mode: u16) -> Result<Fd> {
        self.openat(AT_FDCWD, path, flags, mode)
    }

    /// `openat(2)`: opens `path` and puts it at the lowest free descriptor,
    /// close-on-exec if `flags` has `O_CLOEXEC`. `mode` (masked by the
    /// umask) is used when `O_CREAT` makes the file.
    pub fn openat(&self, dirfd: Fd, path: &[u8], flags: OpenFlags, mode: u16) -> Result<Fd> {
        let file = self.at(dirfd, path, |vfs, ctx, at| vfs.open(ctx, &self.cred(), at, path, flags, mode))?;
        self.fds.lock().install(file, flags.contains(OpenFlags::CLOEXEC))
    }

    /// `execve(2)`: open `path` (from `dirfd`, as for `openat`) to run it:
    /// a regular file with execute permission on a mount that allows
    /// executing (`EACCES` otherwise). Not installed as a descriptor.
    pub fn open_exec_at(&self, dirfd: Fd, path: &[u8]) -> Result<Arc<OpenFile>> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.open_exec(ctx, &self.cred(), at, path))
    }

    /// `stat(2)` (`follow`) and `lstat(2)`, `fstatat(2)` with `dirfd`.
    pub fn statat(&self, dirfd: Fd, path: &[u8], follow: bool) -> Result<Stat> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.stat(ctx, &self.cred(), at, path, follow))
    }

    /// `stat(2)` from the working directory, following a final symbolic link.
    pub fn stat(&self, path: &[u8]) -> Result<Stat> {
        self.statat(AT_FDCWD, path, true)
    }

    /// `lstat(2)`: the link itself.
    pub fn lstat(&self, path: &[u8]) -> Result<Stat> {
        self.statat(AT_FDCWD, path, false)
    }

    /// `readlinkat(2)`: the whole target (the caller truncates to the user's
    /// buffer, as `readlink` does).
    pub fn readlinkat(&self, dirfd: Fd, path: &[u8]) -> Result<Vec<u8>> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.readlink(ctx, &self.cred(), at, path))
    }

    /// `mkdirat(2)`.
    pub fn mkdirat(&self, dirfd: Fd, path: &[u8], mode: u16) -> Result<()> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.mkdir(ctx, &self.cred(), at, path, mode))
    }

    /// `unlinkat(2)`: with [`AT_REMOVEDIR`], `rmdir`; other flags `EINVAL`.
    pub fn unlinkat(&self, dirfd: Fd, path: &[u8], flags: u32) -> Result<()> {
        match flags {
            0 => self.at(dirfd, path, |vfs, ctx, at| vfs.unlink(ctx, &self.cred(), at, path)),
            AT_REMOVEDIR => self.at(dirfd, path, |vfs, ctx, at| vfs.rmdir(ctx, &self.cred(), at, path)),
            _ => Err(Errno::EINVAL),
        }
    }

    /// `symlinkat(2)`: a link at `path` to `target`.
    pub fn symlinkat(&self, target: &[u8], dirfd: Fd, path: &[u8]) -> Result<()> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.symlink(ctx, &self.cred(), target, at, path))
    }

    /// `chdir(2)`.
    pub fn chdir(&self, path: &[u8]) -> Result<()> {
        // Resolved unlocked, like every walk (see `at`).
        let dir = self.at(AT_FDCWD, path, |vfs, ctx, at| vfs.lookup(ctx, &self.cred(), at, path, true))?;
        self.ctx.lock().set_cwd(&self.cred(), dir)
    }

    /// `fchdir(2)`.
    pub fn fchdir(&self, fd: Fd) -> Result<()> {
        let dir = self.file(fd)?.node().clone();
        self.ctx.lock().set_cwd(&self.cred(), dir)
    }

    /// `getcwd(2)`: the working directory's absolute path.
    pub fn getcwd(&self) -> Result<Vec<u8>> {
        // On a copy, unlocked: the walk up may read procfs (see `at`).
        let ctx = self.ctx.lock().clone();
        vfs().getcwd(&ctx)
    }

    /// `umask(2)`: sets the mask, returns the old one.
    pub fn umask(&self, mask: u16) -> u16 {
        self.ctx.lock().set_umask(mask)
    }

    // ---- conveniences for the kernel itself (shell, tests) ----------------

    /// The whole contents of the file at `path`, up to `limit` bytes; also
    /// whether there was more.
    pub fn read_to_end(&self, path: &[u8], limit: usize) -> Result<(Vec<u8>, bool)> {
        let fd = self.open(path, OpenFlags::RDONLY, 0)?;
        let mut data = Vec::new();
        let mut chunk = [0u8; 512];
        let result = loop {
            let want = chunk.len().min(limit + 1 - data.len().min(limit));
            match self.read(fd, &mut chunk[..want]) {
                Ok(0) => break Ok(false),
                Ok(n) => {
                    data.extend_from_slice(&chunk[..n]);
                    if data.len() > limit {
                        data.truncate(limit);
                        break Ok(true);
                    }
                }
                Err(e) => break Err(e),
            }
        };
        let _ = self.close(fd);
        result.map(|more| (data, more))
    }

    /// The entries of the directory at `path` (without `.` and `..`):
    /// name, type and inode number, in the order the file system lists them.
    pub fn read_dir(&self, path: &[u8]) -> Result<Vec<(Vec<u8>, FileType, poly_vfs::Ino)>> {
        let fd = self.open(path, OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0)?;
        let mut entries = Vec::new();
        let result = loop {
            let mut batch = Vec::new();
            match self.getdents(fd, &mut |e| {
                if e.name != b"." && e.name != b".." {
                    batch.push((e.name.to_vec(), e.kind, e.ino));
                }
                true
            }) {
                Ok(0) => break Ok(()),
                Ok(_) => entries.append(&mut batch),
                Err(e) => break Err(e),
            }
        };
        let _ = self.close(fd);
        result.map(|()| entries)
    }
}
