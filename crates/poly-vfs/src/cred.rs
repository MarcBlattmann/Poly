//! Credentials and the discretionary access check.
//!
//! The rules are POSIX's and Linux's (`path_resolution(7)`, "Permissions"
//! and "Capabilities"): the owner class applies if the caller owns the
//! file, else the group class if the file's group is the caller's group or
//! one of its supplementary groups, else the other class; only that one
//! class is consulted. Privilege is a set of capabilities
//! (`capabilities(7)`), not a user id: `CAP_DAC_OVERRIDE` may read and
//! write anything, search any directory and execute a file that has at
//! least one execute bit set; `CAP_DAC_READ_SEARCH` may read any file and
//! read and search any directory. The other capabilities the VFS checks
//! are listed at [`cap`].

use crate::{Attr, Errno, FileType, Result};
use alloc::sync::Arc;

/// Capability numbers the VFS checks (Linux UAPI `linux/capability.h`).
/// A personality without capabilities gives its superuser
/// [`CapSet::ALL`] and everyone else [`CapSet::EMPTY`].
pub mod cap {
    /// Change the owner of a file, and its group to any group (chown(2)).
    pub const CHOWN: u32 = 0;
    /// Bypass read, write and execute checks (execute needs one `x` bit).
    pub const DAC_OVERRIDE: u32 = 1;
    /// Bypass read checks on files and read and search checks on
    /// directories.
    pub const DAC_READ_SEARCH: u32 = 2;
    /// Act as the owner: chmod, utimensat with explicit times, the sticky
    /// bit, `O_NOATIME`.
    pub const FOWNER: u32 = 3;
    /// Keep set-user-ID and set-group-ID bits where they would be cleared.
    pub const FSETID: u32 = 4;
    /// Set and clear the append-only and immutable inode flags
    /// ([`crate::fileattr`]).
    pub const LINUX_IMMUTABLE: u32 = 9;
    /// Create device nodes (mknod(2)).
    pub const MKNOD: u32 = 27;
    /// chroot(2).
    pub const SYS_CHROOT: u32 = 18;
    /// Mount and unmount file systems.
    pub const SYS_ADMIN: u32 = 21;
    /// The highest capability number Linux defines (`CAP_LAST_CAP`, which
    /// is `CAP_CHECKPOINT_RESTORE`).
    pub const LAST: u32 = 40;
}

/// A set of capabilities: bit `n` is capability `n` ([`cap`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CapSet(pub u64);

impl CapSet {
    pub const EMPTY: CapSet = CapSet(0);
    /// Every capability up to [`cap::LAST`].
    pub const ALL: CapSet = CapSet((1 << (cap::LAST + 1)) - 1);

    /// The set with only `cap`.
    pub const fn only(cap: u32) -> CapSet {
        CapSet(1 << cap)
    }

    pub const fn contains(self, cap: u32) -> bool {
        cap <= cap::LAST && self.0 & (1 << cap) != 0
    }

    pub const fn without(self, cap: u32) -> CapSet {
        CapSet(self.0 & !(1 << cap))
    }

    pub const fn with(self, cap: u32) -> CapSet {
        CapSet(self.0 | (1 << cap))
    }
}

/// Who asks: the ids and capabilities a permission check uses. For Linux
/// these are the file system ids (normally the effective ids), the
/// supplementary groups and the effective capabilities; `access(2)` checks
/// with the real ids, so its caller builds a `Cred` from those instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cred {
    pub uid: u32,
    pub gid: u32,
    /// Supplementary groups.
    pub groups: Arc<[u32]>,
    /// Effective capabilities.
    pub caps: CapSet,
}

impl Default for Cred {
    /// The superuser.
    fn default() -> Cred {
        Cred::root()
    }
}

impl Cred {
    /// The superuser: ids 0 and every capability.
    pub fn root() -> Cred {
        Cred { uid: 0, gid: 0, groups: Arc::from([]), caps: CapSet::ALL }
    }

    /// An ordinary user with no supplementary groups and no capabilities.
    pub fn user(uid: u32, gid: u32) -> Cred {
        Cred { uid, gid, groups: Arc::from([]), caps: CapSet::EMPTY }
    }

    pub fn with_groups(mut self, groups: &[u32]) -> Cred {
        self.groups = Arc::from(groups);
        self
    }

    pub fn with_caps(mut self, caps: CapSet) -> Cred {
        self.caps = caps;
        self
    }

    /// Whether the caller has capability `cap` ([`cap`]).
    pub fn has(&self, cap: u32) -> bool {
        self.caps.contains(cap)
    }

    /// Whether the caller is user 0 with every capability: the kernel
    /// itself, or a Linux root that dropped nothing. Permission decisions
    /// use [`Cred::has`].
    pub fn is_root(&self) -> bool {
        self.uid == 0 && self.caps == CapSet::ALL
    }

    /// Whether `gid` is the caller's group or one of its supplementary groups.
    pub fn in_group(&self, gid: u32) -> bool {
        self.gid == gid || self.groups.contains(&gid)
    }

    /// Whether this caller may access a node with these attributes in the
    /// ways `want` asks for.
    pub fn permits(&self, attr: &Attr, want: Access) -> bool {
        let class = if self.uid == attr.uid {
            attr.perm >> 6
        } else if self.in_group(attr.gid) {
            attr.perm >> 3
        } else {
            attr.perm
        } & 0o7;
        let mut missing = want.0 & !(class as u8);
        if missing == 0 {
            return true;
        }
        let dir = attr.kind == FileType::Directory;
        if self.has(cap::DAC_READ_SEARCH) {
            // Read anything; search directories.
            missing &= !Access::READ.0;
            if dir {
                missing &= !Access::EXEC.0;
            }
        }
        if self.has(cap::DAC_OVERRIDE) {
            missing &= !(Access::READ.0 | Access::WRITE.0);
            // Execute needs some execute bit on anything but a directory.
            if dir || attr.perm & 0o111 != 0 {
                missing &= !Access::EXEC.0;
            }
        }
        missing == 0
    }

    /// [`Cred::permits`] as a result: `EACCES` when denied.
    pub fn check(&self, attr: &Attr, want: Access) -> Result<()> {
        if self.permits(attr, want) { Ok(()) } else { Err(Errno::EACCES) }
    }

    /// Whether the caller owns the node or may act as its owner
    /// (`CAP_FOWNER`): what `chmod`, `utimensat` with explicit times,
    /// `O_NOATIME` and the sticky-bit rule ask for.
    pub fn owns(&self, attr: &Attr) -> bool {
        self.uid == attr.uid || self.has(cap::FOWNER)
    }
}

/// A set of access kinds, with `access(2)`'s values (`R_OK` 4, `W_OK` 2,
/// `X_OK` 1), which are also the bit order of a permission class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access(u8);

impl Access {
    /// Existence only (`F_OK`).
    pub const NONE: Access = Access(0);
    pub const READ: Access = Access(4);
    pub const WRITE: Access = Access(2);
    /// Execute a file, or search a directory.
    pub const EXEC: Access = Access(1);

    /// From `access(2)`'s mode argument; `EINVAL` for unknown bits.
    pub fn from_bits(bits: u32) -> Result<Access> {
        if bits & !7 != 0 { Err(Errno::EINVAL) } else { Ok(Access(bits as u8)) }
    }

    pub const fn bits(self) -> u32 {
        self.0 as u32
    }

    pub const fn contains(self, other: Access) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Access) -> Access {
        Access(self.0 | other.0)
    }
}

impl core::ops::BitOr for Access {
    type Output = Access;

    fn bitor(self, rhs: Access) -> Access {
        self.union(rhs)
    }
}
