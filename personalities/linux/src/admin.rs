//! System administration services the kernel provides for the calls in
//! `sys::admin` (ADR 0051): mounting file systems (mount(2), umount2(2)),
//! device nodes (mknod(2)), chroot(2), the host and domain names
//! (sethostname(2), setdomainname(2)) and rebooting or powering off
//! (reboot(2)).
//!
//! Like [`LinuxFs`](crate::fs::LinuxFs) it is a trait of its own, reached
//! through [`LinuxKernel::admin`](crate::kernel::LinuxKernel::admin), so
//! the shared kernel trait stays small. The personality decodes and checks
//! the arguments (flags, privileges, sizes); the kernel does the work with
//! arguments already in kernel memory.

use crate::errno::Errno;
use crate::mountapi::{AttrChange, Command, FsContext, MountDesc, MountLink, PathAt};
use alloc::vec::Vec;

/// `mount(2)` flags (`linux/mount.h`).
pub mod ms {
    pub const RDONLY: u64 = 1;
    pub const NOSUID: u64 = 2;
    pub const NODEV: u64 = 4;
    pub const NOEXEC: u64 = 8;
    pub const SYNCHRONOUS: u64 = 16;
    pub const REMOUNT: u64 = 32;
    pub const MANDLOCK: u64 = 64;
    pub const DIRSYNC: u64 = 128;
    pub const NOSYMFOLLOW: u64 = 256;
    pub const NOATIME: u64 = 1024;
    pub const NODIRATIME: u64 = 2048;
    pub const BIND: u64 = 4096;
    pub const MOVE: u64 = 8192;
    pub const REC: u64 = 16384;
    pub const SILENT: u64 = 32768;
    pub const POSIXACL: u64 = 1 << 16;
    pub const UNBINDABLE: u64 = 1 << 17;
    pub const PRIVATE: u64 = 1 << 18;
    pub const SLAVE: u64 = 1 << 19;
    pub const SHARED: u64 = 1 << 20;
    pub const RELATIME: u64 = 1 << 21;
    pub const KERNMOUNT: u64 = 1 << 22;
    pub const I_VERSION: u64 = 1 << 23;
    pub const STRICTATIME: u64 = 1 << 24;
    pub const LAZYTIME: u64 = 1 << 25;
    /// The magic number old programs put in the top 16 bits (`MS_MGC_VAL`
    /// under the mask `MS_MGC_MSK`); mount(2): ignored.
    pub const MGC_VAL: u64 = 0xC0ED_0000;
    pub const MGC_MSK: u64 = 0xFFFF_0000;
    /// The propagation types.
    pub const PROPAGATION: u64 = SHARED | PRIVATE | SLAVE | UNBINDABLE;
}

/// `umount2(2)` flags (`linux/fs.h`, `sys/mount.h`).
pub mod mnt {
    pub const FORCE: u32 = 1;
    pub const DETACH: u32 = 2;
    pub const EXPIRE: u32 = 4;
    pub const UMOUNT_NOFOLLOW: u32 = 8;
}

/// The per-mount flags a mount or remount sets (mount(2)): the rest of
/// `mountflags` (access times, synchronous writes, ...) are accepted and
/// have no effect on Poly's file systems.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MountFlags {
    pub read_only: bool,
    pub no_suid: bool,
    pub no_dev: bool,
    pub no_exec: bool,
}

impl MountFlags {
    pub const fn from_bits(flags: u64) -> MountFlags {
        MountFlags {
            read_only: flags & ms::RDONLY != 0,
            no_suid: flags & ms::NOSUID != 0,
            no_dev: flags & ms::NODEV != 0,
            no_exec: flags & ms::NOEXEC != 0,
        }
    }
}

