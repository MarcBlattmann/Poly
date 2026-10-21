//! Credentials (ADR 0045): setuid(2) and its family, the file system ids,
//! supplementary groups, capget(2) and capset(2). The rules are in
//! [`crate::cred`]; these handlers decode arguments and copy results.
//! Every change applies to the calling thread only (credentials(7): the C
//! library makes it process-wide).

use crate::args::Args;
use crate::cred::{self, CapSet, NGROUPS_MAX, optional_id};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, Target, UserPtr};
use crate::signal::deliver::with_target;
use alloc::vec::Vec;

/// `_LINUX_CAPABILITY_VERSION_1`: one 32-bit word per set (UAPI
/// `linux/capability.h`).
pub const CAPABILITY_VERSION_1: u32 = 0x1998_0330;
/// `_LINUX_CAPABILITY_VERSION_2` (deprecated) and `_3`: two words.
pub const CAPABILITY_VERSION_2: u32 = 0x2007_1026;
pub const CAPABILITY_VERSION_3: u32 = 0x2008_0522;
/// `struct __user_cap_header_struct { __u32 version; int pid; }`.
const CAP_HEADER_SIZE: usize = 8;
/// `struct __user_cap_data_struct { __u32 effective, permitted,
/// inheritable; }`.
const CAP_DATA_SIZE: usize = 12;

fn id_result(id: u32) -> SysResult {
    Ok(u64::from(id))
}

// The ids user space passes and gets are its user namespace's; the
// credentials hold kernel ids (ADR 0054): an id without a mapping is
// `EINVAL` ("not valid in this user namespace").

/// An optional user id argument as a kernel id.
fn kuid(c: &cred::Creds, raw: u32) -> Result<Option<u32>, Errno> {
    optional_id(raw).map(|u| c.kuid(u)).transpose()
}

/// An optional group id argument as a kernel id.
fn kgid(c: &cred::Creds, raw: u32) -> Result<Option<u32>, Errno> {
    optional_id(raw).map(|g| c.kgid(g)).transpose()
}

/// setuid(uid): [`cred::Creds::set_uid`].
pub fn setuid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let raw = a.uint(0);
    cred::change(k, |c| {
        let uid = if raw == cred::NO_ID { raw } else { c.kuid(raw)? };
        c.set_uid(uid)
    })
    .map(|()| 0)
}

/// setgid(gid): [`cred::Creds::set_gid`].
pub fn setgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let raw = a.uint(0);
    cred::change(k, |c| {
        let gid = if raw == cred::NO_ID { raw } else { c.kgid(raw)? };
        c.set_gid(gid)
    })
    .map(|()| 0)
}

/// setreuid(ruid, euid), `-1` for "unchanged".
pub fn setreuid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    cred::change(k, |c| {
        let (r, e) = (kuid(c, a.uint(0))?, kuid(c, a.uint(1))?);
        c.set_reuid(r, e)
    })
    .map(|()| 0)
}

/// setregid(rgid, egid).
pub fn setregid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    cred::change(k, |c| {
        let (r, e) = (kgid(c, a.uint(0))?, kgid(c, a.uint(1))?);
        c.set_regid(r, e)
    })
    .map(|()| 0)
}

/// setresuid(ruid, euid, suid).
pub fn setresuid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    cred::change(k, |c| {
        let (r, e, s) = (kuid(c, a.uint(0))?, kuid(c, a.uint(1))?, kuid(c, a.uint(2))?);
        c.set_resuid(r, e, s)
    })
    .map(|()| 0)
}

/// setresgid(rgid, egid, sgid).
pub fn setresgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    cred::change(k, |c| {
        let (r, e, s) = (kgid(c, a.uint(0))?, kgid(c, a.uint(1))?, kgid(c, a.uint(2))?);
        c.set_resgid(r, e, s)
    })
    .map(|()| 0)
}

/// Store three ids at three pointers, in order; `EFAULT` at the first
/// that cannot be written (getresuid(2)).
fn put_ids(k: &mut dyn LinuxKernel, a: &Args, ids: [u32; 3]) -> SysResult {
    for (i, id) in ids.iter().enumerate() {
        k.copy_to_user(a.ptr(i), &id.to_le_bytes())?;
    }
    Ok(0)
}

/// getresuid(ruid, euid, suid).
pub fn getresuid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let c = cred::current(k);
    put_ids(k, a, [c.uid_view(c.uid), c.uid_view(c.euid), c.uid_view(c.suid)])
}

/// getresgid(rgid, egid, sgid).
pub fn getresgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let c = cred::current(k);
    put_ids(k, a, [c.gid_view(c.gid), c.gid_view(c.egid), c.gid_view(c.sgid)])
}

/// setfsuid(fsuid): the old file system user id, whether or not it
/// changed (an id without a mapping changes nothing).
pub fn setfsuid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    id_result(cred::change(k, |c| {
        let new = c.kuid(a.uint(0)).unwrap_or(cred::NO_ID);
        let old = c.set_fsuid(new);
        c.uid_view(old)
    }))
}

/// setfsgid(fsgid).
pub fn setfsgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    id_result(cred::change(k, |c| {
        let new = c.kgid(a.uint(0)).unwrap_or(cred::NO_ID);
        let old = c.set_fsgid(new);
        c.gid_view(old)
    }))
}

