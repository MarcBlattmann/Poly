//! Mounts, superblocks and the lookup (dentry) cache.
//!
//! A [`Mount`] puts the subtree of one file system below inode `root`
//! (its root for a normal mount, any directory for a bind mount) at a
//! mount point: an inode on another mount. Mounts on the same mount point
//! stack; the last one is visible, and unmounting it uncovers the one below.
//! Mounting on `/` covers the old root the same way, which is how a real
//! root file system later replaces the initial RAM disk.
//!
//! Every file system mounted at least once has one [`Superblock`], shared by
//! all its mounts (bind mounts included): its device number and, if the file
//! system allows it, the cache of name lookups. Caching is safe because every
//! change to a directory of such a file system goes through the [`Vfs`],
//! which forgets the names it changes.
//!
//! [`Vfs`]: crate::Vfs

use crate::fs::{FileSystem, FsInfo};
use crate::sync::SpinMutex;
use crate::{DeviceId, Errno, Ino, Result};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

/// Identifies a mount for as long as the VFS lives (ids are not reused).
pub type MountId = u32;

/// Per-mount options (`mount(2)`'s `MS_RDONLY`, `MS_NOEXEC`, `MS_NOSUID`,
/// `MS_NODEV`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MountFlags {
    /// Changes fail with `EROFS`.
    pub read_only: bool,
    /// Files here cannot be executed; see [`Vfs::open_exec`](crate::Vfs::open_exec).
    pub no_exec: bool,
    /// Set-user-ID and set-group-ID bits are ignored at exec (the ELF
    /// loader asks [`Mount::flags`]).
    pub no_suid: bool,
    /// Device nodes here cannot be opened (`EACCES`).
    pub no_dev: bool,
}

/// One mounted subtree.
pub struct Mount {
    pub(crate) id: MountId,
    pub(crate) sb: Arc<Superblock>,
    pub(crate) root: Ino,
    /// The mount point: a mount and an inode on it. `None` for the first
    /// root.
    pub(crate) parent: Option<(Arc<Mount>, Ino)>,
    pub(crate) flags: MountFlags,
}

impl Mount {
    pub fn id(&self) -> MountId {
        self.id
    }

    pub fn flags(&self) -> MountFlags {
        self.flags
    }

    pub fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.sb.fs
    }

    /// The file system's device number (`st_dev`).
    pub fn dev(&self) -> DeviceId {
        self.sb.dev
    }

    /// The inode this mount shows at its mount point.
    pub fn root_ino(&self) -> Ino {
        self.root
    }

    /// Whether changes must fail with `EROFS`.
    pub fn read_only(&self) -> bool {
        self.flags.read_only || self.sb.info.read_only
    }
}

impl core::fmt::Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("id", &self.id)
            .field("fs", &self.sb.info.name)
            .field("root", &self.root)
            .field("parent", &self.parent.as_ref().map(|(m, ino)| (m.id, *ino)))
            .field("flags", &self.flags)
            .finish()
    }
}

/// What [`Vfs::mounts`](crate::Vfs::mounts) reports about a mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountInfo {
    pub id: MountId,
    /// The mount the mount point is on, and the mount point's inode.
    pub parent: Option<(MountId, Ino)>,
    pub fs_name: &'static str,
    pub dev: DeviceId,
    pub root: Ino,
    pub flags: MountFlags,
}

/// The state shared by all mounts of one file system.
pub(crate) struct Superblock {
    pub(crate) fs: Arc<dyn FileSystem>,
    pub(crate) info: FsInfo,
    pub(crate) dev: DeviceId,
    cache: Option<SpinMutex<NameCache>>,
}

impl Superblock {
    /// `fs.lookup`, through the cache when the file system allows one.
    pub(crate) fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        let Some(cache) = &self.cache else { return self.fs.lookup(dir, name) };
        if name == b".." {
            // Not cached: a directory's parent changes when it is moved,
            // and the move is recorded under the directory's name, not "..".
            return self.fs.lookup(dir, name);
        }
        if let Some(ino) = cache.lock().get(dir, name) {
            return Ok(ino);
        }
        let ino = self.fs.lookup(dir, name)?;
        cache.lock().insert(dir, name, ino);
        Ok(ino)
    }

    /// Drops what the cache knows about `dir/name`.
    pub(crate) fn forget(&self, dir: Ino, name: &[u8]) {
        if let Some(cache) = &self.cache {
            cache.lock().remove(dir, name);
        }
    }

    #[cfg(test)]
    pub(crate) fn cached(&self) -> usize {
        self.cache.as_ref().map_or(0, |c| c.lock().entries)
    }
}