/// What one mount(2) call asks for, decided by the flags in the order
/// mount(2) gives: remount, bind, propagation change, move, new mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountRequest {
    /// A new mount of a file system of type `fstype` from `source` (a
    /// block device for disk file systems, ignored by the others) with
    /// the file system's options `data` (a comma-separated string, its
    /// NUL dropped).
    New { source: Vec<u8>, target: Vec<u8>, fstype: Vec<u8>, flags: MountFlags, data: Vec<u8> },
    /// `MS_REMOUNT`: new per-mount flags for the mount at `target`
    /// (with `MS_BIND`, only those; otherwise also the file system's
    /// options in `data`, and `MS_RDONLY` for the file system itself).
    Remount { target: Vec<u8>, flags: MountFlags, data: Vec<u8>, bind: bool },
    /// `MS_BIND`: what `source` names, visible at `target` too.
    Bind { source: Vec<u8>, target: Vec<u8>, flags: MountFlags, recursive: bool },
    /// `MS_SHARED` / `MS_PRIVATE` / `MS_SLAVE` / `MS_UNBINDABLE` (`kind`,
    /// the one type given) for the mount at `target`, and with `MS_REC`
    /// (`recursive`) every mount below it. Poly propagates nothing: the
    /// type is recorded (statmount(2) reports it, pivot_root(2) refuses
    /// shared mounts) and a slave without a master stays private.
    Propagation { target: Vec<u8>, kind: u64, recursive: bool },
    /// `MS_MOVE`: the mount at `source` moves to `target`.
    Move { source: Vec<u8>, target: Vec<u8> },
}

/// The file system types mount(2) knows (`/proc/filesystems` lists them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsType {
    /// proc(5).
    Proc,
    /// sysfs(5).
    Sysfs,
    /// The pseudo-terminal slaves, pts(4).
    Devpts,
    /// tmpfs(5) (and `ramfs`, which Poly serves the same way).
    Tmpfs,
    /// ext2, ext3 or ext4 on a block device (one driver, `poly-ext2`).
    Ext2,
}

impl FsType {
    /// The type mount(2)'s `filesystemtype` names (`ENODEV` for others).
    pub fn from_name(name: &[u8]) -> Result<FsType, Errno> {
        Ok(match name {
            b"proc" => FsType::Proc,
            b"sysfs" => FsType::Sysfs,
            b"devpts" => FsType::Devpts,
            b"tmpfs" | b"ramfs" => FsType::Tmpfs,
            b"ext2" | b"ext3" | b"ext4" => FsType::Ext2,
            _ => return Err(Errno::ENODEV),
        })
    }

    /// Whether `source` names a block device to mount (else it is only a
    /// name shown in `/proc/mounts`).
    pub const fn needs_device(self) -> bool {
        matches!(self, FsType::Ext2)
    }
}

/// The options of a file system's `data` (comma-separated `name` or
/// `name=value`), empty ones skipped.
pub fn options(data: &[u8]) -> impl Iterator<Item = (&[u8], Option<&[u8]>)> {
    data.split(|&b| b == b',').filter(|o| !o.is_empty()).map(|o| match o.iter().position(|&b| b == b'=') {
        Some(i) => (&o[..i], Some(&o[i + 1..])),
        None => (o, None),
    })
}

/// The options of a new mount that set the root directory of a memory
/// file system (tmpfs(5): `mode`, `uid`, `gid`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RootOptions {
    pub mode: Option<u16>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// tmpfs(5) `size` (or `nr_blocks` pages): the most file data, in
    /// bytes or as a percentage of RAM; `Count(0)` is no limit.
    pub size: Option<Amount>,
    /// tmpfs(5) `nr_inodes`: the most inodes; `Count(0)` is no limit.
    pub inodes: Option<Amount>,
}

/// The value of a tmpfs size option: a count (bytes, pages or inodes,
/// with the `k`, `m`, `g`, `t`, `p`, `e` suffixes applied) or a
/// percentage of RAM (`%`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Count(u64),
    Percent(u64),
}

/// tmpfs(5)'s `nr_blocks` counts pages of this size.
const PAGE_BYTES: u64 = 4096;

fn number(v: Option<&[u8]>, radix: u32) -> Result<u64, Errno> {
    let v = core::str::from_utf8(v.ok_or(Errno::EINVAL)?).map_err(|_| Errno::EINVAL)?;
    u64::from_str_radix(v, radix).map_err(|_| Errno::EINVAL)
}

