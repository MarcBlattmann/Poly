//! The operations of the new mount API (`open_tree(2)`, `move_mount(2)`,
//! `fsmount(2)`, `mount_setattr(2)`, `statmount(2)`, `listmount(2)`) and
//! `pivot_root(2)` on the mount table (ADR 0122).
//!
//! Detached mounts: a mount made by `fsmount` or cloned by
//! `open_tree(OPEN_TREE_CLONE)` is in no tree and not in the table; the
//! `O_PATH` files open on its root ([`Vfs::open_path`]) hold it, and it
//! goes (with the file system, if nothing else holds that) when the last
//! of them is closed. Attaching it ([`Vfs::move_mount`]) puts it in the
//! table, which then keeps it. A recursive clone's copies of submounts
//! are kept by the detached top ([`Mount::subtree`]), point back to their
//! parents weakly, and are found by path lookups through the top's list;
//! attaching the top moves them into the table.
//!
//! Moving: a mount's mount point can change ([`Mount::parent`]); the table
//! is then put back in an order where every mount follows the mount it is
//! on. `pivot_root` makes another mount the first of its namespace; the
//! namespace keeps the name (id) it was made with (`MountTable::rebase`).

use crate::cred::{Cred, cap};
use crate::file::{Backend, OpenFile, OpenFlags};
use crate::mount::{Mount, MountFlags, MountId, base_of};
use crate::node::{FsContext, Location, NodeRef};
use crate::{Errno, FileSystem, FileType, Result, Vfs};
use alloc::sync::Arc;
use alloc::vec::Vec;

impl Vfs {
    /// An `O_PATH` open file on `node` (what `open_tree(2)` and
    /// `fsmount(2)` return): `fcntl(F_GETFL)` shows `O_PATH` alone.
    pub fn open_path(&self, node: NodeRef) -> Arc<OpenFile> {
        let kind = node.kind().unwrap_or(FileType::Directory);
        Arc::new(OpenFile::new(node, kind, Backend::Path, OpenFlags::PATH))
    }

    /// The root of mount `m`, pinned.
    pub fn mount_root_node(&self, m: &Arc<Mount>) -> Result<NodeRef> {
        Location { mount: m.clone(), ino: m.root }.pin()
    }

    /// A new detached mount of `fs` with per-mount `flags` (`fsmount(2)`):
    /// in no tree until [`move_mount`](Self::move_mount) attaches it.
    pub fn new_detached(&self, fs: Arc<dyn FileSystem>, flags: MountFlags) -> Arc<Mount> {
        let m = self.detached_mount(fs);
        m.change_flags(flags);
        m
    }

