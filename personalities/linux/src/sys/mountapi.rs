//! The new mount API and pivot_root(2) (ADR 0122): open_tree(2),
//! move_mount(2), mount_setattr(2), fsopen(2), fsconfig(2), fsmount(2),
//! fspick(2), statmount(2), listmount(2), checked here (the logic is in
//! [`crate::mountapi`]) and carried out by the kernel's
//! [`LinuxAdmin`].
//!
//! Order of the checks, as the spec sheets give it: the privilege first
//! where the call needs one (`fsopen`, `fsmount`, `fspick`, `move_mount`,
//! `pivot_root`, `open_tree` with `OPEN_TREE_CLONE`; `mount_setattr`
//! after its size and structure), then flags and structures (`EINVAL`,
//! `E2BIG`, `EFAULT`), then the paths, then the kernel's checks on the
//! mounts.

use crate::admin::LinuxAdmin;
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::mountapi::uapi::*;
use crate::mountapi::{
    AttrChange, Command, FsContext, LISTMOUNT_MAX, MntIdReq, PathAt, Value, check_fsmount_attrs, encode_statmount,
    list_mounts, mount_of_unique,
};
use crate::signal::deliver::with_own;
use crate::sys::path::{nonempty_path, path_arg};
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::PIVOT_ROOT, pivot_root),
    (nr::OPEN_TREE, open_tree),
    (nr::MOVE_MOUNT, move_mount),
    (nr::FSOPEN, fsopen),
    (nr::FSCONFIG, fsconfig),
    (nr::FSMOUNT, fsmount),
    (nr::FSPICK, fspick),
    (nr::MOUNT_SETATTR, mount_setattr),
    (nr::STATMOUNT, statmount),
    (nr::LISTMOUNT, listmount),
];

/// The page size: the largest `struct mount_attr` and `fsconfig` value.
const PAGE: usize = 4096;
/// The longest `fsconfig` key, its NUL included.
const KEY_MAX: usize = 256;

fn admin(k: &mut dyn LinuxKernel) -> Result<&mut dyn LinuxAdmin, Errno> {
    k.admin().ok_or(Errno::ENOSYS)
}

/// `CAP_SYS_ADMIN` in the user namespace that owns the caller's mount
/// namespace (`EPERM`), as for mount(2).
fn require_mount(k: &mut dyn LinuxKernel) -> Result<(), Errno> {
    let creds = cred::current(k);
    let owner = with_own(k, |p| p.ns.mnt_owner());
    if crate::ns::user::ns_capable(&creds, &owner, cap::SYS_ADMIN) { Ok(()) } else { Err(Errno::EPERM) }
}

/// A path argument: empty allowed (meaning `dirfd`) when `empty` is set,
/// `ENOENT` otherwise; `EFAULT` for a bad pointer.
fn path_at(k: &dyn LinuxKernel, dirfd: i32, p: UserPtr, empty: bool, follow: bool) -> Result<PathAt, Errno> {
    let path = if empty { path_arg(k, p)? } else { nonempty_path(k, p)? };
    Ok(PathAt { dirfd, path, follow })
}

