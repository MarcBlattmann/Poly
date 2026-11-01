//! unshare(2) and setns(2): leaving and joining namespaces (ADR 0054,
//! ADR 0121). The rules are in [`crate::ns`]; clone(2)'s namespace flags
//! are made by the kernel's `clone_task` with the same [`ns::make`].

use crate::admin::SystemName;
use crate::args::Args;
use crate::cred::Creds;
use crate::errno::{Errno, SysResult};
use crate::fileio::PidFdTarget;
use crate::kernel::{LinuxKernel, Target};
use crate::ns::{self, Handle, Join, Namespaces, Sharing, UtsNames};
use crate::signal::deliver::{with_own, with_target};
use crate::table::{Handler, nr};
use crate::task::clone::{
    CLONE_FILES, CLONE_NEWCGROUP, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWTIME, CLONE_NEWUSER,
    CLONE_NEWUTS, CLONE_SYSVSEM,
};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The calls of this module, registered with the table's groups.
pub const CALLS: &[(u64, Handler)] = &[(nr::UNSHARE, unshare), (nr::SETNS, setns)];

/// The host and domain names of the initial UTS namespace, which a new
/// UTS namespace made from it copies.
pub fn initial_names(k: &mut dyn LinuxKernel) -> UtsNames {
    match k.admin() {
        Some(a) => {
            UtsNames { hostname: a.system_name(SystemName::Host), domainname: a.system_name(SystemName::Domain) }
        }
        None => UtsNames { hostname: k.hostname().to_vec(), domainname: k.domainname().to_vec() },
    }
}

/// unshare(flags): see [`ns::check_unshare`] for the flags,
/// [`ns::check_privilege`] for the privilege and [`ns::make`] for the
/// namespaces. The checks all come before anything changes (unshare's
/// spec sheet: "On failure nothing is unshared"): the flags, a second
/// `CLONE_NEWPID` (`EINVAL`), the privilege (`EPERM`). The calling
/// thread's credentials change with a new user namespace, the process's
/// namespaces with the others (`CLONE_NEWPID` and `CLONE_NEWTIME`: only
/// the children's, unshare(2): "the calling process ... is not moved into
/// the new namespace").
///
/// `CLONE_FILES` copies a descriptor table shared with another process;
/// `CLONE_FS` has nothing to copy (Poly's processes own their directories
/// and umask); `CLONE_SYSVSEM` leaves the semaphore undo list, which no
/// other process shares in Poly: its adjustments are applied now, as at
/// exit ([`crate::sys::sem::process_exit`]).
pub fn unshare(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.ulong(0);
    let threads = with_own(k, |p| p.thread_ids().count());
    let sharing = Sharing { threads, memory: k.memory_shared(), chrooted: k.chrooted() };
    let make = ns::check_unshare(flags, sharing)?;
    let tid = k.tid();
    let (mut namespaces, mut creds) = with_own(k, |p| (p.ns.clone(), p.thread(tid).creds.clone()));
    // unshare's spec sheet: "a second unshare(CLONE_NEWPID) before or
    // after [the first child] is EINVAL".
    if make & CLONE_NEWPID != 0 && !ns::pid::same(&namespaces.pid, &namespaces.pid_for_children) {
        return Err(Errno::EINVAL);
    }
    ns::check_privilege(&creds, make)?;
    if flags & CLONE_FILES != 0
        && let Some(fs) = k.fs()
    {
        fs.unshare_fd_table()?;
    }
    if flags & CLONE_SYSVSEM != 0 {
        crate::sys::sem::process_exit(k);
    }
    if make == 0 {
        return Ok(0);
    }
    let names = if make & CLONE_NEWUTS != 0 { initial_names(k) } else { UtsNames::default() };
    ns::make(&mut namespaces, &mut creds, make, false, names, &mut |owner| k.copy_mounts(owner))?;
    with_own(k, |p| {
        p.ns = namespaces;
        p.thread_mut(tid).creds = creds;
    });
    Ok(0)
}

