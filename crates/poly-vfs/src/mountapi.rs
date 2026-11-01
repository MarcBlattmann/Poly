//! The operations of the new mount API (`open_tree(2)`, `move_mount(2)`,
//! `fsmount(2)`, `mount_setattr(2)`, `statmount(2)`, `listmount(2)`) and
//! `pivot_root(2)` on the mount table (ADR 0122).
//!
//! Detached mounts: a mount made by `fsmount` or cloned by
//! `open_tree(OPEN_TREE_CLONE)` is in no tree and not in the table; the
//! `O_PATH` files open on its root ([`Vfs::open_path`]) hold it, and it
//! goes (with the file system, if nothing else holds that) when the last
//! of them is closed. Attaching it ([`Vfs::move_mount`]) puts it in the
//! table, which then keeps it.
//!
//! Moving: a mount's mount point can change ([`Mount::parent`]); the table
//! is then put back in an order where every mount follows the mount it is
//! on. `pivot_root` makes another mount the first of its namespace; the
//! namespace keeps the name (id) it was made with (`MountTable::rebase`).

use crate::cred::{Cred, cap};
use crate::file::{Backend, OpenFile, OpenFlags};
use crate::mount::{Mount, MountFlags, MountId, PROPAGATION_SHARED, base_of};
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
    /// attributes. Mounts below are not copied (`AT_RECURSIVE` is
    /// accepted and copies only the top mount).
    pub fn clone_mount(&self, cred: &Cred, node: &NodeRef) -> Result<Arc<Mount>> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = node.loc();
        let src = &loc.mount;
        let m = self.mounts.lock().new_mount(src.sb.clone(), loc.ino, None, src.flags());
        m.set_attrs(src.attrs());
        Ok(m)
    }

    /// Whether `m` is attached (in a tree of the table), not detached.
    pub fn is_attached(&self, m: &Arc<Mount>) -> bool {
        self.mounts.lock().contains(m)
    }

    /// `move_mount(2)`: `src` must be the root of a mount (`EINVAL`). A
    /// detached mount is attached on what `dst` shows; an attached one is
    /// moved there with everything mounted below it. A namespace's first
    /// mount cannot move (`EINVAL`), nor can a mount move below itself
    /// (`ELOOP`).
    pub fn move_mount(&self, cred: &Cred, src: &NodeRef, dst: &NodeRef) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let src = src.loc();
        if !src.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let dst = self.follow_mounts(dst.loc().clone());
        if dst.attr()?.kind != FileType::Directory && src.attr()?.kind == FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let m = src.mount.clone();
        let mut table = self.mounts.lock();
        if !table.contains(&m) {
            // A detached mount: attach it.
            m.set_parent(Some((dst.mount.clone(), dst.ino)));
            table.mounts.push(m);
            return Ok(());
        }
        if m.parent().is_none() {
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
        let shared = |m: &Arc<Mount>| m.propagation() == PROPAGATION_SHARED;
        if shared(&new.mount) || shared(&root.mount) || new_parent.as_ref().is_some_and(shared) {
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
            old_mount.set_parent(Some((old_put.mount.clone(), old_put.ino)));
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
    fn is_at_or_below(&self, loc: &Location, top: &Location) -> bool {
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
