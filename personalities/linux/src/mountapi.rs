//! The new mount API (ADR 0122): what `fsopen(2)`, `fsconfig(2)`,
//! `fsmount(2)`, `fspick(2)`, `open_tree(2)`, `move_mount(2)`,
//! `mount_setattr(2)`, `statmount(2)`, `listmount(2)` and `pivot_root(2)`
//! decide without the kernel: the UAPI constants and structures
//! (`linux/mount.h`), the per-mount attribute arithmetic, the file system
//! context's state and message log, the options string a file system
//! shows, the encoding of `struct statmount` and the selection of
//! `listmount`.
//!
//! The kernel does the work on the mount table (`poly_vfs`) through the
//! [`LinuxAdmin`](crate::admin::LinuxAdmin) methods of this API; a file
//! system context's file holds an [`FsContext`] and lets it drive the
//! kernel's superblocks through [`SuperblockOps`].

use crate::admin::{self, Amount, FsType, MountFlags, RootOptions};
use crate::errno::Errno;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

/// Constants of `linux/mount.h` (and the `AT_*` flags these calls take,
/// `linux/fcntl.h`).
pub mod uapi {
    pub const OPEN_TREE_CLONE: u32 = 1;
    /// `OPEN_TREE_CLOEXEC`, `FSOPEN_CLOEXEC`'s cousin: `O_CLOEXEC`.
    pub const OPEN_TREE_CLOEXEC: u32 = 0o2000000;
    pub const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
    pub const AT_NO_AUTOMOUNT: u32 = 0x800;
    pub const AT_EMPTY_PATH: u32 = 0x1000;
    pub const AT_RECURSIVE: u32 = 0x8000;

    pub const MOVE_MOUNT_F_SYMLINKS: u32 = 0x01;
    pub const MOVE_MOUNT_F_AUTOMOUNTS: u32 = 0x02;
    pub const MOVE_MOUNT_F_EMPTY_PATH: u32 = 0x04;
    pub const MOVE_MOUNT_T_SYMLINKS: u32 = 0x10;
    pub const MOVE_MOUNT_T_AUTOMOUNTS: u32 = 0x20;
    pub const MOVE_MOUNT_T_EMPTY_PATH: u32 = 0x40;

    pub const FSOPEN_CLOEXEC: u32 = 0x01;
    pub const FSPICK_CLOEXEC: u32 = 0x01;
    pub const FSPICK_SYMLINK_NOFOLLOW: u32 = 0x02;
    pub const FSPICK_NO_AUTOMOUNT: u32 = 0x04;
    pub const FSPICK_EMPTY_PATH: u32 = 0x08;
    pub const FSMOUNT_CLOEXEC: u32 = 0x01;

    pub const FSCONFIG_SET_FLAG: u32 = 0;
    pub const FSCONFIG_SET_STRING: u32 = 1;
    pub const FSCONFIG_SET_BINARY: u32 = 2;
    pub const FSCONFIG_SET_PATH: u32 = 3;
    pub const FSCONFIG_SET_PATH_EMPTY: u32 = 4;
    pub const FSCONFIG_SET_FD: u32 = 5;
    pub const FSCONFIG_CMD_CREATE: u32 = 6;
    pub const FSCONFIG_CMD_RECONFIGURE: u32 = 7;
    pub const FSCONFIG_CMD_CREATE_EXCL: u32 = 8;

    pub const MOUNT_ATTR_RDONLY: u64 = 0x01;
    pub const MOUNT_ATTR_NOSUID: u64 = 0x02;
    pub const MOUNT_ATTR_NODEV: u64 = 0x04;
    pub const MOUNT_ATTR_NOEXEC: u64 = 0x08;
    pub const MOUNT_ATTR__ATIME: u64 = 0x70;
    pub const MOUNT_ATTR_RELATIME: u64 = 0x00;
    pub const MOUNT_ATTR_NOATIME: u64 = 0x10;
    pub const MOUNT_ATTR_STRICTATIME: u64 = 0x20;
    pub const MOUNT_ATTR_NODIRATIME: u64 = 0x80;
    pub const MOUNT_ATTR_IDMAP: u64 = 0x0010_0000;
    pub const MOUNT_ATTR_NOSYMFOLLOW: u64 = 0x0020_0000;
    /// `sizeof(struct mount_attr)`, first version.
    pub const MOUNT_ATTR_SIZE_VER0: usize = 32;

    /// `struct mnt_id_req` sizes.
    pub const MNT_ID_REQ_SIZE_VER0: u32 = 24;
    pub const MNT_ID_REQ_SIZE_VER1: u32 = 32;
    /// `listmount(2)`: the namespace's root.
    pub const LSMT_ROOT: u64 = u64::MAX;
    pub const LISTMOUNT_REVERSE: u32 = 1;