/// open_tree(dfd, path, flags)
pub fn open_tree(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(2);
    let known =
        OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC | AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_RECURSIVE;
    let clone = flags & OPEN_TREE_CLONE != 0;
    if flags & !known != 0 || (flags & AT_RECURSIVE != 0 && !clone) {
        return Err(Errno::EINVAL);
    }
    if clone {
        require_mount(k)?;
    }
    let at = path_at(k, a.fd(0), a.ptr(1), flags & AT_EMPTY_PATH != 0, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    let clone = clone.then_some(flags & AT_RECURSIVE != 0);
    admin(k)?.open_tree(&at, clone, flags & OPEN_TREE_CLOEXEC != 0).map(|fd| fd as u64)
}

/// move_mount(from_dfd, from_path, to_dfd, to_path, flags)
///
/// `MOVE_MOUNT_SET_GROUP` and `MOVE_MOUNT_BENEATH` (peer groups, mounting
/// beneath the top of a stack) are not supported: `EINVAL`.
pub fn move_mount(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_mount(k)?;
    let flags = a.uint(4);
    let known = MOVE_MOUNT_F_SYMLINKS
        | MOVE_MOUNT_F_AUTOMOUNTS
        | MOVE_MOUNT_F_EMPTY_PATH
        | MOVE_MOUNT_T_SYMLINKS
        | MOVE_MOUNT_T_AUTOMOUNTS
        | MOVE_MOUNT_T_EMPTY_PATH;
    if flags & !known != 0 {
        return Err(Errno::EINVAL);
    }
    let from = path_at(k, a.fd(0), a.ptr(1), flags & MOVE_MOUNT_F_EMPTY_PATH != 0, flags & MOVE_MOUNT_F_SYMLINKS != 0)?;
    let to = path_at(k, a.fd(2), a.ptr(3), flags & MOVE_MOUNT_T_EMPTY_PATH != 0, flags & MOVE_MOUNT_T_SYMLINKS != 0)?;
    admin(k)?.move_mount(&from, &to).map(|()| 0)
}

/// mount_setattr(dfd, path, flags, attr, size)
pub fn mount_setattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.ulong(4);
    if size < MOUNT_ATTR_SIZE_VER0 as u64 {
        return Err(Errno::EINVAL);
    }
    if size > PAGE as u64 {
        return Err(Errno::E2BIG);
    }
    let mut buf = vec![0u8; size as usize];
    k.copy_from_user(a.ptr(3), &mut buf)?;
    if buf[MOUNT_ATTR_SIZE_VER0..].iter().any(|&b| b != 0) {
        return Err(Errno::E2BIG);
    }
    let flags = a.uint(2);
    if flags & !(AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_RECURSIVE) != 0 {
        return Err(Errno::EINVAL);
    }
    let change = AttrChange::from_bytes(&buf[..MOUNT_ATTR_SIZE_VER0]);
    change.check()?;
    require_mount(k)?;
    let at = path_at(k, a.fd(0), a.ptr(1), flags & AT_EMPTY_PATH != 0, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    admin(k)?.mount_setattr(&at, flags & AT_RECURSIVE != 0, &change).map(|()| 0)
}

/// fsopen(fsname, flags)
pub fn fsopen(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_mount(k)?;
    let flags = a.uint(1);
    if flags & !FSOPEN_CLOEXEC != 0 {
        return Err(Errno::EINVAL);
    }
    let name = path_arg(k, a.ptr(0))?;
    let ctx = FsContext::new(&name)?;
    admin(k)?.fsopen(ctx, flags & FSOPEN_CLOEXEC != 0).map(|fd| fd as u64)
}

/// fspick(dfd, path, flags)
pub fn fspick(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_mount(k)?;
    let flags = a.uint(2);
    if flags & !(FSPICK_CLOEXEC | FSPICK_SYMLINK_NOFOLLOW | FSPICK_NO_AUTOMOUNT | FSPICK_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let at = path_at(k, a.fd(0), a.ptr(1), flags & FSPICK_EMPTY_PATH != 0, flags & FSPICK_SYMLINK_NOFOLLOW == 0)?;
    admin(k)?.fspick(&at, flags & FSPICK_CLOEXEC != 0).map(|fd| fd as u64)
}

/// A string argument of fsconfig of at most `max` bytes with its NUL.
fn string(k: &dyn LinuxKernel, p: UserPtr, max: usize) -> Result<Vec<u8>, Errno> {
    let mut buf = vec![0u8; max];
    let n = k.strncpy_from_user(p, &mut buf).map_err(|e| if e == Errno::ENAMETOOLONG { Errno::EINVAL } else { e })?;
    buf.truncate(n);
    Ok(buf)
}

/// fsconfig(fd, cmd, key, value, aux)
///
/// The command and its argument shapes first (`EOPNOTSUPP`, `EINVAL`),
/// then the descriptor (`EBADF`, `EINVAL` for one that is no context),
/// then the context's own rules.
pub fn fsconfig(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, cmd, key, value, aux) = (a.fd(0), a.uint(1), a.ptr(2), a.ptr(3), a.int(4));
    let (has_key, has_value) = (!key.is_null(), !value.is_null());
    let shape_ok = match cmd {
        FSCONFIG_SET_FLAG => has_key && !has_value && aux == 0,
        FSCONFIG_SET_STRING => has_key && has_value && aux == 0,
        FSCONFIG_SET_BINARY => has_key && has_value && aux > 0 && aux as usize <= 1 << 20,
        FSCONFIG_SET_PATH | FSCONFIG_SET_PATH_EMPTY => has_key && has_value,
        FSCONFIG_SET_FD => has_key && !has_value && aux >= 0,
        FSCONFIG_CMD_CREATE | FSCONFIG_CMD_RECONFIGURE | FSCONFIG_CMD_CREATE_EXCL => !has_key && !has_value && aux == 0,
        _ => return Err(Errno::EOPNOTSUPP),
    };
    if !shape_ok {
        return Err(Errno::EINVAL);
    }
    let key = if has_key { string(k, key, KEY_MAX)? } else { Vec::new() };
    let data = match cmd {
        FSCONFIG_SET_STRING | FSCONFIG_SET_PATH | FSCONFIG_SET_PATH_EMPTY => string(k, value, PAGE)?,
        FSCONFIG_SET_BINARY => {
            let mut blob = vec![0u8; aux as usize];
            k.copy_from_user(value, &mut blob)?;
            blob
        }
        _ => Vec::new(),
    };
    let command = match cmd {
        FSCONFIG_SET_FLAG => Command::Set { key: &key, value: Value::Flag },
        FSCONFIG_SET_STRING => Command::Set { key: &key, value: Value::String(&data) },
        FSCONFIG_SET_BINARY => Command::Set { key: &key, value: Value::Binary(&data) },
        FSCONFIG_SET_PATH | FSCONFIG_SET_PATH_EMPTY => Command::Set { key: &key, value: Value::Path(&data) },
        FSCONFIG_SET_FD => Command::Set { key: &key, value: Value::Fd(aux) },
        FSCONFIG_CMD_CREATE => Command::Create,
        FSCONFIG_CMD_CREATE_EXCL => Command::CreateExcl,
        _ => Command::Reconfigure,
    };
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    admin(k)?.fsconfig(fd, &command).map(|()| 0)
}

/// fsmount(fs_fd, flags, attr_flags)
pub fn fsmount(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_mount(k)?;
    let flags = a.uint(1);
    if flags & !FSMOUNT_CLOEXEC != 0 {
        return Err(Errno::EINVAL);
    }
    let attrs = u64::from(a.uint(2));
    check_fsmount_attrs(attrs)?;
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    admin(k)?.fsmount(fd, attrs, flags & FSMOUNT_CLOEXEC != 0).map(|fd| fd as u64)
}

/// The caller's `struct mnt_id_req` (`EFAULT`), unchecked.
fn mnt_id_req(k: &dyn LinuxKernel, p: UserPtr) -> Result<MntIdReq, Errno> {
    let mut b = [0u8; 32];
    k.copy_from_user(p, &mut b[..24])?;
    let req = MntIdReq::from_bytes(&b[..24]);
    if req.size == MNT_ID_REQ_SIZE_VER1 {
        k.copy_from_user(p, &mut b)?;
        return Ok(MntIdReq::from_bytes(&b));
    }
    Ok(req)
}

/// statmount(req, buf, bufsize, flags)
pub fn statmount(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let req = mnt_id_req(k, a.ptr(0))?;
    req.check()?;
    if a.uint(3) != 0 {
        return Err(Errno::EINVAL);
    }
    if req.mnt_id == LSMT_ROOT {
        return Err(Errno::ENOENT);
    }
    let id = mount_of_unique(req.mnt_id).ok_or(Errno::EINVAL)?;
    let desc = admin(k)?.describe_mount(id)?;
    let bytes = encode_statmount(&desc, req.param, a.ulong(2).min(usize::MAX as u64) as usize)?;
    k.copy_to_user(a.ptr(1), &bytes)?;
    Ok(0)
}

/// listmount(req, mnt_ids, nr_mnt_ids, flags)
pub fn listmount(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let req = mnt_id_req(k, a.ptr(0))?;
    let flags = a.uint(3);
    if flags & !LISTMOUNT_REVERSE != 0 {
        return Err(Errno::EINVAL);
    }
    req.check()?;
    let nr = a.ulong(2);
    if nr > LISTMOUNT_MAX {
        return Err(Errno::EOVERFLOW);
    }
    let links = admin(k)?.mount_links()?;
    let ids = list_mounts(&links, &req, flags & LISTMOUNT_REVERSE != 0, nr as usize)?;
    let bytes: Vec<u8> = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
    if !bytes.is_empty() {
        k.copy_to_user(a.ptr(1), &bytes)?;
    }
    Ok(ids.len() as u64)
}

/// pivot_root(new_root, put_old)
pub fn pivot_root(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_mount(k)?;
    let new_root = nonempty_path(k, a.ptr(0))?;
    let put_old = nonempty_path(k, a.ptr(1))?;
    admin(k)?.pivot_root(&new_root, &put_old).map(|()| 0)
}
