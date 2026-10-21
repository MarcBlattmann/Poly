//! Extended attributes (xattr(7)): name–value pairs attached to an inode,
//! beside its data.
//!
//! A name is a namespace prefix and a rest: `user.`, `trusted.`,
//! `security.` or `system.` ([`Namespace`]; UAPI `linux/xattr.h`); any
//! other prefix is `EOPNOTSUPP` ("the namespace prefix of name is not
//! valid"). The VFS ([`NodeRef::getxattr`] and friends) checks names and
//! permissions and leaves storage to the file system
//! ([`FileSystem::getxattr`](crate::FileSystem::getxattr) and friends,
//! `EOPNOTSUPP` by default: a file system that keeps none). [`Xattrs`] is
//! a store a file system can keep per inode (tmpfs does).
//!
//! Who may do what (xattr(7), "Extended attribute namespaces"):
//!
//! | Namespace | Read | Change (set, remove) |
//! |---|---|---|
//! | `user.` | read permission on the file; only regular files and directories have them (`ENODATA` for others) | write permission (`EACCES`); `EPERM` on other kinds of files, and on a sticky directory for a caller who neither owns it nor has `CAP_FOWNER` |
//! | `trusted.` | `CAP_SYS_ADMIN` (without it the names are hidden: `ENODATA`, left out of lists) | `CAP_SYS_ADMIN` (`EPERM`) |
//! | `security.` | everyone (no security module decides otherwise) | `CAP_SYS_ADMIN` (`EPERM`; Poly has no security module to ask) |
//! | `system.posix_acl_access`, `system.posix_acl_default` | everyone | the owner or `CAP_FOWNER` (`EPERM`) |
//!
//! **POSIX ACLs** (acl(5)) are the two `system.` names. Poly enforces no
//! access control lists beyond the permission bits, so it accepts exactly
//! the ACLs the permission bits express: a *minimal* access ACL (the
//! owner, owning group and other entries only), whose setting changes the
//! file's permission bits as chmod(2) would and is not stored; reading
//! `system.posix_acl_access` back is then `ENODATA`, as for a file whose
//! ACL is its mode. Extended access ACLs (named users or groups, a mask)
//! and default ACLs (inheritance by new files) are `EOPNOTSUPP` rather
//! than stored and silently ignored. A file system that holds ACLs Poly
//! did not write (an ext4 volume made elsewhere) returns them when read.
//! Other `system.` names are `EOPNOTSUPP`.
//!
//! Sizes (UAPI `linux/limits.h`): names of 1 to [`XATTR_NAME_MAX`] bytes
//! (`ERANGE` otherwise), values of at most [`XATTR_SIZE_MAX`] bytes;
//! changes on a read-only mount are `EROFS`.

use crate::cred::{Access, Cred, cap};
use crate::node::NodeRef;
use crate::{Errno, FileType, Result, S_ISVTX, SetAttr};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// The longest attribute name (`XATTR_NAME_MAX`).
pub const XATTR_NAME_MAX: usize = 255;
/// The largest attribute value (`XATTR_SIZE_MAX`).
pub const XATTR_SIZE_MAX: usize = 65536;
/// The largest name list listxattr(2) returns (`XATTR_LIST_MAX`).
pub const XATTR_LIST_MAX: usize = 65536;

/// setxattr(2)'s flags (UAPI `linux/xattr.h`).
pub const XATTR_CREATE: u32 = 0x1;
pub const XATTR_REPLACE: u32 = 0x2;

/// The ACL attribute names (`linux/xattr.h`).
pub const POSIX_ACL_ACCESS: &[u8] = b"system.posix_acl_access";
pub const POSIX_ACL_DEFAULT: &[u8] = b"system.posix_acl_default";

/// The namespace of an attribute name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    User,
    Trusted,
    Security,
    System,
}

impl Namespace {
    /// The prefix, with its dot.
    pub const fn prefix(self) -> &'static [u8] {
        match self {
            Namespace::User => b"user.",
            Namespace::Trusted => b"trusted.",
            Namespace::Security => b"security.",
            Namespace::System => b"system.",
        }
    }

    /// The namespace of `name`: `ERANGE` for an empty or too long name,
    /// `EOPNOTSUPP` for an unknown prefix, `EINVAL` for a prefix with
    /// nothing after it.
    pub fn of(name: &[u8]) -> Result<Namespace> {
        if name.is_empty() || name.len() > XATTR_NAME_MAX {
            return Err(Errno::ERANGE);
        }
        for ns in [Namespace::User, Namespace::Trusted, Namespace::Security, Namespace::System] {
            if let Some(rest) = name.strip_prefix(ns.prefix()) {
                return if rest.is_empty() { Err(Errno::EINVAL) } else { Ok(ns) };
            }
        }
        Err(Errno::EOPNOTSUPP)
    }
}