    pub const STATMOUNT_SB_BASIC: u64 = 0x1;
    pub const STATMOUNT_MNT_BASIC: u64 = 0x2;
    pub const STATMOUNT_PROPAGATE_FROM: u64 = 0x4;
    pub const STATMOUNT_MNT_ROOT: u64 = 0x8;
    pub const STATMOUNT_MNT_POINT: u64 = 0x10;
    pub const STATMOUNT_FS_TYPE: u64 = 0x20;
    pub const STATMOUNT_MNT_NS_ID: u64 = 0x40;
    pub const STATMOUNT_MNT_OPTS: u64 = 0x80;
    pub const STATMOUNT_FS_SUBTYPE: u64 = 0x100;
    pub const STATMOUNT_SB_SOURCE: u64 = 0x200;
    pub const STATMOUNT_OPT_ARRAY: u64 = 0x400;
    pub const STATMOUNT_OPT_SEC_ARRAY: u64 = 0x800;
    pub const STATMOUNT_SUPPORTED_MASK: u64 = 0x1000;
    pub const STATMOUNT_MNT_UIDMAP: u64 = 0x2000;
    pub const STATMOUNT_MNT_GIDMAP: u64 = 0x4000;

    /// `SB_RDONLY` (`linux/fs.h`'s superblock flags, as `statmount`'s
    /// `sb_flags` reports them).
    pub const SB_RDONLY: u32 = 1;

    /// Propagation types (`MS_*`).
    pub const MS_UNBINDABLE: u64 = 1 << 17;
    pub const MS_PRIVATE: u64 = 1 << 18;
    pub const MS_SLAVE: u64 = 1 << 19;
    pub const MS_SHARED: u64 = 1 << 20;
}

use uapi::*;

/// Where one of these calls names a file: a path from a directory
/// descriptor; an empty path means the descriptor itself (the caller
/// allowed that with its `*_EMPTY_PATH` flag). `follow` says whether a
/// final symbolic link is followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathAt {
    pub dirfd: i32,
    pub path: Vec<u8>,
    pub follow: bool,
}

// ---- mount ids ---------------------------------------------------------

/// The unique 64-bit mount ids (`statx` `STATX_MNT_ID_UNIQUE`,
/// `statmount`, `listmount`) start at 2^31 (`statmount(2)`); Poly's are
/// the mount's id above that.
pub const UNIQUE_ID_BASE: u64 = 1 << 31;

/// The unique id of mount `id`.
pub const fn unique_id(id: u64) -> u64 {
    UNIQUE_ID_BASE + id
}

/// The mount id a unique id names; `None` for ids below 2^31.
pub const fn mount_of_unique(unique: u64) -> Option<u64> {
    if unique > UNIQUE_ID_BASE { Some(unique - UNIQUE_ID_BASE) } else { None }
}

// ---- per-mount attributes ----------------------------------------------

/// Every `MOUNT_ATTR_*` bit.
pub const ATTR_KNOWN: u64 = MOUNT_ATTR_RDONLY
    | MOUNT_ATTR_NOSUID
    | MOUNT_ATTR_NODEV
    | MOUNT_ATTR_NOEXEC
    | MOUNT_ATTR__ATIME
    | MOUNT_ATTR_NODIRATIME
    | MOUNT_ATTR_IDMAP
    | MOUNT_ATTR_NOSYMFOLLOW;

/// The attributes outside the four [`MountFlags`] (the VFS keeps these as
/// a mount's extra attributes).
const ATTR_EXTRA: u64 = MOUNT_ATTR__ATIME | MOUNT_ATTR_NODIRATIME | MOUNT_ATTR_NOSYMFOLLOW;

/// A mount's attributes as `MOUNT_ATTR_*` bits, from its flags and extra
/// attributes.
pub fn join_attrs(flags: MountFlags, extra: u32) -> u64 {
    let mut a = u64::from(extra) & ATTR_EXTRA;
    for (on, bit) in [
        (flags.read_only, MOUNT_ATTR_RDONLY),
        (flags.no_suid, MOUNT_ATTR_NOSUID),
        (flags.no_dev, MOUNT_ATTR_NODEV),
        (flags.no_exec, MOUNT_ATTR_NOEXEC),
    ] {
        if on {
            a |= bit;
        }
    }
    a
}

/// The reverse of [`join_attrs`].
pub fn split_attrs(attrs: u64) -> (MountFlags, u32) {
    let flags = MountFlags {
        read_only: attrs & MOUNT_ATTR_RDONLY != 0,
        no_suid: attrs & MOUNT_ATTR_NOSUID != 0,
        no_dev: attrs & MOUNT_ATTR_NODEV != 0,
        no_exec: attrs & MOUNT_ATTR_NOEXEC != 0,
    };
    (flags, (attrs & ATTR_EXTRA) as u32)
}

