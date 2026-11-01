//! Open file descriptions.

use crate::device::CharDevice;
use crate::fs::DirEntry;
use crate::mount::Mount;
use crate::node::NodeRef;
use crate::object::FileObject;
use crate::poll::{self, Blocker, PollEvents, Timeout, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, FileType, Ino, MAX_FILE_SIZE, Result, Stat};
use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU32, Ordering};

/// `open(2)` flags. The bit values are Linux's (UAPI
/// `asm-generic/fcntl.h`, which x86-64 uses), so the Linux personality
/// passes its argument through [`OpenFlags::from_bits`] unchanged.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const RDONLY: OpenFlags = OpenFlags(0);
    pub const WRONLY: OpenFlags = OpenFlags(0o1);
    pub const RDWR: OpenFlags = OpenFlags(0o2);
    pub const ACCMODE: OpenFlags = OpenFlags(0o3);
    pub const CREAT: OpenFlags = OpenFlags(0o100);
    pub const EXCL: OpenFlags = OpenFlags(0o200);
    pub const NOCTTY: OpenFlags = OpenFlags(0o400);
    pub const TRUNC: OpenFlags = OpenFlags(0o1000);
    pub const APPEND: OpenFlags = OpenFlags(0o2000);
    /// Reads and writes that would block fail with `EAGAIN` instead.
    pub const NONBLOCK: OpenFlags = OpenFlags(0o4000);
    pub const DSYNC: OpenFlags = OpenFlags(0o10000);
    /// `FASYNC`: signal-driven I/O, recorded only.
    pub const ASYNC: OpenFlags = OpenFlags(0o20000);
    pub const DIRECT: OpenFlags = OpenFlags(0o40000);
    pub const LARGEFILE: OpenFlags = OpenFlags(0o100000);
    pub const DIRECTORY: OpenFlags = OpenFlags(0o200000);
    pub const NOFOLLOW: OpenFlags = OpenFlags(0o400000);
    pub const NOATIME: OpenFlags = OpenFlags(0o1000000);
    pub const CLOEXEC: OpenFlags = OpenFlags(0o2000000);
    /// `O_SYNC` is this together with `DSYNC`.
    pub const SYNC: OpenFlags = OpenFlags(0o4000000 | 0o10000);
    pub const PATH: OpenFlags = OpenFlags(0o10000000);
    /// `O_TMPFILE` is this together with `DIRECTORY`.
    pub const TMPFILE: OpenFlags = OpenFlags(0o20000000 | 0o200000);

    /// Flags `fcntl(F_SETFL)` may change.
    pub const SETFL_MASK: OpenFlags = OpenFlags(0o2000 | 0o4000 | 0o20000 | 0o40000 | 0o1000000);

    /// Every bit; unknown bits are ignored by `open`, as in Linux.
    pub const fn from_bits(bits: u32) -> OpenFlags {
        OpenFlags(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: OpenFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: OpenFlags) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 | other.0)
    }

    pub const fn without(self, other: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 & !other.0)
    }

    pub const fn intersection(self, other: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 & other.0)
    }

    /// Whether the access mode allows reading.
    pub const fn readable(self) -> bool {
        matches!(self.0 & 3, 0 | 2)
    }

    /// Whether the access mode allows writing.
    pub const fn writable(self) -> bool {
        matches!(self.0 & 3, 1 | 2)
    }
}

impl core::ops::BitOr for OpenFlags {
    type Output = OpenFlags;

    fn bitor(self, rhs: OpenFlags) -> OpenFlags {
        self.union(rhs)
    }
}

impl core::fmt::Debug for OpenFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OpenFlags({:#o})", self.0)
    }
}

/// `lseek`'s `whence` (values from Linux UAPI `linux/fs.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whence {
    Set,
    Cur,
    End,
    /// The next offset at or after the given one that holds data.
    Data,
    /// The next hole at or after the given one; the end of the file counts.
    Hole,
}

