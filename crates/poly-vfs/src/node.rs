//! References to inodes on mounts, and the operations that need no path.

use crate::cred::{Access, Cred};
use crate::mount::Mount;
use crate::fs::{FsStats, magic_for};
use crate::{Attr, Errno, FileType, Ino, MAX_FILE_SIZE, Result, S_ISGID, S_ISUID, S_IXGRP, SetAttr, SetTime, Stat};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// A place in the tree during a walk: an inode as seen through a mount.
/// It does not keep the inode alive; [`NodeRef`] does.
#[derive(Clone)]
pub(crate) struct Location {
    pub(crate) mount: Arc<Mount>,
    pub(crate) ino: Ino,
}

impl Location {
    pub(crate) fn same(&self, other: &Location) -> bool {
        Arc::ptr_eq(&self.mount, &other.mount) && self.ino == other.ino
    }

    pub(crate) fn attr(&self) -> Result<Attr> {
        self.mount.sb.fs.getattr(self.ino)
    }

    pub(crate) fn is_mount_root(&self) -> bool {
        self.ino == self.mount.root
    }

    /// Pins the inode.
    pub(crate) fn pin(self) -> Result<NodeRef> {
        self.mount.sb.fs.open(self.ino)?;
        Ok(NodeRef { loc: self })
    }
}

/// A reference to an inode as seen through a mount. While it exists the
/// inode stays alive, also after its last name is removed. Cloning pins it
/// again; dropping unpins it.
pub struct NodeRef {
    loc: Location,
}

impl NodeRef {
    pub(crate) fn loc(&self) -> &Location {
        &self.loc
    }

    pub fn mount(&self) -> &Arc<Mount> {
        &self.loc.mount
    }

    pub fn ino(&self) -> Ino {
        self.loc.ino
    }

    /// Whether both name the same inode through the same mount.
    pub fn same(&self, other: &NodeRef) -> bool {
        self.loc.same(&other.loc)
    }

    pub fn attr(&self) -> Result<Attr> {
        self.loc.attr()
    }

    pub fn stat(&self) -> Result<Stat> {
        Ok(Stat::new(self.loc.mount.dev(), &self.attr()?))
    }

    pub fn kind(&self) -> Result<FileType> {
        Ok(self.attr()?.kind)
    }

    /// A symbolic link's target (`EINVAL` for anything else).
    pub fn readlink(&self) -> Result<Vec<u8>> {
        self.loc.mount.sb.fs.readlink(self.loc.ino)
    }

    /// `statfs(2)` / `fstatfs(2)` for the file system this node is on: its
    /// counts, the magic number by name if the file system left it 0, the
    /// mount's `ST_*` flags, and the device number as `f_fsid` (major,
    /// minor).
    pub fn statfs(&self) -> Result<FsStats> {
        let mount = &self.loc.mount;
        let mut st = mount.sb.fs.statfs()?;
        if st.magic == 0 {
            st.magic = magic_for(mount.sb.info.name);
        }
        let f = mount.flags;
        st.flags = [
            (mount.read_only(), crate::ST_RDONLY),
            (f.no_suid, crate::ST_NOSUID),
            (f.no_dev, crate::ST_NODEV),
            (f.no_exec, crate::ST_NOEXEC),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit);
        let dev = mount.dev();
        st.fsid = [dev.major, dev.minor];
        Ok(st)
    }

    /// `access(2)` on this node: `EACCES` when `cred` may not, `EROFS` when
    /// asking to write a file, directory or link on a read-only mount.
    pub fn access(&self, cred: &Cred, want: Access) -> Result<()> {
        let attr = self.attr()?;
        if want.contains(Access::WRITE) && self.loc.mount.read_only() && is_fs_object(attr.kind) {
            return Err(Errno::EROFS);
        }
        cred.check(&attr, want)
    }

    /// `chmod`: only the owner or root (`EPERM`). A non-root caller outside
    /// the file's group loses the set-group-ID bit (`chmod(2)`).
    pub fn chmod(&self, cred: &Cred, perm: u16) -> Result<()> {
        let attr = self.attr()?;
        self.writable_mount()?;
        if !cred.owns(&attr) {
            return Err(Errno::EPERM);
        }
        let mut perm = perm & crate::PERM_MASK;
        if !cred.is_root() && !cred.in_group(attr.gid) {
            perm &= !S_ISGID;
        }
        self.setattr(&SetAttr { perm: Some(perm), ..SetAttr::default() })
    }

    /// `chown`, `None` for "leave alone" (`-1`). Only root changes the owner;
    /// the owner may change the group to one of its own groups (`EPERM`
    /// otherwise). Changing either clears set-user-ID, and set-group-ID if
    /// the group may execute, on anything but a directory (`chown(2)`).
    pub fn chown(&self, cred: &Cred, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        let attr = self.attr()?;
        self.writable_mount()?;
        let uid = uid.filter(|&u| u != attr.uid || !cred.owns(&attr));
        let gid = gid.filter(|&g| g != attr.gid || !cred.owns(&attr));
        if !cred.is_root() {
            if uid.is_some() {
                return Err(Errno::EPERM);
            }
            if let Some(g) = gid
                && (cred.uid != attr.uid || !cred.in_group(g))
            {
                return Err(Errno::EPERM);
            }
        }
        if uid.is_none() && gid.is_none() {
            // Setting what is already there still touches ctime in Linux;
            // nothing observable depends on that yet.
            return Ok(());
        }
        let mut set = SetAttr { uid, gid, ..SetAttr::default() };
        if attr.kind != FileType::Directory {
            let mut perm = attr.perm & !S_ISUID;
            if perm & S_IXGRP != 0 {
                perm &= !S_ISGID;
            }
            if perm != attr.perm {
                set.perm = Some(perm);
            }
        }
        self.setattr(&set)
    }

