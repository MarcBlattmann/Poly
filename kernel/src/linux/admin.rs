//! The kernel's system administration services for the Linux personality
//! ([`LinuxAdmin`], ADR 0051): mount(2) and umount2(2) on the VFS and the
//! disks, mknod(2), chroot(2), the host and domain names, and reboot(2)
//! through `power`.
//!
//! The personality has checked the flags and the caller's privileges and
//! copied the arguments; the VFS checks the paths as the calling thread.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{disk, mounts};
use crate::sync::SpinLock;
use crate::{kprintln, power};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_personality_linux::admin::{self, FsType, LinuxAdmin, MountRequest, Reboot, SystemName};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fs::{major, minor};
use poly_personality_linux::mountapi::{AttrChange, Command, FsContext, MountDesc, MountLink, PathAt};

use poly_vfs::{AT_FDCWD, DeviceId, FileType, MountFlags};

/// The longest host or domain name (`__NEW_UTS_LEN`).
const NAME_MAX: usize = 64;

/// A name of the system: its bytes and length.
struct Name {
    bytes: [u8; NAME_MAX],
    len: usize,
}

impl Name {
    const fn new(initial: &[u8]) -> Name {
        let mut bytes = [0u8; NAME_MAX];
        let mut i = 0;
        while i < initial.len() {
            bytes[i] = initial[i];
            i += 1;
        }
        Name { bytes, len: initial.len() }
    }

    fn get(&self) -> Vec<u8> {
        self.bytes[..self.len].to_vec()
    }
}

/// `uname`'s `nodename`: "poly" until sethostname(2) changes it.
static HOSTNAME: SpinLock<Name> = SpinLock::new(Name::new(b"poly"));
/// `uname`'s `domainname`: "(none)" until setdomainname(2), as Linux has it.
static DOMAINNAME: SpinLock<Name> = SpinLock::new(Name::new(b"(none)"));
/// Ctrl-Alt-Del resets at once (`LINUX_REBOOT_CMD_CAD_ON`) instead of
/// signalling init; recorded, but the keyboard does not act on it yet.
static CTRL_ALT_DEL_REBOOTS: AtomicBool = AtomicBool::new(true);

fn name(which: SystemName) -> &'static SpinLock<Name> {
    match which {
        SystemName::Host => &HOSTNAME,
        SystemName::Domain => &DOMAINNAME,
    }
}

/// The host name, for procfs (`/proc/sys/kernel/hostname`).
pub fn hostname() -> Vec<u8> {
    HOSTNAME.lock().get()
}

/// The NIS domain name, for procfs.
pub fn domainname() -> Vec<u8> {
    DOMAINNAME.lock().get()
}

/// Set the initial UTS namespace's host or domain name (at most
/// `NAME_MAX` bytes, checked by the caller), for a write to
/// `/proc/sys/kernel/hostname` or `domainname`.
pub fn set_name(which: SystemName, value: &[u8]) {
    *name(which).lock() = Name::new(&value[..value.len().min(NAME_MAX)]);
}

fn vfs_flags(f: admin::MountFlags) -> MountFlags {
    MountFlags { read_only: f.read_only, no_exec: f.no_exec, no_suid: f.no_suid, no_dev: f.no_dev }
}

/// The access time mode a mount(2) call names goes to the mount's
/// attributes (`MOUNT_ATTR_NOATIME` 0x10, `STRICTATIME` 0x20, `NODIRATIME`
/// 0x80; `/proc/<pid>/mountinfo` shows them); a call that names none
/// leaves them.
fn apply_atime(id: poly_vfs::MountId, f: admin::MountFlags) {
    if let (Some(atime), Some(m)) = (f.atime, crate::fs::vfs().mount_by_id(id)) {
        m.set_attrs((m.attrs() & !0xf0) | atime);
    }
}

/// The kernel's log as a `fmt::Write`, for what `power` reports.
struct Log;

impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        crate::kprint!("{s}");
        Ok(())
    }
}

