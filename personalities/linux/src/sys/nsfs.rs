//! The ioctls of a namespace file (`/proc/[pid]/ns/<type>`, ioctl_nsfs(2),
//! ADR 0168): what kind of namespace it is, which user namespace owns it,
//! its parent, the owner's user id, and a mount namespace's id. Numbers
//! from UAPI `linux/nsfs.h` (`NSIO` is `0xb7`).
//!
//! The answers that are namespaces come as new descriptors; they are given
//! only for a namespace inside the caller's own (the caller's user or pid
//! namespace, or one below it): anything further out is `EPERM`, and so is
//! asking beyond the initial namespaces.

use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::ns::user::{IdKind, UserNsRef};
use crate::ns::{self, Handle, Kind};

/// `_IO(NSIO, 1)` to `_IO(NSIO, 4)` and `_IOR(NSIO, 5, __u64)`.
pub const NS_GET_USERNS: u32 = 0xb701;
pub const NS_GET_PARENT: u32 = 0xb702;
pub const NS_GET_NSTYPE: u32 = 0xb703;
pub const NS_GET_OWNER_UID: u32 = 0xb704;
pub const NS_GET_MNTNS_ID: u32 = 0x8008_b705;
/// The magic of the requests (bits 8 to 15).
const NSIO: u32 = 0xb7;

/// The ioctls of a namespace file; `None` if `fd` is not one (the caller
/// goes on with the other ioctls).
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: UserPtr) -> Option<SysResult> {
    if request >> 8 & 0xff != NSIO {
        return None;
    }
    let (inum, kind) = k.namespace_of_fd(fd).ok()?;
    Some(ns_ioctl(k, fd, inum, kind, request, arg))
}

/// The inode number of a user namespace's file.
fn user_inum(ns: &UserNsRef) -> u32 {
    ns.as_ref().map_or(ns::initial_inum(Kind::User), |n| n.inum())
}

/// Whether user namespace `ns` is the caller's or one below it.
fn user_in_scope(k: &mut dyn LinuxKernel, ns: &UserNsRef) -> bool {
    let mine = crate::cred::current(k).user_ns;
    ns::user::same(&mine, ns) || ns::user::is_ancestor(&mine, ns)
}

fn ns_ioctl(k: &mut dyn LinuxKernel, fd: i32, inum: u32, kind: Kind, request: u32, arg: UserPtr) -> SysResult {
    let handle = ns::lookup(inum, kind).ok_or(Errno::EINVAL)?;
    match request {
        NS_GET_NSTYPE => Ok(kind.flag()),
        NS_GET_USERNS => {
            // A user namespace is owned by its parent.
            let owner = match &handle {
                Handle::User(None) => return Err(Errno::EPERM),
                Handle::User(Some(n)) => n.parent().clone(),
                other => other.owner(),
            };
            if !user_in_scope(k, &owner) {
                return Err(Errno::EPERM);
            }
            k.open_namespace(fd, user_inum(&owner), Kind::User).map(|fd| fd as u64)
        }
        NS_GET_PARENT => match &handle {
            Handle::User(None) | Handle::Pid(None) => Err(Errno::EPERM),
            Handle::User(Some(n)) => {
                let parent = n.parent().clone();
                if !user_in_scope(k, &parent) {
                    return Err(Errno::EPERM);
                }
                k.open_namespace(fd, user_inum(&parent), Kind::User).map(|fd| fd as u64)
            }
            Handle::Pid(Some(n)) => {
                let parent = n.parent().clone();
                let mine = k.pid_ns();
                if !(ns::pid::same(&mine, &parent) || ns::pid::is_ancestor(&mine, &parent)) {
                    return Err(Errno::EPERM);
                }
                let inum = parent.as_ref().map_or(ns::initial_inum(Kind::Pid), |p| p.inum());
                k.open_namespace(fd, inum, Kind::Pid).map(|fd| fd as u64)
            }
            // The other kinds have no hierarchy.
            _ => Err(Errno::EINVAL),
        },
        NS_GET_OWNER_UID => {
            let Handle::User(target) = &handle else { return Err(Errno::EINVAL) };
            // The creator's user id, as the caller's user namespace names it.
            let kuid = target.as_ref().map_or(0, |n| n.owner());
            let mine = crate::cred::current(k).user_ns;
            let uid = ns::user::up_or_overflow(&mine, IdKind::Uid, kuid);
            k.copy_to_user(arg, &uid.to_le_bytes()).map(|()| 0)
        }
        NS_GET_MNTNS_ID => {
            if !matches!(handle, Handle::Mnt(_)) {
                return Err(Errno::EINVAL);
            }
            // Never 0, the same for a namespace as long as it lives, and
            // another one's is different: its inode number.
            k.copy_to_user(arg, &u64::from(inum).to_le_bytes()).map(|()| 0)
        }
        _ => Err(Errno::ENOTTY),
    }
}