/// `fsmount(2)`'s `attr_flags`: known bits only, no `MOUNT_ATTR_IDMAP`,
/// an access time mode of 0, `NOATIME` or `STRICTATIME` (`EINVAL`).
pub fn check_fsmount_attrs(attrs: u64) -> Result<(), Errno> {
    if attrs & !ATTR_KNOWN != 0 || attrs & MOUNT_ATTR_IDMAP != 0 || attrs & MOUNT_ATTR__ATIME > MOUNT_ATTR_STRICTATIME {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// What one `mount_setattr(2)` call changes (`struct mount_attr`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrChange {
    pub set: u64,
    pub clr: u64,
    pub propagation: u64,
}

impl AttrChange {
    /// `struct mount_attr` from the first 32 bytes the caller gave
    /// (`attr_set`, `attr_clr`, `propagation`, `userns_fd`).
    pub fn from_bytes(b: &[u8]) -> AttrChange {
        let word = |i: usize| {
            let mut w = [0u8; 8];
            w.copy_from_slice(&b[i * 8..i * 8 + 8]);
            u64::from_le_bytes(w)
        };
        AttrChange { set: word(0), clr: word(1), propagation: word(2) }
    }

    /// The checks of `mount_setattr(2)` on the attributes (`EINVAL`):
    /// unknown bits; `MOUNT_ATTR_IDMAP` (Poly has no idmapped mounts);
    /// an access time mode that is not 0, `NOATIME` or `STRICTATIME`, or
    /// that is set without clearing the whole field; a propagation that
    /// is not 0 or one type.
    pub fn check(&self) -> Result<(), Errno> {
        if (self.set | self.clr) & !ATTR_KNOWN != 0 || (self.set | self.clr) & MOUNT_ATTR_IDMAP != 0 {
            return Err(Errno::EINVAL);
        }
        let atime_set = self.set & MOUNT_ATTR__ATIME;
        let atime_clr = self.clr & MOUNT_ATTR__ATIME;
        if atime_set > MOUNT_ATTR_STRICTATIME || (atime_set != 0 && atime_clr != MOUNT_ATTR__ATIME) {
            return Err(Errno::EINVAL);
        }
        if atime_clr != 0 && atime_clr != MOUNT_ATTR__ATIME {
            return Err(Errno::EINVAL);
        }
        if !matches!(self.propagation, 0 | MS_PRIVATE | MS_SHARED | MS_SLAVE | MS_UNBINDABLE) {
            return Err(Errno::EINVAL);
        }
        Ok(())
    }

    /// Whether the call changes nothing.
    pub fn is_empty(&self) -> bool {
        self.set == 0 && self.clr == 0 && self.propagation == 0
    }

    /// The attributes of a mount that had `old`: `attr_clr` cleared, then
    /// `attr_set` set.
    pub fn apply(&self, old: u64) -> u64 {
        (old & !self.clr) | self.set
    }

    /// The propagation type a mount with `old` gets: Poly has no peers or
    /// masters, so a mount made a slave has no master and stays private
    /// (as a private mount made slave does on Linux).
    pub fn propagation_of(&self, old: u64) -> u64 {
        match self.propagation {
            0 => old,
            MS_SLAVE => MS_PRIVATE,
            p => p,
        }
    }
}

// ---- file system contexts ----------------------------------------------

/// The most messages a context keeps (older ones are dropped).
pub const LOG_MAX: usize = 8;

/// What a context was opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// `fsopen(2)`: a new superblock.
    Create,
    /// `fspick(2)`: reconfiguring a mounted file system.
    Reconfigure,
}

/// Where a context is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Collecting parameters.
    Open,
    /// `FSCONFIG_CMD_CREATE` made the superblock.
    Created,
    /// `fsmount(2)` mounted it.
    Mounted,
    /// A command failed: the context is unusable.
    Failed,
}

/// The value of one `fsconfig(2)` `FSCONFIG_SET_*` parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Flag,
    String(&'a [u8]),
    Binary(&'a [u8]),
    Path(&'a [u8]),
    Fd(i32),
}

/// A decoded `fsconfig(2)` command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    Set { key: &'a [u8], value: Value<'a> },
    Create,
    CreateExcl,
    Reconfigure,
}

/// What `FSCONFIG_CMD_CREATE` asks the kernel to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequest {
    pub fstype: FsType,
    /// The `source` parameter ("none" if none was given).
    pub source: Vec<u8>,
    /// The file system's parameters as `mount(2)` data (`size=1m,mode=750`).
    pub data: Vec<u8>,
    /// What the data sets ([`admin::mount_options`]).
    pub root: RootOptions,
    /// The `ro` flag.
    pub read_only: bool,
}

/// What a context's commands need from the kernel.
pub trait SuperblockOps {
    /// Make the superblock.
    fn create(&mut self, req: &CreateRequest) -> Result<(), Errno>;
    /// Apply `read_only` (`ro`/`rw`, if given) to the picked superblock.
    fn reconfigure(&mut self, read_only: Option<bool>) -> Result<(), Errno>;
}

/// A file system context: the state behind an `fsopen(2)` / `fspick(2)`
/// descriptor.
#[derive(Clone, Debug)]
pub struct FsContext {
    /// The type's name as the messages show it (`tmpfs`).
    name: Vec<u8>,
    fstype: Option<FsType>,
    purpose: Purpose,
    phase: Phase,
    source: Option<Vec<u8>>,
    options: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    read_only: Option<bool>,
    log: VecDeque<Vec<u8>>,
}

/// The parameters each file system type takes (the man pages' lists, as
/// [`admin::mount_options`] accepts them).
fn known_key(fs: FsType, key: &[u8]) -> bool {
    match fs {
        FsType::Tmpfs => matches!(
            key,
            b"mode"
                | b"uid"
                | b"gid"
                | b"size"
                | b"nr_blocks"
                | b"nr_inodes"
                | b"huge"
                | b"mpol"
                | b"noswap"
                | b"inode32"
                | b"inode64"
        ),
        FsType::Proc => matches!(key, b"hidepid" | b"gid" | b"subset"),
        FsType::Devpts => matches!(key, b"uid" | b"gid" | b"max" | b"mode" | b"ptmxmode" | b"newinstance"),
        FsType::Sysfs => false,
        FsType::Ext2 => true,
    }
}