/// The overlay the options in `data` describe: the layers are looked up as
/// the options come (a missing directory is `ENOENT` even when a later option
/// is wrong) and checked against each other. Read-only without an upper
/// layer.
pub(super) fn build_overlay(
    files: &crate::fs::Files,
    data: &[u8],
) -> Result<(alloc::sync::Arc<dyn poly_vfs::FileSystem>, bool), Errno> {
    use poly_overlayfs::{Config, Layer, Overlay, Redirect, is_inside};
    let layer_of = |path: &[u8]| -> Result<Layer, Errno> {
        let node = files.lookupat(AT_FDCWD, path, true).map_err(vfs_errno)?;
        if node.attr().map_err(vfs_errno)?.kind != FileType::Directory {
            return Err(Errno::EINVAL);
        }
        Ok(Layer { fs: node.mount().fs().clone(), root: node.ino() })
    };
    let mut lowers: Vec<Layer> = Vec::new();
    let (mut upper, mut work) = (None, None);
    let (mut userxattr, mut redirect) = (false, Redirect::Off);
    for (name, value) in admin::options(data) {
        match (name, value) {
            (b"lowerdir", Some(v)) => {
                for part in v.split(|&b| b == b':') {
                    lowers.push(layer_of(part)?);
                }
            }
            (b"upperdir", Some(v)) => upper = Some(layer_of(v)?),
            (b"workdir", Some(v)) => work = Some(layer_of(v)?),
            (b"userxattr", None) => userxattr = true,
            (b"redirect_dir", Some(v)) => redirect = Redirect::from_value(v).map_err(vfs_errno)?,
            (b"index" | b"metacopy" | b"nfs_export", Some(b"on" | b"off")) => {}
            (b"xino", Some(b"on" | b"off" | b"auto")) => {}
            (b"volatile" | b"default_permissions", None) => {}
            _ => return Err(Errno::EINVAL),
        }
    }
    if lowers.is_empty() {
        return Err(Errno::EINVAL);
    }
    match (&upper, &work) {
        (None, _) => {
            // Without an upper layer the mount is read-only and needs two layers.
            if lowers.len() < 2 {
                return Err(Errno::EINVAL);
            }
            work = None;
        }
        (Some(u), None) => {
            let _ = u;
            return Err(Errno::EINVAL);
        }
        (Some(u), Some(w)) => {
            if !alloc::sync::Arc::ptr_eq(&u.fs, &w.fs)
                || (u.root == w.root)
                || is_inside(&*w.fs, w.root, u.root)
                || is_inside(&*u.fs, u.root, w.root)
            {
                return Err(Errno::EINVAL);
            }
        }
    }
    let same = |a: &Layer, b: &Layer| alloc::sync::Arc::ptr_eq(&a.fs, &b.fs) && a.root == b.root;
    for (i, l) in lowers.iter().enumerate() {
        if let Some(u) = &upper
            && (same(l, u) || (alloc::sync::Arc::ptr_eq(&l.fs, &u.fs) && is_inside(&*l.fs, l.root, u.root)))
        {
            return Err(Errno::ELOOP);
        }
        if lowers[..i].iter().any(|o| same(o, l)) {
            return Err(Errno::ELOOP);
        }
    }
    let _ = work;
    let read_only = upper.is_none();
    let overlay =
        Overlay::<crate::sync::RawMutex>::new(Config { upper, lowers, userxattr, redirect }).map_err(vfs_errno)?;
    // Not behind the page cache: writes reach the upper layer at once,
    // where the layers can be looked at directly.
    let fs: alloc::sync::Arc<dyn poly_vfs::FileSystem> = alloc::sync::Arc::new(overlay);
    Ok((fs, read_only))
}

