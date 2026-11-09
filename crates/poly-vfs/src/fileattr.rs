//! Inode flags (file_getattr(2), file_setattr(2), the `FS_IOC_FSGETXATTR`
//! ioctl's `struct fsxattr`) and file handles (name_to_handle_at(2),
//! open_by_handle_at(2)).
//!
//! **Inode flags** are the `FS_XFLAG_*` bits of UAPI `linux/fs.h`
//! ([`xflag`]). A file system keeps them per inode
//! ([`FileSystem::xflags`](crate::FileSystem::xflags)); one that keeps
//! none, and every inode that is not a regular file or directory, answers
//! `EOPNOTSUPP`. Reading needs no permission. Changing them (the spec
//! sheet of file_setattr):
//!
//! | Check | Error |
//! |---|---|
//! | a bit `linux/fs.h` does not define | `EINVAL` |
//! | the caller neither owns the file nor has `CAP_FOWNER` | `EPERM` |
//! | `APPEND` or `IMMUTABLE` set or cleared without `CAP_LINUX_IMMUTABLE` | `EPERM` |
//! | a read-only mount | `EROFS` |
//! | a flag, extent size or project id the file system does not keep | `EOPNOTSUPP` |
//!
//! Their effects ([`check_open`], [`check_remove`]): an `APPEND` file opens
//! for writing only with `O_APPEND`, an `IMMUTABLE` one not at all, and
//! neither can be removed (`EPERM`).
//!
//! **File handles**: the file system encodes an inode as a few opaque
//! bytes and a type ([`FileHandle`], at most [`MAX_HANDLE_SZ`] bytes) and
//! decodes them back to a live inode, or `ESTALE`
//! ([`FileSystem::handle`](crate::FileSystem::handle),
//! [`FileSystem::handle_ino`](crate::FileSystem::handle_ino)); file
//! systems that cannot (procfs, file objects) say `EOPNOTSUPP`.

use crate::cred::{Cred, cap};
use crate::file::{Backend, OpenFile, OpenFlags};
use crate::mount::Mount;
use crate::node::{Location, NodeRef};
use crate::{Errno, FileType, Result};
use alloc::sync::Arc;

/// `FS_XFLAG_*` (UAPI `linux/fs.h`).
pub mod xflag {
    pub const REALTIME: u32 = 0x0000_0001;
    pub const PREALLOC: u32 = 0x0000_0002;
    pub const IMMUTABLE: u32 = 0x0000_0008;
    pub const APPEND: u32 = 0x0000_0010;
    pub const SYNC: u32 = 0x0000_0020;
    pub const NOATIME: u32 = 0x0000_0040;
    pub const NODUMP: u32 = 0x0000_0080;
    pub const RTINHERIT: u32 = 0x0000_0100;
    pub const PROJINHERIT: u32 = 0x0000_0200;
    pub const NOSYMLINKS: u32 = 0x0000_0400;
    pub const EXTSIZE: u32 = 0x0000_0800;
    pub const EXTSZINHERIT: u32 = 0x0000_1000;
    pub const NODEFRAG: u32 = 0x0000_2000;
    pub const FILESTREAM: u32 = 0x0000_4000;
    pub const DAX: u32 = 0x0000_8000;
    pub const COWEXTSIZE: u32 = 0x0001_0000;
    pub const HASATTR: u32 = 0x8000_0000;
    /// Every bit `linux/fs.h` defines.
    pub const KNOWN: u64 = (REALTIME
        | PREALLOC
        | IMMUTABLE
        | APPEND
        | SYNC
        | NOATIME
        | NODUMP
        | RTINHERIT
        | PROJINHERIT
        | NOSYMLINKS
        | EXTSIZE
        | EXTSZINHERIT
        | NODEFRAG
        | FILESTREAM
        | DAX
        | COWEXTSIZE
        | HASATTR) as u64;
}

/// `MAX_HANDLE_SZ` (glibc `fcntl.h`, `bits/fcntl-linux.h`; the man page
/// of name_to_handle_at): the most bytes a handle has.
pub const MAX_HANDLE_SZ: usize = 128;

/// `struct file_attr` (UAPI `linux/fs.h`) / `struct fsxattr`: the inode
/// flags and the XFS-style extras, which no Poly file system keeps (0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileAttr {
    pub xflags: u64,
    pub extsize: u32,
    pub nextents: u32,
    pub projid: u32,
    pub cowextsize: u32,
}

/// A file handle: the file system's type and bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle {
    pub kind: i32,
    len: u8,
    bytes: [u8; MAX_HANDLE_SZ],
}