    /// `utimensat`: `None` leaves a time alone (`UTIME_OMIT`). Setting
    /// explicit times needs ownership (`EPERM`); setting both to now needs
    /// ownership or write permission (`EACCES`).
    pub fn set_times(&self, cred: &Cred, atime: Option<SetTime>, mtime: Option<SetTime>) -> Result<()> {
        if atime.is_none() && mtime.is_none() {
            return Ok(());
        }
        let attr = self.attr()?;
        self.writable_mount()?;
        let explicit = matches!(atime, Some(SetTime::At(_))) || matches!(mtime, Some(SetTime::At(_)));
        if !cred.owns(&attr) {
            if explicit {
                return Err(Errno::EPERM);
            }
            cred.check(&attr, Access::WRITE)?;
        }
        self.setattr(&SetAttr { atime, mtime, ..SetAttr::default() })
    }

    /// `truncate(2)` by path: write permission needed.
    pub fn truncate(&self, cred: &Cred, size: u64) -> Result<()> {
        let attr = self.attr()?;
        match attr.kind {
            FileType::Directory => return Err(Errno::EISDIR),
            FileType::Regular => {}
            _ => return Err(Errno::EINVAL),
        }
        self.writable_mount()?;
        cred.check(&attr, Access::WRITE)?;
        self.truncate_unchecked(size)
    }

    /// Truncates without a permission check (`ftruncate` checks the open
    /// file's access mode instead).
    pub(crate) fn truncate_unchecked(&self, size: u64) -> Result<()> {
        if size > MAX_FILE_SIZE {
            return Err(Errno::EFBIG);
        }
        self.loc.mount.sb.fs.truncate(self.loc.ino, size)
    }

    fn setattr(&self, set: &SetAttr) -> Result<()> {
        self.loc.mount.sb.fs.setattr(self.loc.ino, set).map(|_| ())
    }

    fn writable_mount(&self) -> Result<()> {
        if self.loc.mount.read_only() { Err(Errno::EROFS) } else { Ok(()) }
    }
}

/// Whether writing to a node of this kind changes the file system (as
/// opposed to a device or pipe it stands for).
pub(crate) fn is_fs_object(kind: FileType) -> bool {
    matches!(kind, FileType::Regular | FileType::Directory | FileType::Symlink)
}

impl Clone for NodeRef {
    fn clone(&self) -> NodeRef {
        // The inode is pinned by `self`, so it exists and pinning it again
        // cannot fail for a file system that keeps the contract.
        let pinned = self.loc.mount.sb.fs.open(self.loc.ino);
        debug_assert!(pinned.is_ok(), "re-pinning a pinned inode failed: {pinned:?}");
        NodeRef { loc: self.loc.clone() }
    }
}

impl Drop for NodeRef {
    fn drop(&mut self) {
        self.loc.mount.sb.fs.release(self.loc.ino);
    }
}

impl core::fmt::Debug for NodeRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeRef").field("mount", &self.loc.mount.id).field("ino", &self.loc.ino).finish()
    }
}

/// A process's file system context (Linux's `fs_struct`): root directory,
/// working directory and file creation mask. `fork` clones it.
#[derive(Clone, Debug)]
pub struct FsContext {
    pub(crate) root: NodeRef,
    pub(crate) cwd: NodeRef,
    pub(crate) umask: u16,
}

impl FsContext {
    /// The default `umask`: `022`.
    pub const DEFAULT_UMASK: u16 = 0o022;

    pub fn root(&self) -> &NodeRef {
        &self.root
    }

    pub fn cwd(&self) -> &NodeRef {
        &self.cwd
    }

    pub fn umask(&self) -> u16 {
        self.umask
    }

    /// `umask(2)`: sets the mask (permission bits only), returns the old one.
    pub fn set_umask(&mut self, mask: u16) -> u16 {
        core::mem::replace(&mut self.umask, mask & 0o777)
    }

    /// `fchdir(2)`: the node must be a directory (`ENOTDIR`) the caller may
    /// search (`EACCES`).
    pub fn set_cwd(&mut self, cred: &Cred, dir: NodeRef) -> Result<()> {
        let attr = dir.attr()?;
        if attr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        cred.check(&attr, Access::EXEC)?;
        self.cwd = dir;
        Ok(())
    }

    /// `chroot(2)` to a node: root only (`EPERM`), a directory (`ENOTDIR`)
    /// the caller may search. The working directory is left alone, as in
    /// Linux.
    pub fn set_root(&mut self, cred: &Cred, dir: NodeRef) -> Result<()> {
        if !cred.is_root() {
            return Err(Errno::EPERM);
        }
        let attr = dir.attr()?;
        if attr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        cred.check(&attr, Access::EXEC)?;
        self.root = dir;
        Ok(())
    }
}
