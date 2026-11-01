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

fn vfs_flags(f: admin::MountFlags) -> MountFlags {
    MountFlags { read_only: f.read_only, no_exec: f.no_exec, no_suid: f.no_suid, no_dev: f.no_dev }
}

/// The kernel's log as a `fmt::Write`, for what `power` reports.
struct Log;

impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        crate::kprint!("{s}");
        Ok(())
    }
}

impl Thread<'_> {
    fn mount_new(
        &mut self,
        source: &[u8],
        target: &[u8],
        fstype: &[u8],
        flags: admin::MountFlags,
        data: &[u8],
    ) -> Result<(), Errno> {
        let fs = FsType::from_name(fstype)?;
        let root = admin::mount_options(fs, data)?;
        let files = &self.process().files;
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
                m.set_sb_info(source, &poly_personality_linux::mountapi::options_string(fs, &root));
            }
            id
        };
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
                // The file system's own options are only checked for the
                // memory file systems, whose options Poly knows.
                let id = files.mount_at(target, true).map_err(vfs_errno)?;
                let _ = data;
                files.remount(target, vfs_flags(*flags)).map_err(vfs_errno)?;
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
                    files.remount(target, MountFlags { read_only: true, ..vfs_flags(*flags) }).map_err(vfs_errno)?;
                    return Err(Errno::EROFS);
                }
                Ok(())
            }
            MountRequest::Bind { source, target, flags, recursive: _ } => {
                let id = files.mount_bind(source, target, vfs_flags(*flags)).map_err(vfs_errno)?;
                mounts::record_source(id, source);
                Ok(())
            }
            MountRequest::Propagation { target, kind, recursive } => {
                let id = files.mount_at(target, true).map_err(vfs_errno)?;
                let vfs = crate::fs::vfs();
                let m = vfs.mount_by_id(id).ok_or(Errno::EINVAL)?;
                let change = poly_personality_linux::mountapi::AttrChange { propagation: *kind, ..Default::default() };
                for m in vfs.mount_subtree(&m, *recursive) {
                    m.set_propagation(change.propagation_of(u64::from(m.propagation())) as u32);
                }
                Ok(())
            }
            MountRequest::Move { source, target } => {
                let src = files.lookupat(AT_FDCWD, source, true).map_err(vfs_errno)?;
                let dst = files.lookupat(AT_FDCWD, target, true).map_err(vfs_errno)?;
                crate::fs::vfs().move_mount(&files.cred(), &src, &dst).map_err(vfs_errno)
            }
        }
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

    fn mknodat(&mut self, dirfd: i32, path: &[u8], mode: u32, dev: u64) -> Result<(), Errno> {
        let kind = match mode & 0o170000 {
            0o100000 => FileType::Regular,
            0o020000 => FileType::CharDevice,
            0o060000 => FileType::BlockDevice,
            0o010000 => FileType::Fifo,
            0o140000 => FileType::Socket,
            _ => return Err(Errno::EINVAL),
        };
        let rdev = DeviceId::new(major(dev), minor(dev));
        self.process().files.mknodat(dirfd, path, kind, (mode & 0o7777) as u16, rdev).map_err(vfs_errno)
    }

    fn chroot(&mut self, path: &[u8]) -> Result<(), Errno> {
        self.process().files.chroot(path).map_err(vfs_errno)
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
