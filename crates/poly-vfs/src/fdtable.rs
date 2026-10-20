//! Per-process file descriptor tables.
//!
//! The semantics are POSIX's and Linux's (`dup(2)`, `fcntl(2)`, `close(2)`):
//! a new descriptor is always the lowest free number; `dup`, `dup2` and
//! `F_DUPFD` clear close-on-exec on the copy, `dup3` and `F_DUPFD_CLOEXEC`
//! set it as asked; `dup2(fd, fd)` checks `fd` and changes nothing, while
//! `dup3(fd, fd, ..)` is `EINVAL`; `dup2` onto an open descriptor closes it
//! first, silently.

use crate::file::{OpenFile, OpenFlags};
use crate::node::NodeRef;
use crate::{Errno, Result};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// A file descriptor: an index into a [`FdTable`].
pub type Fd = i32;

/// `AT_FDCWD` (Linux UAPI `linux/fcntl.h`): "relative to the working
/// directory" where a directory descriptor is expected.
pub const AT_FDCWD: Fd = -100;

/// The default limit on descriptors per process (`RLIMIT_NOFILE` soft
/// limit Linux starts processes with).
pub const DEFAULT_FD_LIMIT: usize = 1024;

/// Where a relative path starts.
#[derive(Clone, Copy, Debug)]
pub enum At<'a> {
    /// The working directory.
    Cwd,
    /// A node, normally the one a directory descriptor is open on.
    Node(&'a NodeRef),
}

#[derive(Clone, Debug)]
struct Entry {
    file: Arc<OpenFile>,
    cloexec: bool,
}

/// One process's descriptors. `Clone` is `fork`: the copy shares every
/// open file description, offsets included.
#[derive(Clone, Debug)]
pub struct FdTable {
    slots: Vec<Option<Entry>>,
    limit: usize,
}

impl Default for FdTable {
    fn default() -> FdTable {
        FdTable::new()
    }
}

impl FdTable {
    pub fn new() -> FdTable {
        FdTable::with_limit(DEFAULT_FD_LIMIT)
    }

    pub fn with_limit(limit: usize) -> FdTable {
        FdTable { slots: Vec::new(), limit: limit.min(i32::MAX as usize) }
    }

