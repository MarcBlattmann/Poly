//! File objects: open files that are not on any file system's tree
//! (pipes, event counters, timers, epoll sets, signal files), and the
//! anonymous file system that gives them an inode for `fstat`.
//!
//! An object's operations never block: one that would block returns
//! `EAGAIN`, and the open file's blocking calls
//! ([`OpenFile::read_wait`](crate::OpenFile::read_wait),
//! [`OpenFile::write_wait`](crate::OpenFile::write_wait)) wait on its
//! [`WaitQueue`] unless the file is non-blocking (ADR 0042).

use crate::fs::{DirEntry, FileSystem, FsInfo, NewNode, RenameFlags};
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Attr, DeviceId, Errno, FileType, Ino, Result, SetAttr, SetTime, Timespec};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

/// What an open file description reads from and writes to when it is not
/// a file system's inode or a device.
pub trait FileObject: Any + Send + Sync {
    /// Read into `buf`; `EAGAIN` instead of blocking. 0 is end of file.
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    /// Write from `buf`; `EAGAIN` instead of blocking. May take a prefix.
    fn write(&self, buf: &[u8]) -> Result<usize>;

    /// What the object is ready for now.
    fn poll(&self) -> PollEvents;

    /// Where waiters for a change of [`poll`](Self::poll) register; `None`
    /// for an object whose readiness never changes.
    fn wait_queue(&self) -> Option<&Arc<WaitQueue>>;

    /// Device-specific control; `ENOTTY` by default.
    fn ioctl(&self, _request: u32, _arg: u64) -> Result<u64> {
        Err(Errno::ENOTTY)
    }

    /// Bytes a read would return now (`FIONREAD`), for objects that hold
    /// a byte stream.
    fn bytes_readable(&self) -> Option<usize> {
        None
    }