/// A size option (tmpfs(5) `size`, `nr_blocks`, `nr_inodes`): a number
/// with an optional `k`, `m`, `g`, `t`, `p` or `e` suffix (powers of
/// 1024), or a percentage.
fn size(v: Option<&[u8]>) -> Result<Amount, Errno> {
    let v = v.ok_or(Errno::EINVAL)?;
    let (digits, shift) = match v.last().map(u8::to_ascii_lowercase) {
        Some(b'%') => return number(Some(&v[..v.len() - 1]), 10).map(Amount::Percent),
        Some(b'k') => (&v[..v.len() - 1], 10),
        Some(b'm') => (&v[..v.len() - 1], 20),
        Some(b'g') => (&v[..v.len() - 1], 30),
        Some(b't') => (&v[..v.len() - 1], 40),
        Some(b'p') => (&v[..v.len() - 1], 50),
        Some(b'e') => (&v[..v.len() - 1], 60),
        _ => (v, 0),
    };
    let n = number(Some(digits), 10)?;
    n.checked_mul(1 << shift).map(Amount::Count).ok_or(Errno::EINVAL)
}

/// Check a new mount's `data` for file system `fs` and return what Poly
/// applies of it. Each type takes the options its manual page lists
/// (tmpfs(5); proc(5): `hidepid`, `gid`, `subset`; devpts as mount(8)
/// lists them; sysfs none); anything else is `EINVAL`. Poly applies a
/// tmpfs root's `mode`, `uid` and `gid` and its size limits (`size`,
/// `nr_blocks`, `nr_inodes`); the other types' options are accepted and
/// have no effect yet. ext2's many options
/// (mount(8)) are accepted as they are.
pub fn mount_options(fs: FsType, data: &[u8]) -> Result<RootOptions, Errno> {
    let mut root = RootOptions::default();
    for (name, value) in options(data) {
        match (fs, name) {
            (FsType::Tmpfs, b"mode") => root.mode = Some((number(value, 8)? & 0o7777) as u16),
            (FsType::Tmpfs, b"uid") => root.uid = Some(number(value, 10)? as u32),
            (FsType::Tmpfs, b"gid") => root.gid = Some(number(value, 10)? as u32),
            (FsType::Tmpfs, b"size") => root.size = Some(size(value)?),
            (FsType::Tmpfs, b"nr_blocks") => {
                root.size = Some(match size(value)? {
                    Amount::Count(pages) => Amount::Count(pages.checked_mul(PAGE_BYTES).ok_or(Errno::EINVAL)?),
                    percent => percent,
                });
            }
            (FsType::Tmpfs, b"nr_inodes") => root.inodes = Some(size(value)?),
            (FsType::Tmpfs, b"huge" | b"mpol") if value.is_some() => {}
            (FsType::Tmpfs, b"noswap" | b"inode32" | b"inode64") if value.is_none() => {}
            (FsType::Proc, b"hidepid" | b"gid" | b"subset") if value.is_some() => {}
            (FsType::Devpts, b"uid" | b"gid" | b"max") => {
                number(value, 10)?;
            }
            (FsType::Devpts, b"mode" | b"ptmxmode") => {
                number(value, 8)?;
            }
            (FsType::Devpts, b"newinstance") if value.is_none() => {}
            (FsType::Ext2, _) => {}
            _ => return Err(Errno::EINVAL),
        }
    }
    Ok(root)
}

/// reboot(2)'s commands (`linux/reboot.h`).
pub mod cmd {
    pub const RESTART: u32 = 0x0123_4567;
    pub const HALT: u32 = 0xCDEF_0123;
    pub const CAD_ON: u32 = 0x89AB_CDEF;
    pub const CAD_OFF: u32 = 0x0000_0000;
    pub const POWER_OFF: u32 = 0x4321_FEDC;
    pub const RESTART2: u32 = 0xA1B2_C3D4;
    pub const SW_SUSPEND: u32 = 0xD000_FCE2;
    pub const KEXEC: u32 = 0x4558_4543;
}

/// What reboot(2) asks the kernel to do with the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reboot {
    /// Reset the machine ("Restarting system.").
    Restart,
    /// Stop the system ("System halted."), without powering off.
    Halt,
    /// Power off ("Power down."), or halt if the machine cannot.
    PowerOff,
}

/// Which name of the system sethostname(2) / setdomainname(2) set and
/// uname(2) reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemName {
    /// `nodename`.
    Host,
    /// `domainname` (NIS).
    Domain,
}

/// The kernel's administration services for the calling thread.
pub trait LinuxAdmin {
    /// Carry out a checked mount(2) request as the calling thread (paths
    /// relative to its working directory).
    fn mount(&mut self, req: &MountRequest) -> Result<(), Errno>;

