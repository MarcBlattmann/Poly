//! The kernel's file system services for the Linux personality:
//! [`LinuxFs`] for the running thread ([`Thread`]), over the process's
//! [`Files`](crate::fs::Files) and the global VFS (ADR 0043).
//!
//! Arguments arrive validated and copied into kernel memory by the
//! personality (`sys::path`, `sys::fd`); errors are the VFS's, which uses
//! Linux's numbers.

use super::Thread;
use super::services::{file_stat, vfs_errno};
use crate::fs;
use crate::process;
use alloc::vec::Vec;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fs::{DirEntry, FileStatus, LinuxFs, LockHolder, StatFs, TimeChange, XattrTarget};
use poly_personality_linux::kernel::Target;
use poly_vfs::locks::{LockKind, LockOwner};
use poly_vfs::{Access, OpenFlags, RenameFlags, Whence};

/// The VFS error for a Linux errno the personality found (both use Linux's
/// numbers).
fn errno_vfs(e: Errno) -> poly_vfs::Errno {
    poly_vfs::Errno::from_code(i32::from(e.get())).unwrap_or(poly_vfs::Errno::EIO)
}

fn statfs(s: poly_vfs::FsStats) -> StatFs {
    StatFs {
        fs_type: s.magic,
        bsize: s.block_size,
        blocks: s.blocks,
        bfree: s.blocks_free,
        bavail: s.blocks_avail,
        files: s.files,
        ffree: s.files_free,
        fsid: s.fsid,
        namelen: s.name_max,
        frsize: s.fragment_size,
        flags: s.flags,
    }
}

fn status(node: &poly_vfs::NodeRef) -> Result<FileStatus, Errno> {
    let st = node.stat().map_err(vfs_errno)?;
    Ok(FileStatus {
        stat: file_stat(&st),
        mount_id: u64::from(node.mount().id()),
        mount_root: node.ino() == node.mount().root_ino(),
    })
}

