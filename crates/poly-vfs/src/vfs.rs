//! The VFS: mount table, device registry and path-based operations.

use crate::cred::{Access, Cred, cap};
use crate::device::CharDevice;
use crate::fdtable::At;
use crate::file::{Backend, OpenFile, OpenFlags, Origin};
use crate::fs::{FileSystem, NewNode, RenameFlags};
use crate::inotify;
use crate::mount::{Mount, MountFlags, MountId, MountInfo, MountTable};
use crate::node::{FsContext, Location, NodeRef, is_fs_object};
use crate::object::AnonFs;
use crate::path::{Found, Last, PATH_MAX, Parent, PathArg, Resolve, Walk, check_path};
use crate::sync::SpinMutex;
use crate::{Attr, DeviceId, Errno, FileType, Ino, PERM_MASK, Result, S_ISGID, S_ISUID, S_ISVTX, S_IXGRP, Stat};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

mod attrs;
mod context;
mod create;
mod mounting;
mod remove;
mod walk;

/// The directory and name a lookup found its node under, if any.
type Named = Option<(Location, Vec<u8>)>;

/// The virtual file system of one machine: what is mounted where, which
/// character devices exist, and every path-based operation.
///
/// Methods take the caller's [`FsContext`] (root, working directory,
/// umask) and [`Cred`], and an [`At`] saying where a relative path starts.
pub struct Vfs {
    pub(crate) mounts: SpinMutex<MountTable>,
    devices: SpinMutex<BTreeMap<DeviceId, Arc<dyn CharDevice>>>,
    /// Block devices, a number space of their own (a block device node
    /// opens like a character device, seekable, backed by one of these).
    block_devices: SpinMutex<BTreeMap<DeviceId, Arc<dyn CharDevice>>>,
    /// The inodes of file objects (pipes, event files, epoll sets), on a
    /// mount no path reaches (`object.rs`).
    anon: (Arc<AnonFs>, Arc<Mount>),
    /// The pipes of open FIFO inodes (`fifo.rs`).
    fifos: crate::fifo::Fifos,
    /// Counts every change of a name ([`namespace_generation`](Self::namespace_generation)).
    names_changed: core::sync::atomic::AtomicU64,
    /// Names found by scanning directories ([`Vfs::name_in`]), for naming
    /// paths (`/proc/<pid>/maps` names every mapped file): right while the
    /// namespace generation has not moved.
    names: SpinMutex<DirNames>,
}

/// Names of inodes in directories: `(device, directory, child)` to the
/// name, all made at `generation`.
#[derive(Default)]
struct DirNames {
    generation: u64,
    map: BTreeMap<(u32, u32, Ino, Ino), Vec<u8>>,
}

/// The most names [`DirNames`] keeps (it starts again when full).
const NAME_CACHE_MAX: usize = 4096;

impl core::fmt::Debug for Vfs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vfs").field("mounts", &self.mounts.lock().mounts).finish_non_exhaustive()
    }
}

impl Vfs {
    /// A VFS with `root` mounted at `/`.
    pub fn new(root: Arc<dyn FileSystem>) -> Vfs {
        Vfs::with_root_flags(root, MountFlags::default())
    }

    pub fn with_root_flags(root: Arc<dyn FileSystem>, flags: MountFlags) -> Vfs {
        let mut table = MountTable::new();
        let root_ino = root.root();
        let info = root.info();
        let sb = table.superblock(root, info);
        table.add(sb, root_ino, None, flags);
        let anon_fs = Arc::new(AnonFs::default());
        let anon_info = anon_fs.info();
        let anon_sb = table.superblock(anon_fs.clone(), anon_info);
        let anon_mount = table.detached(anon_sb, anon_fs.root());
        Vfs {
            mounts: SpinMutex::new(table),
            devices: SpinMutex::new(BTreeMap::new()),
            block_devices: SpinMutex::new(BTreeMap::new()),
            anon: (anon_fs, anon_mount),
            fifos: crate::fifo::Fifos::default(),
            names_changed: core::sync::atomic::AtomicU64::new(0),
            names: SpinMutex::new(DirNames::default()),
        }
    }

