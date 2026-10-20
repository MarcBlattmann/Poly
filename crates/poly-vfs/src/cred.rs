//! Credentials and the discretionary access check.
//!
//! The rules are POSIX's (`path_resolution(7)`, "Permissions"): the owner
//! class applies if the caller owns the file, else the group class if the
//! file's group is the caller's group or one of its supplementary groups,
//! else the other class; only that one class is consulted. User id 0 stands
//! for the privileged process (`CAP_DAC_OVERRIDE`): it may read and write
//! anything, search any directory, and execute a file that has at least one
//! execute bit set.

use crate::{Attr, Errno, FileType, Result};
use alloc::vec::Vec;

/// The ids a process uses for file access. For Linux these are the file
/// system ids (normally the effective ids); `access(2)` checks with the real
/// ids, so its caller builds a `Cred` from those instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cred {
    pub uid: u32,
    pub gid: u32,
    /// Supplementary groups.
    pub groups: Vec<u32>,
}

impl Cred {
    /// The superuser.
    pub fn root() -> Cred {
        Cred { uid: 0, gid: 0, groups: Vec::new() }
    }

    /// An ordinary user with no supplementary groups.
    pub fn user(uid: u32, gid: u32) -> Cred {
        Cred { uid, gid, groups: Vec::new() }
    }

    pub fn with_groups(mut self, groups: &[u32]) -> Cred {
        self.groups = groups.to_vec();
        self
    }

    /// Whether permission checks are bypassed.
    pub fn is_root(&self) -> bool {
        self.uid == 0
    }

    /// Whether `gid` is the caller's group or one of its supplementary groups.
    pub fn in_group(&self, gid: u32) -> bool {
        self.gid == gid || self.groups.contains(&gid)
    }

    /// Whether this caller may access a node with these attributes in the
    /// ways `want` asks for.
    pub fn permits(&self, attr: &Attr, want: Access) -> bool {
        if self.is_root() {
            // Execute needs some execute bit on anything but a directory.
            return !want.contains(Access::EXEC) || attr.kind == FileType::Directory || attr.perm & 0o111 != 0;
        }
        let class = if self.uid == attr.uid {
            attr.perm >> 6
        } else if self.in_group(attr.gid) {
            attr.perm >> 3
        } else {
            attr.perm
        } & 0o7;
        want.0 & !(class as u8) == 0
    }

    /// [`Cred::permits`] as a result: `EACCES` when denied.
    pub fn check(&self, attr: &Attr, want: Access) -> Result<()> {
        if self.permits(attr, want) { Ok(()) } else { Err(Errno::EACCES) }
    }

    /// Whether the caller owns the node or is root: what `chmod`, `utimes`
    /// with explicit times and the sticky-bit rule ask for.
    pub fn owns(&self, attr: &Attr) -> bool {
        self.is_root() || self.uid == attr.uid
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