impl FsContext {
    /// `fsopen(2)` of the type `name` (`ENODEV` if there is no such type).
    pub fn new(name: &[u8]) -> Result<FsContext, Errno> {
        let fstype = FsType::from_name(name)?;
        Ok(FsContext::make(name, Some(fstype), Purpose::Create))
    }

    /// `fspick(2)` of a mounted file system of type `name`.
    pub fn pick(name: &[u8]) -> FsContext {
        FsContext::make(name, FsType::from_name(name).ok(), Purpose::Reconfigure)
    }

    fn make(name: &[u8], fstype: Option<FsType>, purpose: Purpose) -> FsContext {
        FsContext {
            name: name.to_vec(),
            fstype,
            purpose,
            phase: Phase::Open,
            source: None,
            options: Vec::new(),
            read_only: None,
            log: VecDeque::new(),
        }
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Queue a message (oldest dropped beyond [`LOG_MAX`]).
    fn log(&mut self, text: &[u8]) {
        if self.log.len() == LOG_MAX {
            self.log.pop_front();
        }
        self.log.push_back(text.to_vec());
    }

    /// An error message about parameter `key`, and `EINVAL`.
    fn reject(&mut self, what: &str, key: &[u8]) -> Errno {
        let text = format!(
            "e {}: {} '{}'\n",
            core::str::from_utf8(&self.name).unwrap_or("?"),
            what,
            core::str::from_utf8(key).unwrap_or("?")
        );
        self.log(text.as_bytes());
        Errno::EINVAL
    }

    /// `read(2)` on the context: the oldest message if it fits in `room`
    /// bytes (`EMSGSIZE` otherwise, the message stays); `ENODATA` with
    /// none queued.
    pub fn read_log(&mut self, room: usize) -> Result<Vec<u8>, Errno> {
        let next = self.log.front().ok_or(Errno::ENODATA)?;
        if next.len() > room {
            return Err(Errno::EMSGSIZE);
        }
        Ok(self.log.pop_front().unwrap_or_default())
    }

    /// Whether a message is queued.
    pub fn has_messages(&self) -> bool {
        !self.log.is_empty()
    }

    /// `fsconfig(2)`: one command on the context.
    pub fn config(&mut self, cmd: &Command<'_>, ops: &mut dyn SuperblockOps) -> Result<(), Errno> {
        if self.phase != Phase::Open {
            return Err(Errno::EBUSY);
        }
        match *cmd {
            Command::Set { key, value } => self.set(key, value),
            Command::Create | Command::CreateExcl => {
                if self.purpose != Purpose::Create {
                    return Err(Errno::EBUSY);
                }
                let fstype = self.fstype.ok_or(Errno::EINVAL)?;
                let data = self.data();
                let root = admin::mount_options(fstype, &data)?;
                let req = CreateRequest {
                    fstype,
                    source: self.source.clone().unwrap_or_else(|| b"none".to_vec()),
                    data,
                    root,
                    read_only: self.read_only == Some(true),
                };
                match ops.create(&req) {
                    Ok(()) => {
                        self.phase = Phase::Created;
                        Ok(())
                    }
                    Err(e) => {
                        self.phase = Phase::Failed;
                        Err(e)
                    }
                }
            }
            Command::Reconfigure => {
                if self.purpose != Purpose::Reconfigure {
                    return Err(Errno::EBUSY);
                }
                match ops.reconfigure(self.read_only.take()) {
                    Ok(()) => {
                        self.options.clear();
                        Ok(())
                    }
                    Err(e) => {
                        self.phase = Phase::Failed;
                        Err(e)
                    }
                }
            }
        }
    }

    /// One parameter: the generic ones (`source`, `ro`, `rw`), then the
    /// file system's own. A rejected parameter logs why and leaves the
    /// context usable.
    fn set(&mut self, key: &[u8], value: Value<'_>) -> Result<(), Errno> {
        match (key, value) {
            (b"source", Value::String(s) | Value::Path(s)) => {
                if self.source.is_some() {
                    self.log(b"e Multiple sources\n");
                    return Err(Errno::EINVAL);
                }
                self.source = Some(s.to_vec());
                return Ok(());
            }
            (b"source", _) => return Err(self.reject("Bad value for", key)),
            (b"ro" | b"rw", Value::Flag) => {
                self.read_only = Some(key == b"ro");
                return Ok(());
            }
            (b"ro" | b"rw", _) => return Err(self.reject("Bad value for", key)),
            _ => {}
        }
        let Some(fstype) = self.fstype.filter(|&f| known_key(f, key)) else {
            return Err(self.reject("Unknown parameter", key));
        };
        let value = match value {
            Value::Flag => None,
            Value::String(s) => Some(s.to_vec()),
            _ => return Err(self.reject("Bad value for", key)),
        };
        let mut one = key.to_vec();
        if let Some(v) = &value {
            one.push(b'=');
            one.extend_from_slice(v);
        }
        if one.contains(&b',') || admin::mount_options(fstype, &one).is_err() {
            return Err(self.reject("Bad value for", key));
        }
        self.options.retain(|(k, _)| k.as_slice() != key);
        self.options.push((key.to_vec(), value));
        Ok(())
    }

    /// The parameters given so far as `mount(2)` data.
    fn data(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in &self.options {
            if !out.is_empty() {
                out.push(b',');
            }
            out.extend_from_slice(k);
            if let Some(v) = v {
                out.push(b'=');
                out.extend_from_slice(v);
            }
        }
        out
    }

    /// `fsmount(2)` takes the superblock: `EINVAL` before `CMD_CREATE`
    /// (or after it failed), `EBUSY` once mounted and for a picked
    /// context.
    pub fn begin_mount(&mut self) -> Result<(), Errno> {
        match (self.purpose, self.phase) {
            (Purpose::Reconfigure, _) | (_, Phase::Mounted) => Err(Errno::EBUSY),
            (Purpose::Create, Phase::Created) => {
                self.phase = Phase::Mounted;
                Ok(())
            }
            _ => Err(Errno::EINVAL),
        }
    }
}

/// The options a file system shows (`/proc/self/mountinfo`'s super
/// options without `rw`/`ro`, `statmount(2)`'s `mnt_opts`) after a mount
/// with `root`: for tmpfs the size limit in KiB (whole pages), the inode
/// limit, a root mode other than the default 1777 (octal), and owner ids
/// other than 0 (`size=1024k,mode=750`); nothing for the others.
pub fn options_string(fs: FsType, root: &RootOptions) -> Vec<u8> {
    let mut parts: Vec<alloc::string::String> = Vec::new();
    if fs == FsType::Tmpfs {
        if let Some(Amount::Count(bytes)) = root.size
            && bytes > 0
        {
            parts.push(format!("size={}k", bytes.div_ceil(4096) * 4));
        }
        if let Some(Amount::Count(n)) = root.inodes
            && n > 0
        {
            parts.push(format!("nr_inodes={n}"));
        }
        if let Some(mode) = root.mode
            && mode != 0o1777
        {
            parts.push(format!("mode={mode:03o}"));
        }
        if let Some(uid) = root.uid.filter(|&u| u != 0) {
            parts.push(format!("uid={uid}"));
        }
        if let Some(gid) = root.gid.filter(|&g| g != 0) {
            parts.push(format!("gid={gid}"));
        }
    }
    parts.join(",").into_bytes()
}

// ---- statmount, listmount ----------------------------------------------

/// `struct mnt_id_req` (`linux/mount.h`): `u32 size, spare; u64 mnt_id,
/// param, mnt_ns_id`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MntIdReq {
    pub size: u32,
    pub spare: u32,
    pub mnt_id: u64,
    pub param: u64,
    pub mnt_ns_id: u64,
}