impl LinuxFs for Thread<'_> {
    fn openat(&mut self, dirfd: i32, path: &[u8], flags: u32, mode: u32) -> Result<i32, Errno> {
        let files = &self.process().files;
        let fd = files.openat(dirfd, path, OpenFlags::from_bits(flags), mode as u16).map_err(vfs_errno)?;
        self.fifo_open_wait(fd)
    }

    fn check_fd_free(&mut self) -> Result<(), Errno> {
        self.process().files.check_fd_free().map_err(vfs_errno)
    }

    fn statat(&mut self, dirfd: i32, path: &[u8], follow: bool) -> Result<FileStatus, Errno> {
        let files = &self.process().files;
        let node = if path.is_empty() {
            files.node_at(dirfd).map_err(vfs_errno)?
        } else {
            files.lookupat(dirfd, path, follow).map_err(vfs_errno)?
        };
        status(&node)
    }

    fn readlinkat(&mut self, dirfd: i32, path: &[u8]) -> Result<Vec<u8>, Errno> {
        let files = &self.process().files;
        if path.is_empty() { files.readlink_fd(dirfd) } else { files.readlinkat(dirfd, path) }.map_err(vfs_errno)
    }

    fn getdents(&mut self, fd: i32, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<usize, Errno> {
        self.process()
            .files
            .getdents(fd, &mut |e| {
                emit(&DirEntry { ino: e.ino, next: e.next, kind: e.kind.dirent_type(), name: e.name })
            })
            .map_err(vfs_errno)
    }

    fn accessat(&mut self, dirfd: i32, path: &[u8], mode: u32, effective: bool, follow: bool) -> Result<(), Errno> {
        // access(2) checks with the real ids (and for real root the
        // permitted capabilities); AT_EACCESS with the effective ones
        // (ADR 0045).
        let want = Access::from_bits(mode).map_err(vfs_errno)?;
        let cred = super::cred::current_creds().map_or_else(poly_vfs::Cred::root, |c| {
            super::cred::vfs_cred(&if effective { c.fs_ids() } else { c.access_ids() })
        });
        self.process().files.accessat(&cred, dirfd, path, want, follow).map_err(vfs_errno)
    }

    fn getcwd(&mut self) -> Result<Vec<u8>, Errno> {
        self.process().files.getcwd().map_err(vfs_errno)
    }

    fn chdir(&mut self, path: &[u8]) -> Result<(), Errno> {
        self.process().files.chdir(path).map_err(vfs_errno)
    }

    fn fchdir(&mut self, fd: i32) -> Result<(), Errno> {
        self.process().files.fchdir(fd).map_err(vfs_errno)
    }

    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, Errno> {
        let whence = Whence::from_linux(whence).map_err(vfs_errno)?;
        self.process().files.lseek(fd, offset, whence).map_err(vfs_errno)
    }

    fn pread(&mut self, fd: i32, buf: &mut [u8], offset: u64) -> Result<usize, Errno> {
        self.process().files.pread(fd, buf, offset).map_err(vfs_errno)
    }

    fn pwrite(&mut self, fd: i32, buf: &[u8], offset: u64) -> Result<usize, Errno> {
        // A traced program's output is captured by the open file it reaches
        // (`tty::log_console_files`), whichever call wrote the bytes.
        self.process().files.pwrite(fd, buf, offset).map_err(vfs_errno)
    }

    fn fallocate(&mut self, fd: i32, mode: u32, offset: u64, len: u64) -> Result<(), Errno> {
        self.process().files.fallocate(fd, mode, offset, len).map_err(vfs_errno)
    }

    fn read_nowait(&mut self, fd: i32, buf: &mut [u8], at: Option<u64>) -> Result<usize, Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        if file.on_inode() {
            return Err(Errno::EOPNOTSUPP);
        }
        match at {
            Some(offset) => file.read_at(offset, buf),
            None => file.read_wait_until(buf, &mut self.blocker(), poly_vfs::Timeout::Now),
        }
        .map_err(vfs_errno)
    }

    fn write_flags(
        &mut self,
        fd: i32,
        buf: &[u8],
        at: Option<u64>,
        append: Option<bool>,
        nowait: bool,
    ) -> Result<usize, Errno> {
        let files = &self.process().files;
        if !nowait {
            return files.write_with(fd, buf, at, append).map_err(vfs_errno);
        }
        let file = files.file(fd).map_err(vfs_errno)?;
        if file.on_inode() {
            return Err(Errno::EOPNOTSUPP);
        }
        files.before_write(&file, buf.len()).map_err(vfs_errno)?;
        match at {
            Some(offset) => file.write_at(offset, buf),
            None => file.write_wait_until(buf, &mut self.blocker(), poly_vfs::Timeout::Now),
        }
        .map_err(vfs_errno)
    }

    fn dup(&mut self, fd: i32) -> Result<i32, Errno> {
        self.process().files.dup(fd).map_err(vfs_errno)
    }

    fn dup3(&mut self, old: i32, new: i32, cloexec: bool) -> Result<i32, Errno> {
        let files = &self.process().files;
        if cloexec { files.dup3(old, new, OpenFlags::CLOEXEC) } else { files.dup2(old, new) }.map_err(vfs_errno)
    }

    fn dup_min(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, Errno> {
        self.process().files.dup_from(fd, min, cloexec).map_err(vfs_errno)
    }

    fn cloexec(&mut self, fd: i32) -> Result<bool, Errno> {
        self.process().files.cloexec(fd).map_err(vfs_errno)
    }

    fn set_cloexec(&mut self, fd: i32, cloexec: bool) -> Result<(), Errno> {
        self.process().files.set_cloexec(fd, cloexec).map_err(vfs_errno)
    }

    fn close_range(&mut self, first: u32, last: u32, cloexec: bool) -> Result<(), Errno> {
        self.process().files.close_range(first, last, cloexec);
        Ok(())
    }

    fn unshare_fd_table(&mut self) -> Result<(), Errno> {
        self.process().files.unshare_table();
        Ok(())
    }

    fn test_lock(
        &mut self,
        fd: i32,
        ofd: bool,
        write: bool,
        start: u64,
        end: u64,
    ) -> Result<Option<LockHolder>, Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        let owner = self.lock_owner(&file, ofd);
        let kind = if write { LockKind::Write } else { LockKind::Read };
        Ok(fs::locks::test(owner, &file, kind, start, end).map(|l| LockHolder {
            write: l.kind == LockKind::Write,
            start: l.start,
            end: l.end,
            pid: l.pid,
        }))
    }

    fn set_lock(
        &mut self,
        fd: i32,
        ofd: bool,
        write: Option<bool>,
        start: u64,
        end: u64,
        wait: bool,
    ) -> Result<(), Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        let owner = self.lock_owner(&file, ofd);
        let pid = if ofd { -1 } else { self.process().pid as i32 };
        let kind = write.map(|w| if w { LockKind::Write } else { LockKind::Read });
        loop {
            let generation = fs::locks::generation();
            if fs::locks::set(owner, pid, &file, kind, start, end).is_ok() {
                return Ok(());
            }
            if !wait {
                return Err(Errno::EAGAIN);
            }
            self.wait_for_locks(generation)?;
        }
    }

    fn flock(&mut self, fd: i32, exclusive: Option<bool>, wait: bool) -> Result<(), Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        let generation = fs::locks::generation();
        let wait = wait && exclusive.is_some();
        if fs::locks::flock(&file, exclusive, wait).is_ok() {
            return Ok(());
        }
        if !wait {
            return Err(Errno::EAGAIN);
        }
        // The request waits in the lock table, which hands it the lock
        // when the conflicting ones go (before anyone else asks again).
        let mut generation = generation;
        loop {
            if fs::locks::flock_granted(&file) {
                return Ok(());
            }
            if let Err(e) = self.wait_for_locks(generation) {
                fs::locks::flock_cancel(&file);
                // Granted just before the signal: the lock is taken.
                return if fs::locks::flock_granted(&file) { Ok(()) } else { Err(e) };
            }
            generation = fs::locks::generation();
        }
    }

    fn status_flags(&mut self, fd: i32) -> Result<u32, Errno> {
        let files = &self.process().files;
        let extra = files.file(fd).map_or(0, |f| super::pidfd::extra_status_flags(&f));
        files.status_flags(fd).map(|f| f.bits() | extra).map_err(vfs_errno)
    }

    fn set_status_flags(&mut self, fd: i32, flags: u32) -> Result<(), Errno> {
        self.process().files.set_status_flags(fd, OpenFlags::from_bits(flags)).map_err(vfs_errno)
    }

    fn umask(&mut self, mask: u32) -> u32 {
        u32::from(self.process().files.umask(mask as u16))
    }

    fn mkdirat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno> {
        self.process().files.mkdirat(dirfd, path, mode as u16).map_err(vfs_errno)
    }

    fn unlinkat(&mut self, dirfd: i32, path: &[u8], remove_dir: bool) -> Result<(), Errno> {
        let flags = if remove_dir { fs::AT_REMOVEDIR } else { 0 };
        self.process().files.unlinkat(dirfd, path, flags).map_err(vfs_errno)
    }

    fn renameat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        flags: u32,
    ) -> Result<(), Errno> {
        let flags = RenameFlags::from_linux(flags).map_err(vfs_errno)?;
        self.process().files.renameat(olddirfd, old, newdirfd, new.map_err(errno_vfs), flags).map_err(vfs_errno)
    }

    fn symlinkat(&mut self, target: &[u8], dirfd: i32, path: &[u8]) -> Result<(), Errno> {
        self.process().files.symlinkat(target, dirfd, path).map_err(vfs_errno)
    }

    fn linkat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        follow: bool,
    ) -> Result<(), Errno> {
        self.process().files.linkat(olddirfd, old, newdirfd, new.map_err(errno_vfs), follow).map_err(vfs_errno)
    }

    fn fchmod(&mut self, fd: i32, mode: u32) -> Result<(), Errno> {
        self.process().files.fchmod(fd, mode as u16).map_err(vfs_errno)
    }

    fn fchmodat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno> {
        self.process().files.fchmodat(dirfd, path, mode as u16).map_err(vfs_errno)
    }

    fn fchownat(
        &mut self,
        dirfd: i32,
        path: &[u8],
        uid: Option<u32>,
        gid: Option<u32>,
        follow: bool,
    ) -> Result<(), Errno> {
        self.process().files.fchownat(dirfd, path, uid, gid, follow).map_err(vfs_errno)
    }

    fn fchown(&mut self, fd: i32, uid: Option<u32>, gid: Option<u32>) -> Result<(), Errno> {
        self.process().files.fchown(fd, uid, gid).map_err(vfs_errno)
    }

    fn utimensat(
        &mut self,
        dirfd: i32,
        path: Option<&[u8]>,
        atime: Option<TimeChange>,
        mtime: Option<TimeChange>,
        follow: bool,
    ) -> Result<(), Errno> {
        let (atime, mtime) = (super::cred::set_time(atime), super::cred::set_time(mtime));
        self.process().files.utimensat(dirfd, path, atime, mtime, follow).map_err(vfs_errno)
    }

    fn memfd_create(&mut self, name: &[u8], perm: u32, seals: u32, cloexec: bool) -> Result<i32, Errno> {
        let files = &self.process().files;
        let memfd = poly_vfs::memfd::Memfd::new(name, seals);
        let file =
            fs::vfs().memfd_file(&fs::memfs::memfd_mount(), &files.cred(), perm as u16, memfd).map_err(vfs_errno)?;
        files.install(file, cloexec).map_err(vfs_errno)
    }

    fn add_seals(&mut self, fd: i32, seals: u32) -> Result<(), Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        let memfd = file.memfd().ok_or(Errno::EINVAL)?;
        // fcntl(2): "EPERM ... fd is not open for writing".
        if !file.writable() {
            return Err(Errno::EPERM);
        }
        memfd.add_seals(seals).map_err(vfs_errno)
    }

    fn get_seals(&mut self, fd: i32) -> Result<u32, Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        file.memfd().map(|m| m.seals()).ok_or(Errno::EINVAL)
    }

    fn ftruncate(&mut self, fd: i32, len: u64) -> Result<(), Errno> {
        self.process().files.ftruncate(fd, len).map_err(vfs_errno)
    }

    fn truncate(&mut self, path: &[u8], len: u64) -> Result<(), Errno> {
        self.process().files.truncate(path, len).map_err(vfs_errno)
    }

    /// `fdatasync` is `fsync`: file systems sync everything at once
    /// (ADR 0040: there is no per-file sync).
    fn fsync(&mut self, fd: i32, _data_only: bool) -> Result<(), Errno> {
        self.process().files.fsync(fd).map_err(vfs_errno)
    }

    fn sync(&mut self) {
        // sync(2) always succeeds; failures are logged by the cache.
        let _ = fs::disk::sync_all();
    }

    fn syncfs(&mut self, fd: i32) -> Result<(), Errno> {
        self.process().files.syncfs(fd).map_err(vfs_errno)
    }

    fn statfs(&mut self, path: &[u8]) -> Result<StatFs, Errno> {
        self.process().files.statfs(path).map(statfs).map_err(vfs_errno)
    }

    fn fstatfs(&mut self, fd: i32) -> Result<StatFs, Errno> {
        self.process().files.fstatfs(fd).map(statfs).map_err(vfs_errno)
    }

    fn set_fd_limit(&mut self, target: Target, limit: u64) -> Result<(), Errno> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        match target {
            Target::Current => self.process().files.set_fd_limit(limit),
            Target::Process(pid) => {
                let p = u32::try_from(pid).ok().and_then(process::find).ok_or(Errno::ESRCH)?;
                p.files.set_fd_limit(limit);
            }
            Target::Thread(tid) => {
                let (p, _) = u32::try_from(tid).ok().and_then(process::find_thread).ok_or(Errno::ESRCH)?;
                p.files.set_fd_limit(limit);
            }
        }
        Ok(())
    }

    fn getxattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<Vec<u8>, Errno> {
        super::xattr::get(self, target, name)
    }

    fn setxattr(&mut self, target: XattrTarget<'_>, name: &[u8], value: &[u8], flags: u32) -> Result<(), Errno> {
        super::xattr::set(self, target, name, value, flags)
    }

    fn listxattr(&mut self, target: XattrTarget<'_>) -> Result<Vec<u8>, Errno> {
        super::xattr::list(self, target)
    }

    fn removexattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<(), Errno> {
        super::xattr::remove(self, target, name)
    }
}