/// getgroups(size, list): getgroups(2): "If size is zero, list is not
/// modified, but the total number of supplementary group IDs for the
/// process is returned"; `EINVAL` if `size` is smaller than that (or
/// negative); `EFAULT`.
pub fn getgroups(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.int(0);
    let c = cred::current(k);
    let groups: Vec<u32> = c.groups.iter().map(|&g| c.gid_view(g)).collect();
    if size < 0 {
        return Err(Errno::EINVAL);
    }
    if size == 0 {
        return Ok(groups.len() as u64);
    }
    if (size as usize) < groups.len() {
        return Err(Errno::EINVAL);
    }
    let bytes: Vec<u8> = groups.iter().flat_map(|g| g.to_le_bytes()).collect();
    k.copy_to_user(a.ptr(1), &bytes)?;
    Ok(groups.len() as u64)
}

/// setgroups(size, list): `EINVAL` for a size beyond `NGROUPS_MAX` (or
/// negative), `EPERM` without `CAP_SETGID`, `EFAULT` (in that order, to be
/// confirmed by the diff runner); [`cred::Creds::set_groups`].
pub fn setgroups(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.int(0);
    if size < 0 || size as usize > NGROUPS_MAX {
        return Err(Errno::EINVAL);
    }
    let c = cred::current(k);
    // user_namespaces(7): not until gid_map was written, never after
    // "deny" went to /proc/[pid]/setgroups.
    if !c.capable(cred::cap::SETGID) || !crate::ns::user::setgroups_allowed(&c.user_ns) {
        return Err(Errno::EPERM);
    }
    let mut bytes = alloc::vec![0u8; size as usize * 4];
    k.copy_from_user(a.ptr(1), &mut bytes)?;
    let groups = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .map(|g| if g == cred::NO_ID { Ok(g) } else { c.kgid(g) })
        .collect::<Result<Vec<u32>, Errno>>()?;
    cred::change(k, |c| c.set_groups(&groups)).map(|()| 0)
}

/// The capability header at `hdrp`: its version's word count and the pid.
/// capget(2): an unknown version makes the call fail with `EINVAL` after
/// "the kernel fill[s] hdrp->version with" its preferred version
/// ([`CAPABILITY_VERSION_3`]), which is how callers probe.
fn cap_header(k: &mut dyn LinuxKernel, hdrp: UserPtr) -> Result<(usize, i32), Errno> {
    let mut h = [0u8; CAP_HEADER_SIZE];
    k.copy_from_user(hdrp, &mut h)?;
    let version = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let pid = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    let words = match version {
        CAPABILITY_VERSION_1 => 1,
        CAPABILITY_VERSION_2 | CAPABILITY_VERSION_3 => 2,
        _ => {
            k.copy_to_user(hdrp, &CAPABILITY_VERSION_3.to_le_bytes())?;
            return Err(Errno::EINVAL);
        }
    };
    Ok((words, pid))
}

/// capget(hdrp, datap): the effective, permitted and inheritable sets of
/// thread `pid` (0: the caller) as one or two `__user_cap_data_struct`s
/// (low word first). `EINVAL` for a bad version (see [`cap_header`]) or a
/// negative pid, `ESRCH` for no such thread, `EFAULT`. A null `datap` with
/// a good version only checks the header (to be confirmed by the diff
/// runner).
pub fn capget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (words, pid) = match cap_header(k, a.ptr(0)) {
        // The probe for the preferred version: with a null `datap` it
        // succeeds once the version is written back (capget's spec sheet).
        Err(Errno::EINVAL) if a.ptr(1).is_null() => return Ok(0),
        r => r?,
    };
    if pid < 0 {
        return Err(Errno::EINVAL);
    }
    let caps = if pid == 0 || pid == crate::ns::pid::view(k, k.tid()) {
        cred::current(k).caps
    } else {
        let pid = crate::ns::pid::find(k, pid)?;
        with_target(k, Target::Thread(pid), |p, _| p.has_thread(pid).then(|| p.thread(pid).creds.caps))?
            .ok_or(Errno::ESRCH)?
    };
    if a.ptr(1).is_null() {
        return Ok(0);
    }
    let (e, p, i) = (caps.effective.words(), caps.permitted.words(), caps.inheritable.words());
    let mut out = Vec::with_capacity(words * CAP_DATA_SIZE);
    for w in 0..words {
        let pick = |(lo, hi): (u32, u32)| if w == 0 { lo } else { hi };
        out.extend_from_slice(&pick(e).to_le_bytes());
        out.extend_from_slice(&pick(p).to_le_bytes());
        out.extend_from_slice(&pick(i).to_le_bytes());
    }
    k.copy_to_user(a.ptr(1), &out)?;
    Ok(0)
}

/// capset(hdrp, datap): set the caller's sets. `pid` must be 0 or the
/// caller's thread id (capset(2): "EPERM The caller attempted to use
/// capset() to modify the capabilities of a thread other than itself"),
/// then [`cred::Creds::set_caps`]. With version 1 the high words are 0;
/// bits of capabilities that do not exist are ignored.
pub fn capset(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (words, pid) = cap_header(k, a.ptr(0))?;
    if pid != 0 && pid != crate::ns::pid::view(k, k.tid()) {
        return Err(Errno::EPERM);
    }
    let mut data = [0u8; 2 * CAP_DATA_SIZE];
    k.copy_from_user(a.ptr(1), &mut data[..words * CAP_DATA_SIZE])?;
    let word = |i: usize| u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
    // Words: effective, permitted, inheritable of the low half, then the
    // same of the high half.
    let set = |field: usize| CapSet::from_words(word(field), word(3 + field));
    let (effective, permitted, inheritable) = (set(0), set(1), set(2));
    cred::change(k, |c| c.set_caps(effective, permitted, inheritable)).map(|()| 0)
}