impl Whence {
    pub fn from_linux(whence: u32) -> Result<Whence> {
        Ok(match whence {
            0 => Whence::Set,
            1 => Whence::Cur,
            2 => Whence::End,
            3 => Whence::Data,
            4 => Whence::Hole,
            _ => return Err(Errno::EINVAL),
        })
    }
}

/// What reads and writes go to.
pub(crate) enum Backend {
    /// An inode of the node's file system (regular file or directory).
    Inode,
    Device(Arc<dyn CharDevice>),
    /// A pipe, event counter, timer, epoll set... (`object.rs`).
    Object(Arc<dyn FileObject>),
    /// `O_PATH`: the description only names the node.
    Path,
}

/// Where an open file was found: the directory it was looked up in and
/// the name it had there, so `/proc/<pid>/fd` and `/proc/<pid>/maps` can
/// name it later ([`Vfs::file_path`](crate::Vfs::file_path)). It keeps
/// nothing alive.
#[derive(Clone)]
pub(crate) struct Origin {
    pub(crate) mount: Weak<Mount>,
    pub(crate) dir: Ino,
    pub(crate) name: Box<[u8]>,
}

/// An open file description: what `open` makes and descriptors share.
///
/// Reads and writes on regular files and directories update the offset
/// atomically with the transfer, as POSIX asks for regular files.
pub struct OpenFile {
    node: NodeRef,
    kind: FileType,
    backend: Backend,
    /// Access mode and status flags, as `fcntl(F_GETFL)` reports them.
    flags: AtomicU32,
    offset: SpinMutex<u64>,
    /// How `open` found it, if it did.
    origin: Option<Origin>,
    /// Made by `O_TMPFILE | O_EXCL`: it may never get a name.
    never_named: bool,
    /// Made by memfd_create(2): its label and seals.
    memfd: Option<Arc<crate::memfd::Memfd>>,
    /// Opened to run the file as a program: while it lives, the inode
    /// cannot be written (`ETXTBSY`).
    denies_write: bool,
    /// Opened for writing a regular file: while it lives, the inode
    /// cannot be run as a program (`ETXTBSY`, execve(2)).
    counts_text_writer: bool,
    /// Open for writing an inode of a mount: counted in the mount's and
    /// its superblock's writers (`Mount::writers`) while it lives.
    counts_writer: bool,
    /// Causes no inotify or fanotify events (a descriptor fanotify hands
    /// to its reader).
    quiet: bool,
}

impl OpenFile {
    /// Flags an open file keeps (the rest only matter during `open`).
    const KEPT: OpenFlags = OpenFlags(
        OpenFlags::ACCMODE.0
            | OpenFlags::APPEND.0
            | OpenFlags::NONBLOCK.0
            | OpenFlags::SYNC.0
            | OpenFlags::ASYNC.0
            | OpenFlags::DIRECT.0
            | OpenFlags::LARGEFILE.0
            | OpenFlags::NOATIME.0
            | OpenFlags::DIRECTORY.0
            | OpenFlags::NOFOLLOW.0
            | OpenFlags::PATH.0
            | OpenFlags::TMPFILE.0,
    );

    pub(crate) fn new(node: NodeRef, kind: FileType, backend: Backend, flags: OpenFlags) -> OpenFile {
        let flags = flags.intersection(Self::KEPT);
        let counts_writer = matches!(backend, Backend::Inode) && flags.writable();
        if counts_writer {
            node.loc().mount.count_writer(true);
        }
        OpenFile {
            node,
            kind,
            backend,
            flags: AtomicU32::new(flags.bits()),
            offset: SpinMutex::new(0),
            origin: None,
            never_named: false,
            memfd: None,
            denies_write: false,
            counts_writer,
            counts_text_writer: false,
            quiet: false,
        }
    }

    /// An open file that writes its inode: execve(2) of it is `ETXTBSY`
    /// until it is dropped.
    pub(crate) fn counting_text_writer(mut self) -> OpenFile {
        self.node.loc().mount.sb.add_text_writer(self.node.ino());
        self.counts_text_writer = true;
        self
    }

