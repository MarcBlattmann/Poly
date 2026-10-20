//! Resource limits and system information: prlimit64(2), getrlimit(2),
//! setrlimit(2), sysinfo(2).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::rlimit::{RLIM_NLIMITS, RLIMIT_NOFILE, RLIMIT_SIGPENDING};
use crate::kernel::{LinuxKernel, Target, UserPtr};
use crate::rlimit::Rlimit;

/// The ceiling of `RLIMIT_NOFILE`: `/proc/sys/fs/nr_open`, whose default
/// is 1048576 (proc_sys_fs(5)).
pub const NR_OPEN: u64 = 1 << 20;

/// getrlimit(2) and friends for process `pid` (0: the caller), in the
/// order: the resource (`EINVAL`), the new limit read from user memory
/// (`EFAULT`) and checked (`EINVAL` for a soft limit above the hard one,
/// `EPERM` for `RLIMIT_NOFILE` above [`NR_OPEN`]), the process (`ESRCH`)
/// and the permission (`EPERM`: raising a hard limit, or touching another
/// user's process, needs privilege, which only root has until
/// capabilities exist); then the change, and the old limit written out
/// (`EFAULT`, after the change, which stands).
fn prlimit(k: &mut dyn LinuxKernel, pid: i32, resource: u32, new: UserPtr, old: UserPtr) -> SysResult {
    if resource >= RLIM_NLIMITS {
        return Err(Errno::EINVAL);
    }
    let new = if new.is_null() {
        None
    } else {
        let mut b = [0u8; Rlimit::SIZE];
        k.copy_from_user(new, &mut b)?;
        let n = Rlimit::from_bytes(&b);
        if n.cur > n.max {
            return Err(Errno::EINVAL);
        }
        if resource == RLIMIT_NOFILE && n.max > NR_OPEN {
            return Err(Errno::EPERM);
        }
        Some(n)
    };
    let target = match pid {
        0 => Target::Current,
        p if p > 0 => Target::Process(p),
        _ => return Err(Errno::ESRCH),
    };
    // CAP_SYS_RESOURCE, and the ids, are the calling thread's (ADR 0045).
    let me = crate::cred::current(k);
    // Raising a hard limit takes the machine's resources: CAP_SYS_RESOURCE
    // in the initial user namespace (ADR 0054).
    let privileged = me.capable_initial(crate::cred::cap::SYS_RESOURCE);
    let own = pid == 0 || pid == k.pid();
    let mut result: Result<Rlimit, Errno> = Err(Errno::ESRCH);
    k.with_process(target, &mut |p, info| {
        let current = p.limits.get(resource).unwrap_or(Rlimit::UNLIMITED);
        // prlimit(2): without CAP_SYS_RESOURCE, the caller's real,
        // effective and saved user ids must match the target's real
        // user id (and so for groups).
        let ids = p
            .process_creds(info.pid)
            .map_or(([info.uid, info.euid, info.suid], [info.gid, info.egid, info.sgid]), |c| {
                ([c.uid, c.euid, c.suid], [c.gid, c.egid, c.sgid])
            });
        let same_user = ids.0.iter().all(|&u| u == me.uid) && ids.1.iter().all(|&g| g == me.gid);
        if !own && !privileged && !same_user {
            result = Err(Errno::EPERM);
            return;
        }
        if let Some(n) = new {
            if n.max > current.max && !privileged {
                result = Err(Errno::EPERM);
                return;
            }
            p.limits.set(resource, n);
            if resource == RLIMIT_SIGPENDING {
                p.queue_limit = usize::try_from(n.cur).unwrap_or(usize::MAX);
            }
        }
        result = Ok(current);
    })?;
    let current = result?;
    if let Some(n) = new
        && resource == RLIMIT_NOFILE
        && let Some(fs) = k.fs()
    {
        fs.set_fd_limit(target, n.cur)?;
    }
    if !old.is_null() {
        k.copy_to_user(old, &current.to_bytes())?;
    }
    Ok(0)
}

/// prlimit64(pid, resource, new_limit, old_limit)
pub fn prlimit64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    prlimit(k, a.int(0), a.uint(1), a.ptr(2), a.ptr(3))
}

/// getrlimit(resource, rlim): x86-64's `struct rlimit` is `struct
/// rlimit64` (two unsigned longs). "EFAULT A pointer argument points to a
/// location outside the accessible address space" covers a NULL `rlim`.
pub fn getrlimit(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ptr(1).is_null() {
        return if a.uint(0) >= RLIM_NLIMITS { Err(Errno::EINVAL) } else { Err(Errno::EFAULT) };
    }
    prlimit(k, 0, a.uint(0), UserPtr::NULL, a.ptr(1))
}

/// setrlimit(resource, rlim)
pub fn setrlimit(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ptr(1).is_null() {
        return if a.uint(0) >= RLIM_NLIMITS { Err(Errno::EINVAL) } else { Err(Errno::EFAULT) };
    }
    prlimit(k, 0, a.uint(0), a.ptr(1), UserPtr::NULL)
}

/// sysinfo(info)
pub fn sysinfo(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let info = k.system_info()?;
    k.copy_to_user(a.ptr(0), &info.to_bytes())?;
    Ok(0)
}