impl Thread<'_> {
    /// mount(2) of a squashfs image from a loop device (ADR 0168): the
    /// device's file system (the same one when it is mounted already), never
    /// twice on one directory (`EBUSY`).
    fn mount_squashfs(&mut self, source: &[u8], target: &[u8], rdev: DeviceId, flags: MountFlags) -> Result<(), Errno> {
        if !crate::loopdev::is_loop(rdev) {
            return Err(Errno::ENXIO);
        }
        let files = &self.process().files;
        let fs = crate::loopdev::mount_squashfs(rdev).map_err(vfs_errno)?;
        if let Ok(id) = files.mount_at(target, true)
            && let Some(m) = crate::fs::vfs().mount_by_id(id)
            && alloc::sync::Arc::ptr_eq(m.fs(), &fs)
        {
            return Err(Errno::EBUSY);
        }
        let id = files.mount_fs(target, fs, flags).map_err(vfs_errno)?;
        mounts::record_source(id, source);
        Ok(())
    }

    /// mount(2) of a FUSE file system (ADR 0168): `fd=` names an open
    /// `/dev/fuse` not bound yet; the kernel then asks the server with
    /// `FUSE_INIT`.
    fn mount_fuse(
        &mut self,
        source: &[u8],
        target: &[u8],
        fstype: &[u8],
        flags: admin::MountFlags,
        data: &[u8],
    ) -> Result<(), Errno> {
        use poly_fuse::{Caller, FuseFs, Options};
        let files = &self.process().files;
        let tnode = files.lookupat(AT_FDCWD, target, true).map_err(vfs_errno)?;
        if tnode.attr().map_err(vfs_errno)?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let num = |v: Option<&[u8]>, radix: u32| -> Result<u32, Errno> {
            let t = core::str::from_utf8(v.ok_or(Errno::EINVAL)?).map_err(|_| Errno::EINVAL)?;
            u32::from_str_radix(t, radix).map_err(|_| Errno::EINVAL)
        };
        let (mut fd, mut root_mode, mut user_id, mut group_id) = (None, None, None, None);
        let mut opts = Options::default();
        let mut subtype: Option<Vec<u8>> = None;
        for (name, value) in admin::options(data) {
            match name {
                b"fd" => fd = Some(num(value, 10)?),
                b"rootmode" => root_mode = Some(num(value, 8)?),
                b"user_id" => user_id = Some(num(value, 10)?),
                b"group_id" => group_id = Some(num(value, 10)?),
                b"max_read" => opts.max_read = num(value, 10)?,
                b"default_permissions" => opts.default_permissions = true,
                b"allow_other" => opts.allow_other = true,
                b"subtype" => subtype = Some(value.ok_or(Errno::EINVAL)?.to_vec()),
                b"blksize" | b"max_pages" => {}
                _ => return Err(Errno::EINVAL),
            }
        }
        let (Some(fd), Some(root_mode), Some(user_id), Some(group_id)) = (fd, root_mode, user_id, group_id) else {
            return Err(Errno::EINVAL);
        };
        if root_mode & 0o170000 != 0o040000 {
            return Err(Errno::EINVAL);
        }
        // The owner of the mount is named by ids of the mounter's user
        // namespace: one without a mapping there is EINVAL (the spec sheet
        // of fuse_mount).
        let mounter_ns = super::cred::current_creds().and_then(|c| c.user_ns);
        let kernel_id = |kind, id| poly_personality_linux::ns::user::down(&mounter_ns, kind, id).ok_or(Errno::EINVAL);
        opts.root_mode = root_mode;
        opts.user_id = kernel_id(poly_personality_linux::ns::user::IdKind::Uid, user_id)?;
        opts.group_id = kernel_id(poly_personality_linux::ns::user::IdKind::Gid, group_id)?;
        let file = files.file(fd as i32).map_err(|_| Errno::EINVAL)?;
        let conn = file
            .device()
            .and_then(|d| d.as_any())
            .and_then(|a| a.downcast_ref::<crate::fuse::FuseFile>())
            .map(|f| f.conn().clone())
            .ok_or(Errno::EINVAL)?;
        let cred = files.cred();
        let who = Caller { uid: cred.uid, gid: cred.gid, pid: self.process().pid, umask: 0 };
        conn.bind(who).map_err(vfs_errno)?;
        let name = match (&subtype, fstype) {
            (Some(s), _) => {
                let mut n = b"fuse.".to_vec();
                n.extend_from_slice(s);
                crate::fuse::type_name(&n)
            }
            (None, t) => crate::fuse::type_name(t),
        };
        let fs =
            alloc::sync::Arc::new(FuseFs::new(conn, alloc::sync::Arc::new(crate::fuse::KernelEnv), opts.clone(), name));
        let made = files.mount_fs(target, fs.clone(), vfs_flags(flags));
        fs.finish_mount();
        let id = made.map_err(vfs_errno)?;
        let mut sb = alloc::format!("user_id={user_id},group_id={group_id}").into_bytes();
        if opts.default_permissions {
            sb.extend_from_slice(b",default_permissions");
        }
        if opts.allow_other {
            sb.extend_from_slice(b",allow_other");
        }
        if opts.max_read != 0 {
            sb.extend_from_slice(alloc::format!(",max_read={}", opts.max_read).as_bytes());
        }
        mounts::record_source(id, source);
        if let Some(m) = crate::fs::vfs().mount_by_id(id) {
            m.set_sb_info(source, &sb);
        }
        Ok(())
    }

    /// mount(2) of an overlay (ADR 0168): the layers named in `data`, looked
    /// up as the options come (a missing directory is `ENOENT` even when a
    /// later option is wrong), then checked against each other.
    fn mount_overlay(
        &mut self,
        source: &[u8],
        target: &[u8],
        flags: admin::MountFlags,
        data: &[u8],
    ) -> Result<(), Errno> {
        let files = &self.process().files;
        let tnode = files.lookupat(AT_FDCWD, target, true).map_err(vfs_errno)?;
        if tnode.attr().map_err(vfs_errno)?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let (fs, read_only) = build_overlay(files, data)?;
        let mut mflags = vfs_flags(flags);
        mflags.read_only |= read_only;
        let id = files.mount_fs(target, fs, mflags).map_err(vfs_errno)?;
        mounts::record_source(id, if source.is_empty() { b"overlay" } else { source });
        if let Some(m) = crate::fs::vfs().mount_by_id(id) {
            m.set_sb_info(if source.is_empty() { b"overlay" } else { source }, data);
        }
        Ok(())
    }

    fn mount_new(
        &mut self,
        source: &[u8],
        target: &[u8],
        fstype: &[u8],
        flags: admin::MountFlags,
        data: &[u8],
    ) -> Result<(), Errno> {
        let fs = FsType::from_name(fstype)?;
        if fs == FsType::Overlay {
            return self.mount_overlay(source, target, flags, data);
        }
        if fs == FsType::Fuse {
            return self.mount_fuse(source, target, fstype, flags, data);
        }
        let mut root = admin::mount_options(fs, data)?;
        let files = &self.process().files;
        if matches!(fs, FsType::Tmpfs) {
            // The top directory belongs to the mounter (tmpfs(5)).
            let cred = files.cred();
            root.uid.get_or_insert(cred.uid);
            root.gid.get_or_insert(cred.gid);
        }
        let asked = flags;
        let flags = vfs_flags(flags);
        let id = if fs.needs_device() {
            if source.is_empty() {
                return Err(Errno::ENOENT);
            }
            let node = files.lookupat(AT_FDCWD, source, true).map_err(vfs_errno)?;
            let attr = node.attr().map_err(vfs_errno)?;
            if attr.kind != FileType::BlockDevice {
                return Err(Errno::ENOTBLK);
            }
            if node.mount().flags().no_dev {
                return Err(Errno::EACCES);
            }
            if fs == FsType::Squashfs {
                return self.mount_squashfs(source, target, attr.rdev, flags);
            }
            let disk = mounts::disk_of(attr.rdev).ok_or(Errno::ENXIO)?;
            let mounted = disk::mount_flags(&disk, flags, |fs, flags| {
                let id = files.mount_fs(target, fs, flags)?;
                let path = files.mount_path(id).unwrap_or_else(|_| target.to_vec());
                Ok((id, alloc::string::String::from_utf8_lossy(&path).into_owned()))
            });
            match mounted {
                Ok(m) => m.mount_id,
                Err(e) => return Err(vfs_errno(e.errno())),
            }
        } else {
            let made = mounts::new_virtual(fs, root).map_err(vfs_errno)?;
            let id = files.mount_fs(target, made, flags).map_err(vfs_errno)?;
            if let Some(m) = crate::fs::vfs().mount_by_id(id) {
                let source = if source.is_empty() { &b"none"[..] } else { source };
                // MS_SYNCHRONOUS is the file system's: `sync` leads its options.
                let mut options = if asked.sync { b"sync".to_vec() } else { Vec::new() };
                let own = poly_personality_linux::mountapi::options_string(fs, &root);
                if !options.is_empty() && !own.is_empty() {
                    options.push(b',');
                }
                options.extend_from_slice(&own);
                m.set_sb_info(source, &options);
            }
            id
        };
        apply_atime(id, asked);
        mounts::record_source(id, if source.is_empty() { b"none" } else { source });
        Ok(())
    }
}