impl Thread<'_> {
    /// Who owns a lock this thread takes on `file`: its process (the
    /// descriptor table's id), or the open file description (`ofd`).
    fn lock_owner(&self, file: &alloc::sync::Arc<poly_vfs::OpenFile>, ofd: bool) -> LockOwner {
        if ofd { fs::locks::description(file) } else { LockOwner::Process(self.process().files.lock_owner()) }
    }

    /// Wait until a lock goes (the lock generation moves from
    /// `generation`), or a signal, a kill or the process's exit ends the
    /// wait: `RESTART_IF_SA_RESTART` (signal(7): file lock waits are
    /// restarted under `SA_RESTART`).
    fn wait_for_locks(&self, generation: u64) -> Result<(), Errno> {
        use core::sync::atomic::Ordering;
        let (flags, process) = (&self.flags, self.process());
        let interrupted =
            || flags.notified.load(Ordering::SeqCst) || flags.killed.load(Ordering::SeqCst) || process.is_exiting();
        process::wait_event(|| fs::locks::generation() != generation || interrupted());
        if fs::locks::generation() != generation { Ok(()) } else { Err(Errno::RESTART_IF_SA_RESTART) }
    }
}

/// What sysinfo(2) reports, from what procfs shows (`/proc/meminfo`,
/// `/proc/uptime`, `/proc/loadavg`). Sizes in bytes (`mem_unit` 1).
pub(super) fn system_info() -> poly_personality_linux::fs::SysInfo {
    use poly_procfs::ProcSource;
    let s = fs::proc::KernelProc.system();
    poly_personality_linux::fs::SysInfo {
        uptime: (s.uptime_ns / crate::time::NANOS_PER_SEC) as i64,
        loads: s.loads,
        totalram: s.mem_total,
        freeram: s.mem_free,
        sharedram: s.shmem,
        bufferram: s.buffers,
        totalswap: 0,
        freeswap: 0,
        procs: u16::try_from(process::all().len()).unwrap_or(u16::MAX),
        totalhigh: 0,
        freehigh: 0,
        mem_unit: 1,
    }
}
