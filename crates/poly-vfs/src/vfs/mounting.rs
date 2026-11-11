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
        let mut table = self.mounts.lock();
        let sb = table.superblock(fs, info);
        let m = table.add(sb, root, Some((target.mount.clone(), target.ino)), flags);
        self.propagate_mount(&mut table, &target.mount, target.ino, &m);
        Ok(m.id)
    }

    /// A bind mount (`mount --bind`): what `source` names becomes visible at
    /// `target` too. Both must be directories, or both not (`ENOTDIR`).
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn bind(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        source_at: At<'_>,
        source: &[u8],
        target_at: At<'_>,
        target: &[u8],
        flags: MountFlags,
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
        // What is shown is held while the mount lives (see `Mount::pins_root`).
        let pinned = src.ino != sb.fs.root() && sb.fs.open(src.ino).is_ok();
        let mut table = self.mounts.lock();
        let m = table.add(sb, src.ino, Some((dst.mount.clone(), dst.ino)), flags);
        m.pins_root.store(pinned, core::sync::atomic::Ordering::Relaxed);
        m.set_attrs(src.mount.attrs());
        m.set_groups(peer, master);
        self.propagate_mount(&mut table, &dst.mount, dst.ino, &m);
        Ok(m.id)
    }

    /// `umount2(2)`: `path` must be the root of a mount (`EINVAL`). Without
    /// `lazy`, the mount must have nothing mounted below it and nothing
    /// open, no working directory or root in it (`EBUSY`). With `lazy`
    /// (`MNT_DETACH`) it and everything below it are detached at once and
    /// live on for whoever still uses them. The first root cannot be
    /// unmounted (`EBUSY`).
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
        let mount = loc.mount.clone();
        drop(loc);
        if mount.parent().is_none() {
            return Err(Errno::EBUSY);
        }
        let mut table = self.mounts.lock();
        if !lazy {
            // References: the table's, ours, and one per child mount,
            // pinned node and walk in progress.
            if table.has_children(mount.id) || Arc::strong_count(&mount) > 2 {
                return Err(Errno::EBUSY);
            }
        }
        if let Some((pm, ino)) = mount.parent() {
            self.propagate_umount(&mut table, &pm, ino, &mount);
        }
        table.remove_tree(mount.id)
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
        let mount = loc.mount.clone();
        drop(loc);
        if mount.parent().is_none() {
            return Err(Errno::EBUSY);
        }
        let mut table = self.mounts.lock();
        if table.has_children(mount.id) || Arc::strong_count(&mount) > 2 {
            return Err(Errno::EBUSY);
        }
        let now = self.namespace_generation() + 1;
        if mount.expire.swap(now, core::sync::atomic::Ordering::AcqRel) == now {
            if let Some((pm, ino)) = mount.parent() {
                self.propagate_umount(&mut table, &pm, ino, &mount);
            }
            return table.remove_tree(mount.id);
        }
        Err(Errno::EAGAIN)
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
    pub fn remount(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        flags: MountFlags,
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
        let busy = if bind { mount.busy_for_read_only() } else { mount.sb_busy_for_read_only() };
        if flags.read_only && busy {
            return Err(Errno::EBUSY);
        }
        mount.set_flags(flags);
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
                let copy = table.add(m.sb.clone(), m.root, parent, m.flags());
                copy.set_attrs(m.attrs());
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
        let mut table = self.mounts.lock();
        if table.mounts.first().is_some_and(|m| m.id == base) {
            return Err(Errno::EBUSY);
        }
        let name = base;
        let base = table.current_base(name);
        let doomed: Vec<MountId> = table.tree(base).iter().map(|m| m.id).collect();
        if doomed.is_empty() {
            return Err(Errno::EINVAL);
        }
        table.mounts.retain(|m| !doomed.contains(&m.id));
        table.forget_alias(name);
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
