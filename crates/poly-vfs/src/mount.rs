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
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

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

impl MountFlags {
    const READ_ONLY: u8 = 1;
    const NO_EXEC: u8 = 2;
    const NO_SUID: u8 = 4;
    const NO_DEV: u8 = 8;

    fn to_bits(self) -> u8 {
        [
            (self.read_only, Self::READ_ONLY),
            (self.no_exec, Self::NO_EXEC),
            (self.no_suid, Self::NO_SUID),
            (self.no_dev, Self::NO_DEV),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit)
    }

    fn from_bits(bits: u8) -> MountFlags {
        MountFlags {
            read_only: bits & Self::READ_ONLY != 0,
            no_exec: bits & Self::NO_EXEC != 0,
            no_suid: bits & Self::NO_SUID != 0,
            no_dev: bits & Self::NO_DEV != 0,
        }
    }
}

/// One mounted subtree.
pub struct Mount {
    pub(crate) id: MountId,
    pub(crate) sb: Arc<Superblock>,
    pub(crate) root: Ino,
    /// The mount point: a mount and an inode on it. `None` for the root
    /// of a tree (a namespace's first mount) and for a detached mount.
    /// Moving a mount (`move_mount(2)`, `pivot_root(2)`) changes it.
    parent: SpinMutex<Option<(Arc<Mount>, Ino)>>,
    /// [`MountFlags`] as bits: a remount changes them in place.
    flags: AtomicU8,
    /// The rest of the per-mount attributes, as Linux's `MOUNT_ATTR_*`
    /// bits other than the four of [`MountFlags`] (`linux/mount.h`: the
    /// access time mode `0x70`, `NODIRATIME 0x80`, `NOSYMFOLLOW
    /// 0x200000`): kept for `statmount(2)` and reported by `statfs(2)`,
    /// not enforced.
    attrs: AtomicU32,
    /// The propagation type, as Linux's `MS_PRIVATE`, `MS_SHARED`,
    /// `MS_SLAVE` or `MS_UNBINDABLE` value (`linux/mount.h`). Poly
    /// propagates nothing; the type is kept for `statmount(2)` and the
    /// checks that refuse shared mounts (`pivot_root(2)`).
    propagation: AtomicU32,
    /// Open files writing to inodes through this mount: making it
    /// read-only then is `EBUSY` (`mount_setattr(2)`).
    writers: AtomicU32,
}

/// `MS_PRIVATE` (`linux/mount.h`): the propagation type of a new mount.
pub const PROPAGATION_PRIVATE: u32 = 1 << 18;
/// `MS_SHARED` (`linux/mount.h`).
pub const PROPAGATION_SHARED: u32 = 1 << 20;

impl Mount {
    pub fn id(&self) -> MountId {
        self.id
    }

    pub fn flags(&self) -> MountFlags {
        MountFlags::from_bits(self.flags.load(Ordering::Relaxed))
    }

    pub(crate) fn set_flags(&self, flags: MountFlags) {
        self.flags.store(flags.to_bits(), Ordering::Relaxed);
    }

    /// Change the per-mount flags in place (`mount_setattr(2)`,
    /// `fsmount(2)`'s attributes).
    pub fn change_flags(&self, flags: MountFlags) {
        self.set_flags(flags);
    }

    /// Where this mount is mounted: the mount and inode of its mount
    /// point; `None` for the root of a tree or a detached mount.
    pub fn parent(&self) -> Option<(Arc<Mount>, Ino)> {
        self.parent.lock().clone()
    }

    pub(crate) fn set_parent(&self, parent: Option<(Arc<Mount>, Ino)>) {
        *self.parent.lock() = parent;
    }

    /// Whether this mount is mounted on `(mount, ino)` (any inode of
    /// `mount` when `ino` is `None`).
    pub(crate) fn is_on(&self, mount: MountId, ino: Option<Ino>) -> bool {
        matches!(&*self.parent.lock(), Some((p, i)) if p.id == mount && ino.is_none_or(|x| x == *i))
    }

    /// The Linux `MOUNT_ATTR_*` bits beyond [`MountFlags`] (see the field).
    pub fn attrs(&self) -> u32 {
        self.attrs.load(Ordering::Relaxed)
    }

    pub fn set_attrs(&self, attrs: u32) {
        self.attrs.store(attrs, Ordering::Relaxed);
    }

    /// The propagation type (an `MS_*` value).
    pub fn propagation(&self) -> u32 {
        self.propagation.load(Ordering::Relaxed)
    }

    pub fn set_propagation(&self, propagation: u32) {
        self.propagation.store(propagation, Ordering::Relaxed);
    }

    /// Open files writing through this mount.
    pub fn writers(&self) -> u32 {
        self.writers.load(Ordering::Relaxed)
    }