    /// An open file that causes no file system events (fanotify(7): the
    /// descriptors of events are opened so that they report nothing).
    pub(crate) fn quiet(mut self) -> OpenFile {
        self.quiet = true;
        self
    }

    /// Whether the file causes no file system events.
    pub(crate) fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// An open file that runs its inode as a program: writers get
    /// `ETXTBSY` until it is dropped (execve(2), open(2)).
    pub(crate) fn denying_write(mut self) -> OpenFile {
        self.node.loc().mount.sb.deny_write(self.node.ino());
        self.denies_write = true;
        self
    }

    /// A memfd_create(2) file (`Vfs::memfd_file`): never named.
    pub(crate) fn with_memfd(mut self, memfd: Arc<crate::memfd::Memfd>) -> OpenFile {
        self.memfd = Some(memfd);
        self.never_named = true;
        self
    }

    /// The label and seals of a memfd_create(2) file.
    pub fn memfd(&self) -> Option<&Arc<crate::memfd::Memfd>> {
        self.memfd.as_ref()
    }

    /// Whether this is a memfd_secret(2) file, used only through shared
    /// mappings ([`Memfd::secret`](crate::memfd::Memfd::secret)): reads and
    /// writes are `EINVAL`, positioned ones and seeks `ESPIPE`.
    pub fn secret(&self) -> bool {
        self.memfd.as_ref().is_some_and(|m| m.is_secret())
    }

    /// The size of the file, for the seal checks of a memfd.
    fn size(&self) -> Result<u64> {
        Ok(self.node.attr()?.size)
    }

    /// An `O_TMPFILE | O_EXCL` file: `linkat` may never give it a name.
    pub(crate) fn unnameable(mut self) -> OpenFile {
        self.never_named = true;
        self
    }

    /// Whether this is an `O_TMPFILE` file (`F_GETFL` reports the flag).
    pub fn is_tmpfile(&self) -> bool {
        self.flags().contains(OpenFlags::TMPFILE)
    }

    /// Whether `linkat` may never name this file (`O_TMPFILE | O_EXCL`).
    pub fn is_unnameable(&self) -> bool {
        self.never_named
    }

    /// Records how `open` found the file.
    pub(crate) fn with_origin(mut self, origin: Option<Origin>) -> OpenFile {
        self.origin = origin;
        self
    }

    pub(crate) fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    /// An open file on `object`, with `node` (from
    /// [`Vfs::anon_node`](crate::Vfs::anon_node)) for `fstat`. `flags`
    /// gives the access mode and status flags (`O_NONBLOCK`).
    pub fn on_object(node: NodeRef, object: Arc<dyn FileObject>, flags: OpenFlags) -> OpenFile {
        let kind = node.kind().unwrap_or(FileType::Regular);
        OpenFile::new(node, kind, Backend::Object(object), flags)
    }

    /// The node this file is open on.
    pub fn node(&self) -> &NodeRef {
        &self.node
    }

    /// The node's type when it was opened.
    pub fn kind(&self) -> FileType {
        self.kind
    }

    /// Access mode and status flags (`fcntl(F_GETFL)`).
    pub fn flags(&self) -> OpenFlags {
        OpenFlags(self.flags.load(Ordering::Relaxed))
    }