/// Positive lookups only (a missing name is asked again), bounded: when
/// full it starts over, which is crude but never wrong.
struct NameCache {
    dirs: BTreeMap<Ino, BTreeMap<Box<[u8]>, Ino>>,
    entries: usize,
}

/// Entries the lookup cache holds before it is cleared.
const NAME_CACHE_ENTRIES: usize = 4096;

impl NameCache {
    fn get(&self, dir: Ino, name: &[u8]) -> Option<Ino> {
        self.dirs.get(&dir)?.get(name).copied()
    }

    fn insert(&mut self, dir: Ino, name: &[u8], ino: Ino) {
        if self.entries >= NAME_CACHE_ENTRIES {
            self.dirs.clear();
            self.entries = 0;
        }
        if self.dirs.entry(dir).or_default().insert(name.into(), ino).is_none() {
            self.entries += 1;
        }
    }

    fn remove(&mut self, dir: Ino, name: &[u8]) {
        if let Some(names) = self.dirs.get_mut(&dir) {
            if names.remove(name).is_some() {
                self.entries -= 1;
            }
            if names.is_empty() {
                self.dirs.remove(&dir);
            }
        }
    }
}

/// All mounts, in mount order.
pub(crate) struct MountTable {
    pub(crate) mounts: Vec<Arc<Mount>>,
    next_id: MountId,
    next_anon_minor: u32,
    supers: Vec<Weak<Superblock>>,
}

impl MountTable {
    pub(crate) fn new() -> MountTable {
        MountTable { mounts: Vec::new(), next_id: 1, next_anon_minor: 1, supers: Vec::new() }
    }

    /// The superblock of `fs`, made on its first mount. `info` is
    /// `fs.info()`, asked by the caller before taking the table's lock.
    pub(crate) fn superblock(&mut self, fs: Arc<dyn FileSystem>, info: FsInfo) -> Arc<Superblock> {
        self.supers.retain(|w| w.strong_count() > 0);
        let key = Arc::as_ptr(&fs) as *const u8;
        if let Some(sb) =
            self.supers.iter().filter_map(Weak::upgrade).find(|sb| Arc::as_ptr(&sb.fs) as *const u8 == key)
        {
            return sb;
        }
        // Anonymous device numbers, major 0, as Linux gives memory file systems.
        let dev = DeviceId::new(0, self.next_anon_minor);
        self.next_anon_minor += 1;
        let cache = info.cache_lookups.then(|| SpinMutex::new(NameCache { dirs: BTreeMap::new(), entries: 0 }));
        let sb = Arc::new(Superblock { fs, info, dev, cache });
        self.supers.push(Arc::downgrade(&sb));
        sb
    }

    pub(crate) fn add(
        &mut self,
        sb: Arc<Superblock>,
        root: Ino,
        parent: Option<(Arc<Mount>, Ino)>,
        flags: MountFlags,
    ) -> Arc<Mount> {
        let m = Arc::new(Mount { id: self.next_id, sb, root, parent, flags });
        self.next_id += 1;
        self.mounts.push(m.clone());
        m
    }

    /// A mount that is in no tree: the anonymous file system of file
    /// objects, which no path reaches and nothing can unmount.
    pub(crate) fn detached(&mut self, sb: Arc<Superblock>, root: Ino) -> Arc<Mount> {
        let m = Arc::new(Mount { id: self.next_id, sb, root, parent: None, flags: MountFlags::default() });
        self.next_id += 1;
        m
    }

    /// The topmost mount on mount point `(mount, ino)`.
    pub(crate) fn covering(&self, mount: MountId, ino: Ino) -> Option<&Arc<Mount>> {
        self.mounts.iter().rev().find(|m| matches!(&m.parent, Some((p, i)) if p.id == mount && *i == ino))
    }

    pub(crate) fn has_children(&self, mount: MountId) -> bool {
        self.mounts.iter().any(|m| matches!(&m.parent, Some((p, _)) if p.id == mount))
    }

    /// Removes `id` and, transitively, everything mounted below it.
    pub(crate) fn remove_tree(&mut self, id: MountId) -> Result<()> {
        if !self.mounts.iter().any(|m| m.id == id) {
            return Err(Errno::EINVAL);
        }
        let mut doomed = alloc::vec![id];
        let mut i = 0;
        while i < doomed.len() {
            let d = doomed[i];
            doomed.extend(self.mounts.iter().filter(|m| matches!(&m.parent, Some((p, _)) if p.id == d)).map(|m| m.id));
            i += 1;
        }
        self.mounts.retain(|m| !doomed.contains(&m.id));
        Ok(())
    }
}