/// setns(fd, nstype): join the namespace `fd` refers to (a descriptor of a
/// `/proc/[pid]/ns` file, `nstype` 0 or its `CLONE_NEW*` flag), or the
/// namespaces `nstype` names of the process a pidfd `fd` refers to, by
/// [`ns::join`]'s rules. Order (setns' spec sheet): `EBADF` (the
/// descriptor), `EINVAL` (not a namespace file or pidfd, `nstype`),
/// `ESRCH` (a reaped pidfd target), then [`ns::join`]'s.
pub fn setns(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    let nstype = u64::from(a.uint(1));
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let target = match k.namespace_of_fd(fd) {
        Ok((inum, kind)) => {
            if nstype != 0 && nstype != kind.flag() {
                return Err(Errno::EINVAL);
            }
            ns::lookup(inum, kind).ok_or(Errno::EINVAL)?
        }
        Err(Errno::EINVAL) => {
            let pid = match k.file_io().map(|io| io.pidfd_target(fd)) {
                Some(Ok(Some(PidFdTarget::PidFd { pid, .. }))) => pid,
                _ => return Err(Errno::EINVAL),
            };
            return setns_pidfd(k, pid, nstype);
        }
        Err(e) => return Err(e),
    };
    let tid = k.tid();
    let (namespaces, creds, threads) =
        with_own(k, |p| (p.ns.clone(), p.thread(tid).creds.clone(), p.thread_ids().count()));
    let chrooted = k.chrooted();
    match ns::join(&namespaces, &creds, threads, chrooted, &target, nstype)? {
        Join::User(c) => with_own(k, |p| p.thread_mut(tid).creds = c),
        Join::Namespaces(n) => with_own(k, |p| p.ns = n),
        Join::Mounts(m) => {
            k.enter_mounts(&m)?;
            with_own(k, |p| p.ns.mnt = m);
        }
    }
    Ok(0)
}

/// The namespaces of process `ns` / `user` that `flags` names, as setns(2)
/// joins them through a pidfd: the user namespace first (the others are
/// then joined with the capabilities it gives), the process's own pid and
/// time namespaces.
pub fn pidfd_targets(ns: &Namespaces, user: &ns::user::UserNsRef, flags: u64) -> Vec<Handle> {
    let mut out = Vec::new();
    let has = |f: u64| flags & f != 0;
    if has(CLONE_NEWUSER) {
        out.push(Handle::User(user.clone()));
    }
    if has(CLONE_NEWNS) {
        out.push(Handle::Mnt(ns.mnt.clone()));
    }
    if has(CLONE_NEWPID) {
        out.push(Handle::Pid(ns.pid.clone()));
    }
    if has(CLONE_NEWUTS) {
        out.push(Handle::Uts(ns.uts.clone()));
    }
    if has(CLONE_NEWIPC) {
        out.push(Handle::Ipc(ns.ipc.clone()));
    }
    if has(CLONE_NEWNET) {
        out.push(Handle::Net(ns.net.clone()));
    }
    if has(CLONE_NEWCGROUP) {
        out.push(Handle::Cgroup(ns.cgroup.clone()));
    }
    if has(CLONE_NEWTIME) {
        out.push(Handle::Time(ns.time.clone()));
    }
    out
}

/// What joining every namespace of `targets` in turn does, all or
/// nothing: the new credentials, namespaces and, if a mount namespace was
/// among them, that one.
#[allow(clippy::type_complexity)]
pub fn join_all(
    ns: &Namespaces,
    creds: &Creds,
    threads: usize,
    chrooted: bool,
    targets: &[Handle],
) -> Result<(Creds, Namespaces, Option<Option<Arc<ns::MntNs>>>), Errno> {
    let (mut c, mut n, mut mounts) = (creds.clone(), ns.clone(), None);
    for t in targets {
        match ns::join(&n, &c, threads, chrooted, t, 0)? {
            Join::User(new) => c = new,
            Join::Namespaces(new) => n = new,
            Join::Mounts(m) => {
                n.mnt = m.clone();
                mounts = Some(m);
            }
        }
    }
    Ok((c, n, mounts))
}

/// setns(pidfd, nstype): `nstype` a non-empty set of `CLONE_NEW*` flags
/// (else `EINVAL`), the target process gone `ESRCH`.
fn setns_pidfd(k: &mut dyn LinuxKernel, pid: i32, nstype: u64) -> SysResult {
    if nstype == 0 || nstype & !ns::ALL_NAMESPACES != 0 {
        return Err(Errno::EINVAL);
    }
    let (their_ns, their_user) =
        with_target(k, Target::Process(pid), |p, _| p.process_creds(pid).map(|c| (p.ns.clone(), c.user_ns.clone())))?
            .ok_or(Errno::ESRCH)?;
    let tid = k.tid();
    let (namespaces, creds, threads) =
        with_own(k, |p| (p.ns.clone(), p.thread(tid).creds.clone(), p.thread_ids().count()));
    let targets = pidfd_targets(&their_ns, &their_user, nstype);
    let (c, n, mounts) = join_all(&namespaces, &creds, threads, k.chrooted(), &targets)?;
    if let Some(m) = &mounts {
        k.enter_mounts(m)?;
    }
    with_own(k, |p| {
        p.ns = n;
        p.thread_mut(tid).creds = c;
    });
    Ok(0)
}