    /// umount2(2) of the mount at `target`: `detach` is `MNT_DETACH`,
    /// `follow` false for `UMOUNT_NOFOLLOW`.
    fn umount(&mut self, target: &[u8], detach: bool, follow: bool) -> Result<(), Errno>;

    /// mknodat(2): a regular file, FIFO, socket or device node (`mode`
    /// has the type bits, checked; `dev` is the device number as
    /// `makedev` encodes it).
    fn mknodat(&mut self, dirfd: i32, path: &[u8], mode: u32, dev: u64) -> Result<(), Errno>;

    /// chroot(2): the calling process's root directory becomes `path`.
    fn chroot(&mut self, path: &[u8]) -> Result<(), Errno>;

    /// The current host or domain name.
    fn system_name(&self, which: SystemName) -> Vec<u8>;

    /// Set the host or domain name (checked: at most 64 bytes).
    fn set_system_name(&mut self, which: SystemName, name: &[u8]) -> Result<(), Errno>;

    /// Whether Ctrl-Alt-Del resets at once (`LINUX_REBOOT_CMD_CAD_ON`)
    /// rather than sending `SIGINT` to init.
    fn set_ctrl_alt_del(&mut self, reboots: bool);

    /// Restart, halt or power off the machine. Returns only if it failed.
    fn reboot(&mut self, how: Reboot) -> Errno;

    // ---- the new mount API (ADR 0122, `crate::mountapi`) ----
    //
    // The personality checks flags, privileges and structures; these
    // carry out the rest as the calling thread. `ENOSYS` without them.

    /// open_tree(2): an `O_PATH` descriptor of `at`; with `clone`
    /// (`OPEN_TREE_CLONE`, `Some(recursive)`) of the root of a detached
    /// copy of the mount there (the subtree from `at`).
    fn open_tree(&mut self, _at: &PathAt, _clone: Option<bool>, _cloexec: bool) -> Result<i32, Errno> {
        Err(Errno::ENOSYS)
    }

    /// move_mount(2): attach the detached mount whose root `from` names,
    /// or move the attached one, to what `to` names.
    fn move_mount(&mut self, _from: &PathAt, _to: &PathAt) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }

    /// mount_setattr(2) of the mount whose root `at` names (and with
    /// `recursive` every mount below it); `change` is checked.
    fn mount_setattr(&mut self, _at: &PathAt, _recursive: bool, _change: &AttrChange) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }

    /// fsopen(2): a descriptor (`O_RDWR`) for the new context `ctx`.
    fn fsopen(&mut self, _ctx: FsContext, _cloexec: bool) -> Result<i32, Errno> {
        Err(Errno::ENOSYS)
    }

    /// fspick(2): a context for the superblock of the mount whose root
    /// `at` names (`EINVAL` if it is no mount's root).
    fn fspick(&mut self, _at: &PathAt, _cloexec: bool) -> Result<i32, Errno> {
        Err(Errno::ENOSYS)
    }

    /// fsconfig(2) on descriptor `fd`: `EBADF` if it is not open (or
    /// `O_PATH`), `EINVAL` if it is no context; then
    /// [`FsContext::config`](crate::mountapi::FsContext::config).
    fn fsconfig(&mut self, _fd: i32, _cmd: &Command<'_>) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }

    /// fsmount(2): a detached mount of the superblock context `fd` made,
    /// with the checked `MOUNT_ATTR_*` bits `attrs`; an `O_PATH`
    /// descriptor of its root.
    fn fsmount(&mut self, _fd: i32, _attrs: u64, _cloexec: bool) -> Result<i32, Errno> {
        Err(Errno::ENOSYS)
    }

    /// The mounts of the caller's mount namespace, its root first
    /// (listmount(2)).
    fn mount_links(&mut self) -> Result<Vec<MountLink>, Errno> {
        Err(Errno::ENOSYS)
    }

    /// What statmount(2) reports about mount `id` (Poly's id) of the
    /// caller's namespace (`ENOENT` if it is not in it).
    fn describe_mount(&mut self, _id: u64) -> Result<MountDesc, Errno> {
        Err(Errno::ENOSYS)
    }

    /// pivot_root(2) in the caller's mount namespace.
    fn pivot_root(&mut self, _new_root: &[u8], _put_old: &[u8]) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
}