    /// Descriptors must be below this (`RLIMIT_NOFILE`).
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Changes the limit. Open descriptors at or above it stay open, as
    /// with `setrlimit`; only new ones are limited.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.min(i32::MAX as usize);
    }

    /// Open descriptors.
    pub fn count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    fn entry(&self, fd: Fd) -> Result<&Entry> {
        usize::try_from(fd).ok().and_then(|i| self.slots.get(i)).and_then(Option::as_ref).ok_or(Errno::EBADF)
    }

    /// The open file behind `fd` (`EBADF` if none).
    pub fn get(&self, fd: Fd) -> Result<&Arc<OpenFile>> {
        self.entry(fd).map(|e| &e.file)
    }

    /// Puts `file` at the lowest free descriptor (`EMFILE` if none is
    /// below the limit).
    pub fn install(&mut self, file: Arc<OpenFile>, cloexec: bool) -> Result<Fd> {
        self.install_from(0, file, cloexec)
    }

    /// Puts `file` at the lowest free descriptor `>= min` (`F_DUPFD`):
    /// `EINVAL` if `min` is negative or not below the limit, `EMFILE` if
    /// nothing is free.
    pub fn install_from(&mut self, min: Fd, file: Arc<OpenFile>, cloexec: bool) -> Result<Fd> {
        let min = usize::try_from(min).map_err(|_| Errno::EINVAL)?;
        if min >= self.limit {
            return Err(Errno::EINVAL);
        }
        let free = (min..self.slots.len()).find(|&i| self.slots[i].is_none()).unwrap_or(self.slots.len().max(min));
        if free >= self.limit {
            return Err(Errno::EMFILE);
        }
        self.put(free, Entry { file, cloexec });
        Ok(free as Fd)
    }

    fn put(&mut self, index: usize, entry: Entry) -> Option<Entry> {
        if index >= self.slots.len() {
            self.slots.resize(index + 1, None);
        }
        self.slots[index].replace(entry)
    }

    /// `close(2)`: frees `fd` and returns its open file (dropping it may
    /// release the file's last reference).
    pub fn close(&mut self, fd: Fd) -> Result<Arc<OpenFile>> {
        self.entry(fd)?;
        let e = self.slots[fd as usize].take().ok_or(Errno::EBADF)?;
        while matches!(self.slots.last(), Some(None)) {
            self.slots.pop();
        }
        Ok(e.file)
    }

    /// `dup(2)`.
    pub fn dup(&mut self, fd: Fd) -> Result<Fd> {
        let file = self.get(fd)?.clone();
        self.install(file, false)
    }

    /// `fcntl(F_DUPFD)` / `fcntl(F_DUPFD_CLOEXEC)`.
    pub fn dup_from(&mut self, fd: Fd, min: Fd, cloexec: bool) -> Result<Fd> {
        let file = self.get(fd)?.clone();
        self.install_from(min, file, cloexec)
    }

    /// `dup2(2)`: `EBADF` if `old` is not open or `new` is out of range;
    /// `dup2(fd, fd)` returns `fd` unchanged.
    pub fn dup2(&mut self, old: Fd, new: Fd) -> Result<Fd> {
        let file = self.get(old)?.clone();
        if old == new {
            return Ok(new);
        }
        self.replace(new, file, false)
    }

    /// `dup3(2)`: like `dup2`, but `old == new` is `EINVAL`, and `flags` may
    /// only hold `O_CLOEXEC` (`EINVAL` otherwise).
    pub fn dup3(&mut self, old: Fd, new: Fd, flags: OpenFlags) -> Result<Fd> {
        if flags.without(OpenFlags::CLOEXEC) != OpenFlags::RDONLY {
            return Err(Errno::EINVAL);
        }
        let file = self.get(old)?.clone();
        if old == new {
            return Err(Errno::EINVAL);
        }
        self.replace(new, file, flags.contains(OpenFlags::CLOEXEC))
    }

    fn replace(&mut self, new: Fd, file: Arc<OpenFile>, cloexec: bool) -> Result<Fd> {
        let index = usize::try_from(new).map_err(|_| Errno::EBADF)?;
        if index >= self.limit {
            return Err(Errno::EBADF);
        }
        // The replaced file is dropped here, after the new entry is in
        // place, so the descriptor is never seen free in between.
        drop(self.put(index, Entry { file, cloexec }));
        Ok(new)
    }

    /// `fcntl(F_GETFD)`: whether close-on-exec is set.
    pub fn cloexec(&self, fd: Fd) -> Result<bool> {
        self.entry(fd).map(|e| e.cloexec)
    }

    /// `fcntl(F_SETFD)`.
    pub fn set_cloexec(&mut self, fd: Fd, cloexec: bool) -> Result<()> {
        self.entry(fd)?;
        if let Some(e) = self.slots[fd as usize].as_mut() {
            e.cloexec = cloexec;
        }
        Ok(())
    }

    /// At `execve`: closes every descriptor marked close-on-exec.
    pub fn close_on_exec(&mut self) {
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|e| e.cloexec) {
                *slot = None;
            }
        }
        while matches!(self.slots.last(), Some(None)) {
            self.slots.pop();
        }
    }

    /// At exit: closes everything.
    pub fn close_all(&mut self) {
        self.slots.clear();
    }

    /// Open descriptors in increasing order, with their close-on-exec flag.
    pub fn iter(&self) -> impl Iterator<Item = (Fd, &Arc<OpenFile>, bool)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|e| (i as Fd, &e.file, e.cloexec)))
    }

    /// Where a `*at` system call's relative path starts: the working
    /// directory for [`AT_FDCWD`], otherwise the node `dirfd` is open on
    /// (`EBADF` if it is not open). Whether that node is a directory is
    /// checked by the walk (`ENOTDIR`).
    pub fn at(&self, dirfd: Fd) -> Result<At<'_>> {
        if dirfd == AT_FDCWD { Ok(At::Cwd) } else { Ok(At::Node(self.get(dirfd)?.node())) }
    }

    /// [`FdTable::at`] for `path`: an absolute path ignores `dirfd`
    /// entirely, even an invalid one (`openat(2)`).
    pub fn at_for(&self, dirfd: Fd, path: &[u8]) -> Result<At<'_>> {
        if path.first() == Some(&b'/') { Ok(At::Cwd) } else { self.at(dirfd) }
    }
}
