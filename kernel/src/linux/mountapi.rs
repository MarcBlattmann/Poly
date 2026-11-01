//! The kernel's side of the new mount API and pivot_root(2) (ADR 0122):
//! the [`LinuxAdmin`](poly_personality_linux::admin::LinuxAdmin) methods
//! open_tree, move_mount, mount_setattr, fsopen, fspick, fsconfig,
//! fsmount, statmount's and listmount's views of the mount table, and
//! pivot_root, on the VFS (`poly_vfs::mountapi`).
//!
//! A file system context is a file object ([`ContextFile`]) on an
//! anonymous inode (`anon_inode:[fscontext]`): it holds the personality's
//! [`FsContext`] (parameters, state, message log, read with read(2)) and
//! what its commands made, the new file system (`FSCONFIG_CMD_CREATE`)
//! or the picked mount (fspick).

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{self, mounts};
use crate::sync::Mutex;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::admin;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::mountapi::{
    AttrChange, Command, CreateRequest, FsContext, MountDesc, MountLink, PathAt, SuperblockOps, join_attrs,
    options_string, split_attrs, uapi, unique_id,
};
use poly_vfs::{FileObject, FileSystem, Mount, MountFlags, NodeRef, OpenFile, OpenFlags, PollEvents, WaitQueue};

/// The VFS error for a Linux errno (both use Linux's numbers).
fn errno_vfs(e: Errno) -> poly_vfs::Errno {
    poly_vfs::Errno::from_code(i32::from(e.get())).unwrap_or(poly_vfs::Errno::EIO)
}

fn vfs_flags(f: admin::MountFlags) -> MountFlags {
    MountFlags { read_only: f.read_only, no_exec: f.no_exec, no_suid: f.no_suid, no_dev: f.no_dev }
}

fn admin_flags(f: MountFlags) -> admin::MountFlags {
    admin::MountFlags { read_only: f.read_only, no_suid: f.no_suid, no_dev: f.no_dev, no_exec: f.no_exec }
}

/// What a context's commands made.
#[derive(Default)]
struct Made {
    /// `FSCONFIG_CMD_CREATE`'s file system, its source and options, and
    /// whether `ro` was given.
    fs: Option<Arc<dyn FileSystem>>,
    source: Vec<u8>,
    options: Vec<u8>,
    read_only: bool,
    /// fspick's mount.
    picked: Option<Arc<Mount>>,
}

/// The file behind an fsopen(2) / fspick(2) descriptor.
pub struct ContextFile {
    state: Mutex<FsContext>,
    made: Mutex<Made>,
}

impl FileObject for ContextFile {
    /// One logged message per read (`ENODATA` with none, `EMSGSIZE` if it
    /// does not fit).
    fn read(&self, buf: &mut [u8]) -> poly_vfs::Result<usize> {
        let msg = self.state.lock().read_log(buf.len()).map_err(errno_vfs)?;
        buf[..msg.len()].copy_from_slice(&msg);
        Ok(msg.len())
    }

    fn write(&self, _buf: &[u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.state.lock().has_messages() { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        None
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[fscontext]")
    }
}

/// [`SuperblockOps`] on a context's [`Made`].
struct Ops<'a> {
    made: &'a mut Made,
}

impl SuperblockOps for Ops<'_> {
    fn create(&mut self, req: &CreateRequest) -> Result<(), Errno> {
        let fs = mounts::new_virtual(req.fstype, req.root).map_err(vfs_errno)?;
        self.made.fs = Some(fs);
        self.made.source = req.source.clone();
        self.made.options = options_string(req.fstype, &req.root);
        self.made.read_only = req.read_only;
        Ok(())
    }

    fn reconfigure(&mut self, read_only: Option<bool>) -> Result<(), Errno> {
        let m = self.made.picked.as_ref().ok_or(Errno::EINVAL)?;
        if let Some(ro) = read_only {
            if ro && !m.sb_read_only() && m.sb_writers() > 0 {
                return Err(Errno::EBUSY);
            }
            m.set_sb_read_only(ro);
        }
        Ok(())
    }
}