impl LinuxAdmin for Thread<'_> {
    fn mount(&mut self, req: &MountRequest) -> Result<(), Errno> {
        let files = &self.process().files;
        match req {
            MountRequest::New { source, target, fstype, flags, data } => {
                self.mount_new(source, target, fstype, *flags, data)
            }
            MountRequest::Remount { target, flags, data, bind } => {
                // mount's spec sheet, `order`: the target (ENOENT; EINVAL
                // if not a mount root), then a tmpfs's options (EINVAL),
                // then the writers (EBUSY, in the VFS). A refused remount
                // changes nothing.
                let id = files.mount_at(target, true).map_err(vfs_errno)?;
                if !*bind && let Some(m) = crate::fs::vfs().mount_by_id(id) {
                    admin::check_remount_options(m.fs_name(), data)?;
                }
                files.remount(target, vfs_flags(*flags), *bind).map_err(vfs_errno)?;
                apply_atime(id, *flags);
                if !*bind
                    && disk::mount_of(id).is_none()
                    && let Some(m) = crate::fs::vfs().mount_by_id(id)
                {
                    // Without MS_BIND the file system itself changes too
                    // (statmount(2)'s `sb_flags`, every mount of it).
                    m.set_sb_read_only(flags.read_only);
                }
                if let Some(m) = disk::mount_of(id)
                    && !flags.read_only
                    && m.read_only
                {
                    // The volume itself is read-only (the disk, or its
                    // features): the mount stays so.
                    files
                        .remount(target, MountFlags { read_only: true, ..vfs_flags(*flags) }, *bind)
                        .map_err(vfs_errno)?;
                    return Err(Errno::EROFS);
                }
                Ok(())
            }
            MountRequest::Bind { source, target, flags, recursive: _ } => {
                // The bind shows the source of the mount it was taken from
                // (proc_pid_mountinfo's spec sheet), not the path bound.
                let from = files.lookupat(AT_FDCWD, source, true).ok().map(|n| {
                    let m = n.mount();
                    mounts::source(m.id()).unwrap_or_else(|| disk::source_of(m.id(), m.fs_name()).into_bytes())
                });
                let id = files.mount_bind(source, target, vfs_flags(*flags)).map_err(vfs_errno)?;
                mounts::record_source(id, from.as_deref().unwrap_or(source));
                Ok(())
            }
            MountRequest::Propagation { target, kind, recursive } => {
                let id = files.mount_at(target, true).map_err(vfs_errno)?;
                let vfs = crate::fs::vfs();
                let m = vfs.mount_by_id(id).ok_or(Errno::EINVAL)?;
                let change = poly_personality_linux::mountapi::AttrChange { propagation: *kind, ..Default::default() };
                vfs.set_propagation(&vfs.mount_subtree(&m, *recursive), change.propagation);
                Ok(())
            }
            MountRequest::Move { source, target } => {
                let src = files.lookupat(AT_FDCWD, source, true).map_err(vfs_errno)?;
                let dst = files.lookupat(AT_FDCWD, target, true).map_err(vfs_errno)?;
                crate::fs::vfs().move_mount(&files.cred(), &src, &dst).map_err(vfs_errno)
            }
        }
    }

    fn umount_expire(&mut self, target: &[u8]) -> Result<(), Errno> {
        self.process().files.umount_expire(target).map_err(vfs_errno)
    }

    fn umount(&mut self, target: &[u8], detach: bool, follow: bool) -> Result<(), Errno> {
        let files = &self.process().files;
        let id = files.mount_at(target, follow).map_err(vfs_errno)?;
        match disk::mount_of(id) {
            // A disk is written back and its volume marked clean; it must
            // not be busy, detached or not.
            Some(m) => disk::unmount_mount(&m).map_err(vfs_errno)?,
            None => files.umount(target, detach).map_err(vfs_errno)?,
        }
        mounts::forget(id);
        Ok(())
    }

    fn mknodat(&mut self, dirfd: i32, path: &[u8], mode: u32, dev: u64, may_mknod: bool) -> Result<(), Errno> {
        let kind = match mode & 0o170000 {
            0o100000 => FileType::Regular,
            0o020000 => FileType::CharDevice,
            0o060000 => FileType::BlockDevice,
            0o010000 => FileType::Fifo,
            0o140000 => FileType::Socket,
            _ => return Err(Errno::EINVAL),
        };
        let rdev = DeviceId::new(major(dev), minor(dev));
        self.process().files.mknodat(dirfd, path, kind, (mode & 0o7777) as u16, rdev, may_mknod).map_err(vfs_errno)
    }

    fn chroot(&mut self, path: &[u8], may_chroot: bool) -> Result<(), Errno> {
        self.process().files.chroot(path, may_chroot).map_err(vfs_errno)
    }

    fn system_name(&self, which: SystemName) -> Vec<u8> {
        name(which).lock().get()
    }

    fn set_system_name(&mut self, which: SystemName, value: &[u8]) -> Result<(), Errno> {
        if value.len() > NAME_MAX {
            return Err(Errno::EINVAL);
        }
        *name(which).lock() = Name::new(value);
        Ok(())
    }

    fn set_ctrl_alt_del(&mut self, reboots: bool) {
        CTRL_ALT_DEL_REBOOTS.store(reboots, Ordering::Relaxed);
    }

    fn reboot(&mut self, how: Reboot) -> Errno {
        // reboot(2) names the messages each command prints.
        match how {
            Reboot::Restart => {
                kprintln!("reboot: Restarting system.");
                power::reboot(&mut Log)
            }
            Reboot::Halt => power::halt(&mut Log),
            Reboot::PowerOff => {
                kprintln!("reboot: Power down.");
                power::power_off(&mut Log)
            }
        }
    }

    // The new mount API and pivot_root (ADR 0122): `super::mountapi`.

    fn open_tree(&mut self, at: &PathAt, clone: Option<bool>, cloexec: bool) -> Result<i32, Errno> {
        self.mnt_open_tree(at, clone, cloexec)
    }

    fn move_mount(&mut self, from: &PathAt, to: &PathAt) -> Result<(), Errno> {
        self.mnt_move_mount(from, to)
    }

    fn mount_setattr(&mut self, at: &PathAt, recursive: bool, change: &AttrChange) -> Result<(), Errno> {
        self.mnt_setattr(at, recursive, change)
    }

    fn fsopen(&mut self, ctx: FsContext, cloexec: bool) -> Result<i32, Errno> {
        self.mnt_fsopen(ctx, cloexec)
    }

    fn fspick(&mut self, at: &PathAt, cloexec: bool) -> Result<i32, Errno> {
        self.mnt_fspick(at, cloexec)
    }

    fn fsconfig(&mut self, fd: i32, cmd: &Command<'_>) -> Result<(), Errno> {
        self.mnt_fsconfig(fd, cmd)
    }

    fn fsmount(&mut self, fd: i32, attrs: u64, cloexec: bool) -> Result<i32, Errno> {
        self.mnt_fsmount(fd, attrs, cloexec)
    }

    fn mount_links(&mut self) -> Result<Vec<MountLink>, Errno> {
        self.mnt_links()
    }

    fn describe_mount(&mut self, id: u64) -> Result<MountDesc, Errno> {
        self.mnt_describe(id)
    }

    fn pivot_root(&mut self, new_root: &[u8], put_old: &[u8]) -> Result<(), Errno> {
        self.mnt_pivot_root(new_root, put_old)
    }
}