/// How setxattr(2) may treat an existing (or missing) attribute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetMode {
    /// `XATTR_CREATE`: fail with `EEXIST` if it exists.
    pub create: bool,
    /// `XATTR_REPLACE`: fail with `ENODATA` if it does not.
    pub replace: bool,
}

impl SetMode {
    /// From setxattr(2)'s `flags`; other bits are `EINVAL`.
    pub fn from_linux(flags: u32) -> Result<SetMode> {
        if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 {
            return Err(Errno::EINVAL);
        }
        Ok(SetMode { create: flags & XATTR_CREATE != 0, replace: flags & XATTR_REPLACE != 0 })
    }

    /// `EEXIST` / `ENODATA` as the flags ask, for an attribute that does
    /// (`exists`) or does not exist.
    pub fn check(self, exists: bool) -> Result<()> {
        if exists && self.create {
            Err(Errno::EEXIST)
        } else if !exists && self.replace {
            Err(Errno::ENODATA)
        } else {
            Ok(())
        }
    }
}

/// One inode's attributes, by full name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Xattrs {
    map: BTreeMap<Box<[u8]>, Box<[u8]>>,
}

impl Xattrs {
    pub const fn new() -> Xattrs {
        Xattrs { map: BTreeMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The value of `name`; `ENODATA` if there is none.
    pub fn get(&self, name: &[u8]) -> Result<&[u8]> {
        self.map.get(name).map(|v| &**v).ok_or(Errno::ENODATA)
    }

    pub fn set(&mut self, name: &[u8], value: &[u8], how: SetMode) -> Result<()> {
        how.check(self.map.contains_key(name))?;
        self.map.insert(name.into(), value.into());
        Ok(())
    }

    /// Removes `name`; `ENODATA` if there is none.
    pub fn remove(&mut self, name: &[u8]) -> Result<()> {
        self.map.remove(name).map(|_| ()).ok_or(Errno::ENODATA)
    }

    /// The names, each followed by a NUL, in byte order.
    pub fn list(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for name in self.map.keys() {
            out.extend_from_slice(name);
            out.push(0);
        }
        out
    }

    /// Iterates over `(name, value)` in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.map.iter().map(|(k, v)| (&**k, &**v))
    }
}

/// POSIX ACLs in their extended attribute form (UAPI
/// `linux/posix_acl_xattr.h`, tags and permissions from
/// `linux/posix_acl.h`): a 4-byte header with version 2, then 8-byte
/// entries of tag (16 bits), permissions (16 bits) and id (32 bits), all
/// little-endian.
pub mod acl {
    use crate::{Errno, Result};

    pub const VERSION: u32 = 2;
    pub const HEADER: usize = 4;
    pub const ENTRY: usize = 8;

    pub const USER_OBJ: u16 = 0x01;
    pub const USER: u16 = 0x02;
    pub const GROUP_OBJ: u16 = 0x04;
    pub const GROUP: u16 = 0x08;
    pub const MASK: u16 = 0x10;
    pub const OTHER: u16 = 0x20;
    /// `ACL_READ | ACL_WRITE | ACL_EXECUTE`.
    pub const PERMS: u16 = 0o7;

    /// One entry.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Entry {
        pub tag: u16,
        pub perm: u16,
        pub id: u32,
    }

