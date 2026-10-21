//! System V IPC (svipc(7)): the objects of an IPC namespace. Shared
//! memory ([`shm`]) and semaphore sets ([`sem`]); message queues are not
//! implemented.
//!
//! What every kind of object shares lives here: the key and flag
//! constants, who asks ([`Who`]) and the access rule ([`permits`]).

pub mod sem;
pub mod shm;

use crate::cred::{Creds, cap};
use crate::ns::user::{UserNsRef, ns_capable};
use alloc::sync::Arc;

/// The private key (`linux/ipc.h`).
pub const IPC_PRIVATE: i32 = 0;
/// The flags of the get calls (`linux/ipc.h`).
pub const IPC_CREAT: u32 = 0o1000;
pub const IPC_EXCL: u32 = 0o2000;
pub const IPC_NOWAIT: u32 = 0o4000;
/// The control commands every kind shares (`linux/ipc.h`).
pub const IPC_RMID: u32 = 0;
pub const IPC_SET: u32 = 1;
pub const IPC_STAT: u32 = 2;
/// The "new layout" bit some C libraries add to the command; x86-64 has
/// only the 64-bit layout (`linux/ipc.h`).
pub const IPC_64: u32 = 0x0100;

/// The permission bits of a mode.
pub const MODE_BITS: u32 = 0o777;
/// Access requests as [`permits`] takes them (any class will do).
pub const READ: u32 = 4;
pub const WRITE: u32 = 2;
pub const EXEC: u32 = 1;

/// Who asks: the ids and privileges the permission rules of svipc(7) use.
#[derive(Clone, Debug)]
pub struct Who {
    pub euid: u32,
    pub egid: u32,
    pub groups: Arc<[u32]>,
    /// `CAP_IPC_OWNER` over the namespace: every access check passes.
    pub ipc_owner: bool,
    /// `CAP_SYS_ADMIN` over the namespace: `IPC_SET` and `IPC_RMID` of
    /// anyone's object.
    pub sys_admin: bool,
    /// `CAP_IPC_LOCK`: `SHM_HUGETLB`.
    pub ipc_lock: bool,
}

impl Who {
    /// The caller with `creds` in a namespace owned by `owner`.
    pub fn new(creds: &Creds, owner: &UserNsRef) -> Who {
        Who {
            euid: creds.euid,
            egid: creds.egid,
            groups: creds.groups.clone(),
            ipc_owner: ns_capable(creds, owner, cap::IPC_OWNER),
            sys_admin: ns_capable(creds, owner, cap::SYS_ADMIN),
            ipc_lock: creds.capable(cap::IPC_LOCK),
        }
    }
}

/// The ids of an object: owner and creator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owners {
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
}

impl Owners {
    /// Whether `who` may change or remove the object (`IPC_SET`,
    /// `IPC_RMID`): its owner, its creator, or a caller with
    /// `CAP_SYS_ADMIN`.
    pub fn owned_by(&self, who: &Who) -> bool {
        who.euid == self.uid || who.euid == self.cuid || who.sys_admin
    }
}

/// Whether `who` may have the access in `request` (permission bits as in
/// a mode, of any class) to an object with these owners and `mode`: the
/// r, w and x bits of the three classes are merged, and each must be in
/// the mode bits of the caller's class (owner if its effective uid is the
/// owner's or creator's, else group if its effective or a supplementary
/// gid is the owner's or creator's group, else other). `CAP_IPC_OWNER`
/// passes everything; a request of nothing always passes.
pub fn permits(owners: &Owners, mode: u16, who: &Who, request: u32) -> bool {
    let r = request & MODE_BITS;
    let wanted = (r >> 6 | r >> 3 | r) & 7;
    if wanted == 0 || who.ipc_owner {
        return true;
    }
    let mode = u32::from(mode);
    let granted = if who.euid == owners.uid || who.euid == owners.cuid {
        mode >> 6
    } else if [owners.gid, owners.cgid].iter().any(|g| *g == who.egid || who.groups.contains(g)) {
        mode >> 3
    } else {
        mode
    } & 7;
    wanted & !granted == 0
}