impl FileHandle {
    /// A handle of `kind` with `bytes` (at most [`MAX_HANDLE_SZ`]; longer
    /// is cut, which no file system asks for).
    pub fn new(kind: i32, bytes: &[u8]) -> FileHandle {
        let len = bytes.len().min(MAX_HANDLE_SZ);
        let mut b = [0u8; MAX_HANDLE_SZ];
        b[..len].copy_from_slice(&bytes[..len]);
        FileHandle { kind, len: len as u8, bytes: b }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

/// The flags of a node, `EOPNOTSUPP` if it has none.
fn flags_of(node: &Location) -> Result<u32> {
    node.mount.sb.fs.xflags(node.ino)
}

impl NodeRef {
    /// file_getattr(2): the inode flags; no permission needed.
    pub fn file_getattr(&self) -> Result<FileAttr> {
        Ok(FileAttr { xflags: u64::from(flags_of(self.loc())?), ..FileAttr::default() })
    }

    /// file_setattr(2): replace the inode flags with `attr.xflags` (its
    /// `nextents` is ignored), in the order of the module's table.
    pub fn file_setattr(&self, cred: &Cred, attr: &FileAttr) -> Result<()> {
        if attr.xflags & !xflag::KNOWN != 0 {
            return Err(Errno::EINVAL);
        }
        let old = flags_of(self.loc())?;
        let new = attr.xflags as u32;
        if !cred.owns(&self.attr()?) {
            return Err(Errno::EPERM);
        }
        let guarded = xflag::APPEND | xflag::IMMUTABLE;
        if (old ^ new) & guarded != 0 && !cred.has(cap::LINUX_IMMUTABLE) {
            return Err(Errno::EPERM);
        }
        if self.mount().read_only() {
            return Err(Errno::EROFS);
        }
        if attr.extsize != 0 || attr.projid != 0 || attr.cowextsize != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        self.fs().set_xflags(self.ino(), new)
    }

    /// name_to_handle_at(2): the handle of this node's inode.
    pub fn file_handle(&self) -> Result<FileHandle> {
        self.fs().handle(self.ino())
    }
}

/// Whether an open with `flags` may go on, as the inode's flags decide:
/// writing (or truncating) an `IMMUTABLE` file, or writing an `APPEND`
/// file without `O_APPEND`, is `EPERM`.
pub(crate) fn check_open(node: &Location, flags: OpenFlags, write: bool) -> Result<()> {
    let write = write || flags.contains(OpenFlags::TRUNC);
    if !write {
        return Ok(());
    }
    let f = flags_of(node).unwrap_or(0);
    if f & xflag::IMMUTABLE != 0 || (f & xflag::APPEND != 0 && !flags.contains(OpenFlags::APPEND)) {
        return Err(Errno::EPERM);
    }
    Ok(())
}

/// Whether a name of `node` may be removed: not if it is `IMMUTABLE` or
/// `APPEND` (`EPERM`).
pub(crate) fn check_remove(node: &Location) -> Result<()> {
    if flags_of(node).unwrap_or(0) & (xflag::IMMUTABLE | xflag::APPEND) != 0 {
        return Err(Errno::EPERM);
    }
    Ok(())
}

impl crate::Vfs {
    /// open_by_handle_at(2): open the inode `handle` names on `mount`'s
    /// file system (`ESTALE` if it names none) with `flags` as open(2)
    /// takes them; `O_CREAT`, `O_EXCL` and `O_TMPFILE` do nothing here.
    /// The caller has checked `CAP_DAC_READ_SEARCH`; access to the file is
    /// checked as open(2) does.
    pub fn open_handle(
        &self,
        cred: &Cred,
        mount: &Arc<Mount>,
        handle: &FileHandle,
        flags: OpenFlags,
    ) -> Result<Arc<OpenFile>> {
        let ino = mount.sb.fs.handle_ino(handle.kind, handle.bytes())?;
        let loc = Location { mount: mount.clone(), ino };
        let flags =
            flags.without(OpenFlags::CREAT | OpenFlags::EXCL | OpenFlags::TMPFILE.without(OpenFlags::DIRECTORY));
        if flags.contains(OpenFlags::PATH) {
            let flags = flags.intersection(OpenFlags::PATH | OpenFlags::DIRECTORY | OpenFlags::NOFOLLOW);
            let kind = loc.attr()?.kind;
            return Ok(Arc::new(OpenFile::new(loc.pin()?, kind, Backend::Path, flags)));
        }
        let attr = loc.attr()?;
        if flags.contains(OpenFlags::DIRECTORY) && attr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let neither = flags.intersection(OpenFlags::ACCMODE) == OpenFlags::ACCMODE;
        let write = flags.writable() || neither;
        match attr.kind {
            FileType::Directory if write || flags.contains(OpenFlags::TRUNC) => return Err(Errno::EISDIR),
            FileType::Regular | FileType::Directory => {}
            // Handles of other kinds open as O_PATH would need; Poly makes
            // them only for files and directories it can open this way.
            FileType::Symlink => return Err(Errno::ELOOP),
            _ => return Err(Errno::ENXIO),
        }
        if write && mount.read_only_for(ino) {
            return Err(Errno::EROFS);
        }
        let mut want = crate::Access::NONE;
        if flags.readable() || neither {
            want = want | crate::Access::READ;
        }
        if write || flags.contains(OpenFlags::TRUNC) {
            want = want | crate::Access::WRITE;
        }
        cred.check(&attr, want)?;
        check_open(&loc, flags, write)?;
        let node = loc.pin()?;
        if flags.contains(OpenFlags::TRUNC) && attr.kind == FileType::Regular && attr.size != 0 {
            node.clear_setid(cred)?;
            node.truncate_unchecked(0)?;
        }
        Ok(Arc::new(OpenFile::new(node, attr.kind, Backend::Inode, flags)))
    }
}