    /// What a valid ACL value says, as far as the permission bits go.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Summary {
        /// The permission bits (`rwxrwxrwx`) the ACL gives: owner from
        /// `USER_OBJ`, group from `MASK` if there is one, else `GROUP_OBJ`,
        /// others from `OTHER`.
        pub perm: u16,
        /// Only `USER_OBJ`, `GROUP_OBJ` and `OTHER`: the permission bits
        /// say everything.
        pub minimal: bool,
    }

    /// Check an ACL value (acl(5), "VALID ACLs"): version 2, a whole
    /// number of entries, exactly one each of `USER_OBJ`, `GROUP_OBJ` and
    /// `OTHER`, at most one `MASK`, which is required once there are named
    /// users or groups, entries in tag order and named ones by id without
    /// duplicates, no unknown tags or permission bits. `EINVAL` otherwise.
    pub fn parse(value: &[u8]) -> Result<Summary> {
        if value.len() < HEADER || (value.len() - HEADER) % ENTRY != 0 {
            return Err(Errno::EINVAL);
        }
        let version = u32::from_le_bytes([value[0], value[1], value[2], value[3]]);
        if version != VERSION {
            return Err(Errno::EOPNOTSUPP);
        }
        let mut last: Option<Entry> = None;
        let (mut user_obj, mut group_obj, mut other, mut mask) = (None, None, None, None);
        let mut named = false;
        for raw in value[HEADER..].chunks_exact(ENTRY) {
            let e = Entry {
                tag: u16::from_le_bytes([raw[0], raw[1]]),
                perm: u16::from_le_bytes([raw[2], raw[3]]),
                id: u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
            };
            if e.perm & !PERMS != 0 {
                return Err(Errno::EINVAL);
            }
            if let Some(p) = last {
                let ordered = p.tag < e.tag || ((e.tag == USER || e.tag == GROUP) && p.tag == e.tag && p.id < e.id);
                if !ordered {
                    return Err(Errno::EINVAL);
                }
            }
            match e.tag {
                USER_OBJ => user_obj = Some(e.perm),
                GROUP_OBJ => group_obj = Some(e.perm),
                OTHER => other = Some(e.perm),
                MASK => mask = Some(e.perm),
                USER | GROUP => named = true,
                _ => return Err(Errno::EINVAL),
            }
            last = Some(e);
        }
        let (Some(u), Some(g), Some(o)) = (user_obj, group_obj, other) else {
            return Err(Errno::EINVAL);
        };
        if named && mask.is_none() {
            return Err(Errno::EINVAL);
        }
        let group = mask.unwrap_or(g);
        Ok(Summary { perm: u << 6 | group << 3 | o, minimal: !named && mask.is_none() })
    }

    /// The minimal ACL value for permission bits `perm`.
    pub fn from_mode(perm: u16) -> [u8; HEADER + 3 * ENTRY] {
        let mut out = [0u8; HEADER + 3 * ENTRY];
        out[..4].copy_from_slice(&VERSION.to_le_bytes());
        for (i, (tag, bits)) in [(USER_OBJ, perm >> 6), (GROUP_OBJ, perm >> 3), (OTHER, perm)].iter().enumerate() {
            let at = HEADER + i * ENTRY;
            out[at..at + 2].copy_from_slice(&tag.to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&(bits & PERMS).to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        out
    }
}

/// Whether `name` is one of the two ACL names.
fn is_acl(name: &[u8]) -> bool {
    name == POSIX_ACL_ACCESS || name == POSIX_ACL_DEFAULT
}

impl NodeRef {
    /// getxattr(2): the value of `name` (see the module docs for who may).
    pub fn getxattr(&self, cred: &Cred, name: &[u8]) -> Result<Vec<u8>> {
        let ns = Namespace::of(name)?;
        let attr = self.attr()?;
        match ns {
            Namespace::User => {
                if !matches!(attr.kind, FileType::Regular | FileType::Directory) {
                    return Err(Errno::ENODATA);
                }
                cred.check(&attr, Access::READ)?;
            }
            Namespace::Trusted if !cred.has(cap::SYS_ADMIN) => return Err(Errno::ENODATA),
            Namespace::System if !is_acl(name) => return Err(Errno::EOPNOTSUPP),
            _ => {}
        }
        self.fs().getxattr(self.ino(), name)
    }

    /// listxattr(2): the names `cred` may see, each followed by a NUL.
    pub fn listxattr(&self, cred: &Cred) -> Result<Vec<u8>> {
        let all = self.fs().listxattr(self.ino())?;
        if cred.has(cap::SYS_ADMIN) {
            return Ok(all);
        }
        let mut out = Vec::with_capacity(all.len());
        for name in all.split_inclusive(|&b| b == 0) {
            if !name.starts_with(Namespace::Trusted.prefix()) {
                out.extend_from_slice(name);
            }
        }
        Ok(out)
    }

    /// setxattr(2) with its `flags` (`XATTR_CREATE`, `XATTR_REPLACE`).
    pub fn setxattr(&self, cred: &Cred, name: &[u8], value: &[u8], flags: u32) -> Result<()> {
        let how = SetMode::from_linux(flags)?;
        let ns = Namespace::of(name)?;
        if value.len() > XATTR_SIZE_MAX {
            return Err(Errno::E2BIG);
        }
        self.may_change(cred, ns, name)?;
        if ns == Namespace::System {
            return self.set_acl(cred, name, value, how);
        }
        self.fs().setxattr(self.ino(), name, value, how)
    }

    /// removexattr(2).
    pub fn removexattr(&self, cred: &Cred, name: &[u8]) -> Result<()> {
        let ns = Namespace::of(name)?;
        self.may_change(cred, ns, name)?;
        self.fs().removexattr(self.ino(), name)
    }

    /// The checks before a change of attribute `name` in `ns`.
    fn may_change(&self, cred: &Cred, ns: Namespace, name: &[u8]) -> Result<()> {
        let attr = self.attr()?;
        if self.mount().read_only_for(self.ino()) {
            return Err(Errno::EROFS);
        }
        match ns {
            Namespace::User => {
                if !matches!(attr.kind, FileType::Regular | FileType::Directory) {
                    return Err(Errno::EPERM);
                }
                let sticky_dir = attr.kind == FileType::Directory && attr.perm & S_ISVTX != 0;
                if sticky_dir && !cred.owns(&attr) {
                    return Err(Errno::EPERM);
                }
                cred.check(&attr, Access::WRITE)
            }
            Namespace::Trusted | Namespace::Security if !cred.has(cap::SYS_ADMIN) => Err(Errno::EPERM),
            Namespace::Trusted | Namespace::Security => Ok(()),
            Namespace::System if !is_acl(name) => Err(Errno::EOPNOTSUPP),
            Namespace::System if !cred.owns(&attr) => Err(Errno::EPERM),
            Namespace::System => Ok(()),
        }
    }

    /// Setting an ACL: a minimal access ACL becomes the permission bits
    /// (and any stored access ACL goes); everything else is `EOPNOTSUPP`.
    fn set_acl(&self, cred: &Cred, name: &[u8], value: &[u8], how: SetMode) -> Result<()> {
        let fs = self.fs();
        // A file system without extended attributes has no ACLs either.
        fs.listxattr(self.ino())?;
        let stored = match fs.getxattr(self.ino(), name) {
            Ok(_) => true,
            Err(Errno::ENODATA) => false,
            Err(e) => return Err(e),
        };
        how.check(stored)?;
        let acl = acl::parse(value)?;
        if name != POSIX_ACL_ACCESS || !acl.minimal {
            return Err(Errno::EOPNOTSUPP);
        }
        if stored {
            fs.removexattr(self.ino(), name)?;
        }
        let attr = self.attr()?;
        let mut perm = (attr.perm & !0o777) | acl.perm;
        // As chmod(2): the set-group-ID bit goes for a caller outside the
        // file's group without CAP_FSETID.
        if !cred.has(cap::FSETID) && !cred.in_group(attr.gid) {
            perm &= !crate::S_ISGID;
        }
        fs.setattr(self.ino(), &SetAttr { perm: Some(perm), ..SetAttr::default() }).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_and_name_sizes() {
        assert_eq!(Namespace::of(b"user.x"), Ok(Namespace::User));
        assert_eq!(Namespace::of(b"trusted.overlay.opaque"), Ok(Namespace::Trusted));
        assert_eq!(Namespace::of(b"security.selinux"), Ok(Namespace::Security));
        assert_eq!(Namespace::of(b"system.posix_acl_access"), Ok(Namespace::System));
        assert_eq!(Namespace::of(b"os2.x"), Err(Errno::EOPNOTSUPP));
        assert_eq!(Namespace::of(b"userx"), Err(Errno::EOPNOTSUPP));
        assert_eq!(Namespace::of(b"user."), Err(Errno::EINVAL));
        assert_eq!(Namespace::of(b""), Err(Errno::ERANGE));
        let mut long = b"user.".to_vec();
        long.resize(XATTR_NAME_MAX, b'a');
        assert_eq!(Namespace::of(&long), Ok(Namespace::User));
        long.push(b'a');
        assert_eq!(Namespace::of(&long), Err(Errno::ERANGE));
    }

    #[test]
    fn set_modes() {
        assert_eq!(SetMode::from_linux(4), Err(Errno::EINVAL));
        let mut x = Xattrs::new();
        assert_eq!(x.set(b"user.a", b"1", SetMode::from_linux(XATTR_REPLACE).unwrap()), Err(Errno::ENODATA));
        assert_eq!(x.set(b"user.a", b"1", SetMode::from_linux(XATTR_CREATE).unwrap()), Ok(()));
        assert_eq!(x.set(b"user.a", b"2", SetMode::from_linux(XATTR_CREATE).unwrap()), Err(Errno::EEXIST));
        assert_eq!(x.set(b"user.a", b"3", SetMode::from_linux(XATTR_REPLACE).unwrap()), Ok(()));
        assert_eq!(x.get(b"user.a"), Ok(&b"3"[..]));
        // Both flags: always fails, one way or the other.
        let both = SetMode::from_linux(XATTR_CREATE | XATTR_REPLACE).unwrap();
        assert_eq!(x.set(b"user.a", b"4", both), Err(Errno::EEXIST));
        assert_eq!(x.set(b"user.b", b"4", both), Err(Errno::ENODATA));
        x.set(b"security.z", b"", SetMode::default()).unwrap();
        assert_eq!(x.list(), b"security.z\0user.a\0");
        assert_eq!(x.get(b"security.z"), Ok(&b""[..]));
        assert_eq!(x.remove(b"user.a"), Ok(()));
        assert_eq!(x.remove(b"user.a"), Err(Errno::ENODATA));
        assert_eq!(x.get(b"user.a"), Err(Errno::ENODATA));
    }

    fn acl_value(entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut v = acl::VERSION.to_le_bytes().to_vec();
        for &(tag, perm, id) in entries {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&perm.to_le_bytes());
            v.extend_from_slice(&id.to_le_bytes());
        }
        v
    }

    #[test]
    fn acls() {
        use acl::*;
        let minimal = from_mode(0o640);
        assert_eq!(minimal.len(), 28);
        assert_eq!(parse(&minimal), Ok(Summary { perm: 0o640, minimal: true }));
        let named = acl_value(&[(USER_OBJ, 7, !0), (USER, 5, 1000), (GROUP_OBJ, 5, !0), (MASK, 1, !0), (OTHER, 0, !0)]);
        assert_eq!(parse(&named), Ok(Summary { perm: 0o710, minimal: false }));
        // A mask without named entries is not minimal either.
        let masked = acl_value(&[(USER_OBJ, 7, !0), (GROUP_OBJ, 5, !0), (MASK, 4, !0), (OTHER, 0, !0)]);
        assert_eq!(parse(&masked), Ok(Summary { perm: 0o740, minimal: false }));
        // Invalid ones.
        let no_mask = acl_value(&[(USER_OBJ, 7, !0), (USER, 5, 1000), (GROUP_OBJ, 5, !0), (OTHER, 0, !0)]);
        assert_eq!(parse(&no_mask), Err(Errno::EINVAL));
        let unordered = acl_value(&[(GROUP_OBJ, 5, !0), (USER_OBJ, 7, !0), (OTHER, 0, !0)]);
        assert_eq!(parse(&unordered), Err(Errno::EINVAL));
        let no_other = acl_value(&[(USER_OBJ, 7, !0), (GROUP_OBJ, 5, !0)]);
        assert_eq!(parse(&no_other), Err(Errno::EINVAL));
        let dup_user = acl_value(&[
            (USER_OBJ, 7, !0),
            (USER, 5, 1000),
            (USER, 5, 1000),
            (GROUP_OBJ, 5, !0),
            (MASK, 7, !0),
            (OTHER, 0, !0),
        ]);
        assert_eq!(parse(&dup_user), Err(Errno::EINVAL));
        let bad_perm = acl_value(&[(USER_OBJ, 8, !0), (GROUP_OBJ, 5, !0), (OTHER, 0, !0)]);
        assert_eq!(parse(&bad_perm), Err(Errno::EINVAL));
        assert_eq!(parse(&minimal[..27]), Err(Errno::EINVAL));
        assert_eq!(parse(&[]), Err(Errno::EINVAL));
        let mut v1 = minimal;
        v1[0] = 1;
        assert_eq!(parse(&v1), Err(Errno::EOPNOTSUPP));
    }
}
