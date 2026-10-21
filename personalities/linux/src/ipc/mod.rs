//! System V IPC (svipc(7)): the objects of an IPC namespace. Shared
//! memory ([`shm`]), semaphore sets ([`sem`]) and message queues
//! ([`msg`]).

pub mod msg;
pub mod sem;
pub mod shm;

use shm::Who;

/// Whether `who` may have the access in `request` (permission bits as in
/// a mode, of any class) to an object with permission bits `mode`, owned
/// by `uid`/`gid` and made by `cuid`/`cgid` (svipc(7)): the r, w and x
/// bits of the three classes are merged, and each must be in the mode
/// bits of the caller's class (owner if its effective uid is the owner's
/// or creator's, else group if its effective or a supplementary gid is
/// the owner's or creator's group, else other). `CAP_IPC_OWNER` passes
/// everything; a request of nothing always passes.
pub fn allows(mode: u16, (uid, cuid): (u32, u32), (gid, cgid): (u32, u32), who: &Who, request: u32) -> bool {
    let r = request & 0o777;
    let wanted = (r >> 6 | r >> 3 | r) & 7;
    if wanted == 0 || who.ipc_owner {
        return true;
    }
    let mode = u32::from(mode);
    let granted = if who.euid == uid || who.euid == cuid {
        mode >> 6
    } else if [gid, cgid].iter().any(|g| *g == who.egid || who.groups.contains(g)) {
        mode >> 3
    } else {
        mode
    } & 7;
    wanted & !granted == 0
}