impl MntIdReq {
    /// The fields from the first 24 bytes (and `mnt_ns_id` from 32 if
    /// `b` has them).
    pub fn from_bytes(b: &[u8]) -> MntIdReq {
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let u64_at = |i: usize| {
            let mut w = [0u8; 8];
            w.copy_from_slice(&b[i..i + 8]);
            u64::from_le_bytes(w)
        };
        MntIdReq {
            size: u32_at(0),
            spare: u32_at(4),
            mnt_id: u64_at(8),
            param: u64_at(16),
            mnt_ns_id: if b.len() >= 32 { u64_at(24) } else { 0 },
        }
    }

    /// The checks both calls make (`EINVAL`): a known size, the spare
    /// word 0, a mount id (0 names none).
    pub fn check(&self) -> Result<(), Errno> {
        if self.size != MNT_ID_REQ_SIZE_VER0 && self.size != MNT_ID_REQ_SIZE_VER1 {
            return Err(Errno::EINVAL);
        }
        if self.spare != 0 || self.mnt_id == 0 {
            return Err(Errno::EINVAL);
        }
        Ok(())
    }
}

/// What the kernel knows about one mount for `statmount(2)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountDesc {
    pub sb_dev_major: u32,
    pub sb_dev_minor: u32,
    pub sb_magic: u64,
    pub sb_read_only: bool,
    pub fs_type: Vec<u8>,
    /// Poly's mount ids (the legacy ids); the unique ids are derived.
    pub mnt_id: u64,
    pub parent_id: u64,
    pub flags: MountFlags,
    pub extra_attrs: u32,
    pub propagation: u64,
    pub peer_group: u64,
    pub mnt_root: Vec<u8>,
    pub mnt_point: Vec<u8>,
    pub ns_id: u64,
    pub source: Vec<u8>,
    pub options: Vec<u8>,
}

/// `sizeof(struct statmount)` (`linux/mount.h`): the fixed part before
/// the string area.
pub const STATMOUNT_SIZE: usize = 512;

/// The `STATMOUNT_*` parts Poly fills.
pub const STATMOUNT_SUPPORTED: u64 = STATMOUNT_SB_BASIC
    | STATMOUNT_MNT_BASIC
    | STATMOUNT_PROPAGATE_FROM
    | STATMOUNT_MNT_ROOT
    | STATMOUNT_MNT_POINT
    | STATMOUNT_FS_TYPE
    | STATMOUNT_MNT_NS_ID
    | STATMOUNT_MNT_OPTS
    | STATMOUNT_SB_SOURCE
    | STATMOUNT_OPT_ARRAY
    | STATMOUNT_OPT_SEC_ARRAY
    | STATMOUNT_SUPPORTED_MASK;