impl Thread<'_> {
    /// The node `at` names (an empty path: its descriptor's).
    fn resolve_at(&self, at: &PathAt) -> Result<NodeRef, Errno> {
        let files = &self.process().files;
        let node =
            if at.path.is_empty() { files.node_at(at.dirfd) } else { files.lookupat(at.dirfd, &at.path, at.follow) };
        node.map_err(vfs_errno)
    }

    fn install_file(&self, file: Arc<OpenFile>, cloexec: bool) -> Result<i32, Errno> {
        self.process().files.install(file, cloexec).map_err(vfs_errno)
    }

    /// The context file open on `fd`: `EBADF` (also for `O_PATH`), or
    /// `EINVAL` if it is something else.
    fn context_file(&self, fd: i32) -> Result<Arc<ContextFile>, Errno> {
        let file = self.process().files.file(fd).map_err(vfs_errno)?;
        if file.is_path_only() {
            return Err(Errno::EBADF);
        }
        file.object_as::<ContextFile>().ok_or(Errno::EINVAL)
    }

    fn install_context(&self, ctx: FsContext, made: Made, cloexec: bool) -> Result<i32, Errno> {
        let files = &self.process().files;
        let node = fs::vfs().anon_inode(0o600, &files.cred()).map_err(vfs_errno)?;
        let obj: Arc<dyn FileObject> = Arc::new(ContextFile { state: Mutex::new(ctx), made: Mutex::new(made) });
        self.install_file(Arc::new(OpenFile::on_object(node, obj, OpenFlags::RDWR)), cloexec)
    }

    pub(super) fn mnt_open_tree(&mut self, at: &PathAt, clone: Option<bool>, cloexec: bool) -> Result<i32, Errno> {
        let vfs = fs::vfs();
        let mut node = self.resolve_at(at)?;
        if let Some(recursive) = clone {
            let m = vfs.clone_mount(&self.process().files.cred(), &node, recursive).map_err(vfs_errno)?;
            node = vfs.mount_root_node(&m).map_err(vfs_errno)?;
        }
        self.install_file(vfs.open_path(node), cloexec)
    }

    pub(super) fn mnt_move_mount(&mut self, from: &PathAt, to: &PathAt) -> Result<(), Errno> {
        let src = self.resolve_at(from)?;
        let dst = self.resolve_at(to)?;
        fs::vfs().move_mount(&self.process().files.cred(), &src, &dst).map_err(vfs_errno)
    }

    pub(super) fn mnt_setattr(&mut self, at: &PathAt, recursive: bool, change: &AttrChange) -> Result<(), Errno> {
        let node = self.resolve_at(at)?;
        if change.is_empty() {
            return Ok(());
        }
        if node.ino() != node.mount().root_ino() {
            return Err(Errno::EINVAL);
        }
        let mounts = fs::vfs().mount_subtree(node.mount(), recursive);
        let attrs_of = |m: &Mount| join_attrs(admin_flags(m.flags()), m.attrs());
        // All or nothing: no mount made read-only while it has writers.
        for m in &mounts {
            let (old, new) = (attrs_of(m), change.apply(attrs_of(m)));
            if new & uapi::MOUNT_ATTR_RDONLY != 0 && old & uapi::MOUNT_ATTR_RDONLY == 0 && m.writers() > 0 {
                return Err(Errno::EBUSY);
            }
        }
        for m in &mounts {
            let (flags, extra) = split_attrs(change.apply(attrs_of(m)));
            m.change_flags(vfs_flags(flags));
            m.set_attrs(extra);
            m.set_propagation(change.propagation_of(u64::from(m.propagation())) as u32);
        }
        Ok(())
    }

    pub(super) fn mnt_fsopen(&mut self, ctx: FsContext, cloexec: bool) -> Result<i32, Errno> {
        self.install_context(ctx, Made::default(), cloexec)
    }

    pub(super) fn mnt_fspick(&mut self, at: &PathAt, cloexec: bool) -> Result<i32, Errno> {
        let node = self.resolve_at(at)?;
        if node.ino() != node.mount().root_ino() {
            return Err(Errno::EINVAL);
        }
        let m = node.mount().clone();
        let ctx = FsContext::pick(m.fs_name().as_bytes());
        self.install_context(ctx, Made { picked: Some(m), ..Made::default() }, cloexec)
    }

    pub(super) fn mnt_fsconfig(&mut self, fd: i32, cmd: &Command<'_>) -> Result<(), Errno> {
        let obj = self.context_file(fd)?;
        let mut state = obj.state.lock();
        let mut made = obj.made.lock();
        state.config(cmd, &mut Ops { made: &mut made })
    }

    pub(super) fn mnt_fsmount(&mut self, fd: i32, attrs: u64, cloexec: bool) -> Result<i32, Errno> {
        let vfs = fs::vfs();
        let obj = self.context_file(fd)?;
        obj.state.lock().begin_mount()?;
        let made = obj.made.lock();
        let fs = made.fs.clone().ok_or(Errno::EINVAL)?;
        let (flags, extra) = split_attrs(attrs);
        let m = vfs.new_detached(fs, vfs_flags(flags));
        m.set_attrs(extra);
        m.set_sb_info(&made.source, &made.options);
        if made.read_only {
            m.set_sb_read_only(true);
        }
        drop(made);
        let node = vfs.mount_root_node(&m).map_err(vfs_errno)?;
        self.install_file(vfs.open_path(node), cloexec)
    }

    pub(super) fn mnt_links(&mut self) -> Result<Vec<MountLink>, Errno> {
        let ctx = self.process().files.context();
        let mut links: Vec<MountLink> = fs::vfs()
            .namespace_tree(&ctx)
            .iter()
            .map(|m| MountLink { id: u64::from(m.id()), parent: m.parent().map(|(p, _)| u64::from(p.id())) })
            .collect();
        // The namespace's root first.
        links.sort_by_key(|l| l.parent.is_some());
        Ok(links)
    }

    pub(super) fn mnt_describe(&mut self, id: u64) -> Result<MountDesc, Errno> {
        let vfs = fs::vfs();
        let ctx = self.process().files.context();
        let id = u32::try_from(id).map_err(|_| Errno::ENOENT)?;
        let m = vfs.namespace_mount(&ctx, id).ok_or(Errno::ENOENT)?;
        let parent = m.parent().map_or(m.id(), |(p, _)| p.id());
        let magic = m.fs().statfs().map(|s| s.magic).ok().filter(|&x| x != 0);
        let mut source = m.sb_source();
        if source.is_empty() {
            source = mounts::source(m.id()).unwrap_or_else(|| b"none".to_vec());
        }
        let propagation = u64::from(m.propagation());
        Ok(MountDesc {
            sb_dev_major: m.dev().major,
            sb_dev_minor: m.dev().minor,
            sb_magic: magic.unwrap_or_else(|| poly_vfs::magic_for(m.fs_name())),
            sb_read_only: m.sb_read_only(),
            fs_type: m.fs_name().as_bytes().to_vec(),
            mnt_id: u64::from(m.id()),
            parent_id: u64::from(parent),
            flags: admin_flags(m.flags()),
            extra_attrs: m.attrs(),
            propagation,
            peer_group: if propagation == uapi::MS_SHARED { u64::from(m.id()) } else { 0 },
            mnt_root: vfs.mount_root_path(&m).unwrap_or_else(|_| b"/".to_vec()),
            mnt_point: vfs.mount_path(&ctx, m.id()).unwrap_or_default(),
            ns_id: unique_id(u64::from(vfs.namespace_of(&ctx))),
            source,
            options: m.sb_options(),
        })
    }

    pub(super) fn mnt_pivot_root(&mut self, new_root: &[u8], put_old: &[u8]) -> Result<(), Errno> {
        let files = &self.process().files;
        let new = files.lookupat(fs::AT_FDCWD, new_root, true).map_err(vfs_errno)?;
        let old = files.lookupat(fs::AT_FDCWD, put_old, true).map_err(vfs_errno)?;
        files.pivot_root(&new, &old).map_err(vfs_errno)
    }
}