    /// `open_tree(OPEN_TREE_CLONE)`: a detached copy of the mount `node`
    /// is on, showing the subtree from `node` (its root, or any directory
    /// below it, like a bind mount's source), with the same per-mount
    /// attributes. With `recursive` (`AT_RECURSIVE`) the mounts below
    /// `node` are copied too, at any depth, on the same mount points of
    /// the copies; the copy's top keeps them ([`Mount::subtree`]) until it
    /// is attached or its last descriptor is closed.
    pub fn clone_mount(&self, cred: &Cred, node: &NodeRef, recursive: bool) -> Result<Arc<Mount>> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = node.loc();
        let src = loc.mount.clone();
        if src.is_unbindable() {
            return Err(Errno::EINVAL);
        }
        // Only the mounts on mount points at or below `node`.
        let below = self.mounts_below(loc);
        // A copy without them would show what a mount locked in place
        // covers (ADR 0177).
        if !recursive && below.iter().any(|c| c.locked_in_place()) {
            return Err(Errno::EINVAL);
        }
        let m = {
            let mut table = self.mounts.lock();
            let m = table.copy_of(&src, loc.ino);
            if recursive {
                let kept = Self::copy_below(&mut table, &src, below, &m, false);
                m.set_subtree(kept);
            }
            m
        };
        self.settle_pins();
        Ok(m)
    }

    /// The mounts directly on `loc`'s mount whose mount points are `loc`
    /// or below it, in mount order: what a copy of the subtree from `loc`
    /// (a bind mount, a clone) would leave out or take along.
    pub(crate) fn mounts_below(&self, loc: &Location) -> Vec<Arc<Mount>> {
        let direct = {
            let table = self.mounts.lock();
            Self::children_of(&table, &loc.mount)
        };
        direct
            .into_iter()
            .filter(|c| c.parent().is_some_and(|(p, ino)| self.is_at_or_below(&Location { mount: p, ino }, loc)))
            .collect()
    }

    /// Copies of the trees of mounts `first` (mounts directly on `src`,
    /// which `top` is the copy of) for `top`, at any depth, parents before
    /// their children, each on the mount point of its original in the copy
    /// of its original's parent. A copy keeps its original's flags and
    /// locks, the lock in place too: it stays where it is in the copy.
    ///
    /// For a bind mount (`bind`) unbindable mounts and what is below them
    /// are left out, the copies inherit the propagation of their originals
    /// like any bind and hold their parents; the caller puts them in the
    /// table. For a detached tree they are private and point back weakly;
    /// its top keeps them ([`Mount::subtree`]).
    ///
    /// The originals are listed before any copy is made, so a copy that
    /// lands inside the tree it copies is not copied again.
    pub(crate) fn copy_below(
        table: &mut crate::mount::MountTable,
        src: &Arc<Mount>,
        first: Vec<Arc<Mount>>,
        top: &Arc<Mount>,
        bind: bool,
    ) -> Vec<Arc<Mount>> {
        /// The most mounts one copy takes along.
        const MOST: usize = 4096;
        let mut originals: Vec<Arc<Mount>> = first.into_iter().filter(|m| !(bind && m.is_unbindable())).collect();
        let mut i = 0;
        while i < originals.len() && originals.len() < MOST {
            let children = Self::children_of(table, &originals[i]);
            originals.extend(children.into_iter().filter(|m| !(bind && m.is_unbindable())));
            i += 1;
        }
        originals.truncate(MOST);
        let mut copies: alloc::collections::BTreeMap<MountId, Arc<Mount>> = alloc::collections::BTreeMap::new();
        copies.insert(src.id(), top.clone());
        let mut made = Vec::new();
        for old in originals {
            let Some((p, ino)) = old.parent() else { continue };
            let Some(parent) = copies.get(&p.id()).cloned() else { continue };
            let copy = table.copy_of(&old, old.root_ino());
            copy.set_locks(old.locks());
            if bind {
                copy.set_parent(Some((parent, ino)));
                let peer = old.peer_group();
                copy.set_groups(peer, if peer == 0 { old.master_group() } else { 0 });
            } else {
                copy.set_weak_parent(&parent, ino);
            }
            copies.insert(old.id(), copy.clone());
            made.push(copy);
        }
        made
    }

    /// Pin the copies made of mounts that pin their root
    /// ([`Mount::pin_like`]): after the table's lock is released, because
    /// the file system is asked.
    pub(crate) fn settle_pins(&self) {
        let pending = self.mounts.lock().take_pending_pins();
        for (original, copy) in pending {
            copy.pin_like(&original);
        }
    }

    /// The mounts directly on `m` (in the table, or kept by the detached
    /// tree `m` is in), in mount order.
    fn children_of(table: &crate::mount::MountTable, m: &Arc<Mount>) -> Vec<Arc<Mount>> {
        let mut out: Vec<Arc<Mount>> = table.mounts.iter().filter(|c| c.is_on(m.id, None)).cloned().collect();
        out.extend(m.top().subtree().into_iter().filter(|c| c.is_on(m.id, None)));
        out
    }

    /// Whether `m` is below `top` (at any depth), not `top` itself.
    fn is_below(m: &Arc<Mount>, top: &Arc<Mount>) -> bool {
        let mut cur = m.parent().map(|(p, _)| p);
        while let Some(c) = cur {
            if Arc::ptr_eq(&c, top) {
                return true;
            }
            cur = c.parent().map(|(p, _)| p);
        }
        false
    }

    /// Whether `m` is attached (in a tree of the table), not detached.
    pub fn is_attached(&self, m: &Arc<Mount>) -> bool {
        self.mounts.lock().contains(m)
    }

    /// `move_mount(2)`: `src` must be the root of a mount (`EINVAL`). A
    /// detached mount is attached on what `dst` shows; an attached one is
    /// moved there with everything mounted below it.
    ///
    /// Errors (the spec sheet of mntapi_move_mount): a directory mount
    /// onto something that is no directory, or the mount of a file onto a
    /// directory, `EINVAL`; a mount that is attached in another namespace
    /// than the target is in, and a mount that was attached before and is
    /// in no namespace now (a detached mount is attached once), `EINVAL`;
    /// a mount onto itself or below itself, also a namespace's first
    /// mount, which everything is below, `ELOOP`; a mount locked in place,
    /// one whose parent is shared, and an unbindable one below a shared
    /// mount, `EINVAL`.
    pub fn move_mount(&self, cred: &Cred, src: &NodeRef, dst: &NodeRef) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let src = src.loc();
        if !src.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let dst = self.follow_mounts(dst.loc().clone());
        if (dst.attr()?.kind == FileType::Directory) != (src.attr()?.kind == FileType::Directory) {
            return Err(Errno::EINVAL);
        }
        let m = src.mount.clone();
        let mut table = self.mounts.lock();
        if !table.contains(&m) {
            // A detached mount: attach it, with what its tree keeps below.
            if m.parent().is_some() {
                // It was attached and left its namespace (a lazy unmount,
                // a namespace that ended), or it is below the top of a
                // detached tree.
                return Err(Errno::EINVAL);
            }
            m.set_parent(Some((dst.mount.clone(), dst.ino)));
            table.mounts.push(m.clone());
            let below = m.take_subtree();
            table.mounts.extend(below);
            return Ok(());
        }
        // Attached: only within its own namespace.
        if base_of(&m) != base_of(&dst.mount) {
            return Err(Errno::EINVAL);
        }
        let Some((old_parent, _)) = m.parent() else {
            // The first mount of the namespace: the target is below it.
            return Err(Errno::ELOOP);
        };
        // A mount locked in place stays on its mount point (ADR 0177).
        if m.locked_in_place() {
            return Err(Errno::EINVAL);
        }
        // mount(2) MS_MOVE: a mount whose parent is shared cannot be moved
        // (it would have to be moved in every peer), nor can an unbindable
        // one go below a shared mount.
        if old_parent.peer_group() != 0 || (dst.mount.peer_group() != 0 && m.is_unbindable()) {
            return Err(Errno::EINVAL);
        }
        // Not below itself: `dst`'s mount and the mounts it is on.
        let mut cur = Some(dst.mount.clone());
        while let Some(c) = cur {
            if Arc::ptr_eq(&c, &m) {
                return Err(Errno::ELOOP);
            }
            cur = c.parent().map(|(p, _)| p);
        }
        m.set_parent(Some((dst.mount.clone(), dst.ino)));
        let base = base_of(&m);
        table.reorder(base);
        Ok(())
    }

    /// `move_mount(2)` with `MOVE_MOUNT_SET_GROUP`: the mount `dst` is the
    /// root of joins the propagation group of the mount `src` is the root
    /// of (its peers, or the group it is a slave of; a private source
    /// makes it private). Both must be roots of mounts of the same file
    /// system, as two bind mounts are (`EINVAL`); nothing moves.
    pub fn share_group(&self, cred: &Cred, src: &NodeRef, dst: &NodeRef) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let (src, dst) = (src.loc(), dst.loc());
        if !src.is_mount_root() || !dst.is_mount_root() || !src.mount.same_sb(&dst.mount) {
            return Err(Errno::EINVAL);
        }
        let _table = self.mounts.lock();
        dst.mount.set_groups(src.mount.peer_group(), src.mount.master_group());
        Ok(())
    }

    /// `pivot_root(2)`: the mount `new_root` names becomes the first mount
    /// of the caller's namespace and the old first mount is mounted on
    /// `put_old`; `ctx`'s root and working directory, where they were the
    /// old root, move to the new one. Returns the old and the new root,
    /// for the caller to move other processes' directories the same way.
    ///
    /// Errors: not directories `ENOTDIR`; `new_root` or `put_old` on the
    /// root's mount (also the root itself) `EBUSY`; `new_root` not a
    /// mount's root, `put_old` not at or below `new_root`, a shared mount
    /// involved, or a root that is not the namespace's first mount
    /// (a chroot) `EINVAL`.
    ///
    /// `put_old` may be `new_root` itself (`pivot_root(".", ".")`): the
    /// old root is then stacked beneath the new root's top directory
    /// ([`Mount::is_beneath`]), hidden by the new tree, which is `/` at
    /// once; an unmount of that directory removes the old root
    /// ([`Vfs::umount`]). A second such call is `EBUSY`: `new_root` is the
    /// root's mount already.
    pub fn pivot_root(
        &self,
        ctx: &mut FsContext,
        cred: &Cred,
        new_root: &NodeRef,
        put_old: &NodeRef,
    ) -> Result<(NodeRef, NodeRef)> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let new = self.follow_mounts(new_root.loc().clone());
        let old_put = self.follow_mounts(put_old.loc().clone());
        if new.attr()?.kind != FileType::Directory || old_put.attr()?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let root = self.follow_mounts(ctx.root.loc().clone());
        if Arc::ptr_eq(&new.mount, &root.mount) || Arc::ptr_eq(&old_put.mount, &root.mount) {
            return Err(Errno::EBUSY);
        }
        if !root.is_mount_root() || root.mount.parent().is_some() || !new.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        if !self.is_at_or_below(&old_put, &new) {
            return Err(Errno::EINVAL);
        }
        let new_parent = new.mount.parent().map(|(p, _)| p);
        let shared = |m: &Arc<Mount>| m.peer_group() != 0;
        if shared(&new.mount)
            || shared(&root.mount)
            || shared(&old_put.mount)
            || new_parent.as_ref().is_some_and(shared)
        {
            return Err(Errno::EINVAL);
        }
        let old_mount = root.mount.clone();
        let new_mount = new.mount.clone();
        let old_root = Location { mount: old_mount.clone(), ino: old_mount.root };
        let new_top = Location { mount: new_mount.clone(), ino: new_mount.root };
        let root_was_old = self.follow_mounts(ctx.root.loc().clone()).same(&root);
        let cwd_was_old = self.follow_mounts(ctx.cwd.loc().clone()).same(&root);
        {
            let mut table = self.mounts.lock();
            if !table.contains(&new_mount) || base_of(&new_mount) != old_mount.id {
                return Err(Errno::EINVAL);
            }
            // What was stacked beneath the old root's top (an earlier
            // root) is on a mount point like any other from now on.
            for m in table.mounts.iter().filter(|m| m.is_on(old_mount.id, Some(old_mount.root))) {
                m.set_beneath(false);
            }
            old_mount.set_parent(Some((old_put.mount.clone(), old_put.ino)));
            old_mount.set_beneath(old_put.same(&new_top));
            new_mount.set_parent(None);
            table.reorder(new_mount.id);
            table.rebase(old_mount.id, new_mount.id);
        }
        let new_pin = new_top.pin()?;
        if root_was_old {
            ctx.root = new_pin.clone();
        }
        if cwd_was_old {
            ctx.cwd = new_pin.clone();
        }
        Ok((old_root.pin()?, new_pin))
    }

    /// After [`pivot_root`](Self::pivot_root) returned `(old, new)`:
    /// another process's context whose root or working directory was the
    /// old root gets the new one (pivot_root(2): "changes the root
    /// directory and the current working directory of each process or
    /// thread in the same mount namespace to new_root if they point to
    /// the old root directory").
    pub fn repoint_context(ctx: &mut FsContext, old: &NodeRef, new: &NodeRef) {
        if ctx.root.same(old) {
            ctx.root = new.clone();
        }
        if ctx.cwd.same(old) {
            ctx.cwd = new.clone();
        }
    }

    /// Whether `loc` is `top` or below it, climbing `..` and out of mounts.
    pub(crate) fn is_at_or_below(&self, loc: &Location, top: &Location) -> bool {
        let mut cur = loc.clone();
        for _ in 0..4096 {
            if cur.same(top) {
                return true;
            }
            if cur.is_mount_root() {
                match cur.mount.parent() {
                    Some((m, ino)) => cur = Location { mount: m, ino },
                    None => return false,
                }
                continue;
            }
            match cur.mount.sb.fs.lookup(cur.ino, b"..") {
                Ok(p) if p != cur.ino => cur = Location { mount: cur.mount.clone(), ino: p },
                _ => return false,
            }
        }
        false
    }

    /// The mounts below `m` (at any depth) in table order, `m` first;
    /// only `m` unless `recursive`.
    pub fn mount_subtree(&self, m: &Arc<Mount>, recursive: bool) -> Vec<Arc<Mount>> {
        let mut out = alloc::vec![m.clone()];
        if recursive {
            out.extend(self.mounts.lock().descendants(m.id));
            out.extend(m.top().subtree().into_iter().filter(|c| Self::is_below(c, m)));
        }
        out
    }

    /// The mounts of the namespace `ctx`'s root is in, in table order (its
    /// first mount first).
    pub fn namespace_tree(&self, ctx: &FsContext) -> Vec<Arc<Mount>> {
        let base = self.namespace_of(ctx);
        self.mounts.lock().tree(base)
    }

    /// The mount with id `id` in the namespace `ctx`'s root is in.
    pub fn namespace_mount(&self, ctx: &FsContext, id: MountId) -> Option<Arc<Mount>> {
        self.namespace_tree(ctx).into_iter().find(|m| m.id == id)
    }

    /// The path of `m`'s root inside its own file system (`/` for a
    /// mount of the whole file system, `/sub` for a bind mount of `sub`):
    /// `statmount(2)`'s `mnt_root`.
    pub fn mount_root_path(&self, m: &Mount) -> Result<Vec<u8>> {
        let fs = &m.sb.fs;
        let top = fs.root();
        let mut cur = m.root;
        let mut names: Vec<Vec<u8>> = Vec::new();
        while cur != top && names.len() < 4096 {
            let parent = fs.lookup(cur, b"..")?;
            if parent == cur {
                break;
            }
            names.push(Self::name_of(m, parent, cur)?);
            cur = parent;
        }
        let mut path = Vec::new();
        for n in names.iter().rev() {
            path.push(b'/');
            path.extend_from_slice(n);
        }
        if path.is_empty() {
            path.push(b'/');
        }
        Ok(path)
    }

    fn name_of(m: &Mount, dir: crate::Ino, child: crate::Ino) -> Result<Vec<u8>> {
        let mut found = None;
        m.sb.fs.readdir(dir, 0, &mut |e| {
            if e.ino == child && e.name != b"." && e.name != b".." {
                found = Some(e.name.to_vec());
                return false;
            }
            true
        })?;
        found.ok_or(Errno::ENOENT)
    }

    /// The mount `id`, wherever it is (attached in any namespace).
    pub fn mount_by_id(&self, id: MountId) -> Option<Arc<Mount>> {
        self.mounts.lock().mounts.iter().find(|m| m.id == id).cloned()
    }
}
