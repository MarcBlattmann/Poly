//! Mounting and mount namespaces.

use super::*;

impl Vfs {
    /// `mount(2)`: puts `fs` on what `path` names (`CAP_SYS_ADMIN`, else
    /// `EPERM`): a directory for a file system whose root is one, and
    /// something else for a root that is not a directory (a FUSE file
    /// system can have one); a mismatch is `ENOTDIR`.
    pub fn mount(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        fs: Arc<dyn FileSystem>,
        flags: MountFlags,
    ) -> Result<MountId> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let target = self.resolve(ctx, cred, at, path, true)?;
        let root = fs.root();
        let onto_dir = target.attr()?.kind == FileType::Directory;
        if (fs.getattr(root)?.kind == FileType::Directory) != onto_dir {
            return Err(Errno::ENOTDIR);
        }
        let info = fs.info();
        let id = {
            let mut table = self.mounts.lock();
            let sb = table.superblock(fs, info);
            let m = table.add(sb, root, Some((target.mount.clone(), target.ino)), flags);
            self.propagate_mount(&mut table, &target.mount, target.ino, &m);
            m.id
        };
        self.settle_pins();
        Ok(id)
    }

    /// A bind mount (`mount --bind`): what `source` names becomes visible at
    /// `target` too. Both must be directories, or both not (`ENOTDIR`).
    /// The new mount has the per-mount flags of the mount it was made from
    /// (mount(2): "the bind mount has the same mount options as the
    /// underlying mount"; a remount changes them).
    ///
    /// A plain bind shows the source directory without the mounts below
    /// it. With `recursive` (`MS_REC`) every mount below the source, at
    /// any depth, is copied onto the same place below the new mount,
    /// except unbindable mounts and what is below those; every copy is a
    /// mount of its own with the flags of its original (ADR 0177).
    ///
    /// Locks ([`crate::mount::lock`]): the new mount keeps the flag locks
    /// of its source but is not locked in place (who made it may unmount
    /// it); the copies below it stay locked in place if their originals
    /// were. A plain bind of a directory with a mount locked in place
    /// below it is `EINVAL`: it would show what that mount covers.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn bind(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        source_at: At<'_>,
        source: &[u8],
        target_at: At<'_>,
        target: &[u8],
        recursive: bool,
    ) -> Result<MountId> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let src = self.resolve(ctx, cred, source_at, source, true)?;
        let dst = self.resolve(ctx, cred, target_at, target, true)?;
        let src_dir = src.attr()?.kind == FileType::Directory;
        let dst_dir = dst.attr()?.kind == FileType::Directory;
        if src_dir != dst_dir {
            return Err(Errno::ENOTDIR);
        }
        let sb = src.mount.sb.clone();
        let (peer, master) = self.bind_inherit(&src.mount)?;
        let below = self.mounts_below(&src);
        if !recursive && below.iter().any(|c| c.locked_in_place()) {
            return Err(Errno::EINVAL);
        }
        // What is shown is held while the mount lives (see `Mount::pins_root`).
        let pinned = src.ino != sb.fs.root() && sb.fs.open(src.ino).is_ok();
        let id = {
            let mut table = self.mounts.lock();
            let m = table.copy_of(&src.mount, src.ino);
            m.pins_root.store(pinned, core::sync::atomic::Ordering::Relaxed);
            m.set_groups(peer, master);
            m.set_parent(Some((dst.mount.clone(), dst.ino)));
            // The originals are listed before the new mount is in the
            // table: a bind into its own source is not copied again.
            let copies = if recursive { Self::copy_below(&mut table, &src.mount, below, &m, true) } else { Vec::new() };
            table.mounts.push(m.clone());
            table.mounts.extend(copies);
            self.propagate_mount(&mut table, &dst.mount, dst.ino, &m);
            m.id
        };
        self.settle_pins();
        Ok(id)
    }

    /// `umount2(2)`: `path` must be the root of a mount (`EINVAL`). Without
    /// `lazy`, the mount must have nothing mounted below it and nothing
    /// open, no working directory or root in it (`EBUSY`). With `lazy`
    /// (`MNT_DETACH`) it and everything below it are detached at once and
    /// live on for whoever still uses them. The first root cannot be
    /// unmounted (`EBUSY`), nor a mount locked in place (`EINVAL`, with
    /// or without `lazy`; [`crate::mount::lock::PLACE`]).
    pub fn umount(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], lazy: bool) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = match self.resolve(ctx, cred, at, path, true) {
            // A file system that cannot answer (a FUSE server that is gone)
            // can still be unmounted: found by its mount point instead.
            Err(Errno::ENOTCONN | Errno::ECONNREFUSED) => self.dead_mount_root(ctx, cred, at, path)?,
            r => r?,
        };
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let mount = self.unmount_target(&loc);
        drop(loc);
        if mount.parent().is_none() {
            return Err(Errno::EBUSY);
        }
        if mount.locked_in_place() {
            return Err(Errno::EINVAL);
        }
        // What leaves the table is dropped after its lock is released
        // (`MountTable::remove_tree`).
        let removed = {
            let mut table = self.mounts.lock();
            if !lazy {
                // References: the table's, ours, and one per child mount,
                // pinned node and walk in progress.
                if table.has_children(mount.id) || Arc::strong_count(&mount) > 2 {
                    return Err(Errno::EBUSY);
                }
            }
            let mut removed = Vec::new();
            if let Some((pm, ino)) = mount.parent() {
                self.propagate_umount(&mut table, &pm, ino, &mount, &mut removed);
            }
            removed.extend(table.remove_tree(mount.id)?);
            removed
        };
        drop(removed);
        Ok(())
    }

    /// The mount an unmount of `loc` (a mount's root) means: that mount,
    /// or for the first mount of a namespace, which cannot go, the mount
    /// stacked beneath its root if there is one: the old root after
    /// `pivot_root(".", ".")`, which `umount2(".", MNT_DETACH)` removes
    /// (pivot_root(2)).
    fn unmount_target(&self, loc: &Location) -> Arc<Mount> {
        if loc.mount.parent().is_none()
            && let Some(under) = self.mounts.lock().beneath(loc.mount.id, loc.ino)
        {
            return under.clone();
        }
        loc.mount.clone()
    }

    /// The mount `umount2(2)` of `path` would remove (`EINVAL` if `path`
    /// is not a mount's root): see [`umount`](Self::umount).
    pub fn unmount_id(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<MountId> {
        let loc = match self.resolve(ctx, cred, at, path, follow) {
            Err(Errno::ENOTCONN | Errno::ECONNREFUSED) => self.dead_mount_root(ctx, cred, at, path)?,
            r => r?,
        };
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        Ok(self.unmount_target(&loc).id)
    }

    /// The root of the mount on the directory `path` names, found without
    /// asking the mounted file system (which does not answer).
    fn dead_mount_root(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Location> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let crate::path::Last::Name(name) = &p.last else { return Err(Errno::ENOTCONN) };
        let ino = p.dir.mount.sb.fs.lookup(p.dir.ino, name)?;
        let table = self.mounts.lock();
        let m = table.covering(p.dir.mount.id, ino).cloned().ok_or(Errno::EINVAL)?;
        Ok(Location { ino: m.root, mount: m })
    }

    /// `umount2(2)` with `MNT_EXPIRE`: an idle mount is marked by the first
    /// call (`EAGAIN`) and unmounted by the second; anything that changed a
    /// name in between takes the mark away again.
    pub fn umount_expire(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = self.resolve(ctx, cred, at, path, true)?;
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let mount = self.unmount_target(&loc);
        drop(loc);
        if mount.parent().is_none() {
            return Err(Errno::EBUSY);
        }
        if mount.locked_in_place() {
            return Err(Errno::EINVAL);
        }
        let removed = {
            let mut table = self.mounts.lock();
            if table.has_children(mount.id) || Arc::strong_count(&mount) > 2 {
                return Err(Errno::EBUSY);
            }
            let now = self.namespace_generation() + 1;
            if mount.expire.swap(now, core::sync::atomic::Ordering::AcqRel) != now {
                return Err(Errno::EAGAIN);
            }
            let mut removed = Vec::new();
            if let Some((pm, ino)) = mount.parent() {
                self.propagate_umount(&mut table, &pm, ino, &mount, &mut removed);
            }
            removed.extend(table.remove_tree(mount.id)?);
            removed
        };
        drop(removed);
        Ok(())
    }

    /// `mount(2)` with `MS_REMOUNT`: the mount whose root `path` names gets
    /// new per-mount flags (`CAP_SYS_ADMIN`, else `EPERM`; not the root of
    /// a mount, `EINVAL`). A file system that is read-only itself stays
    /// so whatever the flags say.
    ///
    /// Going read-only is `EBUSY` while something writes (mount(2)'s
    /// spec sheet, `busy_writers`): with `bind` (`MS_BIND`, this mount
    /// only) a file open for writing through this mount
    /// ([`Mount::busy_for_read_only`]); without it (the whole file system)
    /// a file open for writing through any of its mounts, or an unlinked
    /// file still open ([`Mount::sb_busy_for_read_only`]). A refused
    /// remount changes nothing.
    ///
    /// `atime` is the access time mode the call names (`MOUNT_ATTR_*`
    /// bits: the mode in `0x70`, `NODIRATIME` `0x80`), `None` if it names
    /// none: the mount then keeps the mode it has.
    ///
    /// Locks ([`crate::mount::lock`]; before the writers): leaving out a
    /// flag that is locked on, or naming another access time mode than a
    /// mount with a locked mode has, is `EPERM`.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn remount(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        flags: MountFlags,
        atime: Option<u32>,
        bind: bool,
    ) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = self.resolve(ctx, cred, at, path, true)?;
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let mount = loc.mount.clone();
        drop(loc);
        if mount.flags_locked_against(flags) || atime.is_some_and(|a| mount.atime_locked_against(a)) {
            return Err(Errno::EPERM);
        }
        let busy = if bind { mount.busy_for_read_only() } else { mount.sb_busy_for_read_only() };
        if flags.read_only && busy {
            return Err(Errno::EBUSY);
        }
        mount.set_flags(flags);
        if let Some(atime) = atime {
            mount.set_attrs((mount.attrs() & !0xf0) | (atime & 0xf0));
        }
        Ok(())
    }

    /// The mount whose root `path` names (`EINVAL` if `path` is not a
    /// mount point), as a `Vfs::mounts` entry: what `umount` callers check
    /// before unmounting a mount the kernel tracks itself (a disk).
    pub fn mount_at(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<MountId> {
        let loc = match self.resolve(ctx, cred, at, path, follow) {
            Err(Errno::ENOTCONN | Errno::ECONNREFUSED) => self.dead_mount_root(ctx, cred, at, path)?,
            r => r?,
        };
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        Ok(loc.mount.id)
    }

    /// Where mount `id` is mounted, as an absolute path from `ctx`'s root
    /// (what `/proc/mounts` and `mount` show). `ENOENT` if there is no such
    /// mount or it cannot be reached from that root.
    pub fn mount_path(&self, ctx: &FsContext, id: MountId) -> Result<Vec<u8>> {
        let mount = self.mounts.lock().mounts.iter().find(|m| m.id == id).cloned().ok_or(Errno::ENOENT)?;
        let root = Location { ino: mount.root, mount }.pin()?;
        self.path_of(ctx, &root)
    }

    // ----- mount namespaces -----
    //
    // A mount namespace is a tree of mounts: the mounts below one first
    // mount (a mount without a mount point), which names it. The first
    // root's tree is the initial namespace. A copy duplicates every mount
    // of a tree, so mounting and unmounting in one copy does not show in
    // the others; the file systems (superblocks) are shared, so files are.

    /// The mount namespace `ctx`'s root is in.
    pub fn namespace_of(&self, ctx: &FsContext) -> MountId {
        crate::mount::base_of(&ctx.root.loc().mount)
    }

    /// Whether `ctx`'s root shows something else than its mount
    /// namespace's root (chroot(2) moved it). A root that is the
    /// namespace's first mount, or what is mounted over it, is not.
    pub fn is_chrooted(&self, ctx: &FsContext) -> bool {
        let mut first = ctx.root.loc().mount.clone();
        while let Some((p, _)) = first.parent() {
            first = p;
        }
        let top = self.follow_mounts(Location { ino: first.root, mount: first });
        !self.follow_mounts(ctx.root.loc().clone()).same(&top)
    }

    /// `unshare(CLONE_NEWNS)`: copies the mount namespace `ctx` is in and
    /// moves `ctx`'s root and working directory to the same places in the
    /// copy (a working directory in another tree stays). Returns the new
    /// namespace. The flags of each mount are copied.
    ///
    /// `to_slave`: the copy is for a user namespace other than the one
    /// that owns the original (mount_namespaces(7)). Shared mounts then
    /// become slaves of the original's peer groups, and every mount of the
    /// copy is locked ([`crate::mount::lock`], ADR 0177): onto its mount
    /// point (all but the first, which is on none), the flags it has, and
    /// its access time mode. Otherwise the copies keep their originals'
    /// locks.
    pub fn copy_namespace(&self, ctx: &mut FsContext, to_slave: bool) -> Result<MountId> {
        let base = self.namespace_of(ctx);
        let mut copies: BTreeMap<MountId, Arc<Mount>> = BTreeMap::new();
        let new_base = {
            let mut table = self.mounts.lock();
            let mut new_base = None;
            for m in table.tree(base) {
                let parent = match m.parent() {
                    None => None,
                    // A parent that left the table (a lazy unmount) takes
                    // its subtree with it.
                    Some((p, ino)) => match copies.get(&p.id) {
                        Some(c) => Some((c.clone(), ino)),
                        None => continue,
                    },
                };
                let is_base = parent.is_none();
                let copy = table.copy_of(&m, m.root);
                copy.set_parent(parent);
                copy.set_beneath(m.is_beneath());
                table.mounts.push(copy.clone());
                if to_slave {
                    let place = if is_base { 0 } else { crate::mount::lock::PLACE };
                    copy.set_locks(m.locks() | copy.locks_of_flags() | place);
                } else {
                    copy.set_locks(m.locks());
                }
                // A copy keeps the original's groups (its mounts are peers
                // of theirs); made for another user namespace, shared
                // mounts become slaves instead.
                if to_slave && m.peer_group() != 0 {
                    copy.set_groups(0, m.peer_group());
                } else {
                    copy.set_groups(m.peer_group(), m.master_group());
                }
                copy.set_unbindable(m.is_unbindable());
                if is_base {
                    new_base = Some(copy.id);
                }
                copies.insert(m.id, copy);
            }
            new_base.ok_or(Errno::EINVAL)?
        };
        self.settle_pins();
        let moved = |n: &NodeRef| -> Result<Option<NodeRef>> {
            match copies.get(&n.loc().mount.id) {
                Some(m) => Location { mount: m.clone(), ino: n.loc().ino }.pin().map(Some),
                None => Ok(None),
            }
        };
        let root = moved(&ctx.root)?;
        let cwd = moved(&ctx.cwd)?;
        if let Some(r) = root {
            ctx.root = r;
        }
        if let Some(c) = cwd {
            ctx.cwd = c;
        }
        Ok(new_base)
    }

    /// `setns(2)` into mount namespace `base`: `ctx`'s root and working
    /// directory become its root. `EINVAL` if there is no such namespace.
    pub fn enter_namespace(&self, ctx: &mut FsContext, base: MountId) -> Result<()> {
        let table = self.mounts.lock();
        let base = table.current_base(base);
        let first = table.mounts.iter().find(|m| m.id == base && m.parent().is_none()).cloned().ok_or(Errno::EINVAL)?;
        drop(table);
        let root = Location { ino: first.root, mount: first }.pin()?;
        ctx.cwd = root.clone();
        ctx.root = root;
        Ok(())
    }

    /// The last process left mount namespace `base`: its mounts go (files
    /// open on them keep them alive). The initial namespace stays
    /// (`EBUSY`).
    pub fn drop_namespace(&self, base: MountId) -> Result<()> {
        // The mounts are dropped after the table's lock is released: one
        // of them may pin a namespace file whose namespace's end comes
        // back here (`MountTable::remove_tree`).
        let doomed = {
            let mut table = self.mounts.lock();
            if table.mounts.first().is_some_and(|m| m.id == base) {
                return Err(Errno::EBUSY);
            }
            let name = base;
            let base = table.current_base(name);
            let doomed = table.tree(base);
            if doomed.is_empty() {
                return Err(Errno::EINVAL);
            }
            table.mounts.retain(|m| !doomed.iter().any(|d| Arc::ptr_eq(d, m)));
            table.forget_alias(name);
            doomed
        };
        drop(doomed);
        Ok(())
    }

    /// The mounts of namespace `base`, in mount order.
    pub fn namespace_mounts(&self, base: MountId) -> Vec<MountInfo> {
        let ids: Vec<MountId> = {
            let table = self.mounts.lock();
            table.tree(table.current_base(base)).iter().map(|m| m.id).collect()
        };

        self.mounts().into_iter().filter(|m| ids.contains(&m.id)).collect()
    }

    /// Every mount, in mount order.
    pub fn mounts(&self) -> Vec<MountInfo> {
        self.mounts
            .lock()
            .mounts
            .iter()
            .map(|m| MountInfo {
                id: m.id,
                parent: m.parent().map(|(p, ino)| (p.id, ino)),
                fs_name: m.sb.info.name,
                dev: m.sb.dev,
                root: m.root,
                flags: m.flags(),
            })
            .collect()
    }
}