/// `struct statmount` for `d` with the parts `mask` asks for (unknown and
/// unsupported bits ignored): the fixed part, then the strings, each
/// NUL-terminated, at the offsets the fixed part gives; `size` is the
/// bytes used. `EOVERFLOW` if that is more than `bufsize`.
pub fn encode_statmount(d: &MountDesc, mask: u64, bufsize: usize) -> Result<Vec<u8>, Errno> {
    let want = mask & STATMOUNT_SUPPORTED;
    let mut b = alloc::vec![0u8; STATMOUNT_SIZE];
    let mut strings: Vec<u8> = Vec::new();
    fn push_str(strings: &mut Vec<u8>, s: &[u8]) -> u32 {
        let at = strings.len() as u32;
        strings.extend_from_slice(s);
        strings.push(0);
        at
    }
    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(b: &mut [u8], at: usize, v: u64) {
        b[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    if want & STATMOUNT_SB_BASIC != 0 {
        put32(&mut b, 16, d.sb_dev_major);
        put32(&mut b, 20, d.sb_dev_minor);
        put64(&mut b, 24, d.sb_magic);
        put32(&mut b, 32, if d.sb_read_only { SB_RDONLY } else { 0 });
    }
    if want & STATMOUNT_FS_TYPE != 0 {
        let at = push_str(&mut strings, &d.fs_type);
        put32(&mut b, 36, at);
    }
    if want & STATMOUNT_MNT_BASIC != 0 {
        put64(&mut b, 40, unique_id(d.mnt_id));
        put64(&mut b, 48, unique_id(d.parent_id));
        put32(&mut b, 56, d.mnt_id as u32);
        put32(&mut b, 60, d.parent_id as u32);
        put64(&mut b, 64, join_attrs(d.flags, d.extra_attrs));
        put64(&mut b, 72, d.propagation);
        put64(&mut b, 80, d.peer_group);
        put64(&mut b, 88, 0);
    }
    if want & STATMOUNT_PROPAGATE_FROM != 0 {
        put64(&mut b, 96, 0);
    }
    if want & STATMOUNT_MNT_ROOT != 0 {
        let at = push_str(&mut strings, &d.mnt_root);
        put32(&mut b, 104, at);
    }
    if want & STATMOUNT_MNT_POINT != 0 {
        let at = push_str(&mut strings, &d.mnt_point);
        put32(&mut b, 108, at);
    }
    if want & STATMOUNT_MNT_NS_ID != 0 {
        put64(&mut b, 112, d.ns_id);
    }
    if want & STATMOUNT_SB_SOURCE != 0 {
        let at = push_str(&mut strings, &d.source);
        put32(&mut b, 124, at);
    }
    if want & STATMOUNT_MNT_OPTS != 0 {
        let at = push_str(&mut strings, &d.options);
        put32(&mut b, 4, at);
    }
    if want & STATMOUNT_OPT_ARRAY != 0 {
        let opts: Vec<&[u8]> = d.options.split(|&c| c == b',').filter(|o| !o.is_empty()).collect();
        let at = strings.len() as u32;
        for o in &opts {
            push_str(&mut strings, o);
        }
        put32(&mut b, 128, opts.len() as u32);
        put32(&mut b, 132, at);
    }
    if want & STATMOUNT_OPT_SEC_ARRAY != 0 {
        put32(&mut b, 136, 0);
        put32(&mut b, 140, strings.len() as u32);
    }
    if want & STATMOUNT_SUPPORTED_MASK != 0 {
        put64(&mut b, 144, STATMOUNT_SUPPORTED);
    }
    let size = STATMOUNT_SIZE + strings.len();
    if size > bufsize {
        return Err(Errno::EOVERFLOW);
    }
    put32(&mut b, 0, size as u32);
    put64(&mut b, 8, want);
    b.extend_from_slice(&strings);
    Ok(b)
}

/// One mount of the caller's namespace, as `listmount(2)` sees it: its id
/// and the id of the mount it is on (`None` for the namespace's root).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountLink {
    pub id: u64,
    pub parent: Option<u64>,
}

/// The most ids one `listmount(2)` may ask for: larger counts are
/// `EOVERFLOW` (the array could not be written in one go:
/// `MAX_RW_COUNT`, `INT_MAX` rounded down to a page, of `u64`s).
pub const LISTMOUNT_MAX: u64 = 0x7fff_f000 / 8;

/// `listmount(2)`'s answer: the unique ids of the mounts below `req`'s
/// mount (at any depth; for `LSMT_ROOT` the namespace's root and every
/// other mount), ascending or descending (`reverse`), after `req.param`
/// if it is not 0, at most `nr`. `links` is the namespace, its root
/// first. `ENOENT` for a mount that is not in it.
pub fn list_mounts(links: &[MountLink], req: &MntIdReq, reverse: bool, nr: usize) -> Result<Vec<u64>, Errno> {
    let mut ids: Vec<u64> = if req.mnt_id == LSMT_ROOT {
        links.iter().map(|l| l.id).collect()
    } else {
        let top = mount_of_unique(req.mnt_id).ok_or(Errno::ENOENT)?;
        if !links.iter().any(|l| l.id == top) {
            return Err(Errno::ENOENT);
        }
        let mut below = alloc::vec![top];
        let mut i = 0;
        while i < below.len() {
            let p = below[i];
            below.extend(links.iter().filter(|l| l.parent == Some(p)).map(|l| l.id));
            i += 1;
        }
        below.retain(|&id| id != top);
        below
    };
    ids.sort_unstable();
    if reverse {
        ids.reverse();
    }
    let mut out: Vec<u64> = ids
        .into_iter()
        .map(unique_id)
        .filter(|&u| match (req.param, reverse) {
            (0, _) => true,
            (after, false) => u > after,
            (after, true) => u < after,
        })
        .collect();
    out.truncate(nr);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    struct Fake {
        created: Option<CreateRequest>,
        fail: Option<Errno>,
        reconfigured: Vec<Option<bool>>,
    }

    impl SuperblockOps for Fake {
        fn create(&mut self, req: &CreateRequest) -> Result<(), Errno> {
            if let Some(e) = self.fail {
                return Err(e);
            }
            self.created = Some(req.clone());
            Ok(())
        }
        fn reconfigure(&mut self, read_only: Option<bool>) -> Result<(), Errno> {
            if let Some(e) = self.fail {
                return Err(e);
            }
            self.reconfigured.push(read_only);
            Ok(())
        }
    }

    fn fake() -> Fake {
        Fake { created: None, fail: None, reconfigured: Vec::new() }
    }

    fn set<'a>(key: &'a [u8], value: Value<'a>) -> Command<'a> {
        Command::Set { key, value }
    }

    #[test]
    fn context_logs_rejected_parameters_in_order() {
        let mut ops = fake();
        let mut c = FsContext::new(b"tmpfs").unwrap();
        assert_eq!(c.read_log(256), Err(Errno::ENODATA));
        assert_eq!(c.config(&set(b"bogus", Value::Flag), &mut ops), Err(Errno::EINVAL));
        assert_eq!(c.config(&set(b"size", Value::String(b"junk")), &mut ops), Err(Errno::EINVAL));
        assert_eq!(c.read_log(4), Err(Errno::EMSGSIZE));
        assert_eq!(c.read_log(256).unwrap(), b"e tmpfs: Unknown parameter 'bogus'\n");
        assert_eq!(c.read_log(256).unwrap(), b"e tmpfs: Bad value for 'size'\n");
        assert_eq!(c.read_log(256), Err(Errno::ENODATA));
        assert_eq!(c.config(&set(b"size", Value::String(b"1m")), &mut ops), Ok(()));
        assert!(!c.has_messages());
        assert_eq!(FsContext::new(b"nosuchfs").unwrap_err(), Errno::ENODEV);
        assert_eq!(FsContext::new(b"").unwrap_err(), Errno::ENODEV);
    }

    #[test]
    fn context_create_then_busy() {
        let mut ops = fake();
        let mut c = FsContext::new(b"tmpfs").unwrap();
        assert_eq!(c.config(&Command::Reconfigure, &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.begin_mount(), Err(Errno::EINVAL));
        c.config(&set(b"size", Value::String(b"1m")), &mut ops).unwrap();
        c.config(&set(b"mode", Value::String(b"0750")), &mut ops).unwrap();
        c.config(&set(b"source", Value::String(b"e_src")), &mut ops).unwrap();
        assert_eq!(c.config(&set(b"source", Value::String(b"again")), &mut ops), Err(Errno::EINVAL));
        assert_eq!(c.read_log(64).unwrap(), b"e Multiple sources\n");
        assert_eq!(c.config(&Command::Create, &mut ops), Ok(()));
        let req = ops.created.clone().unwrap();
        assert_eq!(req.source, b"e_src");
        assert_eq!(req.data, b"size=1m,mode=0750");
        assert_eq!(options_string(FsType::Tmpfs, &req.root), b"size=1024k,mode=750");
        assert_eq!(c.config(&Command::Create, &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.config(&set(b"size", Value::String(b"2m")), &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.config(&Command::Reconfigure, &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.begin_mount(), Ok(()));
        assert_eq!(c.begin_mount(), Err(Errno::EBUSY));
    }

    #[test]
    fn failed_command_spoils_the_context() {
        let mut ops = fake();
        let mut c = FsContext::pick(b"tmpfs");
        assert_eq!(c.config(&Command::Create, &mut ops), Err(Errno::EBUSY));
        c.config(&set(b"ro", Value::Flag), &mut ops).unwrap();
        c.config(&Command::Reconfigure, &mut ops).unwrap();
        assert_eq!(ops.reconfigured, vec![Some(true)]);
        ops.fail = Some(Errno::EBUSY);
        c.config(&set(b"ro", Value::Flag), &mut ops).unwrap();
        assert_eq!(c.config(&Command::Reconfigure, &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.config(&set(b"rw", Value::Flag), &mut ops), Err(Errno::EBUSY));
        assert_eq!(c.begin_mount(), Err(Errno::EBUSY));
    }

    #[test]
    fn attribute_changes() {
        let a = AttrChange { set: MOUNT_ATTR_NOATIME, clr: MOUNT_ATTR__ATIME, propagation: 0 };
        assert_eq!(a.check(), Ok(()));
        assert_eq!(a.apply(MOUNT_ATTR_STRICTATIME | MOUNT_ATTR_RDONLY), MOUNT_ATTR_NOATIME | MOUNT_ATTR_RDONLY);
        assert_eq!(AttrChange { set: MOUNT_ATTR_NOATIME, ..Default::default() }.check(), Err(Errno::EINVAL));
        assert_eq!(AttrChange { set: 0x30, clr: 0x70, propagation: 0 }.check(), Err(Errno::EINVAL));
        assert_eq!(AttrChange { set: 1 << 30, ..Default::default() }.check(), Err(Errno::EINVAL));
        assert_eq!(AttrChange { propagation: 12345, ..Default::default() }.check(), Err(Errno::EINVAL));
        let slave = AttrChange { propagation: MS_SLAVE, ..Default::default() };
        assert_eq!(slave.propagation_of(MS_SHARED), MS_PRIVATE);
        let (flags, extra) = split_attrs(MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NODIRATIME);
        assert!(flags.no_suid && flags.no_dev && !flags.read_only);
        assert_eq!(join_attrs(flags, extra), MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NODIRATIME);
        assert_eq!(check_fsmount_attrs(0x30), Err(Errno::EINVAL));
        assert_eq!(check_fsmount_attrs(MOUNT_ATTR_IDMAP), Err(Errno::EINVAL));
        assert_eq!(check_fsmount_attrs(MOUNT_ATTR_RDONLY | MOUNT_ATTR_NOATIME), Ok(()));
    }

    #[test]
    fn statmount_layout() {
        let d = MountDesc {
            sb_magic: 0x0102_1994,
            fs_type: b"tmpfs".to_vec(),
            mnt_id: 7,
            parent_id: 3,
            flags: MountFlags { no_suid: true, no_dev: true, ..MountFlags::default() },
            propagation: MS_PRIVATE,
            mnt_root: b"/".to_vec(),
            mnt_point: b"/tmp/x".to_vec(),
            source: b"src".to_vec(),
            options: b"size=1024k,mode=755".to_vec(),
            ..MountDesc::default()
        };
        let all = STATMOUNT_SB_BASIC | STATMOUNT_MNT_BASIC | STATMOUNT_FS_TYPE | STATMOUNT_MNT_POINT;
        let b = encode_statmount(&d, all | 0x8000_0000, 4096).unwrap();
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        assert_eq!(u32_at(0) as usize, b.len());
        assert_eq!(u64_at(8), all, "unknown bits are not reported");
        assert_eq!(u64_at(24), 0x0102_1994);
        assert_eq!(u64_at(40), unique_id(7));
        assert_eq!(u64_at(48), unique_id(3));
        assert_eq!(u64_at(64), MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV);
        assert_eq!(u64_at(72), MS_PRIVATE);
        let s = |off: u32| {
            let at = STATMOUNT_SIZE + off as usize;
            let end = b[at..].iter().position(|&c| c == 0).unwrap();
            b[at..at + end].to_vec()
        };
        assert_eq!(s(u32_at(36)), b"tmpfs");
        assert_eq!(s(u32_at(108)), b"/tmp/x");
        assert_eq!(encode_statmount(&d, all, STATMOUNT_SIZE), Err(Errno::EOVERFLOW));
        let only = encode_statmount(&d, STATMOUNT_FS_TYPE, 4096).unwrap();
        assert_eq!(&only[STATMOUNT_SIZE..], b"tmpfs\0");
        assert_eq!(u64::from_le_bytes(only[24..32].try_into().unwrap()), 0);
    }

    #[test]
    fn listmount_selection() {
        // root 1, /tmp 2, D 10 with a 11, b 12, c 13 and a/deep 14.
        let l = |id, parent| MountLink { id, parent };
        let links = [
            l(1, None),
            l(2, Some(1)),
            l(10, Some(2)),
            l(11, Some(10)),
            l(12, Some(10)),
            l(14, Some(11)),
            l(13, Some(10)),
        ];
        let req = |id: u64, param: u64| MntIdReq { size: 24, mnt_id: id, param, ..MntIdReq::default() };
        let u = unique_id;
        assert_eq!(list_mounts(&links, &req(u(10), 0), false, 8).unwrap(), vec![u(11), u(12), u(13), u(14)]);
        assert_eq!(list_mounts(&links, &req(u(10), 0), true, 8).unwrap(), vec![u(14), u(13), u(12), u(11)]);
        assert_eq!(list_mounts(&links, &req(u(10), u(11)), false, 8).unwrap(), vec![u(12), u(13), u(14)]);
        assert_eq!(list_mounts(&links, &req(u(10), u(13)), true, 8).unwrap(), vec![u(12), u(11)]);
        assert_eq!(list_mounts(&links, &req(u(10), 0), false, 2).unwrap(), vec![u(11), u(12)]);
        assert_eq!(list_mounts(&links, &req(u(11), 0), false, 8).unwrap(), vec![u(14)]);
        assert!(list_mounts(&links, &req(u(12), 0), false, 8).unwrap().is_empty());
        assert_eq!(list_mounts(&links, &req(LSMT_ROOT, 0), false, 8).unwrap()[0], u(1));
        assert_eq!(list_mounts(&links, &req(u(99), 0), false, 8), Err(Errno::ENOENT));
        assert_eq!(MntIdReq { size: 16, mnt_id: 1, ..MntIdReq::default() }.check(), Err(Errno::EINVAL));
        assert_eq!(MntIdReq { size: 24, spare: 1, mnt_id: 1, ..MntIdReq::default() }.check(), Err(Errno::EINVAL));
        assert_eq!(MntIdReq { size: 32, mnt_id: 0, ..MntIdReq::default() }.check(), Err(Errno::EINVAL));
    }
}