    /// `fcntl(F_SETFL)`: changes `O_APPEND`, `O_NONBLOCK`, `O_ASYNC`,
    /// `O_DIRECT` and `O_NOATIME`, ignores the rest.
    pub fn set_flags(&self, new: OpenFlags) {
        let mask = OpenFlags::SETFL_MASK.bits();
        let _ = self
            .flags
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| Some((old & !mask) | (new.bits() & mask)));
    }

    pub fn is_path_only(&self) -> bool {
        matches!(self.backend, Backend::Path)
    }

    pub fn readable(&self) -> bool {
        !self.is_path_only() && self.flags().readable()
    }

    pub fn writable(&self) -> bool {
        !self.is_path_only() && self.flags().writable()
    }

    fn nonblock(&self) -> bool {
        self.flags().contains(OpenFlags::NONBLOCK)
    }

    fn fs(&self) -> &dyn crate::FileSystem {
        &**self.node.mount().fs()
    }

    /// The current offset.
    pub fn offset(&self) -> u64 {
        *self.offset.lock()
    }

    /// The current offset without waiting: `None` while a read, write or
    /// listing of this description holds it (which may be the caller's own,
    /// as when a process reads its `/proc/self/fdinfo` entry of the very
    /// file being read).
    pub fn offset_now(&self) -> Option<u64> {
        self.offset.try_lock().map(|o| *o)
    }

    /// `fstat(2)`: the node's status; an object on an anonymous inode
    /// ([`FileObject::anon_name`]) has no file type bits in its mode.
    pub fn stat(&self) -> Result<Stat> {
        let mut st = self.node.stat()?;
        if self.object().is_some_and(|o| o.anon_name().is_some()) {
            st.mode &= !crate::S_IFMT;
        }
        Ok(st)
    }

    /// `read(2)`: from the offset, advancing it.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.readable() {
            return Err(Errno::EBADF);
        }
        if self.secret() {
            return Err(Errno::EINVAL);
        }
        match &self.backend {
            Backend::Inode if self.kind == FileType::Directory => Err(Errno::EISDIR),
            Backend::Inode => {
                let n = {
                    let mut off = self.offset.lock();
                    let n = self.fs().read(self.node.ino(), *off, buf)?;
                    *off += n as u64;
                    n
                };
                self.accessed(n);
                Ok(n)
            }
            Backend::Device(dev) if dev.seekable() => {
                let mut off = self.offset.lock();
                let n = dev.read(*off, buf, self.nonblock())?;
                *off = off.saturating_add(n as u64);
                Ok(n)
            }
            Backend::Device(dev) => dev.read(0, buf, self.nonblock()),
            Backend::Object(obj) => obj.read(buf),
            Backend::Path => Err(Errno::EBADF),
        }
    }

    /// `write(2)`: at the offset (at the end with `O_APPEND`), advancing it.
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.writable() {
            return Err(Errno::EBADF);
        }
        if self.secret() {
            return Err(Errno::EINVAL);
        }
        match &self.backend {
            // Nothing to write changes nothing: not the size, not the times.
            Backend::Inode if buf.is_empty() => Ok(0),
            Backend::Inode => {
                let n = {
                    let mut off = self.offset.lock();
                    if let Some(m) = &self.memfd {
                        let size = self.size()?;
                        let at = if self.flags().contains(OpenFlags::APPEND) { size } else { *off };
                        m.check_write(at, buf.len() as u64, size)?;
                    }
                    if self.flags().contains(OpenFlags::APPEND) {
                        let (at, n) = self.fs().append(self.node.ino(), buf)?;
                        *off = at + n as u64;
                        n
                    } else {
                        let n = self.fs().write(self.node.ino(), *off, buf)?;
                        *off += n as u64;
                        n
                    }
                };
                self.modified(n);
                Ok(n)
            }
            Backend::Device(dev) if dev.seekable() => {
                let mut off = self.offset.lock();
                let n = dev.write(*off, buf, self.nonblock())?;
                *off = off.saturating_add(n as u64);
                Ok(n)
            }
            Backend::Device(dev) => dev.write(0, buf, self.nonblock()),
            Backend::Object(obj) => obj.write(buf),
            Backend::Path => Err(Errno::EBADF),
        }
    }

    /// `pread(2)`: at `offset`, leaving the file offset alone. `ESPIPE` on
    /// unseekable files, `EINVAL` for offsets past `off_t`.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if !self.readable() {
            return Err(Errno::EBADF);
        }
        if self.secret() {
            return Err(Errno::ESPIPE);
        }
        if offset > MAX_FILE_SIZE {
            return Err(Errno::EINVAL);
        }
        match &self.backend {
            Backend::Inode if self.kind == FileType::Directory => Err(Errno::EISDIR),
            Backend::Inode => {
                let n = self.fs().read(self.node.ino(), offset, buf)?;
                self.accessed(n);
                Ok(n)
            }
            Backend::Device(dev) if dev.seekable() => dev.read(offset, buf, self.nonblock()),
            Backend::Device(_) | Backend::Object(_) => Err(Errno::ESPIPE),
            Backend::Path => Err(Errno::EBADF),
        }
    }

    /// `pwrite(2)`: at `offset`, leaving the file offset alone. As on Linux
    /// (`pwrite(2)`, BUGS), a file opened with `O_APPEND` is appended to
    /// whatever the offset.
    pub fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize> {
        if !self.writable() {
            return Err(Errno::EBADF);
        }
        if self.secret() {
            return Err(Errno::ESPIPE);
        }
        if offset > MAX_FILE_SIZE {
            return Err(Errno::EINVAL);
        }
        if let (Some(m), Backend::Inode) = (&self.memfd, &self.backend) {
            let size = self.size()?;
            let at = if self.flags().contains(OpenFlags::APPEND) { size } else { offset };
            m.check_write(at, buf.len() as u64, size)?;
        }
        match &self.backend {
            Backend::Inode if buf.is_empty() => Ok(0),
            Backend::Inode if self.flags().contains(OpenFlags::APPEND) => {
                let n = self.fs().append(self.node.ino(), buf).map(|(_, n)| n)?;
                self.modified(n);
                Ok(n)
            }
            Backend::Inode => {
                let n = self.fs().write(self.node.ino(), offset, buf)?;
                self.modified(n);
                Ok(n)
            }
            Backend::Device(dev) if dev.seekable() => dev.write(offset, buf, self.nonblock()),
            Backend::Device(_) | Backend::Object(_) => Err(Errno::ESPIPE),
            Backend::Path => Err(Errno::EBADF),
        }
    }

    /// `pwritev2(2)` with `RWF_APPEND` (`append: Some(true)`) or
    /// `RWF_NOAPPEND` (`Some(false)`): this one write appends, or goes to
    /// the offset although the file is `O_APPEND`; `None` keeps the file's
    /// own mode. `at` is the explicit offset (the file offset is left
    /// alone) or `None` for the file offset, which then advances (to the
    /// new end after an append). The open file's flags do not change. Only
    /// files on a file system care; the others write as usual.
    pub fn write_with(&self, at: Option<u64>, append: Option<bool>, buf: &[u8]) -> Result<usize> {
        if !self.writable() {
            return Err(Errno::EBADF);
        }
        if self.secret() {
            return Err(if at.is_some() { Errno::ESPIPE } else { Errno::EINVAL });
        }
        if !matches!(self.backend, Backend::Inode) {
            return match at {
                Some(offset) => self.write_at(offset, buf),
                None => self.write(buf),
            };
        }
        if at.is_some_and(|o| o > MAX_FILE_SIZE) {
            return Err(Errno::EINVAL);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let append = append.unwrap_or_else(|| self.flags().contains(OpenFlags::APPEND));
        match at {
            Some(_) if append => self.fs().append(self.node.ino(), buf).map(|(_, n)| n),
            Some(offset) => self.fs().write(self.node.ino(), offset, buf),
            None => {
                let mut off = self.offset.lock();
                if append {
                    let (end, n) = self.fs().append(self.node.ino(), buf)?;
                    *off = end + n as u64;
                    Ok(n)
                } else {
                    let n = self.fs().write(self.node.ino(), *off, buf)?;
                    *off += n as u64;
                    Ok(n)
                }
            }
        }
    }

    /// fallocate(2) (see [`FileSystem::fallocate`](crate::FileSystem::fallocate)):
    /// `EBADF` unless open for writing, `ESPIPE` on a pipe, `ENODEV` on
    /// anything else that is not a regular file, `EFBIG` for a range
    /// ending past [`MAX_FILE_SIZE`]. The offset does not move.
    pub fn fallocate(&self, mode: u32, offset: u64, len: u64) -> Result<()> {
        if !self.writable() {
            return Err(Errno::EBADF);
        }
        match &self.backend {
            Backend::Inode if self.kind == FileType::Regular => {}
            Backend::Path => return Err(Errno::EBADF),
            _ if self.kind == FileType::Fifo => return Err(Errno::ESPIPE),
            _ => return Err(Errno::ENODEV),
        }
        let Some(end) = offset.checked_add(len).filter(|&end| end <= MAX_FILE_SIZE) else {
            return Err(Errno::EFBIG);
        };
        // A memfd's seals: punching a hole writes, growing grows.
        if self.secret() {
            return Err(Errno::EOPNOTSUPP);
        }
        if let Some(m) = &self.memfd {
            let size = self.size()?;
            if mode & crate::FALLOC_FL_PUNCH_HOLE != 0 {
                m.check_write(offset, len, size.max(end))?;
            } else if mode & crate::FALLOC_FL_KEEP_SIZE == 0 {
                m.check_truncate(size, size.max(end))?;
            }
        }
        self.fs().fallocate(self.node.ino(), mode, offset, len)
    }

    /// Whether this is an open file on a file system (not a device, pipe,
    /// socket or other object): `RWF_NOWAIT` is `EOPNOTSUPP` there.
    pub fn on_inode(&self) -> bool {
        matches!(self.backend, Backend::Inode)
    }

    /// `lseek(2)`; returns the new offset. Directories accept `SEEK_SET` and
    /// `SEEK_CUR` to any position from 0 up (the file system decides what a
    /// listing reads from it), `SEEK_END`, `SEEK_DATA` and `SEEK_HOLE` are
    /// `EINVAL`; data/hole seeking treats the whole file as data. A failed
    /// seek leaves the offset alone.
    pub fn seek(&self, offset: i64, whence: Whence) -> Result<u64> {
        if self.secret() {
            return Err(Errno::ESPIPE);
        }
        let size = match &self.backend {
            Backend::Path => return Err(Errno::EBADF),
            Backend::Device(dev) if !dev.seekable() => return Err(Errno::ESPIPE),
            Backend::Object(o) if o.seek_stays_at_zero() => return Ok(0),
            Backend::Object(o) => o.seek_size().ok_or(Errno::ESPIPE)?,
            Backend::Device(_) => 0,
            Backend::Inode => self.node.attr()?.size,
        };
        let mut off = self.offset.lock();
        let new = match whence {
            Whence::Set => offset as i128,
            Whence::Cur => *off as i128 + offset as i128,
            Whence::End if self.kind == FileType::Directory => return Err(Errno::EINVAL),
            Whence::End => size as i128 + offset as i128,
            Whence::Data | Whence::Hole if self.kind != FileType::Regular => return Err(Errno::EINVAL),
            Whence::Data | Whence::Hole => {
                if offset < 0 || offset as u64 >= size {
                    return Err(Errno::ENXIO);
                }
                if whence == Whence::Data { offset as i128 } else { size as i128 }
            }
        };
        if new < 0 {
            return Err(Errno::EINVAL);
        }
        if new > MAX_FILE_SIZE as i128 {
            return Err(Errno::EOVERFLOW);
        }
        *off = new as u64;
        Ok(*off)
    }

    /// Lists the directory from the offset (`getdents64`), advancing it past
    /// every entry `emit` accepts. `emit` returns `false` when its buffer is
    /// full; that entry is listed again next time. Returns the number of
    /// entries accepted.
    pub fn read_dir(&self, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<usize> {
        if self.is_path_only() {
            return Err(Errno::EBADF);
        }
        if self.kind != FileType::Directory || self.object().is_some() {
            return Err(Errno::ENOTDIR);
        }
        let mut off = self.offset.lock();
        let mut accepted = 0;
        let mut refused = false;
        let mut next = *off;
        let r = self.fs().readdir(self.node.ino(), *off, &mut |e| {
            if emit(e) {
                accepted += 1;
                next = e.next;
                true
            } else {
                refused = true;
                false
            }
        });
        if r.is_ok() && accepted == 0 && !refused {
            // The end: the file system may park the position there.
            next = self.fs().readdir_end().unwrap_or(next);
        }
        *off = next;
        r.map(|()| accepted)
    }

    /// `ftruncate(2)`: the file must be open for writing (`EINVAL` otherwise,
    /// as Linux reports it; `EBADF` for `O_PATH`) and regular.
    pub fn truncate(&self, size: u64) -> Result<()> {
        if self.is_path_only() {
            return Err(Errno::EBADF);
        }
        if !self.writable() || self.kind != FileType::Regular || self.object().is_some() {
            return Err(Errno::EINVAL);
        }
        if self.node.mount().read_only() {
            return Err(Errno::EROFS);
        }
        if let Some(m) = &self.memfd {
            m.check_truncate(self.size()?, size)?;
        }
        self.node.truncate_unchecked(size)?;
        crate::inotify::file_event(self, crate::inotify::mask::IN_MODIFY);
        Ok(())
    }

    /// inotify(7): `IN_ACCESS` after a read that returned data.
    fn accessed(&self, n: usize) {
        if n > 0 {
            crate::inotify::file_event(self, crate::inotify::mask::IN_ACCESS);
        }
    }

    /// inotify(7): `IN_MODIFY` after a write that wrote something.
    fn modified(&self, n: usize) {
        if n > 0 {
            crate::inotify::file_event(self, crate::inotify::mask::IN_MODIFY);
        }
    }

    /// `fchmod(2)` of the file: [`NodeRef::chmod`], which a memfd's
    /// `F_SEAL_EXEC` may refuse (`EPERM` for a change of an execute bit).
    pub fn chmod(&self, cred: &crate::Cred, perm: u16) -> Result<()> {
        if let Some(m) = &self.memfd {
            m.check_chmod(self.node.attr()?.perm, perm)?;
        }
        self.node.chmod(cred, perm)
    }

    /// `fsync(2)`.
    pub fn sync(&self) -> Result<()> {
        match &self.backend {
            Backend::Path => Err(Errno::EBADF),
            Backend::Inode => self.fs().sync(),
            Backend::Device(_) | Backend::Object(_) => Err(Errno::EINVAL),
        }
    }

    /// `ioctl(2)`: devices only; `ENOTTY` for everything else.
    pub fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        match &self.backend {
            Backend::Device(dev) => dev.ioctl(request, arg),
            Backend::Object(obj) => obj.ioctl(request, arg),
            Backend::Path => Err(Errno::EBADF),
            Backend::Inode => Err(Errno::ENOTTY),
        }
    }

    /// The device behind this file, so the kernel can wait on it (a blocking
    /// read) without going through the VFS.
    pub fn device(&self) -> Option<&Arc<dyn CharDevice>> {
        match &self.backend {
            Backend::Device(dev) => Some(dev),
            _ => None,
        }
    }

    /// The object behind this file, if it is one.
    pub fn object(&self) -> Option<&Arc<dyn FileObject>> {
        match &self.backend {
            Backend::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// The object behind this file as a `T`, if it is one.
    pub fn object_as<T: FileObject>(&self) -> Option<Arc<T>> {
        let any: Arc<dyn core::any::Any + Send + Sync> = self.object()?.clone();
        any.downcast::<T>().ok()
    }

    // ---- readiness (ADR 0042) ------------------------------------------

    /// What the file is ready for now (`poll(2)`). Regular files and
    /// directories are always ready to read and write; devices and objects
    /// say for themselves.
    pub fn poll(&self) -> PollEvents {
        match &self.backend {
            Backend::Inode | Backend::Path => PollEvents::READABLE | PollEvents::WRITABLE,
            Backend::Device(dev) => dev.poll(),
            Backend::Object(obj) => obj.poll_as(self.nonblock()),
        }
    }

    /// Where to wait for [`poll`](Self::poll) to change; `None` if it never
    /// does by itself.
    pub fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        match &self.backend {
            Backend::Device(dev) => dev.wait_queue(),
            Backend::Object(obj) => obj.wait_queue(),
            Backend::Inode | Backend::Path => None,
        }
    }

    /// Bytes a read would return now (`FIONREAD`), for pipes and the like.
    pub fn bytes_readable(&self) -> Option<usize> {
        self.object().and_then(|o| o.bytes_readable())
    }

    /// Whether a blocking call waits on this file: it has a wait queue and
    /// is not `O_NONBLOCK`.
    fn waits(&self) -> Option<&Arc<WaitQueue>> {
        if self.nonblock() { None } else { self.wait_queue() }
    }

    /// `read(2)` that blocks: like [`read`](Self::read), but where that
    /// would fail with `EAGAIN` on a blocking file with a wait queue, wait
    /// until data (or end of file) comes. `EINTR` if the thread is
    /// interrupted first.
    pub fn read_wait(&self, buf: &mut [u8], blocker: &mut dyn Blocker) -> Result<usize> {
        self.read_wait_until(buf, blocker, Timeout::Never)
    }

    /// [`read_wait`](Self::read_wait) that gives up with `EAGAIN` at
    /// `timeout` (a socket's `SO_RCVTIMEO`).
    pub fn read_wait_until(&self, buf: &mut [u8], blocker: &mut dyn Blocker, timeout: Timeout) -> Result<usize> {
        match self.waits() {
            Some(queue) => poll::wait_until_done(queue, PollEvents::READABLE, blocker, timeout, || self.read(buf)),
            None => self.read(buf),
        }
    }

    /// `write(2)` that blocks: on a blocking file with a wait queue, write
    /// everything, waiting for room as often as needed (pipe(7): a
    /// blocking write "may block until all n bytes have been written").
    /// Interrupted or failing after some bytes went out, it returns how
    /// many (a partial write); before, the error (`EINTR`, `EPIPE`, ...).
    pub fn write_wait(&self, buf: &[u8], blocker: &mut dyn Blocker) -> Result<usize> {
        self.write_wait_until(buf, blocker, Timeout::Never)
    }

    /// [`write_wait`](Self::write_wait) that gives up at `timeout` (a
    /// socket's `SO_SNDTIMEO`): with the count written so far, or `EAGAIN`
    /// if nothing was.
    pub fn write_wait_until(&self, buf: &[u8], blocker: &mut dyn Blocker, timeout: Timeout) -> Result<usize> {
        let Some(queue) = self.waits() else { return self.write(buf) };
        let mut done = 0;
        loop {
            let result =
                poll::wait_until_done(queue, PollEvents::WRITABLE, blocker, timeout, || self.write(&buf[done..]));
            match result {
                Ok(n) => {
                    done += n;
                    if done >= buf.len() || n == 0 {
                        return Ok(done);
                    }
                }
                Err(_) if done > 0 => return Ok(done),
                Err(e) => return Err(e),
            }
        }
    }
}

/// inotify(7): closing the last descriptor of an open file description
/// is `IN_CLOSE_WRITE` if it was open for writing, else
/// `IN_CLOSE_NOWRITE`. (Dropping the node afterwards may free an unlinked
/// inode: `IN_DELETE_SELF`, see `NodeRef`'s drop.)
impl Drop for OpenFile {
    fn drop(&mut self) {
        if self.denies_write {
            self.node.loc().mount.sb.allow_write(self.node.ino());
        }
        if self.counts_text_writer {
            self.node.loc().mount.sb.drop_text_writer(self.node.ino());
        }
        if self.counts_writer {
            self.node.loc().mount.count_writer(false);
        }

        if crate::inotify::active() && matches!(self.backend, Backend::Inode | Backend::Device(_)) {
            use crate::inotify::mask::{IN_CLOSE_NOWRITE, IN_CLOSE_WRITE};
            crate::inotify::file_event(self, if self.writable() { IN_CLOSE_WRITE } else { IN_CLOSE_NOWRITE });
        }
    }
}

impl core::fmt::Debug for OpenFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenFile")
            .field("node", &self.node)
            .field("kind", &self.kind)
            .field("flags", &self.flags())
            .finish_non_exhaustive()
    }
}