    /// A number that grows with every change of a name anywhere in this VFS
    /// (a file, directory, node or link made, a name removed, a rename),
    /// and never otherwise: a cache of directory contents made at one value
    /// is still right while it has not moved (the NT personality's
    /// case-insensitive lookup, ADR 0091).
    pub fn namespace_generation(&self) -> u64 {
        self.names_changed.load(core::sync::atomic::Ordering::Acquire)
    }

    /// Count a change of a name if `r` says it happened.
    fn name_changed<T>(&self, r: Result<T>) -> Result<T> {
        if r.is_ok() {
            self.names_changed.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
        }
        r
    }

    /// A new inode for a file object (`kind` and `perm` as `fstat` will
    /// report them, owned by `cred`), pinned by the returned reference and
    /// gone with its last one. Open files on the object take clones
    /// ([`OpenFile::on_object`](crate::OpenFile::on_object)); a pipe's two ends
    /// share one inode.
    pub fn anon_node(&self, kind: FileType, perm: u16, cred: &Cred) -> Result<NodeRef> {
        let (fs, mount) = &self.anon;
        let ino = fs.alloc(kind, perm & PERM_MASK, cred.uid, cred.gid);
        Location { mount: mount.clone(), ino }.pin()
    }

    /// A mount of `fs` that no path reaches, for files that never get a
    /// name (memfd_create(2)'s, [`memfd_file`](Self::memfd_file)).
    pub fn detached_mount(&self, fs: Arc<dyn FileSystem>) -> Arc<Mount> {
        let root = fs.root();
        let info = fs.info();
        let mut table = self.mounts.lock();
        let sb = table.superblock(fs, info);
        table.detached(sb, root)
    }

    /// memfd_create(2): a new empty regular file without links in the root
    /// of `mount` (a [`detached_mount`](Self::detached_mount) of a memory
    /// file system), mode `perm` (no umask), owned by `cred`'s file system
    /// ids, open `O_RDWR | O_LARGEFILE`, carrying `memfd`'s label and seals.
    pub fn memfd_file(
        &self,
        mount: &Arc<Mount>,
        cred: &Cred,
        perm: u16,
        memfd: Arc<crate::memfd::Memfd>,
    ) -> Result<Arc<OpenFile>> {
        let node = NewNode {
            kind: FileType::Regular,
            perm: perm & PERM_MASK,
            uid: cred.uid,
            gid: cred.gid,
            rdev: DeviceId::default(),
        };
        let ino = mount.sb.fs.create_tmpfile(mount.root, &node)?;
        let loc = Location { mount: mount.clone(), ino };
        let file = OpenFile::new(loc.pin()?, FileType::Regular, Backend::Inode, OpenFlags::RDWR | OpenFlags::LARGEFILE);
        Ok(Arc::new(file.with_memfd(memfd)))
    }

    /// An anonymous inode without a file type (an event file, timer,
    /// epoll set or signal file): `st_mode` is the permission bits alone
    /// and `fchmod` is `EOPNOTSUPP`, as on Linux.
    pub fn anon_inode(&self, perm: u16, cred: &Cred) -> Result<NodeRef> {
        let (fs, mount) = &self.anon;
        let ino = fs.alloc_typeless(perm & PERM_MASK, cred.uid, cred.gid);
        Location { mount: mount.clone(), ino }.pin()
    }

    #[cfg(test)]
    pub(crate) fn anon_inodes(&self) -> usize {
        self.anon.0.live()
    }

    /// A context with root and working directory at `/` and the default
    /// umask: what the first process starts with.
    pub fn root_context(&self) -> Result<FsContext> {
        let first = self.mounts.lock().mounts[0].clone();
        let root = Location { ino: first.root, mount: first }.pin()?;
        Ok(FsContext { cwd: root.clone(), root, umask: FsContext::DEFAULT_UMASK })
    }
}