    /// One more (`true`) or one fewer open file writes through this mount
    /// and to its superblock.
    pub(crate) fn count_writer(&self, more: bool) {
        if more {
            self.writers.fetch_add(1, Ordering::Relaxed);
            self.sb.writers.fetch_add(1, Ordering::Relaxed);
        } else {
            self.writers.fetch_sub(1, Ordering::Relaxed);
            self.sb.writers.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Open files writing to the file system this mount shows, through
    /// any of its mounts (a read-only reconfiguration is then `EBUSY`).
    pub fn sb_writers(&self) -> u32 {
        self.sb.writers.load(Ordering::Relaxed)
    }

    /// Whether the file system itself is read-only (made so by a remount
    /// without `MS_BIND` or an `fsconfig(2)` reconfiguration, or by its
    /// nature): every mount of it is then read-only.
    pub fn sb_read_only(&self) -> bool {
        self.sb.ro.load(Ordering::Relaxed) || self.sb.info.read_only
    }

    /// Make the file system read-only as a whole, or writable again.
    pub fn set_sb_read_only(&self, ro: bool) {
        self.sb.ro.store(ro, Ordering::Relaxed);
    }

    /// What the file system was mounted from (`mount(2)`'s source, the
    /// `source` parameter of `fsconfig(2)`), as recorded by
    /// [`set_sb_info`](Self::set_sb_info).
    pub fn sb_source(&self) -> Vec<u8> {
        self.sb.meta.lock().0.clone()
    }

    /// The file system's options as `/proc/self/mountinfo` shows them
    /// (without `rw`/`ro`), as recorded by [`set_sb_info`](Self::set_sb_info).
    pub fn sb_options(&self) -> Vec<u8> {
        self.sb.meta.lock().1.clone()
    }

    /// Record the source and options of the file system (shared by all
    /// its mounts).
    pub fn set_sb_info(&self, source: &[u8], options: &[u8]) {
        *self.sb.meta.lock() = (source.to_vec(), options.to_vec());
    }

    /// The file system's type name (`"tmpfs"`).
    pub fn fs_name(&self) -> &'static str {
        self.sb.info.name
    }

    /// Whether `other` shows the same file system (superblock).
    pub fn same_sb(&self, other: &Mount) -> bool {
        Arc::ptr_eq(&self.sb, &other.sb)
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
        self.flags().read_only || self.sb_read_only()
    }

    /// Whether writing inode `ino` must fail with `EROFS`: as
    /// [`read_only`](Self::read_only), but a file the file system makes
    /// writable ([`FileSystem::writable_file`]) may be written unless the
    /// mount is read-only.
    pub fn read_only_for(&self, ino: Ino) -> bool {
        self.flags().read_only
            || self.sb.ro.load(Ordering::Relaxed)
            || (self.sb.info.read_only && !self.sb.fs.writable_file(ino))
    }
}

impl core::fmt::Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("id", &self.id)
            .field("fs", &self.sb.info.name)
            .field("root", &self.root)
            .field("parent", &self.parent().map(|(m, ino)| (m.id, ino)))
            .field("flags", &self.flags())
            .finish()
    }
}