    /// How long a blocking read (first) or write (second) may wait, in
    /// nanoseconds; `None`: until it can go on. Sockets answer their
    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` here (socket(7)); everything else
    /// waits for ever.
    fn timeouts(&self) -> (Option<u64>, Option<u64>) {
        (None, None)
    }

    /// Whether `lseek(2)` succeeds without moving (returning 0) instead of
    /// failing with `ESPIPE`: event counters and epoll sets do so (their
    /// spec sheets), pipes and sockets do not.
    fn seek_stays_at_zero(&self) -> bool {
        false
    }

    /// For an object Linux keeps on an anonymous inode: what follows
    /// `anon_inode:` in its `/proc/<pid>/fd` link (`inotify` for an inotify
    /// instance). Such a file's `fstat` mode has no file type bits.
    fn anon_name(&self) -> Option<&'static str> {
        None
    }

    /// For an object that seeks like a file of this many bytes (a dma-buf:
    /// `lseek(fd, 0, SEEK_END)` gives its size, the kernel's dma-buf
    /// documentation): `lseek` moves the offset as on a regular file of
    /// that size. `None` (the default): `ESPIPE`, or see
    /// [`seek_stays_at_zero`](Self::seek_stays_at_zero).
    fn seek_size(&self) -> Option<u64> {
        None
    }
}

/// The inode number of the anonymous file system's root directory.
const ROOT: Ino = 1;

/// The file system of [`FileObject`]s: one inode per object (a pipe's two
/// ends share theirs), alive while an open file pins it, never reachable
/// by a path. Mounted, invisibly, by every [`Vfs`](crate::Vfs).
#[derive(Default)]
pub(crate) struct AnonFs {
    nodes: SpinMutex<AnonNodes>,
}

#[derive(Default)]
struct AnonNodes {
    next: Ino,
    /// The attributes and the pins of every live inode but the root.
    live: BTreeMap<Ino, (Attr, usize)>,
    /// Inodes without a file type: what Linux calls anonymous inodes
    /// (event files, timers, epoll sets, signal files), whose `st_mode` is
    /// `0600` with no type bits and whose mode cannot change (the spec of
    /// fchmod: `anonymous_inodes`).
    typeless: BTreeSet<Ino>,
}

fn attr(ino: Ino, kind: FileType, perm: u16, uid: u32, gid: u32) -> Attr {
    Attr {
        ino,
        kind,
        perm,
        nlink: 1,
        uid,
        gid,
        size: 0,
        rdev: DeviceId::default(),
        atime: Timespec::default(),
        mtime: Timespec::default(),
        ctime: Timespec::default(),
        blocks: 0,
        blksize: 4096,
    }
}

impl AnonFs {
    /// A new inode, not yet pinned (the caller pins it at once).
    pub(crate) fn alloc(&self, kind: FileType, perm: u16, uid: u32, gid: u32) -> Ino {
        let mut n = self.nodes.lock();
        n.next = n.next.max(ROOT) + 1;
        let ino = n.next;
        n.live.insert(ino, (attr(ino, kind, perm, uid, gid), 0));
        ino
    }

    /// A new inode without a file type (see `typeless`), not yet pinned.
    pub(crate) fn alloc_typeless(&self, perm: u16, uid: u32, gid: u32) -> Ino {
        let ino = self.alloc(FileType::Regular, perm, uid, gid);
        self.nodes.lock().typeless.insert(ino);
        ino
    }

    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.nodes.lock().live.len()
    }
}

impl FileSystem for AnonFs {
    fn info(&self) -> FsInfo {
        FsInfo { name: "anon", read_only: false, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, _dir: Ino, name: &[u8]) -> Result<Ino> {
        if name == b".." { Ok(ROOT) } else { Err(Errno::ENOENT) }
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        if ino == ROOT {
            return Ok(Attr { nlink: 2, ..attr(ROOT, FileType::Directory, 0o500, 0, 0) });
        }
        self.nodes.lock().live.get(&ino).map(|(a, _)| *a).ok_or(Errno::ENOENT)
    }

    /// `fchmod` / `fchown` / `futimens` on an object change what `fstat`
    /// reports and nothing else; the mode and the owner of an inode without
    /// a type do not change (`EOPNOTSUPP`), only its times.
    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        if set.size.is_some() {
            return Err(Errno::EINVAL);
        }
        let mut n = self.nodes.lock();
        // Neither the mode nor the owner of an inode without a type
        // changes, not even by chown(-1, -1) (the specs of fchmod and
        // fchown: `anonymous_inodes`); only its times may be set.
        if n.typeless.contains(&ino) && set.atime.is_none() && set.mtime.is_none() {
            return Err(Errno::EOPNOTSUPP);
        }
        let (a, _) = n.live.get_mut(&ino).ok_or(Errno::EPERM)?;
        if let Some(perm) = set.perm {
            a.perm = perm;
        }
        if let Some(uid) = set.uid {
            a.uid = uid;
        }
        if let Some(gid) = set.gid {
            a.gid = gid;
        }
        let time = |t: SetTime| match t {
            SetTime::At(t) => t,
            SetTime::Now => Timespec::default(),
        };
        if let Some(t) = set.atime {
            a.atime = time(t);
        }
        if let Some(t) = set.mtime {
            a.mtime = time(t);
        }
        if let Some(t) = set.ctime {
            a.ctime = t;
        }
        Ok(*a)
    }

    fn create(&self, _: Ino, _: &[u8], _: &NewNode) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn mkdir(&self, _: Ino, _: &[u8], _: u16, _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn symlink(&self, _: Ino, _: &[u8], _: &[u8], _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn link(&self, _: Ino, _: &[u8], _: Ino) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn unlink(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::ENOENT)
    }

    fn rmdir(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::ENOENT)
    }

    fn rename(&self, _: Ino, _: &[u8], _: Ino, _: &[u8], _: RenameFlags) -> Result<()> {
        Err(Errno::ENOENT)
    }

    fn readlink(&self, _: Ino) -> Result<Vec<u8>> {
        Err(Errno::EINVAL)
    }

    fn read(&self, _: Ino, _: u64, _: &mut [u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: Ino, _: u64, _: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        let dots: [&[u8]; 2] = [b".", b".."];
        for (i, name) in dots.iter().enumerate().skip(pos as usize) {
            if !emit(&DirEntry { ino: ROOT, kind: FileType::Directory, name, next: i as u64 + 1 }) {
                break;
            }
        }
        Ok(())
    }

    fn open(&self, ino: Ino) -> Result<()> {
        if ino == ROOT {
            return Ok(());
        }
        let mut n = self.nodes.lock();
        let (_, pins) = n.live.get_mut(&ino).ok_or(Errno::ENOENT)?;
        *pins += 1;
        Ok(())
    }

    fn release(&self, ino: Ino) {
        let mut n = self.nodes.lock();
        if let Some((_, pins)) = n.live.get_mut(&ino) {
            *pins = pins.saturating_sub(1);
            if *pins == 0 {
                n.live.remove(&ino);
                n.typeless.remove(&ino);
            }
        }
    }

    fn typeless(&self, ino: Ino) -> bool {
        self.nodes.lock().typeless.contains(&ino)
    }
}