/// The first mount of the tree `m` belongs to: the root of its mount
/// namespace ([`Vfs::copy_namespace`](crate::Vfs::copy_namespace)).
pub(crate) fn base_of(m: &Arc<Mount>) -> MountId {
    let mut cur = m.clone();
    while let Some((p, _)) = cur.parent() {
        cur = p;
    }
    cur.id
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
    /// Inodes running as programs, and how many open files say so
    /// ([`Vfs::open_exec`](crate::Vfs::open_exec)): opening them for
    /// writing or truncating them is `ETXTBSY` (open(2), truncate(2)).
    exec_busy: SpinMutex<BTreeMap<Ino, u32>>,
    /// Made read-only as a whole (see [`Mount::sb_read_only`]).
    ro: AtomicBool,
    /// Open files writing to it ([`Mount::sb_writers`]).
    writers: AtomicU32,
    /// The source and options recorded by [`Mount::set_sb_info`].
    meta: SpinMutex<(Vec<u8>, Vec<u8>)>,
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

    /// Whether `ino` is running as a program (`ETXTBSY` for writers).
    pub(crate) fn exec_busy(&self, ino: Ino) -> bool {
        self.exec_busy.lock().contains_key(&ino)
    }

    /// One more open file runs `ino` as a program.
    pub(crate) fn deny_write(&self, ino: Ino) {
        *self.exec_busy.lock().entry(ino).or_insert(0) += 1;
    }

    /// One open file running `ino` as a program is gone.
    pub(crate) fn allow_write(&self, ino: Ino) {
        let mut busy = self.exec_busy.lock();
        if let Some(n) = busy.get_mut(&ino) {
            *n -= 1;
            if *n == 0 {
                busy.remove(&ino);
            }
        }
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
    /// Namespaces whose first mount changed (`pivot_root(2)`): the id a
    /// namespace was named by when it was made, and its first mount now.
    aliases: BTreeMap<MountId, MountId>,
}

impl MountTable {
    pub(crate) fn new() -> MountTable {
        MountTable { mounts: Vec::new(), next_id: 1, next_anon_minor: 1, supers: Vec::new(), aliases: BTreeMap::new() }
    }

    /// The first mount now of the namespace named `name` (its first mount
    /// when it was made).
    pub(crate) fn current_base(&self, name: MountId) -> MountId {
        self.aliases.get(&name).copied().unwrap_or(name)
    }

    /// The namespace whose first mount was `old` now starts at `new`.
    pub(crate) fn rebase(&mut self, old: MountId, new: MountId) {
        let mut renamed = false;
        for v in self.aliases.values_mut() {
            if *v == old {
                *v = new;
                renamed = true;
            }
        }
        if !renamed {
            self.aliases.insert(old, new);
        }
    }

    /// The namespace named `name` is gone.
    pub(crate) fn forget_alias(&mut self, name: MountId) {
        self.aliases.remove(&name);
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
        let sb = Arc::new(Superblock {
            fs,
            info,
            dev,
            cache,
            exec_busy: SpinMutex::new(BTreeMap::new()),
            ro: AtomicBool::new(false),
            writers: AtomicU32::new(0),
            meta: SpinMutex::new((Vec::new(), Vec::new())),
        });
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
        let m = self.new_mount(sb, root, parent, flags);
        self.mounts.push(m.clone());
        m
    }

    /// A mount with the next id, in no list yet.
    pub(crate) fn new_mount(
        &mut self,
        sb: Arc<Superblock>,
        root: Ino,
        parent: Option<(Arc<Mount>, Ino)>,
        flags: MountFlags,
    ) -> Arc<Mount> {
        let m = Arc::new(Mount {
            id: self.next_id,
            sb,
            root,
            parent: SpinMutex::new(parent),
            flags: AtomicU8::new(flags.to_bits()),
            attrs: AtomicU32::new(0),
            propagation: AtomicU32::new(PROPAGATION_PRIVATE),
            writers: AtomicU32::new(0),
        });
        self.next_id += 1;
        m
    }

    /// Whether `m` is in the table (attached), not detached.
    pub(crate) fn contains(&self, m: &Arc<Mount>) -> bool {
        self.mounts.iter().any(|x| Arc::ptr_eq(x, m))
    }

    /// The mounts below `id` (at any depth), not `id` itself, in table
    /// order.
    pub(crate) fn descendants(&self, id: MountId) -> Vec<Arc<Mount>> {
        let mut ids = alloc::vec![id];
        let mut i = 0;
        while i < ids.len() {
            let d = ids[i];
            ids.extend(self.mounts.iter().filter(|m| m.is_on(d, None)).map(|m| m.id));
            i += 1;
        }
        self.mounts.iter().filter(|m| m.id != id && ids.contains(&m.id)).cloned().collect()
    }

    /// Put the tree whose first mount is `base` back in an order where
    /// every mount comes after the mount it is on (what [`tree`](Self::tree)
    /// and namespace copies rely on) after mounts moved, keeping the
    /// relative order of mounts on the same mount point (the last one
    /// stays the visible one).
    pub(crate) fn reorder(&mut self, base: MountId) {
        let members = self.tree(base);
        let mut ordered: Vec<Arc<Mount>> = members.iter().filter(|m| m.id == base).cloned().collect();
        let mut i = 0;
        while i < ordered.len() {
            let parent = ordered[i].id;
            ordered.extend(members.iter().filter(|m| m.is_on(parent, None)).cloned());
            i += 1;
        }
        self.mounts.retain(|m| !members.iter().any(|x| Arc::ptr_eq(x, m)));
        self.mounts.extend(ordered);
    }

    /// A mount that is in no tree: the anonymous file system of file
    /// objects, which no path reaches and nothing can unmount.
    pub(crate) fn detached(&mut self, sb: Arc<Superblock>, root: Ino) -> Arc<Mount> {
        self.new_mount(sb, root, None, MountFlags::default())
    }

    /// The topmost mount on mount point `(mount, ino)`.
    pub(crate) fn covering(&self, mount: MountId, ino: Ino) -> Option<&Arc<Mount>> {
        self.mounts.iter().rev().find(|m| m.is_on(mount, Some(ino)))
    }

    /// The mounts of the tree whose first mount is `base`, in mount order
    /// (parents before their children).
    pub(crate) fn tree(&self, base: MountId) -> Vec<Arc<Mount>> {
        self.mounts.iter().filter(|m| base_of(m) == base).cloned().collect()
    }

    pub(crate) fn has_children(&self, mount: MountId) -> bool {
        self.mounts.iter().any(|m| m.is_on(mount, None))
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
            doomed.extend(self.mounts.iter().filter(|m| m.is_on(d, None)).map(|m| m.id));
            i += 1;
        }
        self.mounts.retain(|m| !doomed.contains(&m.id));
        Ok(())
    }
}
